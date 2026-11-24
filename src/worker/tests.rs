//! Tests of the kuna worker.

use super::*;
use std::sync::mpsc;

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
    spawn(Arc::new(pinned_kuna()), hello(), queue, tx)
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
    let worker = spawn(Arc::new(pinned_kuna()), hello(), queue, tx).expect("spawns");
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
    spawn(Arc::new(pinned_kuna()), hello(), queue, tx)
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
    let worker = spawn(Arc::clone(&kuna), hello(), queue, tx).expect("spawns");
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
    spawn(Arc::new(pinned_kuna()), hello(), queue, tx)
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
    spawn(kuna, PathBuf::from("/nowhere"), queue, tx)
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
