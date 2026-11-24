//! Tests for the app state, the loop and the worker.
use super::*;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use std::sync::mpsc;

/// An app and the queue its requests for the worker go to.
fn app_with(binary: &str, version: &str, warning: Option<&str>) -> (App, Receiver<Request>) {
    let (requests, queue) = mpsc::channel();
    let app = App::new(
        PathBuf::from(binary),
        version.to_string(),
        warning.map(str::to_string),
        requests,
    );
    (app, queue)
}

/// An app whose queue is gone: nothing it asks for is received.
fn app() -> App {
    app_with("/samples/hello.elf", "1.1005", None).0
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
fn motions_on_a_list_that_is_not_there_yet_are_silent() {
    let mut app = app();
    type_text(&mut app, "5jggGzz");
    assert!(app.running());
    assert_eq!(
        (app.status.as_deref(), app.message.as_deref()),
        (None, None)
    );
    assert!(matches!(app.functions, Functions::Loading));
}

#[test]
fn the_views_of_later_issues_say_they_are_not_available_yet() {
    let mut app = app();
    type_text(&mut app, "gr");
    assert_eq!(app.message.as_deref(), Some("not available yet"));
    assert!(app.running());
}

#[test]
fn a_view_with_no_function_to_show_says_why() {
    let mut app = app();
    type_text(&mut app, "gd");
    assert_eq!(
        app.message.as_deref(),
        Some("the functions are still loading")
    );
    app.update(Msg::Functions(Err(kuna::Error::Cancelled)))
        .expect("updates");
    enter(&mut app);
    assert_eq!(
        app.message.as_deref(),
        Some("the functions could not be listed")
    );
    let (mut app, _queue) = listed(&[]);
    enter(&mut app);
    assert_eq!(app.message.as_deref(), Some("no function selected"));
    assert!(app.shown.is_none());
}

#[test]
fn the_functions_fill_the_list_and_keep_the_version_warning() {
    let (mut app, _queue) = app_with("a", "9", Some("old kuna"));
    app.update(answer(&["main", "add"], None)).expect("updates");
    assert!(matches!(&app.functions, Functions::Ready(list) if list.len() == 2));
    assert_eq!(app.list.lines(), ["main", "add"]);
    assert_eq!(app.warning.as_deref(), Some("old kuna"));
    assert_eq!(app.status, None);
}

#[test]
fn what_kuna_said_while_succeeding_does_not_replace_the_warning() {
    let (mut app, _queue) = app_with("a", "9", Some("old kuna"));
    app.update(answer(&["main"], Some("repaired the header")))
        .expect("updates");
    assert_eq!(app.status.as_deref(), Some("repaired the header"));
    assert_eq!(app.warning.as_deref(), Some("old kuna"));
}

#[test]
fn a_failed_call_does_not_replace_the_warning() {
    let (mut app, _queue) = app_with("a", "9", Some("old kuna"));
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

const ALPHA: u64 = 0x10;
const BETA: u64 = 0x20;
const GAMMA: u64 = 0x30;

fn function_at(name: &str, address: u64) -> Function {
    let text = format!(r#"{{"name":"{name}","address":{address},"size":32}}"#);
    serde_json::from_str(&text).expect("a function")
}

/// An app after kuna listed `functions`, and its queue.
fn listed(functions: &[(&str, u64)]) -> (App, Receiver<Request>) {
    let (mut app, queue) = app_with("/samples/hello.elf", "1.1005", None);
    let data = functions
        .iter()
        .map(|&(name, address)| function_at(name, address))
        .collect();
    let reply = Reply { data, notes: None };
    app.update(Msg::Functions(Ok(reply))).expect("updates");
    (app, queue)
}

fn three() -> (App, Receiver<Request>) {
    listed(&[("alpha", ALPHA), ("beta", BETA), ("gamma", GAMMA)])
}

fn code(view: View, address: u64, lines: &[&str], notes: Option<&str>) -> Msg {
    Msg::Code {
        view,
        address,
        answer: Ok(Reply {
            data: lines.iter().map(|line| line.to_string()).collect(),
            notes: notes.map(str::to_string),
        }),
    }
}

fn failed(view: View, address: u64) -> Msg {
    Msg::Code {
        view,
        address,
        answer: Err(kuna::Error::Cancelled),
    }
}

fn ctrl(app: &mut App, c: char) {
    let press = key(KeyCode::Char(c), KeyModifiers::CONTROL, KeyEventKind::Press);
    app.update(press).expect("updates");
}

/// `Ctrl-w` and then `c`.
fn ctrl_w(app: &mut App, c: char) {
    ctrl(app, 'w');
    type_text(app, &c.to_string());
}

fn code_pane(app: &App) -> &Pane {
    match app.current() {
        Some((_, Loaded::Ready(pane, _))) => pane,
        _ => panic!("no code is shown"),
    }
}

fn asked(view: View, address: u64) -> Request {
    Request { view, address }
}

#[test]
fn j_and_k_move_the_selection_in_the_list() {
    let (mut app, _queue) = three();
    type_text(&mut app, "j");
    assert_eq!(app.list.line(), 1);
    type_text(&mut app, "G");
    assert_eq!(app.list.line(), 2);
    type_text(&mut app, "gg");
    assert_eq!(app.list.line(), 0);
    type_text(&mut app, "k");
    assert_eq!(app.list.line(), 0);
}

#[test]
fn enter_opens_the_selected_function_in_decompile_and_asks_for_it() {
    let (mut app, queue) = three();
    type_text(&mut app, "j");
    enter(&mut app);
    assert_eq!(queue.try_recv(), Ok(asked(View::Decompile, BETA)));
    assert_eq!(app.focus, Focus::Code);
    assert_eq!(app.loading().as_deref(), Some("decompiling beta…"));
    assert!(queue.try_recv().is_err());
}

#[test]
fn ga_opens_the_disassembly_and_gd_the_decompilation_of_the_selection() {
    let (mut app, queue) = three();
    type_text(&mut app, "ga");
    assert_eq!(queue.try_recv(), Ok(asked(View::Disassembly, ALPHA)));
    assert_eq!(app.loading().as_deref(), Some("disassembling alpha…"));
    ctrl_w(&mut app, 'h');
    type_text(&mut app, "jgd");
    assert_eq!(queue.try_recv(), Ok(asked(View::Decompile, BETA)));
}

#[test]
fn in_the_code_gd_and_ga_switch_the_view_of_the_function_shown() {
    let (mut app, queue) = three();
    type_text(&mut app, "j");
    enter(&mut app);
    queue.try_recv().expect("asked");
    type_text(&mut app, "ga");
    assert_eq!(queue.try_recv(), Ok(asked(View::Disassembly, BETA)));
    assert_eq!(app.loading().as_deref(), Some("disassembling beta…"));
}

#[test]
fn the_answer_replaces_loading_with_the_lines() {
    let (mut app, _queue) = three();
    enter(&mut app);
    app.update(code(
        View::Decompile,
        ALPHA,
        &["int f(void)", "{", "}"],
        None,
    ))
    .expect("updates");
    assert_eq!(app.loading(), None);
    assert_eq!(code_pane(&app).lines(), ["int f(void)", "{", "}"]);
}

#[test]
fn j_moves_the_pane_that_has_the_focus() {
    let (mut app, _queue) = three();
    enter(&mut app);
    app.update(code(View::Decompile, ALPHA, &["a", "b", "c"], None))
        .expect("updates");
    type_text(&mut app, "j");
    assert_eq!((code_pane(&app).line(), app.list.line()), (1, 0));
    ctrl_w(&mut app, 'h');
    type_text(&mut app, "j");
    assert_eq!((code_pane(&app).line(), app.list.line()), (1, 1));
}

#[test]
fn a_view_already_loaded_opens_again_without_asking() {
    let (mut app, queue) = three();
    enter(&mut app);
    queue.try_recv().expect("asked once");
    app.update(code(View::Decompile, ALPHA, &["x"], None))
        .expect("updates");
    ctrl_w(&mut app, 'h');
    enter(&mut app);
    assert!(queue.try_recv().is_err(), "it was cached");
    assert_eq!(app.focus, Focus::Code);
    assert_eq!(code_pane(&app).lines(), ["x"]);
}

#[test]
fn a_view_that_never_answered_is_asked_for_again() {
    let (mut app, queue) = three();
    enter(&mut app);
    ctrl_w(&mut app, 'h');
    enter(&mut app);
    assert_eq!(queue.try_recv(), Ok(asked(View::Decompile, ALPHA)));
    assert_eq!(queue.try_recv(), Ok(asked(View::Decompile, ALPHA)));
}

#[test]
fn a_failed_view_says_why_and_is_asked_for_again_when_reopened() {
    let (mut app, queue) = three();
    enter(&mut app);
    queue.try_recv().expect("asked");
    app.update(failed(View::Decompile, ALPHA)).expect("updates");
    assert_eq!(app.view_note(), Some("the kuna call was cancelled"));
    assert_eq!(app.status, None);
    assert!(matches!(
        app.current(),
        Some((_, Loaded::Failed(why))) if why == "the kuna call was cancelled"
    ));
    ctrl_w(&mut app, 'h');
    enter(&mut app);
    assert_eq!(queue.try_recv(), Ok(asked(View::Decompile, ALPHA)));
    assert_eq!(app.loading().as_deref(), Some("decompiling alpha…"));
}

#[test]
fn a_late_failure_does_not_replace_a_view_that_loaded() {
    let (mut app, _queue) = three();
    enter(&mut app);
    app.update(code(View::Decompile, ALPHA, &["a", "b", "c"], None))
        .expect("updates");
    type_text(&mut app, "jj");
    app.update(failed(View::Decompile, ALPHA)).expect("updates");
    assert_eq!(code_pane(&app).lines(), ["a", "b", "c"]);
    assert_eq!(code_pane(&app).line(), 2);
    assert_eq!(app.view_note(), None);
}

#[test]
fn an_answer_for_a_view_no_longer_shown_leaves_the_status_alone_and_is_kept() {
    let (mut app, _queue) = three();
    enter(&mut app);
    ctrl_w(&mut app, 'h');
    type_text(&mut app, "j");
    enter(&mut app);
    let late = code(View::Decompile, ALPHA, &["late"], Some("repaired"));
    app.update(late).expect("updates");
    assert_eq!((app.status.as_deref(), app.view_note()), (None, None));
    ctrl_w(&mut app, 'h');
    type_text(&mut app, "k");
    enter(&mut app);
    assert_eq!(code_pane(&app).lines(), ["late"]);
    assert_eq!(app.view_note(), Some("repaired"));
}

#[test]
fn the_notes_shown_are_those_of_the_view_shown() {
    let (mut app, _queue) = three();
    enter(&mut app);
    app.update(code(View::Decompile, ALPHA, &["a"], Some("cut short")))
        .expect("updates");
    assert_eq!(app.view_note(), Some("cut short"));
    ctrl_w(&mut app, 'h');
    type_text(&mut app, "j");
    enter(&mut app);
    assert_eq!(app.view_note(), None, "beta is still loading");
    app.update(code(View::Decompile, BETA, &["b"], None))
        .expect("updates");
    assert_eq!(app.view_note(), None);
}

#[test]
fn a_failure_of_a_view_no_longer_shown_does_not_reach_the_status() {
    let (mut app, _queue) = three();
    enter(&mut app);
    ctrl_w(&mut app, 'h');
    type_text(&mut app, "j");
    enter(&mut app);
    app.update(failed(View::Decompile, ALPHA)).expect("updates");
    assert_eq!((app.status.as_deref(), app.view_note()), (None, None));
}

#[test]
fn what_kuna_said_about_the_shown_view_is_its_note() {
    let (mut app, _queue) = three();
    enter(&mut app);
    app.update(code(View::Decompile, ALPHA, &["x"], Some("repaired")))
        .expect("updates");
    assert_eq!(app.view_note(), Some("repaired"));
    assert_eq!(app.status, None);
}

#[test]
fn a_view_without_notes_keeps_what_kuna_said_about_the_functions() {
    let (mut app, _queue) = app_with("a", "9", None);
    app.update(answer(&["main"], Some("repaired the header")))
        .expect("updates");
    enter(&mut app);
    app.update(code(View::Decompile, 4_198_720, &["x"], None))
        .expect("updates");
    assert_eq!(app.status.as_deref(), Some("repaired the header"));
}

#[test]
fn a_second_answer_does_not_move_the_cursor_the_user_is_reading_with() {
    let (mut app, _queue) = three();
    enter(&mut app);
    app.update(code(View::Decompile, ALPHA, &["a", "b", "c"], None))
        .expect("updates");
    type_text(&mut app, "jj");
    app.update(code(View::Decompile, ALPHA, &["new"], None))
        .expect("updates");
    assert_eq!(code_pane(&app).line(), 2);
    assert_eq!(code_pane(&app).lines(), ["a", "b", "c"]);
}

#[test]
fn ctrl_w_moves_the_focus_between_the_panes() {
    let (mut app, _queue) = three();
    enter(&mut app);
    for (key, want) in [
        ('h', Focus::List),
        ('l', Focus::Code),
        ('w', Focus::List),
        ('w', Focus::Code),
        ('W', Focus::List),
    ] {
        ctrl_w(&mut app, key);
        assert_eq!(app.focus, want, "after Ctrl-w {key}");
    }
}

#[test]
fn the_code_pane_cannot_take_the_focus_while_it_shows_nothing() {
    let (mut app, _queue) = three();
    ctrl_w(&mut app, 'l');
    assert_eq!(app.focus, Focus::List);
    assert_eq!(app.message.as_deref(), Some("no code open"));
}

#[test]
fn the_other_views_are_not_available_yet() {
    let (mut app, _queue) = three();
    type_text(&mut app, "gr");
    assert_eq!(app.message.as_deref(), Some("not available yet"));
}

#[test]
fn a_worker_that_is_gone_is_reported_not_waited_for() {
    let (mut app, queue) = three();
    drop(queue);
    enter(&mut app);
    assert_eq!(app.view_note(), Some("the kuna worker has stopped"));
    assert!(matches!(app.current(), Some((_, Loaded::Failed(_)))));
}

#[test]
fn gd_and_ga_in_the_code_use_the_function_shown_not_the_selection() {
    let (mut app, queue) = three();
    enter(&mut app);
    queue.try_recv().expect("asked");
    ctrl_w(&mut app, 'h');
    type_text(&mut app, "jj");
    ctrl_w(&mut app, 'l');
    type_text(&mut app, "ga");
    assert_eq!(queue.try_recv(), Ok(asked(View::Disassembly, ALPHA)));
}

#[test]
fn the_cursor_of_the_code_pane_stays_in_view_when_the_screen_shrinks() {
    let lines: Vec<String> = (0..30).map(|n| format!("line {n}")).collect();
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let (mut app, _queue) = three();
    app.set_rows(10);
    enter(&mut app);
    app.update(code(View::Decompile, ALPHA, &lines, None))
        .expect("updates");
    type_text(&mut app, "G");
    assert_eq!(code_pane(&app).top(), 20);
    app.set_rows(5);
    assert_eq!(code_pane(&app).top(), 25);
}

#[test]
fn a_view_left_in_the_background_fits_the_screen_when_it_is_shown_again() {
    let lines: Vec<String> = (0..30).map(|n| format!("line {n}")).collect();
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let (mut app, _queue) = three();
    app.set_rows(10);
    enter(&mut app);
    app.update(code(View::Decompile, ALPHA, &lines, None))
        .expect("updates");
    type_text(&mut app, "G");
    ctrl_w(&mut app, 'h');
    type_text(&mut app, "j");
    enter(&mut app);
    app.set_rows(5);
    ctrl_w(&mut app, 'h');
    type_text(&mut app, "k");
    enter(&mut app);
    assert_eq!((code_pane(&app).line(), code_pane(&app).top()), (29, 25));
}

#[test]
fn only_so_many_loaded_views_are_kept_and_the_one_shown_stays() {
    let names: Vec<(String, u64)> = (0..100).map(|n| (format!("f{n}"), n)).collect();
    let pairs: Vec<(&str, u64)> = names
        .iter()
        .map(|(name, at)| (name.as_str(), *at))
        .collect();
    let (mut app, _queue) = listed(&pairs);
    for at in 0..100 {
        enter(&mut app);
        app.update(code(View::Decompile, at, &["x"], None))
            .expect("updates");
        ctrl_w(&mut app, 'h');
        type_text(&mut app, "j");
    }
    assert!(app.views.len() <= MAX_VIEWS, "{}", app.views.len());
    ctrl_w(&mut app, 'l');
    assert_eq!(code_pane(&app).lines(), ["x"]);
}

#[test]
fn pages_use_the_height_of_the_panes() {
    let names: Vec<(String, u64)> = (0..100).map(|n| (format!("f{n}"), n)).collect();
    let pairs: Vec<(&str, u64)> = names
        .iter()
        .map(|(name, at)| (name.as_str(), *at))
        .collect();
    let (mut app, _queue) = listed(&pairs);
    app.set_rows(10);
    ctrl(&mut app, 'd');
    assert_eq!((app.list.line(), app.list.top()), (5, 5));
}

#[test]
fn a_smaller_screen_keeps_the_cursor_in_view() {
    let names: Vec<(String, u64)> = (0..30).map(|n| (format!("f{n}"), n)).collect();
    let pairs: Vec<(&str, u64)> = names
        .iter()
        .map(|(name, at)| (name.as_str(), *at))
        .collect();
    let (mut app, _queue) = listed(&pairs);
    app.set_rows(10);
    type_text(&mut app, "G");
    assert_eq!(app.list.top(), 20);
    app.set_rows(5);
    assert_eq!(app.list.top(), 25);
}

#[test]
fn the_loop_gives_the_app_the_height_of_its_panes() {
    let (tx, rx) = mpsc::channel();
    tx.send(press(KeyCode::Char('q'))).expect("sends");
    let mut terminal = Terminal::new(TestBackend::new(100, 12)).expect("a test terminal");
    let mut app = app();
    run(&mut terminal, &mut app, &rx).expect("runs");
    // 12 rows less the 2-row status, less the 2 border rows.
    assert_eq!(app.rows, 8);
}

fn function_with(name: &str, address: u64, size: u64, aliases: &[&str]) -> Function {
    let aliases = serde_json::to_string(aliases).expect("json");
    let text =
        format!(r#"{{"name":"{name}","address":{address},"size":{size},"aliases":{aliases}}}"#);
    serde_json::from_str(&text).expect("a function")
}

fn found(word: &str) -> Option<u64> {
    let list = [
        function_with("main", 0x100, 16, &["entry"]),
        function_with("tail", 0x110, 0, &[]),
    ];
    function_named(&list, word, true).map(|function| function.address)
}

#[test]
fn a_function_is_found_by_name_alias_or_any_address_inside_it() {
    assert_eq!(found("main"), Some(0x100));
    assert_eq!(found("entry"), Some(0x100));
    assert_eq!(found("0x100"), Some(0x100));
    assert_eq!(found("0x10f"), Some(0x100));
    assert_eq!(found("0X10F"), Some(0x100));
    assert_eq!(
        found("0x110"),
        Some(0x110),
        "a size of 0 still has its entry"
    );
    for word in ["0x111", "0xzz", "0x", "nope", ""] {
        assert_eq!(found(word), None, "{word:?}");
    }
}

#[test]
fn an_address_is_the_function_that_starts_there_before_one_that_contains_it() {
    let list = [
        function_with("wide", 0x100, 0x100, &[]),
        function_with("inner", 0x120, 8, &[]),
    ];
    let name = |word| function_named(&list, word, true).map(|function| function.name.as_str());
    assert_eq!(name("0x120"), Some("inner"));
}

#[test]
fn an_address_inside_a_function_is_found_only_when_asked_for() {
    let list = [function_with("main", 0x100, 16, &[])];
    assert_eq!(
        function_named(&list, "0x104", true).map(|f| f.address),
        Some(0x100)
    );
    assert!(function_named(&list, "0x104", false).is_none());
    assert!(function_named(&list, "0x100", false).is_some());
    assert!(function_named(&list, "main", false).is_some());
}

fn colon(app: &mut App, line: &str) {
    type_text(app, &format!(":{line}"));
    enter(app);
}

/// alpha open in the code pane with `lines`, asked for and answered.
fn alpha_open(lines: &[&str]) -> (App, Receiver<Request>) {
    let (mut app, queue) = three();
    enter(&mut app);
    queue.try_recv().expect("asked");
    app.update(code(View::Decompile, ALPHA, lines, None))
        .expect("updates");
    (app, queue)
}

fn shown_name(app: &App) -> Option<&str> {
    app.shown.as_ref().map(|shown| shown.name.as_str())
}

#[test]
fn enter_on_a_function_name_in_the_code_opens_that_function() {
    let (mut app, queue) = alpha_open(&["beta(1);"]);
    enter(&mut app);
    assert_eq!(queue.try_recv(), Ok(asked(View::Decompile, BETA)));
    assert_eq!(app.loading().as_deref(), Some("decompiling beta…"));
}

#[test]
fn enter_on_an_address_in_the_disassembly_stays_in_the_disassembly() {
    let (mut app, queue) = three();
    type_text(&mut app, "ga");
    queue.try_recv().expect("asked");
    let listing = code(View::Disassembly, ALPHA, &["0x10  CALL 0x20"], None);
    app.update(listing).expect("updates");
    type_text(&mut app, "ww");
    enter(&mut app);
    assert_eq!(queue.try_recv(), Ok(asked(View::Disassembly, BETA)));
}

#[test]
fn enter_on_something_that_is_no_function_says_so_and_asks_for_nothing() {
    let (mut app, queue) = alpha_open(&["int v1;", "{"]);
    enter(&mut app);
    assert_eq!(app.message.as_deref(), Some("no function: int"));
    type_text(&mut app, "j");
    enter(&mut app);
    assert_eq!(app.message.as_deref(), Some("no word under the cursor"));
    assert!(queue.try_recv().is_err());
}

#[test]
fn enter_follows_only_an_entry_address_but_colon_e_takes_any_address_in_a_function() {
    let (mut app, queue) = alpha_open(&["0x34"]);
    enter(&mut app);
    assert_eq!(app.message.as_deref(), Some("no function: 0x34"));
    assert!(queue.try_recv().is_err());
    colon(&mut app, "e 0x34");
    assert_eq!(queue.try_recv(), Ok(asked(View::Decompile, BETA)));
}

#[test]
fn follow_and_marks_wait_for_the_code_to_load() {
    let (mut app, _queue) = three();
    enter(&mut app);
    enter(&mut app);
    assert_eq!(app.message.as_deref(), Some("nothing to follow yet"));
    type_text(&mut app, "ma");
    assert_eq!(app.message.as_deref(), Some("nothing to mark yet"));
}

#[test]
fn colon_e_jumps_to_a_function_by_name_or_address() {
    let (mut app, queue) = three();
    colon(&mut app, "e beta");
    assert_eq!(queue.try_recv(), Ok(asked(View::Decompile, BETA)));
    assert_eq!(app.focus, Focus::Code);
    assert_eq!(app.loading().as_deref(), Some("decompiling beta…"));
    colon(&mut app, "edit 0x30");
    assert_eq!(queue.try_recv(), Ok(asked(View::Decompile, GAMMA)));
}

#[test]
fn colon_e_with_nothing_to_open_says_why() {
    let mut early = app();
    colon(&mut early, "e main");
    assert_eq!(
        early.message.as_deref(),
        Some("the functions are still loading")
    );
    let (mut app, queue) = three();
    colon(&mut app, "e nope");
    assert_eq!(app.message.as_deref(), Some("no function: nope"));
    colon(&mut app, "e");
    assert_eq!(
        app.message.as_deref(),
        Some(":e needs a name or an address")
    );
    assert!(queue.try_recv().is_err());
}

#[test]
fn a_jump_with_g_is_walked_back_and_forth_with_ctrl_o_and_ctrl_i() {
    let (mut app, _queue) = alpha_open(&["a", "b", "c", "d", "e"]);
    type_text(&mut app, "jjG");
    assert_eq!(code_pane(&app).line(), 4);
    ctrl(&mut app, 'o');
    assert_eq!(code_pane(&app).line(), 2);
    ctrl(&mut app, 'i');
    assert_eq!(code_pane(&app).line(), 4);
}

#[test]
fn ordinary_moves_are_not_jumps_and_an_empty_jumplist_does_nothing() {
    let mut nothing_open = app();
    ctrl(&mut nothing_open, 'o');
    assert_eq!(nothing_open.message, None);
    let (mut app, _queue) = alpha_open(&["a", "b", "c"]);
    type_text(&mut app, "jj");
    ctrl(&mut app, 'o');
    assert_eq!((code_pane(&app).line(), app.message.as_deref()), (2, None));
    ctrl(&mut app, 'i');
    assert_eq!(code_pane(&app).line(), 2);
}

#[test]
fn ctrl_o_returns_to_the_function_left_and_ctrl_i_to_the_one_followed() {
    let (mut app, queue) = alpha_open(&["a", "b", "c"]);
    type_text(&mut app, "jj");
    colon(&mut app, "e beta");
    assert_eq!(queue.try_recv(), Ok(asked(View::Decompile, BETA)));
    app.update(code(View::Decompile, BETA, &["x", "y"], None))
        .expect("updates");
    ctrl(&mut app, 'o');
    assert_eq!(
        (shown_name(&app), code_pane(&app).line()),
        (Some("alpha"), 2)
    );
    assert!(queue.try_recv().is_err(), "alpha was loaded already");
    ctrl(&mut app, 'i');
    assert_eq!(shown_name(&app), Some("beta"));
}

#[test]
fn a_new_jump_after_ctrl_o_forgets_the_way_forward() {
    let (mut app, _queue) = alpha_open(&["a"]);
    colon(&mut app, "e beta");
    app.update(code(View::Decompile, BETA, &["b"], None))
        .expect("updates");
    ctrl(&mut app, 'o');
    colon(&mut app, "e gamma");
    app.update(code(View::Decompile, GAMMA, &["g"], None))
        .expect("updates");
    ctrl(&mut app, 'i');
    assert_eq!(shown_name(&app), Some("gamma"));
    ctrl(&mut app, 'o');
    assert_eq!(shown_name(&app), Some("alpha"));
}

#[test]
fn opening_from_the_list_leaves_the_view_shown_in_the_jumplist() {
    let (mut app, _queue) = alpha_open(&["a", "b", "c"]);
    type_text(&mut app, "jj");
    ctrl_w(&mut app, 'h');
    type_text(&mut app, "j");
    enter(&mut app);
    assert_eq!(shown_name(&app), Some("beta"));
    ctrl(&mut app, 'o');
    assert_eq!(
        (shown_name(&app), code_pane(&app).line()),
        (Some("alpha"), 2)
    );
}

fn at(app: &App) -> (usize, usize) {
    (code_pane(app).line(), code_pane(app).col())
}

#[test]
fn a_mark_returns_to_its_line_and_with_a_backtick_to_its_column() {
    let (mut app, _queue) = alpha_open(&["a", "  bb", "c", "d"]);
    type_text(&mut app, "jlmaG'a");
    assert_eq!(at(&app), (1, 2));
    type_text(&mut app, "G`a");
    assert_eq!(at(&app), (1, 1));
}

#[test]
fn a_jump_to_a_mark_can_be_undone_with_ctrl_o() {
    let (mut app, _queue) = alpha_open(&["a", "b", "c", "d"]);
    type_text(&mut app, "maG'a");
    assert_eq!(code_pane(&app).line(), 0);
    ctrl(&mut app, 'o');
    assert_eq!(code_pane(&app).line(), 3);
}

#[test]
fn a_lower_case_mark_belongs_to_its_view_and_an_upper_case_one_to_the_session() {
    let (mut app, _queue) = alpha_open(&["a", "b", "c"]);
    type_text(&mut app, "jmajmB");
    colon(&mut app, "e beta");
    app.update(code(View::Decompile, BETA, &["x"], None))
        .expect("updates");
    type_text(&mut app, "'a");
    assert_eq!(app.message.as_deref(), Some("mark not set: a"));
    type_text(&mut app, "'B");
    assert_eq!(
        (shown_name(&app), code_pane(&app).line()),
        (Some("alpha"), 2)
    );
}

#[test]
fn marks_are_for_the_code_pane() {
    let (mut app, _queue) = three();
    for keys in ["ma", "'a"] {
        type_text(&mut app, keys);
        assert_eq!(
            app.message.as_deref(),
            Some("marks are for the code pane"),
            "{keys}"
        );
    }
}

fn slash(app: &mut App, keys: &str) {
    type_text(app, keys);
    enter(app);
}

const SEARCHED: &[&str] = &["foo bar", "baz foo", "Foo foobar"];

#[test]
fn slash_jumps_to_the_next_match_and_n_and_capital_n_walk_on_and_back() {
    let (mut app, _queue) = alpha_open(SEARCHED);
    slash(&mut app, "/foo");
    assert_eq!(at(&app), (1, 4));
    type_text(&mut app, "n");
    assert_eq!(at(&app), (2, 0));
    type_text(&mut app, "N");
    assert_eq!(at(&app), (1, 4));
}

#[test]
fn a_search_that_wraps_says_so() {
    let (mut app, _queue) = alpha_open(SEARCHED);
    slash(&mut app, "/bar");
    assert_eq!((at(&app), app.message.as_deref()), ((0, 4), None));
    type_text(&mut app, "G");
    slash(&mut app, "/baz");
    assert_eq!(at(&app), (1, 0));
    assert_eq!(
        app.message.as_deref(),
        Some("search hit BOTTOM, continuing at TOP")
    );
    type_text(&mut app, "gg");
    slash(&mut app, "?foo");
    assert_eq!(at(&app), (2, 4));
    assert_eq!(
        app.message.as_deref(),
        Some("search hit TOP, continuing at BOTTOM")
    );
}

#[test]
fn question_mark_searches_backward_and_n_keeps_that_direction() {
    let (mut app, _queue) = alpha_open(SEARCHED);
    slash(&mut app, "?foo");
    assert_eq!(at(&app), (2, 4));
    type_text(&mut app, "n");
    assert_eq!(at(&app), (2, 0));
    type_text(&mut app, "N");
    assert_eq!(at(&app), (2, 4));
}

#[test]
fn star_and_hash_search_the_whole_word_under_the_cursor_case_sensitive() {
    let (mut app, _queue) = alpha_open(SEARCHED);
    type_text(&mut app, "*");
    assert_eq!(at(&app), (1, 4));
    type_text(&mut app, "n");
    assert_eq!(at(&app), (0, 0));
    type_text(&mut app, "#");
    assert_eq!(at(&app), (1, 4));
}

#[test]
fn a_search_that_finds_nothing_says_so_and_stays_put() {
    let (mut app, _queue) = alpha_open(SEARCHED);
    slash(&mut app, "/zzz");
    assert_eq!(app.message.as_deref(), Some("not found: zzz"));
    assert_eq!(at(&app), (0, 0));
    type_text(&mut app, "n");
    assert_eq!(app.message.as_deref(), Some("not found: zzz"));
}

#[test]
fn n_with_no_search_and_star_with_no_word_say_so() {
    let (mut app, _queue) = alpha_open(&["  ", "x"]);
    type_text(&mut app, "n");
    assert_eq!(app.message.as_deref(), Some("no previous search"));
    type_text(&mut app, "*");
    assert_eq!(app.message.as_deref(), Some("no word under the cursor"));
}

#[test]
fn a_search_in_the_code_leaves_its_line_in_the_jumplist() {
    let (mut app, _queue) = alpha_open(SEARCHED);
    slash(&mut app, "/baz");
    assert_eq!(at(&app), (1, 0));
    ctrl(&mut app, 'o');
    assert_eq!(code_pane(&app).line(), 0);
}

#[test]
fn a_search_in_the_list_moves_the_selection() {
    let (mut app, _queue) = three();
    slash(&mut app, "/gam");
    assert_eq!(app.list.line(), 2);
    type_text(&mut app, "N");
    assert_eq!(app.list.line(), 2);
    slash(&mut app, "?alp");
    assert_eq!(app.list.line(), 0);
}

#[test]
fn star_and_hash_start_from_the_beginning_of_the_word() {
    let (mut app, _queue) = alpha_open(&["foo bar foo", "a  foo foo"]);
    type_text(&mut app, "l#");
    assert_eq!(at(&app), (1, 7));
    assert_eq!(
        app.message.as_deref(),
        Some("search hit TOP, continuing at BOTTOM")
    );
    type_text(&mut app, "0l*");
    assert_eq!(at(&app), (1, 7));
}

#[test]
fn the_last_search_is_shared_by_both_panes() {
    let (mut app, _queue) = alpha_open(&["x", "gam", "y"]);
    ctrl_w(&mut app, 'h');
    slash(&mut app, "/gam");
    assert_eq!(app.list.line(), 2);
    ctrl_w(&mut app, 'l');
    type_text(&mut app, "n");
    assert_eq!(at(&app), (1, 0));
}

#[test]
fn searching_code_that_has_not_loaded_says_so() {
    let (mut app, _queue) = three();
    enter(&mut app);
    slash(&mut app, "/foo");
    assert_eq!(app.message.as_deref(), Some("nothing to search yet"));
}
