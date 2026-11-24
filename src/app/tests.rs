//! Tests for the app state, the loop and the worker.
use super::*;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

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

fn hello() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hello.elf")
}

fn request(view: View, address: u64) -> Request {
    Request { view, address }
}

/// The lines of the next `Msg::Code` answer, which must be a success for `view` at `address`.
fn next_lines(rx: &Receiver<Msg>, view: View, address: u64) -> Vec<String> {
    match rx.recv().expect("an answer") {
        Msg::Code {
            view: got,
            address: at,
            answer: Ok(reply),
        } if (got, at) == (view, address) => reply.data,
        Msg::Code { answer, .. } => panic!("not the answer asked for: {answer:?}"),
        _ => panic!("expected a code answer"),
    }
}

#[test]
fn the_worker_sends_the_functions_of_a_binary() {
    let (tx, rx) = mpsc::channel();
    let (requests, queue) = mpsc::channel();
    drop(requests);
    spawn_worker(Arc::new(pinned_kuna()), hello(), queue, tx)
        .expect("spawns")
        .join()
        .expect("finishes");
    let Msg::Functions(Ok(reply)) = rx.recv().expect("an answer") else {
        panic!("kuna should list the functions");
    };
    let names: Vec<&str> = reply.data.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["square", "add", "main", "_start"]);
    assert!(rx.try_recv().is_err(), "nothing was asked for");
}

#[test]
fn the_worker_decompiles_and_disassembles_on_request() {
    let (tx, rx) = mpsc::channel();
    let (requests, queue) = mpsc::channel();
    let worker = spawn_worker(Arc::new(pinned_kuna()), hello(), queue, tx).expect("spawns");
    assert!(matches!(rx.recv(), Ok(Msg::Functions(Ok(_)))));
    requests
        .send(request(View::Decompile, 0x201140))
        .expect("sends");
    let code = next_lines(&rx, View::Decompile, 0x201140);
    assert!(
        code.iter().any(|line| line.contains("square(3)")),
        "{code:?}"
    );
    requests
        .send(request(View::Disassembly, 0x201140))
        .expect("sends");
    let code = next_lines(&rx, View::Disassembly, 0x201140);
    assert!(
        code.iter().any(|line| line.ends_with("CALL 0x201120")),
        "{code:?}"
    );
    drop(requests);
    worker.join().expect("ends when nobody can ask any more");
}

#[test]
fn of_the_requests_queued_only_the_newest_of_each_view_is_answered() {
    let (tx, rx) = mpsc::channel();
    let (requests, queue) = mpsc::channel();
    for (view, address) in [
        (View::Decompile, 0x201140),
        (View::Disassembly, 0x201140),
        (View::Decompile, 0x201120),
    ] {
        requests.send(request(view, address)).expect("sends");
    }
    drop(requests);
    spawn_worker(Arc::new(pinned_kuna()), hello(), queue, tx)
        .expect("spawns")
        .join()
        .expect("finishes");
    assert!(matches!(rx.recv(), Ok(Msg::Functions(Ok(_)))));
    let listing = next_lines(&rx, View::Disassembly, 0x201140);
    assert!(
        listing.iter().any(|line| line.ends_with("CALL 0x201120")),
        "{listing:?}"
    );
    let code = next_lines(&rx, View::Decompile, 0x201120);
    assert!(code.iter().any(|line| line.contains("square")), "{code:?}");
    assert!(rx.try_recv().is_err(), "the older decompile was dropped");
}

#[test]
fn an_idle_worker_ends_once_kuna_is_cancelled_even_while_a_sender_lives() {
    let kuna = Arc::new(pinned_kuna());
    let (tx, rx) = mpsc::channel();
    let (requests, queue) = mpsc::channel::<Request>();
    let worker = spawn_worker(Arc::clone(&kuna), hello(), queue, tx).expect("spawns");
    assert!(matches!(rx.recv(), Ok(Msg::Functions(Ok(_)))));
    kuna.cancel();
    let (done, finished) = mpsc::channel();
    thread::spawn(move || {
        worker.join().expect("finishes");
        let _ = done.send(());
    });
    finished
        .recv_timeout(Duration::from_secs(10))
        .expect("the worker should end once kuna is cancelled");
    drop(requests);
}

#[test]
fn a_view_kuna_cannot_give_yet_is_an_error_not_silence() {
    let (tx, rx) = mpsc::channel();
    let (requests, queue) = mpsc::channel();
    requests.send(request(View::Hex, 0x201140)).expect("sends");
    drop(requests);
    spawn_worker(Arc::new(pinned_kuna()), hello(), queue, tx)
        .expect("spawns")
        .join()
        .expect("finishes");
    assert!(matches!(rx.recv(), Ok(Msg::Functions(_))));
    let Ok(Msg::Code {
        view: View::Hex,
        answer: Err(kuna::Error::Reported(message)),
        ..
    }) = rx.recv()
    else {
        panic!("expected an error answer for the Hex view");
    };
    assert!(message.contains("Hex"), "{message}");
}

#[test]
fn a_worker_whose_kuna_was_cancelled_reports_it_for_every_request() {
    let kuna = Arc::new(pinned_kuna());
    kuna.cancel();
    let (tx, rx) = mpsc::channel();
    let (requests, queue) = mpsc::channel();
    requests
        .send(request(View::Decompile, 0x201140))
        .expect("sends");
    drop(requests);
    spawn_worker(kuna, PathBuf::from("/nowhere"), queue, tx)
        .expect("spawns")
        .join()
        .expect("finishes");
    assert!(matches!(
        rx.recv().expect("an answer"),
        Msg::Functions(Err(kuna::Error::Cancelled))
    ));
    assert!(matches!(
        rx.recv().expect("an answer"),
        Msg::Code {
            answer: Err(kuna::Error::Cancelled),
            ..
        }
    ));
}

fn decompiled(code: &str) -> kuna::Decompiled {
    let text = serde_json::json!({
        "name": "f", "address": 16, "code": code, "error": null, "warnings": []
    });
    serde_json::from_value(text).expect("a decompiled function")
}

fn decompile(code: &str, notes: Option<&str>) -> Result<Reply<Vec<String>>, kuna::Error> {
    decompile_reply(Reply {
        data: decompiled(code),
        notes: notes.map(str::to_string),
    })
}

#[test]
fn decompiled_code_is_split_into_lines_with_tabs_made_wide() {
    let reply = decompile("int f(void)\n{\n\treturn 1;\n}", None).expect("lines");
    assert_eq!(reply.data, ["int f(void)", "{", "    return 1;", "}"]);
    assert_eq!(reply.notes, None);
}

#[test]
fn what_kuna_said_while_decompiling_survives() {
    let reply = decompile("int f(void);", Some("repaired the header")).expect("lines");
    assert_eq!(reply.notes.as_deref(), Some("repaired the header"));
}

#[test]
fn a_decompiled_function_without_code_is_an_error_not_a_blank_pane() {
    let text = r#"{"name":"f","address":16,"code":null,"error":null,"warnings":[]}"#;
    let function: kuna::Decompiled = serde_json::from_str(text).expect("parses");
    let error = decompile_reply(Reply {
        data: function,
        notes: None,
    })
    .expect_err("an error");
    assert!(error.to_string().contains("no code"), "{error}");
}

#[test]
fn more_lines_than_the_cap_are_cut_and_said_so() {
    let (lines, cut) = bounded((0..MAX_LINES + 5).map(|n| n.to_string()));
    assert_eq!((lines.len(), cut), (MAX_LINES, true));
    let (lines, cut) = bounded((0..MAX_LINES).map(|n| n.to_string()));
    assert_eq!((lines.len(), cut), (MAX_LINES, false));
}

#[test]
fn a_line_longer_than_the_cap_is_cut_by_characters() {
    let long = "é".repeat(MAX_LINE_CHARS + 10);
    let (lines, cut) = bounded([long, "short".to_string()].into_iter());
    assert_eq!(lines[0].chars().count(), MAX_LINE_CHARS);
    assert_eq!((lines[1].as_str(), cut), ("short", true));
}

#[test]
fn a_cut_decompilation_says_so_in_the_notes() {
    let reply = decompile(&"x".repeat(MAX_LINE_CHARS + 1), Some("repaired")).expect("lines");
    let notes = reply.notes.expect("notes");
    assert!(
        notes.contains("repaired") && notes.contains("cut"),
        "{notes}"
    );
}

#[test]
fn a_panic_in_a_call_becomes_an_error_that_says_what_it_was() {
    let error = guarded::<()>(|| panic!("boom")).expect_err("an error");
    assert!(error.to_string().contains("boom"), "{error}");
}

#[test]
fn a_failed_code_answer_is_shown_in_the_status() {
    let mut app = app();
    app.update(Msg::Code {
        view: View::Decompile,
        address: 1,
        answer: Err(kuna::Error::Cancelled),
    })
    .expect("updates");
    assert_eq!(app.status.as_deref(), Some("the kuna call was cancelled"));
}

#[test]
fn a_good_code_answer_keeps_what_kuna_said_about_the_functions() {
    let mut app = app();
    app.update(answer(&["main"], Some("repaired the header")))
        .expect("updates");
    app.update(Msg::Code {
        view: View::Decompile,
        address: 1,
        answer: Ok(Reply {
            data: vec!["int f(void);".to_string()],
            notes: None,
        }),
    })
    .expect("updates");
    assert_eq!(app.status.as_deref(), Some("repaired the header"));
}

fn disassembly(truncated: bool, notes: &[&str]) -> kuna::Disassembly {
    let text = serde_json::json!({
        "target": {"name": "main", "address": 16},
        "start": 16, "end": 24, "truncated": truncated, "notes": notes,
        "instructions": [
            {"address": 16, "bytes": "50", "mnemonic": "PUSH", "operands": "RAX", "text": "PUSH RAX"},
            {"address": 17, "bytes": "c3", "mnemonic": "RET", "operands": "", "text": "RET"}
        ]
    });
    serde_json::from_value(text).expect("a disassembly")
}

#[test]
fn a_disassembly_is_one_line_per_instruction_with_its_address() {
    let reply = disassembly_reply(Reply {
        data: disassembly(false, &[]),
        notes: None,
    });
    assert_eq!(reply.data, ["0x10  PUSH RAX", "0x11  RET"]);
    assert_eq!(reply.notes, None);
}

#[test]
fn a_cut_listing_and_kunas_notes_reach_the_status() {
    let reply = disassembly_reply(Reply {
        data: disassembly(true, &["no function here"]),
        notes: Some("repaired".to_string()),
    });
    let notes = reply.notes.expect("notes");
    for part in ["repaired", "no function here", "0x18"] {
        assert!(notes.contains(part), "{notes}");
    }
}
