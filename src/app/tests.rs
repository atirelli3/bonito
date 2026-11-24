//! Tests for the app state, the loop and the worker.
use super::*;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use std::path::Path;
use std::sync::mpsc;

fn app() -> App {
    App::new(
        PathBuf::from("/samples/hello.elf"),
        "1.1005".to_string(),
        None,
    )
}

fn function(name: &str) -> Function {
    let text = format!(r#"{{"name":"{name}","address":4198720,"size":32}}"#);
    serde_json::from_str(&text).expect("a function")
}

fn answer(names: &[&str], notes: Option<&str>) -> Msg {
    Msg::Functions(Ok(Reply {
        data: names.iter().map(|name| function(name)).collect(),
        notes: notes.map(str::to_string),
    }))
}

fn key(code: KeyCode, modifiers: KeyModifiers, kind: KeyEventKind) -> Msg {
    Msg::Input(Ok(Event::Key(KeyEvent::new_with_kind(
        code, modifiers, kind,
    ))))
}

fn press(code: KeyCode) -> Msg {
    key(code, KeyModifiers::NONE, KeyEventKind::Press)
}

#[test]
fn q_quits() {
    let mut app = app();
    app.update(press(KeyCode::Char('q'))).expect("updates");
    assert!(!app.running());
}

#[test]
fn ctrl_c_quits() {
    let mut app = app();
    let ctrl_c = key(
        KeyCode::Char('c'),
        KeyModifiers::CONTROL,
        KeyEventKind::Press,
    );
    app.update(ctrl_c).expect("updates");
    assert!(!app.running());
}

#[test]
fn other_keys_and_a_key_release_do_not_quit() {
    let mut app = app();
    app.update(press(KeyCode::Char('x'))).expect("updates");
    app.update(press(KeyCode::Char('c'))).expect("updates");
    let release = key(
        KeyCode::Char('q'),
        KeyModifiers::NONE,
        KeyEventKind::Release,
    );
    app.update(release).expect("updates");
    app.update(Msg::Input(Ok(Event::Resize(80, 24))))
        .expect("updates");
    assert!(app.running());
}

/// Types `text` as plain key presses.
fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        app.update(press(KeyCode::Char(c))).expect("updates");
    }
}

fn enter(app: &mut App) {
    app.update(press(KeyCode::Enter)).expect("updates");
}

#[test]
fn colon_q_enter_quits_but_not_before_enter() {
    let mut app = app();
    type_text(&mut app, ":q");
    assert!(app.running());
    enter(&mut app);
    assert!(!app.running());
}

#[test]
fn quit_commands_quit() {
    for command in [":quit", ":q!", ":q", ": q "] {
        let mut app = app();
        type_text(&mut app, command);
        enter(&mut app);
        assert!(!app.running(), "{command}");
    }
}

#[test]
fn zz_quits() {
    let mut app = app();
    type_text(&mut app, "ZZ");
    assert!(!app.running());
}

#[test]
fn an_unknown_command_is_reported_and_the_app_keeps_running() {
    let mut app = app();
    type_text(&mut app, ":nope now");
    enter(&mut app);
    assert!(app.running());
    assert_eq!(app.message.as_deref(), Some("not a command: nope now"));
    assert_eq!(app.status, None);
}

#[test]
fn a_bad_command_does_not_hide_why_kuna_failed() {
    let mut app = app();
    app.update(Msg::Functions(Err(kuna::Error::NotOnPath)))
        .expect("updates");
    let why = app.status.clone();
    type_text(&mut app, ":help");
    enter(&mut app);
    assert_eq!(app.message.as_deref(), Some("not a command: help"));
    assert_eq!(app.status, why);
    assert!(why.is_some());
}

#[test]
fn a_late_answer_from_kuna_does_not_erase_a_command_message() {
    let mut app = app();
    type_text(&mut app, ":nope");
    enter(&mut app);
    app.update(answer(&["main"], Some("repaired the header")))
        .expect("updates");
    assert_eq!(app.message.as_deref(), Some("not a command: nope"));
    assert_eq!(app.status.as_deref(), Some("repaired the header"));
}

#[test]
fn the_next_key_clears_the_message() {
    let mut app = app();
    type_text(&mut app, ":nope");
    enter(&mut app);
    assert!(app.message.is_some());
    type_text(&mut app, "j");
    assert_eq!(app.message, None);
}

#[test]
fn a_blank_command_says_nothing() {
    let mut app = app();
    type_text(&mut app, ":   ");
    enter(&mut app);
    assert_eq!(app.message, None);
    assert!(app.running());
}

#[test]
fn escape_closes_a_prompt_without_running_it() {
    let mut app = app();
    type_text(&mut app, ":q");
    app.update(press(KeyCode::Esc)).expect("updates");
    assert_eq!(app.keymap.prompt(), None);
    assert!(app.running());
}

#[test]
fn q_is_text_inside_a_prompt() {
    let mut app = app();
    type_text(&mut app, ":qq");
    assert!(app.running());
    enter(&mut app);
    assert_eq!(app.message.as_deref(), Some("not a command: qq"));
}

#[test]
fn ctrl_c_in_a_prompt_closes_it_and_a_second_one_quits() {
    let mut app = app();
    let ctrl_c = || {
        key(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
            KeyEventKind::Press,
        )
    };
    type_text(&mut app, ":ab");
    app.update(ctrl_c()).expect("updates");
    assert!(app.running());
    assert_eq!(app.keymap.prompt(), None);
    app.update(ctrl_c()).expect("updates");
    assert!(!app.running());
}

#[test]
fn motions_are_silent_until_the_views_exist() {
    let mut app = app();
    type_text(&mut app, "5jggGzzma'a");
    assert!(app.running());
    assert_eq!(
        (app.status.as_deref(), app.message.as_deref()),
        (None, None)
    );
    assert!(matches!(app.functions, Functions::Loading));
}

#[test]
fn searches_views_and_follow_say_they_are_not_available_yet() {
    let mut app = app();
    type_text(&mut app, "/foo");
    enter(&mut app);
    assert_eq!(app.message.as_deref(), Some("not available yet"));
    type_text(&mut app, "j");
    assert_eq!(app.message, None);
    type_text(&mut app, "gd");
    assert_eq!(app.message.as_deref(), Some("not available yet"));
    type_text(&mut app, "j");
    enter(&mut app);
    assert_eq!(app.message.as_deref(), Some("not available yet"));
    assert!(app.running());
}

#[test]
fn the_functions_fill_the_list_and_keep_the_version_warning() {
    let mut app = App::new(
        PathBuf::from("a"),
        "9".to_string(),
        Some("old kuna".to_string()),
    );
    app.update(answer(&["main", "add"], None)).expect("updates");
    assert!(matches!(&app.functions, Functions::Ready(list) if list.len() == 2));
    assert_eq!(app.warning.as_deref(), Some("old kuna"));
    assert_eq!(app.status, None);
}

#[test]
fn what_kuna_said_while_succeeding_does_not_replace_the_warning() {
    let mut app = App::new(
        PathBuf::from("a"),
        "9".to_string(),
        Some("old kuna".to_string()),
    );
    app.update(answer(&["main"], Some("repaired the header")))
        .expect("updates");
    assert_eq!(app.status.as_deref(), Some("repaired the header"));
    assert_eq!(app.warning.as_deref(), Some("old kuna"));
}

#[test]
fn a_failed_call_does_not_replace_the_warning() {
    let mut app = App::new(
        PathBuf::from("a"),
        "9".to_string(),
        Some("old kuna".to_string()),
    );
    app.update(Msg::Functions(Err(kuna::Error::Cancelled)))
        .expect("updates");
    assert_eq!(app.warning.as_deref(), Some("old kuna"));
}

#[test]
fn a_failed_call_shows_its_error() {
    let mut app = app();
    app.update(Msg::Functions(Err(kuna::Error::Cancelled)))
        .expect("updates");
    assert!(matches!(app.functions, Functions::Failed));
    assert_eq!(app.status.as_deref(), Some("the kuna call was cancelled"));
    assert!(app.running());
}

#[test]
fn a_terminal_that_cannot_be_read_is_an_error() {
    let mut app = app();
    let failure = Msg::Input(Err(io::Error::other("gone")));
    assert!(app.update(failure).is_err());
}

fn screen(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer();
    buffer.content().iter().map(|cell| cell.symbol()).collect()
}

fn terminal() -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(100, 3)).expect("a test terminal")
}

#[test]
fn the_loop_draws_each_answer_and_stops_on_q() {
    let (tx, rx) = mpsc::channel();
    tx.send(answer(&["main"], None)).expect("sends");
    tx.send(press(KeyCode::Char('q'))).expect("sends");
    let mut terminal = terminal();
    run(&mut terminal, &mut app(), &rx).expect("runs");
    let screen = screen(&terminal);
    assert!(
        screen.contains("functions: 1 · hello.elf · kuna 1.1005"),
        "{screen}"
    );
}

#[test]
fn the_loop_fails_when_every_sender_is_gone() {
    let (tx, rx) = mpsc::channel::<Msg>();
    drop(tx);
    let mut terminal = terminal();
    let error = run(&mut terminal, &mut app(), &rx).expect_err("fails");
    assert_eq!(error.to_string(), "the event sources stopped");
    assert!(screen(&terminal).contains("loading"));
}

#[test]
fn the_loop_returns_a_terminal_read_failure() {
    let (tx, rx) = mpsc::channel();
    tx.send(Msg::Input(Err(io::Error::other("gone"))))
        .expect("sends");
    let mut terminal = terminal();
    let error = run(&mut terminal, &mut app(), &rx).expect_err("fails");
    assert_eq!(error.to_string(), "cannot read the terminal: gone");
}

fn pinned_kuna() -> Kuna {
    let exe = if cfg!(windows) { "kuna.exe" } else { "kuna" };
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/kuna")
        .join(exe);
    let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hello.elf");
    kuna::locate(Some(&path), &sample).expect("the pinned kuna (scripts/kuna.sh)")
}

#[test]
fn the_worker_sends_the_functions_of_a_binary() {
    let binary = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hello.elf");
    let (tx, rx) = mpsc::channel();
    let worker = spawn_functions(Arc::new(pinned_kuna()), binary, tx).expect("spawns");
    worker.join().expect("finishes");
    let Msg::Functions(Ok(reply)) = rx.recv().expect("an answer") else {
        panic!("kuna should list the functions");
    };
    let names: Vec<&str> = reply.data.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["square", "add", "main", "_start"]);
}

#[test]
fn a_worker_whose_kuna_was_cancelled_reports_it() {
    let kuna = Arc::new(pinned_kuna());
    kuna.cancel();
    let (tx, rx) = mpsc::channel();
    spawn_functions(kuna, PathBuf::from("/nowhere"), tx)
        .expect("spawns")
        .join()
        .expect("finishes");
    assert!(matches!(
        rx.recv().expect("an answer"),
        Msg::Functions(Err(kuna::Error::Cancelled))
    ));
}
