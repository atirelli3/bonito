//! The application state, its workers and the event loop.

use crate::jumps::{Jumps, Spot};
use crate::keymap::{self, Action, Keymap, Motion, View};
use crate::kuna::{self, Function, Reply};
use crate::ui;
use crate::view::Pane;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::crossterm::event::{self, Event, KeyEvent, KeyEventKind};
use ratatui::layout::Rect;
use std::collections::HashMap;
use std::error::Error;
use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::thread::{self, JoinHandle};

/// How many loaded views are kept: each can hold a hundred thousand lines.
const MAX_VIEWS: usize = 64;
/// What a mark key says outside the code pane.
const CODE_ONLY: &str = "marks are for the code pane";

/// The last search, which `n` and `N` repeat.
#[derive(Clone)]
struct Query {
    text: String,
    /// Only the whole word matches, and the case counts (`*` and `#`).
    whole: bool,
    forward: bool,
}

/// Everything the loop reacts to.
pub enum Msg {
    /// A terminal event, or the failure to read one.
    Input(io::Result<Event>),
    /// The answer to the functions request.
    Functions(Result<Reply<Vec<Function>>, kuna::Error>),
    /// The answer to a `Request`: the lines of that view of the function at `address`.
    Code {
        view: View,
        address: u64,
        answer: Result<Reply<Vec<String>>, kuna::Error>,
    },
    /// SIGTERM or SIGHUP arrived: its name.
    #[cfg(unix)]
    Signal(&'static str),
}

/// A view of the function at `address`, asked of the worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Request {
    pub view: View,
    pub address: u64,
}

/// Which pane the keys move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    List,
    Code,
}

/// The view the code pane shows.
pub struct Shown {
    pub request: Request,
    /// The function's name, for the title.
    pub name: String,
}

/// What became of a request for a view.
pub enum Loaded {
    /// Asked for, not answered yet.
    Loading,
    /// The lines with the cursor the user left in them, and what kuna said while making them.
    Ready(Pane, Option<String>),
    /// kuna failed; this says why.
    Failed(String),
}

/// How a view is called in a pane title.
pub fn view_name(view: View) -> &'static str {
    match view {
        View::Decompile => "decompile",
        View::Disassembly => "disassembly",
        View::Xrefs => "xrefs",
        View::Strings => "strings",
        View::CallGraph => "call graph",
        View::Hex => "hex",
        View::PCode => "p-code",
        View::Summary => "summary",
    }
}

/// The functions kuna found, as far as they are known.
pub enum Functions {
    /// The request is still running.
    Loading,
    /// kuna listed them, in address order.
    Ready(Vec<Function>),
    /// The request failed; the status line says why.
    Failed,
}

/// What the screen shows.
pub struct App {
    /// The binary being analysed.
    pub binary: PathBuf,
    /// The version of the kuna that is driven.
    pub kuna_version: String,
    /// The functions of the binary.
    pub functions: Functions,
    /// What kuna said while still succeeding, or why the last request failed.
    pub status: Option<String>,
    /// What the last key or command answered ("not a command: x"); the next key clears it.
    pub message: Option<String>,
    /// Set when kuna is not the version bonito was tested with; it stays for the whole session.
    pub warning: Option<String>,
    /// Turns keys into actions; the screen shows its prompt and pending keys.
    pub keymap: Keymap,
    /// The pane the keys move.
    pub focus: Focus,
    /// The function names, with the selection as its cursor.
    pub list: Pane,
    /// The view the code pane shows, if one was opened.
    pub shown: Option<Shown>,
    views: HashMap<Request, Loaded>,
    jumps: Jumps,
    /// Lower case marks belong to the view they were set in (`Some`), upper case ones to the session (`None`).
    marks: HashMap<(Option<Request>, char), Spot>,
    search: Option<Query>,
    requests: Sender<Request>,
    /// The height of the panes' text, as drawn last.
    rows: usize,
    running: bool,
}

impl App {
    /// A new app for `binary`; `warning` is shown for the whole session, and views are asked for on `requests`.
    pub fn new(
        binary: PathBuf,
        kuna_version: String,
        warning: Option<String>,
        requests: Sender<Request>,
    ) -> Self {
        Self {
            binary,
            kuna_version,
            functions: Functions::Loading,
            status: None,
            message: None,
            warning,
            keymap: Keymap::default(),
            focus: Focus::List,
            list: Pane::new(Vec::new()),
            shown: None,
            views: HashMap::new(),
            jumps: Jumps::default(),
            marks: HashMap::new(),
            search: None,
            requests,
            rows: 0,
            running: true,
        }
    }

    /// Takes the height of the panes' text; the cursors stay in view.
    pub fn set_rows(&mut self, rows: usize) {
        self.rows = rows;
        self.list.keep_visible(rows);
        if let Some(pane) = self.code_mut() {
            pane.keep_visible(rows);
        }
    }

    /// The view in the code pane and what became of it.
    pub fn current(&self) -> Option<(&Shown, &Loaded)> {
        let shown = self.shown.as_ref()?;
        Some((shown, self.views.get(&shown.request)?))
    }

    /// "decompiling main…" while the code pane waits for kuna.
    pub fn loading(&self) -> Option<String> {
        let (shown, Loaded::Loading) = self.current()? else {
            return None;
        };
        let verb = match shown.request.view {
            View::Decompile => "decompiling",
            View::Disassembly => "disassembling",
            View::Xrefs
            | View::Strings
            | View::CallGraph
            | View::Hex
            | View::PCode
            | View::Summary => "loading",
        };
        Some(format!("{verb} {}…", shown.name))
    }

    /// What kuna said while making the view shown, or why it failed.
    pub fn view_note(&self) -> Option<&str> {
        match self.current()?.1 {
            Loaded::Ready(_, notes) => notes.as_deref(),
            Loaded::Failed(why) => Some(why),
            Loaded::Loading => None,
        }
    }

    /// Whether the loop should keep going.
    pub fn running(&self) -> bool {
        self.running
    }

    /// Applies one message; a terminal that can no longer be read, or a signal, is an error that ends the loop.
    pub fn update(&mut self, msg: Msg) -> io::Result<()> {
        match msg {
            Msg::Input(Err(error)) => {
                let message = format!("cannot read the terminal: {error}");
                return Err(io::Error::new(error.kind(), message));
            }
            Msg::Input(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => self.on_key(key),
            Msg::Input(Ok(_)) => {}
            Msg::Functions(Ok(reply)) => {
                let names = reply.data.iter().map(|f| f.name.as_str().to_string());
                self.list = Pane::new(names.collect());
                self.functions = Functions::Ready(reply.data);
                self.status = reply.notes;
            }
            Msg::Functions(Err(error)) => {
                self.functions = Functions::Failed;
                self.status = Some(error.to_string());
            }
            Msg::Code {
                view,
                address,
                answer,
            } => self.answered(Request { view, address }, answer),
            #[cfg(unix)]
            Msg::Signal(name) => return Err(io::Error::other(format!("stopped by {name}"))),
        }
        Ok(())
    }

    /// Keeps the answer for `request`, unless that view is loaded already.
    fn answered(&mut self, request: Request, answer: Result<Reply<Vec<String>>, kuna::Error>) {
        if matches!(self.views.get(&request), Some(Loaded::Ready(..))) {
            return;
        }
        let loaded = match answer {
            Ok(reply) => Loaded::Ready(Pane::new(reply.data), reply.notes),
            Err(error) => Loaded::Failed(error.to_string()),
        };
        self.views.insert(request, loaded);
        self.evict();
    }

    /// Drops loaded views other than the one shown while more than `MAX_VIEWS` are kept.
    // ponytail: the victims are arbitrary and a dropped view loses its cursor; #11 replaces this with a cache.
    fn evict(&mut self) {
        let keep = self.shown.as_ref().map(|shown| shown.request);
        while self.views.len() > MAX_VIEWS {
            let victim = self
                .views
                .iter()
                .find(|(request, loaded)| {
                    Some(**request) != keep && matches!(loaded, Loaded::Ready(..))
                })
                .map(|(request, _)| *request);
            match victim {
                Some(request) => self.views.remove(&request),
                None => break,
            };
        }
    }

    /// Feeds the key to the keymap and applies the action it completes; the last message goes with the key.
    fn on_key(&mut self, key: KeyEvent) {
        self.message = None;
        if let Some(action) = self.keymap.key(key) {
            self.apply(action);
        }
    }

    /// Does what `action` asks.
    // ponytail: #9 acts on the other views; until then they say it is not available.
    // Every action is named so a new one cannot be dropped by accident.
    fn apply(&mut self, action: Action) {
        match action {
            Action::Quit => self.running = false,
            Action::Command(line) => self.command(&line),
            Action::Move { motion, count } => self.move_cursor(motion, count),
            Action::Scroll(to) => self.with_pane(|pane, rows| pane.scroll(to, rows)),
            Action::Pane(to) => self.focus_pane(to),
            Action::Open(view) => self.open_view(view),
            Action::Follow => match self.focus {
                Focus::List => self.open_view(View::Decompile),
                Focus::Code => self.follow(),
            },
            Action::Back => self.walk(true),
            Action::Forward => self.walk(false),
            Action::SetMark(mark) => self.set_mark(mark),
            Action::JumpMark { mark, exact } => self.jump_mark(mark, exact),
            Action::Search { text, forward } => self.search(text, false, forward),
            Action::SearchNext { reverse } => self.repeat_search(reverse),
            Action::SearchWord { forward } => self.search_word(forward),
        }
    }

    /// `/` and `?`: remembers the search, then looks for it.
    fn search(&mut self, text: String, whole: bool, forward: bool) {
        self.search = Some(Query {
            text,
            whole,
            forward,
        });
        self.find(false);
    }

    /// `n`, or `N` if `reverse`: the last search again.
    fn repeat_search(&mut self, reverse: bool) {
        if self.search.is_some() {
            self.find(reverse);
        } else {
            self.message = Some("no previous search".to_string());
        }
    }

    /// `*` and `#`: the whole word under the cursor.
    fn search_word(&mut self, forward: bool) {
        let rows = self.rows;
        let Some(pane) = self.focused_mut() else {
            return self.nothing_to_search();
        };
        match pane.go_to_word(rows) {
            Some(word) => self.search(word, true, forward),
            None => self.message = Some("no word under the cursor".to_string()),
        }
    }

    fn nothing_to_search(&mut self) {
        self.message = Some("nothing to search yet".to_string());
    }

    /// Moves the focused pane to the next match of the last search (the other way if `reverse`); a jump in the code
    /// leaves the line it started from in the jumplist.
    fn find(&mut self, reverse: bool) {
        let Some(query) = self.search.clone() else {
            return;
        };
        let forward = query.forward != reverse;
        let rows = self.rows;
        let before = (self.focus == Focus::Code).then(|| self.here()).flatten();
        let Some(pane) = self.focused_mut() else {
            return self.nothing_to_search();
        };
        let from = (pane.line(), pane.col());
        if !pane.find(&query.text, query.whole, forward, rows) {
            self.message = Some(format!("not found: {}", query.text));
            return;
        }
        let to = (pane.line(), pane.col());
        let wrapped = if forward { to <= from } else { to >= from };
        self.message = wrapped.then(|| {
            let (hit, then) = if forward {
                ("BOTTOM", "TOP")
            } else {
                ("TOP", "BOTTOM")
            };
            format!("search hit {hit}, continuing at {then}")
        });
        self.record_jump(before);
    }

    /// Leaves `before` in the jumplist if the cursor has since changed line.
    fn record_jump(&mut self, before: Option<Spot>) {
        if let Some(before) = before
            && self.here().is_some_and(|now| now.line != before.line)
        {
            self.jumps.record(before);
        }
    }

    fn not_yet(&mut self) {
        self.message = Some("not available yet".to_string());
    }

    /// Moves the cursor of the pane that has the focus; a jump motion in the code leaves its line in the jumplist.
    fn move_cursor(&mut self, motion: Motion, count: Option<u32>) {
        let jump = self.focus == Focus::Code
            && matches!(
                motion,
                Motion::Top
                    | Motion::Bottom
                    | Motion::MatchBrace
                    | Motion::ParagraphBack
                    | Motion::ParagraphForward
            );
        let before = jump.then(|| self.here()).flatten();
        self.with_pane(|pane, rows| pane.motion(motion, count, rows));
        self.record_jump(before);
    }

    /// The view shown with the cursor in it; the top while it loads.
    fn here(&self) -> Option<Spot> {
        let (shown, loaded) = self.current()?;
        let (line, col) = match loaded {
            Loaded::Ready(pane, _) => (pane.line(), pane.col()),
            Loaded::Loading | Loaded::Failed(_) => (0, 0),
        };
        Some(Spot {
            request: shown.request,
            name: shown.name.clone(),
            line,
            col,
        })
    }

    /// Leaves the view shown in the jumplist.
    fn leave(&mut self) {
        if let Some(here) = self.here() {
            self.jumps.record(here);
        }
    }

    /// Leaves the view shown in the jumplist, then shows `request`.
    fn jump(&mut self, request: Request, name: String) {
        self.leave();
        self.open(request, name);
    }

    /// Shows `spot` again, cursor included, without touching the jumplist.
    // ponytail: a view dropped from the cache (`evict`) comes back with the cursor at the top.
    fn go(&mut self, spot: Spot) {
        self.open(spot.request, spot.name);
        let rows = self.rows;
        if let Some(pane) = self.code_mut() {
            pane.go(spot.line, spot.col, rows);
        }
    }

    /// `Ctrl-o` (`back`) or `Ctrl-i`; nowhere to go is silent.
    fn walk(&mut self, back: bool) {
        let spot = if back {
            self.here().and_then(|here| self.jumps.back(here))
        } else {
            self.jumps.forward()
        };
        if let Some(spot) = spot {
            self.go(spot);
        }
    }

    /// Enter in the code: opens the function named by the word under the cursor, in the view shown.
    fn follow(&mut self) {
        let Some((shown, Loaded::Ready(pane, _))) = self.current() else {
            self.message = Some("nothing to follow yet".to_string());
            return;
        };
        let (view, word) = (shown.request.view, pane.word());
        let Some(word) = word else {
            self.message = Some("no word under the cursor".to_string());
            return;
        };
        match self.lookup(&word, false) {
            Ok((address, name)) => self.jump(Request { view, address }, name),
            Err(why) => self.message = Some(why),
        }
    }

    /// The functions of the binary, or why they are not known.
    fn listed(&self) -> Result<&[Function], &'static str> {
        match &self.functions {
            Functions::Loading => Err("the functions are still loading"),
            Functions::Failed => Err("the functions could not be listed"),
            Functions::Ready(list) => Ok(list),
        }
    }

    /// The address and name of the function `word` names (see `function_named`), or why there is none.
    fn lookup(&self, word: &str, contained: bool) -> Result<(u64, String), String> {
        let list = self.listed().map_err(str::to_string)?;
        function_named(list, word, contained)
            .map(|function| (function.address, function.name.as_str().to_string()))
            .ok_or_else(|| format!("no function: {word}"))
    }

    /// The key a mark is kept under: a lower case mark belongs to the view shown.
    fn mark_key(&self, mark: char) -> (Option<Request>, char) {
        let scope = self.shown.as_ref().map(|shown| shown.request);
        (scope.filter(|_| mark.is_ascii_lowercase()), mark)
    }

    /// The cursor of the code pane, once its lines are there, or why a mark cannot be set.
    fn mark_spot(&self) -> Result<Spot, &'static str> {
        match (self.focus, self.current()) {
            (Focus::Code, Some((_, Loaded::Ready(..)))) => self.here().ok_or(CODE_ONLY),
            (Focus::Code, _) => Err("nothing to mark yet"),
            (Focus::List, _) => Err(CODE_ONLY),
        }
    }

    /// `m` and a letter: remembers the cursor of the code pane.
    fn set_mark(&mut self, mark: char) {
        match self.mark_spot() {
            Ok(spot) => {
                self.marks.insert(self.mark_key(mark), spot);
            }
            Err(why) => self.message = Some(why.to_string()),
        }
    }

    /// `'` or `` ` `` and a letter: goes to the mark's line (first non-blank character) or, `exact`, its column.
    fn jump_mark(&mut self, mark: char, exact: bool) {
        if self.focus != Focus::Code {
            self.message = Some(CODE_ONLY.to_string());
            return;
        }
        let Some(spot) = self.marks.get(&self.mark_key(mark)).cloned() else {
            self.message = Some(format!("mark not set: {mark}"));
            return;
        };
        self.leave();
        self.go(spot);
        if !exact {
            self.with_pane(|pane, rows| pane.motion(Motion::FirstNonBlank, None, rows));
        }
    }

    /// Lets `change` act on the pane that has the focus, with the height of the panes.
    fn with_pane(&mut self, change: impl FnOnce(&mut Pane, usize)) {
        let rows = self.rows;
        if let Some(pane) = self.focused_mut() {
            change(pane, rows);
        }
    }

    /// The pane that has the focus; none while the code pane has nothing loaded.
    fn focused_mut(&mut self) -> Option<&mut Pane> {
        match self.focus {
            Focus::List => Some(&mut self.list),
            Focus::Code => self.code_mut(),
        }
    }

    /// The pane of the view shown, once kuna has answered.
    fn code_mut(&mut self) -> Option<&mut Pane> {
        let request = self.shown.as_ref()?.request;
        match self.views.get_mut(&request) {
            Some(Loaded::Ready(pane, _)) => Some(pane),
            _ => None,
        }
    }

    /// `Ctrl-w`: the focus moves to the other pane, but the code pane takes it only with something to show.
    fn focus_pane(&mut self, to: keymap::Pane) {
        let code = match to {
            keymap::Pane::Left => false,
            keymap::Pane::Right => true,
            keymap::Pane::Next | keymap::Pane::Prev => self.focus == Focus::List,
        };
        if code && self.shown.is_none() {
            self.message = Some("no code open".to_string());
            return;
        }
        self.focus = if code { Focus::Code } else { Focus::List };
    }

    /// The function under the list's cursor.
    fn selected(&self) -> Option<&Function> {
        match &self.functions {
            Functions::Ready(list) => list.get(self.list.line()),
            Functions::Loading | Functions::Failed => None,
        }
    }

    /// `gd` or `ga`, or Enter in the list: `view` of the selected function, or of the one shown.
    fn open_view(&mut self, view: View) {
        if !matches!(view, View::Decompile | View::Disassembly) {
            return self.not_yet();
        }
        let target = match self.focus {
            Focus::Code => self
                .shown
                .as_ref()
                .map(|shown| (shown.request.address, shown.name.clone())),
            Focus::List => self
                .selected()
                .map(|function| (function.address, function.name.as_str().to_string())),
        };
        let nothing = self.listed().err().unwrap_or("no function selected");
        match target {
            Some((address, name)) => self.jump(Request { view, address }, name),
            None => self.message = Some(nothing.to_string()),
        }
    }

    /// Shows `request` in the code pane and gives it the focus; a view not loaded is asked for.
    // ponytail: a view still loading is asked for again, as the worker drops requests a newer one replaced.
    fn open(&mut self, request: Request, name: String) {
        self.shown = Some(Shown { request, name });
        self.focus = Focus::Code;
        let rows = self.rows;
        if let Some(Loaded::Ready(pane, _)) = self.views.get_mut(&request) {
            return pane.keep_visible(rows);
        }
        self.views.insert(request, Loaded::Loading);
        if self.requests.send(request).is_err() {
            let why = "the kuna worker has stopped".to_string();
            self.views.insert(request, Loaded::Failed(why));
        }
    }

    /// Runs a `:` command: `q` `q!` `quit`, and `e NAME|0xADDR` (`edit`).
    fn command(&mut self, line: &str) {
        let line = line.trim();
        let (name, argument) = line
            .split_once(char::is_whitespace)
            .map_or((line, ""), |(name, rest)| (name, rest.trim()));
        match (name, argument) {
            ("q" | "q!" | "quit", "") => self.running = false,
            ("e" | "edit", "") => self.message = Some(":e needs a name or an address".to_string()),
            ("e" | "edit", word) => match self.lookup(word, true) {
                Ok((address, function)) => {
                    let request = Request {
                        view: View::Decompile,
                        address,
                    };
                    self.jump(request, function);
                }
                Err(why) => self.message = Some(why),
            },
            ("", _) => {}
            _ => self.message = Some(format!("not a command: {line}")),
        }
    }
}

/// The function `word` names: its name, an alias or its entry address (`0x…`); `contained` takes any address in it.
// ponytail: the first match in address order wins; duplicate names, and an alias equal to another's name, are not told apart.
fn function_named<'a>(
    functions: &'a [Function],
    word: &str,
    contained: bool,
) -> Option<&'a Function> {
    let address = word
        .strip_prefix("0x")
        .or_else(|| word.strip_prefix("0X"))
        .and_then(|digits| u64::from_str_radix(digits, 16).ok());
    let entry = |function: &&Function| Some(function.address) == address;
    let inside = |function: &&Function| {
        let end = function.address.saturating_add(function.size.max(1));
        contained && address.is_some_and(|at| (function.address..end).contains(&at))
    };
    let named = |function: &&Function| {
        function.name.as_str() == word
            || function.aliases.iter().any(|alias| alias.as_str() == word)
    };
    functions
        .iter()
        .find(entry)
        .or_else(|| functions.iter().find(inside))
        .or_else(|| functions.iter().find(named))
}

/// Draws the app and applies messages until it quits or the terminal fails; no sender left is an error too.
pub fn run<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    rx: &Receiver<Msg>,
) -> Result<(), Box<dyn Error>>
where
    B::Error: 'static,
{
    while app.running() {
        app.set_rows(ui::text_rows(Rect::from(terminal.size()?)));
        terminal.draw(|frame| ui::render(frame, app))?;
        let msg = rx.recv().map_err(|_| "the event sources stopped")?;
        app.update(msg)?;
    }
    Ok(())
}

/// Forwards terminal events to the loop; a read failure is sent too, then the thread ends.
// ponytail: never joined: it blocks in `event::read` until the process exits.
pub fn spawn_input(tx: Sender<Msg>) -> io::Result<JoinHandle<()>> {
    thread::Builder::new().spawn(move || {
        loop {
            let event = event::read();
            let failed = event.is_err();
            if tx.send(Msg::Input(event)).is_err() || failed {
                break;
            }
        }
    })
}

/// Catches SIGTERM and SIGHUP from here on and forwards each one to the loop.
// ponytail: never joined: it waits for signals until the process exits. It never ends on its own: a dropped
// `Signals` would leave later signals ignored, not fatal. A repeat waits for the same cleanup (a terminal often
// sends SIGHUP then SIGTERM); only SIGKILL cuts a hung one short.
#[cfg(unix)]
pub fn spawn_signals(tx: Sender<Msg>) -> io::Result<JoinHandle<()>> {
    use signal_hook::consts::{SIGHUP, SIGTERM};
    use signal_hook::iterator::Signals;
    use signal_hook::low_level::signal_name;
    let mut signals = Signals::new([SIGTERM, SIGHUP])?;
    thread::Builder::new().spawn(move || {
        for signal in signals.forever() {
            // ponytail: no loop left to tell: bonito is already ending.
            let _ = tx.send(Msg::Signal(signal_name(signal).unwrap_or("a signal")));
        }
    })
}

#[cfg(test)]
mod tests;
