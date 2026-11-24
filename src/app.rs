//! The application state, its workers and the event loop.

use crate::keymap::{Action, Keymap, View};
use crate::kuna::{self, Decompiled, Disassembly, Function, Kuna, Reply};
use crate::ui;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::crossterm::event::{self, Event, KeyEvent, KeyEventKind};
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
    // ponytail: the views (#51) read it; until then the loop only drops it.
    #[allow(dead_code)]
    Code {
        view: View,
        address: u64,
        answer: Result<Reply<Vec<String>>, kuna::Error>,
    },
}

/// A view of the function at `address`, asked of the worker.
// ponytail: the views (#51) send these; until then only the tests do.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub view: View,
    pub address: u64,
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
    running: bool,
}

impl App {
    /// A new app for `binary`; `warning` is shown for the whole session.
    pub fn new(binary: PathBuf, kuna_version: String, warning: Option<String>) -> Self {
        Self {
            binary,
            kuna_version,
            functions: Functions::Loading,
            status: None,
            message: None,
            warning,
            keymap: Keymap::default(),
            running: true,
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
                self.functions = Functions::Ready(reply.data);
                self.status = reply.notes;
            }
            Msg::Functions(Err(error)) => {
                self.functions = Functions::Failed;
                self.status = Some(error.to_string());
            }
            // ponytail: #51 shows the lines; a failure already reaches the status.
            Msg::Code {
                answer: Err(error), ..
            } => self.status = Some(error.to_string()),
            Msg::Code { answer: Ok(_), .. } => {}
        }
        Ok(())
    }

    /// Feeds the key to the keymap and applies the action it completes; the last message goes with the key.
    fn on_key(&mut self, key: KeyEvent) {
        self.message = None;
        if let Some(action) = self.keymap.key(key) {
            self.apply(action);
        }
    }

    /// Does what `action` asks.
    // ponytail: the views of #8-#15 act on the rest. Until then a search, a view or a follow says it is not
    // available and the motions are silent; every action is named so a new one cannot be dropped by accident.
    fn apply(&mut self, action: Action) {
        match action {
            Action::Quit => self.running = false,
            Action::Command(line) => self.command(&line),
            Action::Search { .. } | Action::Open(_) | Action::Follow => {
                self.message = Some("not available yet".to_string());
            }
            Action::Move { .. }
            | Action::Scroll(_)
            | Action::Pane(_)
            | Action::Back
            | Action::Forward
            | Action::SetMark(_)
            | Action::JumpMark { .. }
            | Action::SearchNext { .. }
            | Action::SearchWord { .. } => {}
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
