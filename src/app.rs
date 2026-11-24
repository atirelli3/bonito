//! The application state, its workers and the event loop.

use crate::keymap::{Action, Keymap};
use crate::kuna::{self, Function, Kuna, Reply};
use crate::ui;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::crossterm::event::{self, Event, KeyEvent, KeyEventKind};
use std::error::Error;
use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread::{self, JoinHandle};

/// Everything the loop reacts to.
pub enum Msg {
    /// A terminal event, or the failure to read one.
    Input(io::Result<Event>),
    /// The answer to the functions request.
    Functions(Result<Reply<Vec<Function>>, kuna::Error>),
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

/// Lists the functions of `binary` on a worker thread and sends the answer; a panic in it is sent as an error.
pub fn spawn_functions(
    kuna: Arc<Kuna>,
    binary: PathBuf,
    tx: Sender<Msg>,
) -> io::Result<JoinHandle<()>> {
    thread::Builder::new().spawn(move || {
        let answer = panic::catch_unwind(AssertUnwindSafe(|| kuna.functions(&binary)))
            .unwrap_or_else(|_| {
                Err(kuna::Error::Control(io::Error::other(
                    "the worker thread panicked",
                )))
            });
        // ponytail: a closed channel means the loop is gone; nobody is left to tell.
        let _ = tx.send(Msg::Functions(answer));
    })
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
