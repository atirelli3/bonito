//! Drawing: a function of the app state, nothing else.

use crate::app::{App, Functions};
use crate::kuna::single_line;
use ratatui::Frame;
use ratatui::widgets::Paragraph;

/// How many characters of the binary's path the status line keeps.
const PATH_LIMIT: usize = 200;

/// Draws `app` into `frame`.
pub fn render(frame: &mut Frame, app: &App) {
    frame.render_widget(Paragraph::new(status_line(app)), frame.area());
}

/// `binary · kuna VERSION · state`, then the status message if there is one.
fn status_line(app: &App) -> String {
    let state = match &app.functions {
        Functions::Loading => "loading…".to_string(),
        Functions::Ready(list) => format!("functions: {}", list.len()),
        Functions::Failed => "failed".to_string(),
    };
    // The path is typed by the user but names a file that may come from a hostile source: clean it like kuna's text.
    let binary = single_line(&app.binary.to_string_lossy(), PATH_LIMIT);
    let mut line = format!("{binary} · kuna {} · {state}", app.kuna_version);
    for message in [&app.status, &app.warning].into_iter().flatten() {
        line.push_str(" · ");
        line.push_str(message);
    }
    line
}
