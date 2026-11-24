//! Drawing: a function of the app state, nothing else.

use crate::app::{App, Functions};
use crate::kuna::single_line;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::widgets::{Block, List, ListItem, Paragraph, Wrap};

/// The share of the width the function list takes, in percent.
const LIST_PERCENT: u16 = 30;
/// How many rows the status area has: a long message wraps instead of running off the screen.
const STATUS_ROWS: u16 = 2;
/// How many characters of the binary's file name the status line keeps.
const NAME_LIMIT: usize = 60;
/// How many characters of the whole status line are kept.
const STATUS_LIMIT: usize = 600;

/// Draws `app` into `frame`: the function list and the code pane side by side, the status below them.
pub fn render(frame: &mut Frame, app: &App) {
    let [body, status] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(STATUS_ROWS)]).areas(frame.area());
    let [list, code] =
        Layout::horizontal([Constraint::Percentage(LIST_PERCENT), Constraint::Fill(1)]).areas(body);
    render_functions(frame, list, app);
    let placeholder = Paragraph::new("no function selected").block(Block::bordered().title("Code"));
    frame.render_widget(placeholder, code);
    render_status(frame, status, app);
}

/// The last row of `area`.
fn last_row(area: Rect) -> Rect {
    Rect {
        y: area.bottom().saturating_sub(1),
        height: area.height.min(1),
        ..area
    }
}

/// The status area: the status line, with the pending keys at its right. An open prompt takes the last row of the
/// screen, so it shows even when the terminal is too short for the status area.
fn render_status(frame: &mut Frame, area: Rect, app: &App) {
    let status = Paragraph::new(status_line(app));
    if let Some((leader, text)) = app.keymap.prompt() {
        frame.render_widget(
            status,
            Rect {
                height: area.height.min(1),
                ..area
            },
        );
        render_prompt(frame, last_row(frame.area()), leader, text);
        return;
    }
    frame.render_widget(status.wrap(Wrap { trim: true }), area);
    let pending = app.keymap.pending();
    if !pending.is_empty() {
        frame.render_widget(Paragraph::new(pending).right_aligned(), last_row(area));
    }
}

/// The prompt on one row, showing its end when it is longer than the row, with the cursor after it.
// ponytail: one column per character; a wide character shifts the cursor.
fn render_prompt(frame: &mut Frame, area: Rect, leader: char, text: &str) {
    if area.is_empty() {
        return;
    }
    let shown = format!("{leader}{}", single_line(text, STATUS_LIMIT));
    let room = usize::from(area.width).saturating_sub(1);
    let skipped = shown.chars().count().saturating_sub(room);
    let tail: String = shown.chars().skip(skipped).collect();
    let column = u16::try_from(tail.chars().count()).unwrap_or_default();
    frame.render_widget(Paragraph::new(tail), area);
    frame.set_cursor_position((area.x + column, area.y));
}

/// The function list, as many rows as fit.
fn render_functions(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title("Functions");
    let rows = usize::from(block.inner(area).height);
    frame.render_widget(
        List::new(function_items(&app.functions, rows)).block(block),
        area,
    );
}

/// The list rows for `functions`: at most `rows` of them, so a huge binary costs one screen, not one list.
// ponytail: counts from the first function; scrolling (#8) makes it `skip(offset).take(rows)`.
fn function_items(functions: &Functions, rows: usize) -> Vec<ListItem<'_>> {
    match functions {
        Functions::Loading => vec![ListItem::new("loading…")],
        Functions::Failed => vec![ListItem::new("failed")],
        Functions::Ready(list) => list
            .iter()
            .take(rows)
            .map(|function| ListItem::new(function.name.as_str()))
            .collect(),
    }
}

/// `state`, the last message, what kuna said, the version warning, then `name · kuna VERSION`; cleaned for the
/// terminal. The reasons come first: on a narrow terminal the end of the line is what gets cut.
fn status_line(app: &App) -> String {
    let state = match &app.functions {
        Functions::Loading => "loading…".to_string(),
        Functions::Ready(list) => format!("functions: {}", list.len()),
        Functions::Failed => "failed".to_string(),
    };
    let path = app.binary.as_path();
    let name = path.file_name().unwrap_or(path.as_os_str());
    let name = single_line(&name.to_string_lossy(), NAME_LIMIT);
    let mut parts = vec![state];
    parts.extend(
        [&app.message, &app.status, &app.warning]
            .into_iter()
            .flatten()
            .cloned(),
    );
    parts.push(format!("{name} · kuna {}", app.kuna_version));
    single_line(&parts.join(" · "), STATUS_LIMIT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Msg;
    use crate::kuna::{Error, Function, Reply};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use std::path::PathBuf;

    fn function(name: &str) -> Function {
        let text = format!(r#"{{"name":"{name}","address":4198720,"size":32}}"#);
        serde_json::from_str(&text).expect("a function")
    }

    fn app(binary: &str) -> App {
        App::new(PathBuf::from(binary), "1.1005".to_string(), None)
    }

    /// The rows of a `width` x `height` frame of `app`, one string per row.
    fn rows(app: &App, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test terminal");
        terminal.draw(|frame| render(frame, app)).expect("draws");
        let buffer = terminal.backend().buffer();
        match width {
            0 => Vec::new(),
            _ => buffer
                .content()
                .chunks(usize::from(width))
                .map(|row| row.iter().map(|cell| cell.symbol()).collect())
                .collect(),
        }
    }

    fn screen(app: &App, width: u16, height: u16) -> String {
        rows(app, width, height).join("\n")
    }

    fn ready(app: &mut App, names: &[&str]) {
        let data = names.iter().map(|name| function(name)).collect();
        let reply = Reply { data, notes: None };
        app.update(Msg::Functions(Ok(reply))).expect("updates");
    }

    #[test]
    fn a_loading_frame_has_the_list_the_code_pane_and_the_status() {
        let rows = rows(&app("/samples/hello.elf"), 80, 12);
        assert!(
            rows[0].contains("Functions") && rows[0].contains("Code"),
            "{rows:?}"
        );
        assert!(rows[1].contains("loading…"), "{rows:?}");
        let status = rows[10..].join("\n");
        assert!(
            status.contains("loading… · hello.elf · kuna 1.1005"),
            "{status}"
        );
    }

    #[test]
    fn a_ready_frame_lists_the_functions_and_counts_them() {
        let mut app = app("hello.elf");
        ready(&mut app, &["square", "add", "main", "_start"]);
        let screen = screen(&app, 80, 12);
        for name in ["square", "add", "main", "_start"] {
            assert!(screen.contains(name), "{screen}");
        }
        assert!(screen.contains("functions: 4 · hello.elf"), "{screen}");
    }

    #[test]
    fn the_list_builds_only_the_rows_that_fit() {
        let names: Vec<String> = (0..50).map(|n| format!("fn_{n:02}")).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut app = app("big.elf");
        ready(&mut app, &names);
        assert_eq!(function_items(&app.functions, 3).len(), 3);
        assert_eq!(function_items(&app.functions, 0).len(), 0);
        assert_eq!(function_items(&app.functions, 500).len(), 50);
        assert_eq!(function_items(&Functions::Loading, 0).len(), 1);
    }

    #[test]
    fn a_failed_frame_shows_why_on_a_narrow_terminal_with_a_long_name() {
        let mut app = app(&format!("/samples/{}.elf", "d".repeat(150)));
        app.update(Msg::Functions(Err(Error::NotOnPath)))
            .expect("updates");
        let screen = screen(&app, 40, 8);
        assert!(screen.contains("failed · kuna not found"), "{screen}");
    }

    #[test]
    fn what_kuna_said_and_the_version_warning_are_both_shown() {
        let mut app = App::new(
            PathBuf::from("hello.elf"),
            "1.1006".to_string(),
            Some("tested with 1.1005".to_string()),
        );
        let reply = Reply {
            data: vec![function("main")],
            notes: Some("repaired the header".to_string()),
        };
        app.update(Msg::Functions(Ok(reply))).expect("updates");
        let screen = screen(&app, 100, 12);
        assert!(screen.contains("repaired the header"), "{screen}");
        assert!(screen.contains("tested with 1.1005"), "{screen}");
    }

    #[test]
    fn a_hostile_file_name_is_cleaned() {
        let line = status_line(&app("/samples/evil\u{1b}[2Jname\u{202e}.elf"));
        assert_eq!(line, "loading… · evil [2Jname .elf · kuna 1.1005");
    }

    #[test]
    fn hostile_text_in_the_messages_is_cleaned() {
        let mut app = App::new(
            PathBuf::from("a.elf"),
            "1".to_string(),
            Some("warn\u{1b}[31m\u{202e}".to_string()),
        );
        let reply = Reply {
            data: Vec::new(),
            notes: Some("note\u{1b}[2J".to_string()),
        };
        app.update(Msg::Functions(Ok(reply))).expect("updates");
        let line = status_line(&app);
        assert!(
            !line.contains('\u{1b}') && !line.contains('\u{202e}'),
            "{line:?}"
        );
        assert!(
            line.contains("note [2J") && line.contains("warn [31m"),
            "{line:?}"
        );
    }

    #[test]
    fn a_long_file_name_is_cut() {
        let line = status_line(&app(&format!("/samples/{}", "n".repeat(70))));
        let name = line.rsplit(" · ").nth(1).expect("the name part");
        assert_eq!(name.chars().count(), NAME_LIMIT);
    }

    /// Types `text` as plain key presses.
    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            let key = KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
            app.update(Msg::Input(Ok(Event::Key(key))))
                .expect("updates");
        }
    }

    /// Where the cursor ends up after drawing `app` on a `width` x `height` terminal.
    fn cursor_after(app: &App, width: u16, height: u16) -> (u16, u16) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test terminal");
        terminal.draw(|frame| render(frame, app)).expect("draws");
        let position = terminal.get_cursor_position().expect("a cursor position");
        (position.x, position.y)
    }

    #[test]
    fn an_open_prompt_is_on_the_last_row_with_the_cursor_after_it() {
        let mut app = app("hello.elf");
        type_text(&mut app, ":ab");
        let rows = rows(&app, 40, 8);
        assert!(rows[7].starts_with(":ab"), "{rows:?}");
        assert_eq!(cursor_after(&app, 40, 8), (3, 7));
    }

    #[test]
    fn the_status_stays_above_the_prompt() {
        let mut app = app("hello.elf");
        type_text(&mut app, ":");
        let rows = rows(&app, 60, 8);
        assert!(rows[6].contains("loading… · hello.elf"), "{rows:?}");
    }

    #[test]
    fn a_search_prompt_shows_its_leader() {
        let mut forward = app("hello.elf");
        type_text(&mut forward, "/foo");
        assert!(rows(&forward, 40, 8)[7].starts_with("/foo"));
        let mut backward = app("hello.elf");
        type_text(&mut backward, "?foo");
        assert!(rows(&backward, 40, 8)[7].starts_with("?foo"));
    }

    #[test]
    fn a_prompt_longer_than_the_row_shows_its_end() {
        let mut app = app("hello.elf");
        type_text(&mut app, &format!(":{}END", "a".repeat(60)));
        let rows = rows(&app, 20, 8);
        // 19 columns of text and one blank under the cursor.
        assert_eq!(rows[7], format!("{}END ", "a".repeat(16)));
        assert_eq!(cursor_after(&app, 20, 8), (19, 7));
    }

    #[test]
    fn a_prompt_that_exactly_fits_keeps_its_leader_and_the_cursor_after_it() {
        let mut app = app("hello.elf");
        type_text(&mut app, &format!(":{}", "a".repeat(18)));
        let rows = rows(&app, 20, 8);
        assert_eq!(rows[7], format!(":{} ", "a".repeat(18)));
        assert_eq!(cursor_after(&app, 20, 8), (19, 7));
    }

    #[test]
    fn the_prompt_shows_on_terminals_too_short_for_the_status_area() {
        let mut app = app("hello.elf");
        type_text(&mut app, ":abc");
        for height in 1..=3 {
            let rows = rows(&app, 30, height);
            let last = rows.last().expect("a last row");
            assert!(last.starts_with(":abc"), "height {height}: {rows:?}");
            assert_eq!(cursor_after(&app, 30, height), (4, height - 1));
        }
    }

    #[test]
    fn the_last_message_is_in_the_status_line() {
        let mut app = app("hello.elf");
        type_text(&mut app, ":nope");
        app.update(Msg::Input(Ok(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))))
        .expect("updates");
        let line = status_line(&app);
        assert!(
            line.starts_with("loading… · not a command: nope · hello.elf"),
            "{line}"
        );
    }

    #[test]
    fn no_prompt_without_a_prompt_and_pending_keys_show_on_the_right() {
        let mut app = app("hello.elf");
        assert!(!screen(&app, 30, 8).contains("2g"));
        type_text(&mut app, "2g");
        let rows = rows(&app, 30, 8);
        assert!(rows[7].trim_end().ends_with("2g"), "{rows:?}");
        assert!(rows[6].contains("loading…"), "{rows:?}");
    }

    #[test]
    fn terminals_of_any_tiny_size_do_not_panic_with_a_prompt_open() {
        let mut app = app("hello.elf");
        type_text(&mut app, ":abc");
        for width in 0..=3 {
            for height in 0..=3 {
                rows(&app, width, height);
                cursor_after(&app, width, height);
            }
        }
    }

    #[test]
    fn terminals_of_any_tiny_size_do_not_panic() {
        let mut app = app("hello.elf");
        ready(&mut app, &["main"]);
        for width in 0..=3 {
            for height in 0..=3 {
                rows(&app, width, height);
            }
        }
        assert!(screen(&app, 4, 3).contains("func"));
    }
}
