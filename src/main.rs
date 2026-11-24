//! bonito — a terminal UI for the kuna decompiler.

mod app;
mod args;
mod kuna;
mod ui;

use app::App;
use args::Command;
use kuna::Kuna;
use std::error::Error;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::{env, fs, panic};

/// The exit code for a failure after the command line was read.
const EXIT_FAILURE: u8 = 1;
/// The exit code for a command line that cannot be read.
const EXIT_USAGE: u8 = 2;

fn main() -> ExitCode {
    match args::parse(env::args_os().skip(1)) {
        Ok(Command::Help) => {
            // ponytail: a closed stdout is not worth a message: `bonito --help | head -0` just ends.
            let _ = writeln!(io::stdout(), "{}", args::USAGE);
            ExitCode::SUCCESS
        }
        Ok(Command::Open { binary, kuna }) => match open(binary, kuna) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("bonito: {error}");
                ExitCode::from(EXIT_FAILURE)
            }
        },
        Err(message) => {
            eprintln!("bonito: {message}\n{}", args::USAGE);
            ExitCode::from(EXIT_USAGE)
        }
    }
}

/// Stops kuna and waits for its worker when dropped, however `open` is left (an error or a panic too).
struct Stop {
    kuna: Arc<Kuna>,
    worker: Option<JoinHandle<()>>,
}

impl Drop for Stop {
    fn drop(&mut self) {
        self.kuna.cancel();
        if let Some(worker) = self.worker.take() {
            // ponytail: a worker that panicked already sent its error to the loop, which is gone: nothing to add.
            let _ = worker.join();
        }
    }
}

/// Starts kuna on `binary` and runs the UI until the user quits; kuna is always stopped before this returns.
fn open(binary: PathBuf, explicit_kuna: Option<PathBuf>) -> Result<(), Box<dyn Error>> {
    require_file(&binary)?;
    let kuna = Arc::new(kuna::locate(explicit_kuna.as_deref(), &binary)?);
    require_terminal()?;
    let mut app = App::new(
        binary.clone(),
        kuna.version().to_string(),
        kuna.version_warning(),
    );
    let (tx, rx) = mpsc::channel();
    let worker = app::spawn_functions(Arc::clone(&kuna), binary, tx.clone())?;
    let _stop = Stop {
        kuna,
        worker: Some(worker),
    };
    interact(&mut app, tx, &rx)
}

/// Takes over the terminal, runs the loop, and gives the terminal back before reporting any failure.
fn interact(
    app: &mut App,
    tx: Sender<app::Msg>,
    rx: &Receiver<app::Msg>,
) -> Result<(), Box<dyn Error>> {
    let mut terminal = ratatui::try_init().map_err(|error| {
        // ponytail: best effort; if the terminal could not be taken over there is little to give back.
        let _ = ratatui::try_restore();
        format!("cannot start the terminal UI: {error}")
    })?;
    restore_the_terminal_on_main_thread_panics_only();
    let outcome = app::spawn_input(tx)
        .map_err(Box::<dyn Error>::from)
        .and_then(|_| app::run(&mut terminal, app, rx));
    let restored = ratatui::try_restore();
    outcome?;
    Ok(restored?)
}

/// Makes the panic hook ratatui installed act on the main thread only: a worker's panic is reported through
/// the loop as an error, and must not take the terminal away while the loop is still drawing.
fn restore_the_terminal_on_main_thread_panics_only() {
    let hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        if thread::current().name() == Some("main") {
            hook(info);
        }
    }));
}

/// `path` must be an existing file; the error shows it escaped, since it is typed by the user.
fn require_file(path: &Path) -> Result<(), String> {
    match fs::metadata(path) {
        Ok(meta) if meta.is_file() => Ok(()),
        Ok(_) => Err(format!("{path:?} is not a file")),
        Err(error) => Err(format!("cannot open {path:?}: {error}")),
    }
}

/// The UI reads the keyboard and draws on the terminal, so both ends must be one.
// ponytail: crossterm falls back to /dev/tty when stdin is not a terminal; without this check a script could
// start the UI on the user's own terminal.
fn require_terminal() -> Result<(), String> {
    match io::stdin().is_terminal() && io::stdout().is_terminal() {
        true => Ok(()),
        false => Err("bonito needs an interactive terminal".to_string()),
    }
}
