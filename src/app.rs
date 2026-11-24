//! The application state, its workers and the event loop.

use crate::keymap::{self, Action, Keymap, View};
use crate::kuna::{self, Decompiled, Disassembly, Function, Kuna, Reply};
use crate::ui;
use crate::view::Pane;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::crossterm::event::{self, Event, KeyEvent, KeyEventKind};
use ratatui::layout::Rect;
use std::collections::HashMap;
use std::error::Error;
use std::io;
use std::iter::once;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// What a tab becomes in a code pane.
// ponytail: four spaces, not tab stops.
const TAB: &str = "    ";
/// How many lines a view keeps: a hostile binary can make kuna print far more.
const MAX_LINES: usize = 100_000;
/// How many characters of a line a view keeps.
const MAX_LINE_CHARS: usize = 4096;
/// How many loaded views are kept: each can hold a hundred thousand lines.
const MAX_VIEWS: usize = 64;
/// How many characters of a panic message are kept.
const PANIC_LIMIT: usize = 200;
/// How often an idle worker checks whether kuna was cancelled.
const IDLE: Duration = Duration::from_millis(100);

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

    /// Applies one message; a terminal that can no longer be read is the only error.
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
    // ponytail: #52 acts on follow in the code, the jumplist and marks, #53 on searches, #9 on the other views;
    // until then they say it is not available. Every action is named so a new one cannot be dropped by accident.
    fn apply(&mut self, action: Action) {
        match action {
            Action::Quit => self.running = false,
            Action::Command(line) => self.command(&line),
            Action::Move { motion, count } => {
                self.with_pane(|pane, rows| pane.motion(motion, count, rows));
            }
            Action::Scroll(to) => self.with_pane(|pane, rows| pane.scroll(to, rows)),
            Action::Pane(to) => self.focus_pane(to),
            Action::Open(view) => self.open_view(view),
            Action::Follow => match self.focus {
                Focus::List => self.open_view(View::Decompile),
                Focus::Code => self.not_yet(),
            },
            Action::Search { .. }
            | Action::Back
            | Action::Forward
            | Action::SetMark(_)
            | Action::JumpMark { .. }
            | Action::SearchNext { .. }
            | Action::SearchWord { .. } => self.not_yet(),
        }
    }

    fn not_yet(&mut self) {
        self.message = Some("not available yet".to_string());
    }

    /// Lets `change` act on the pane that has the focus, with the height of the panes.
    fn with_pane(&mut self, change: impl FnOnce(&mut Pane, usize)) {
        let rows = self.rows;
        match self.focus {
            Focus::List => change(&mut self.list, rows),
            Focus::Code => {
                if let Some(pane) = self.code_mut() {
                    change(pane, rows);
                }
            }
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
        let nothing = match &self.functions {
            Functions::Loading => "the functions are still loading",
            Functions::Failed => "the functions could not be listed",
            Functions::Ready(_) => "no function selected",
        };
        match target {
            Some((address, name)) => self.open(Request { view, address }, name),
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

    /// Runs a `:` command; `q`, `quit` and `q!` are the only ones so far.
    fn command(&mut self, line: &str) {
        match line.trim() {
            "q" | "q!" | "quit" => self.running = false,
            "" => {}
            other => self.message = Some(format!("not a command: {other}")),
        }
    }
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

/// Lists the functions of `binary` on a worker thread, then answers `requests` until every sender is gone or
/// kuna is cancelled. Of the requests waiting, only the newest of each view is answered: the user has moved on
/// from the rest. A panic in a kuna call is sent as an error.
// ponytail: a call in flight is not cancelled by a newer request, only by `Kuna::cancel` (#11 caches instead).
pub fn spawn_worker(
    kuna: Arc<Kuna>,
    binary: PathBuf,
    requests: Receiver<Request>,
    tx: Sender<Msg>,
) -> io::Result<JoinHandle<()>> {
    thread::Builder::new().spawn(move || {
        let functions = guarded(|| kuna.functions(&binary));
        // ponytail: a closed channel means the loop is gone; nobody is left to tell.
        if tx.send(Msg::Functions(functions)).is_err() {
            return;
        }
        loop {
            let oldest = match requests.recv_timeout(IDLE) {
                Ok(request) => request,
                Err(RecvTimeoutError::Timeout) if !kuna.is_cancelled() => continue,
                Err(_) => return,
            };
            for Request { view, address } in newest_per_view(oldest, &requests) {
                let answer = guarded(|| lines_of(&kuna, &binary, view, address));
                let msg = Msg::Code {
                    view,
                    address,
                    answer,
                };
                if tx.send(msg).is_err() {
                    return;
                }
            }
        }
    })
}

/// `first` and the requests waiting, in order, without those a later request for the same view replaces.
fn newest_per_view(first: Request, waiting: &Receiver<Request>) -> Vec<Request> {
    let all: Vec<Request> = once(first).chain(waiting.try_iter()).collect();
    all.iter()
        .enumerate()
        .filter(|(at, request)| all[at + 1..].iter().all(|later| later.view != request.view))
        .map(|(_, request)| *request)
        .collect()
}

/// Runs `work`; a panic in it becomes an error that keeps the panic's message.
fn guarded<T>(work: impl FnOnce() -> Result<T, kuna::Error>) -> Result<T, kuna::Error> {
    panic::catch_unwind(AssertUnwindSafe(work)).unwrap_or_else(|payload| {
        let reason = match (
            payload.downcast_ref::<&str>(),
            payload.downcast_ref::<String>(),
        ) {
            (Some(text), _) => (*text).to_string(),
            (None, Some(text)) => text.clone(),
            (None, None) => "no message".to_string(),
        };
        let reason = kuna::single_line(&reason, PANIC_LIMIT);
        Err(kuna::Error::Control(io::Error::other(format!(
            "the worker thread panicked: {reason}"
        ))))
    })
}

/// The lines of `view` of the function at `address`, with what kuna said while making them.
fn lines_of(
    kuna: &Kuna,
    binary: &Path,
    view: View,
    address: u64,
) -> Result<Reply<Vec<String>>, kuna::Error> {
    match view {
        View::Decompile => decompile_reply(kuna.decompile(binary, address)?),
        View::Disassembly => Ok(disassembly_reply(kuna.disassemble(binary, address)?)),
        // ponytail: #9 adds these; nothing asks for them yet.
        View::Xrefs | View::Strings | View::CallGraph | View::Hex | View::PCode | View::Summary => {
            Err(kuna::Error::Reported(format!(
                "the {view:?} view is not available in bonito yet"
            )))
        }
    }
}

/// At most `MAX_LINES` lines of at most `MAX_LINE_CHARS` characters, and whether anything was cut.
fn bounded(mut lines: impl Iterator<Item = String>) -> (Vec<String>, bool) {
    let mut kept: Vec<String> = lines.by_ref().take(MAX_LINES).collect();
    let mut cut = lines.next().is_some();
    for line in &mut kept {
        if let Some((end, _)) = line.char_indices().nth(MAX_LINE_CHARS) {
            line.truncate(end);
            cut = true;
        }
    }
    (kept, cut)
}

/// The notes of a reply, joined for the status line; none when there are none.
fn join_notes(notes: impl IntoIterator<Item = String>, cut: bool) -> Option<String> {
    let cut = cut.then(|| format!("cut to {MAX_LINES} lines of {MAX_LINE_CHARS} characters"));
    let notes: Vec<String> = notes.into_iter().chain(cut).collect();
    (!notes.is_empty()).then(|| notes.join(" · "))
}

/// The C text of a function, one string per line; a function kuna gave no code for is an error.
fn decompile_reply(reply: Reply<Decompiled>) -> Result<Reply<Vec<String>>, kuna::Error> {
    let Reply { data, notes } = reply;
    let code = data.code.as_ref().ok_or_else(|| {
        kuna::Error::Reported("kuna returned no code for this function".to_string())
    })?;
    let (lines, cut) = bounded(code.as_str().lines().map(str::to_string));
    Ok(Reply {
        data: lines
            .into_iter()
            .map(|line| line.replace('\t', TAB))
            .collect(),
        notes: join_notes(notes, cut),
    })
}

/// A disassembly as `0x201140  PUSH RAX` lines; kuna's notes and a cut listing join the reply's notes.
fn disassembly_reply(reply: Reply<Disassembly>) -> Reply<Vec<String>> {
    let Reply { data, notes } = reply;
    let (lines, cut) =
        bounded(data.instructions.iter().map(|instruction| {
            format!("{:#x}  {}", instruction.address, instruction.text.as_str())
        }));
    let stopped = data
        .truncated
        .then(|| format!("the listing stops at {:#x}", data.end));
    let said = notes
        .into_iter()
        .chain(data.notes.iter().map(|note| note.as_str().to_string()))
        .chain(stopped);
    Reply {
        data: lines,
        notes: join_notes(said, cut),
    }
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

#[cfg(test)]
mod tests;
