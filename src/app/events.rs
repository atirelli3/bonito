//! The threads that feed the loop: terminal events and, on Unix, SIGTERM and SIGHUP.

use super::Msg;
use ratatui::crossterm::event;
use std::io;
use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};

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
