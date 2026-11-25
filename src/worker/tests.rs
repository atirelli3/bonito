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
            ..
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
fn the_worker_answers_call_graph_requests_for_two_functions() {
    let (tx, rx) = mpsc::channel();
    let (requests, queue) = mpsc::channel();
    let worker = spawn(Arc::new(pinned_kuna()), hello(), queue, tx).expect("spawns");
    assert!(matches!(rx.recv(), Ok(Msg::Functions(Ok(_)))));
    for (address, title) in [(0x201140, "main  0x201140"), (0x201120, "square  0x201120")] {
        requests
            .send(request(View::CallGraph, address))
            .expect("sends");
        let lines = next_lines(&rx, View::CallGraph, address);
        assert_eq!(lines[0], title);
    }
    drop(requests);
    worker.join().expect("finishes");
}

fn xrefs_of(text: &str) -> Reply<Vec<String>> {
    let document: kuna::Xrefs = serde_json::from_str(text).expect("parses");
    xrefs_reply(Reply {
        data: document,
        notes: None,
    })
}

#[test]
fn xrefs_are_one_line_per_reference_with_the_instruction() {
    let reply = xrefs_of(include_str!("../../tests/fixtures/xrefs-square.json"));
    assert_eq!(
        reply.data,
        [
            "0x201146  call  main  CALL 0x201120",
            "0x201166  call  _start  CALL 0x201120"
        ]
    );
    assert_eq!(reply.notes, None);
}

#[test]
fn a_function_nobody_references_says_so() {
    let reply = xrefs_of(
        r#"{"target":{"name":"orphan","address":16},"coverage":{"decoded":true,"notes":[]},
        "data_refs_coverage":"none","xrefs":[],"data_refs":[]}"#,
    );
    assert_eq!(reply.data, ["no references to orphan"]);
}

#[test]
fn stored_pointers_are_listed_and_partial_coverage_is_said() {
    let reply = xrefs_of(
        r#"{"target":{"name":"f","address":16},"coverage":{"decoded":true,"notes":["not decoded"]},
        "data_refs_coverage":"partial","data_refs_reason":"some relocations unreadable",
        "xrefs":[{"from_address":32,"kind":"call","instruction":"CALL 0x10","from_function":null}],
        "data_refs":[{"kind":"pointer","from_address":4096,"from_name":null}]}"#,
    );
    assert_eq!(
        reply.data,
        ["0x20  call  ?  CALL 0x10", "0x1000  pointer  ?"]
    );
    assert_eq!(
        reply.notes.as_deref(),
        Some("not decoded · stored pointers partial: some relocations unreadable")
    );
}

#[test]
fn only_an_incomplete_pointer_list_is_said() {
    let notes = |coverage: &str| {
        let text = format!(
            r#"{{"target":{{"name":"f","address":16}},"coverage":{{"decoded":true,"notes":[]}},
            "data_refs_coverage":"{coverage}","data_refs_reason":"why","xrefs":[],"data_refs":[]}}"#
        );
        xrefs_of(&text).notes
    };
    assert_eq!(
        notes("unsupported").as_deref(),
        Some("stored pointers unsupported: why")
    );
    assert_eq!(notes("none"), None);
    assert_eq!(notes("dynamic-relocations"), None);
    assert_eq!(
        notes("novel").as_deref(),
        Some("stored pointers novel: why")
    );
}

/// The reply for a target with `coverage` (the JSON of the `coverage` object) and no references.
fn empty_xrefs(target: &str, coverage: &str) -> Reply<Vec<String>> {
    xrefs_of(&format!(
        r#"{{"target":{target},"coverage":{coverage},"data_refs_coverage":"none","xrefs":[],"data_refs":[]}}"#
    ))
}

#[test]
fn a_target_kuna_did_not_decode_is_not_reported_as_unreferenced() {
    let reply = empty_xrefs(
        r#"{"name":null,"address":0}"#,
        r#"{"decoded":false,"notes":[]}"#,
    );
    assert_eq!(
        reply.data,
        ["no references found to 0x0 (see the status line)"]
    );
    assert_eq!(
        reply.notes.as_deref(),
        Some("the target was not decoded: no code reference can be found")
    );
}

#[test]
fn code_kuna_did_not_walk_is_said_because_callers_may_be_missing() {
    let reply = empty_xrefs(
        r#"{"name":"f","address":16}"#,
        r#"{"decoded":true,"unwalked":[{"a":1},"b"],"notes":[]}"#,
    );
    assert_eq!(
        reply.data,
        ["no references found to f (see the status line)"]
    );
    assert_eq!(
        reply.notes.as_deref(),
        Some("2 regions were not decoded: references may be missing")
    );
}

#[test]
fn an_empty_answer_with_a_short_pointer_list_is_not_reported_as_unreferenced() {
    let reply = xrefs_of(
        r#"{"target":{"name":"f","address":16},"coverage":{"decoded":true,"notes":[]},
        "data_refs_coverage":"partial","xrefs":[],"data_refs":[]}"#,
    );
    assert_eq!(
        reply.data,
        ["no references found to f (see the status line)"]
    );
}

#[test]
fn a_nameless_target_is_named_by_its_address() {
    let reply = empty_xrefs(
        r#"{"name":null,"address":16}"#,
        r#"{"decoded":true,"notes":[]}"#,
    );
    assert_eq!(reply.data, ["no references to 0x10"]);
    assert_eq!(reply.notes, None);
}

#[test]
fn a_short_pointer_list_without_a_reason_is_still_said() {
    let reply = xrefs_of(
        r#"{"target":{"name":"f","address":16},"coverage":{"decoded":true,"notes":[]},
        "data_refs_coverage":"partial","xrefs":[],"data_refs":[]}"#,
    );
    assert_eq!(reply.notes.as_deref(), Some("stored pointers partial"));
}

#[test]
fn a_coverage_note_kuna_also_printed_on_stderr_is_said_once() {
    let document: kuna::Xrefs = serde_json::from_str(
        r#"{"target":{"name":"f","address":16},"coverage":{"decoded":true,"notes":["no walk"]},
        "data_refs_coverage":"none","xrefs":[],"data_refs":[]}"#,
    )
    .expect("parses");
    let reply = xrefs_reply(Reply {
        data: document,
        notes: Some("no walk".to_string()),
    });
    assert_eq!(reply.notes.as_deref(), Some("no walk"));
}

#[test]
fn a_flood_of_notes_is_capped() {
    let joined = join_notes((0..1000).map(|n| format!("note {n}")), false).expect("notes");
    assert!(
        joined.contains("note 9") && !joined.contains("note 10 "),
        "{joined}"
    );
    assert!(joined.contains("more than 10 notes"), "{joined}");
    assert!(joined.len() < 500, "{} bytes", joined.len());
}

fn hex_of(text: &str) -> Reply<Vec<String>> {
    let dump: kuna::Hexdump = serde_json::from_str(text).expect("parses");
    hex_reply(Reply {
        data: dump,
        notes: None,
    })
}

fn pcode_of(text: &str) -> Reply<Vec<String>> {
    let listing: kuna::Pcode = serde_json::from_str(text).expect("parses");
    pcode_reply(Reply {
        data: listing,
        notes: None,
    })
}

#[test]
fn a_hexdump_is_one_line_per_row_with_spaced_bytes_and_the_text() {
    let reply = hex_of(include_str!("../../tests/fixtures/read-main.json"));
    assert_eq!(
        reply.data,
        [
            "0x201140  50 bf 03 00 00 00 e8 d5 ff ff ff 89 c7 be 04 00  P...............",
            "0x201150  00 00 58 e9 d8 ff ff ff 0f 1f 84 00 00 00 00 00  ..X............."
        ]
    );
    assert_eq!(reply.notes, None);
}

/// The metadata of a hexdump whose bytes are the file's, untouched.
const CLEAN: &str = r#""fixups_coverage":"dynamic-relocations","fixups_reason":null,"fixups":[],
    "provenance":[{"source":"file","overlaps":[]}]"#;

/// A hexdump document of function `f` at 0x10 that ends at 0x20, with `rows` and `metadata` (JSON members).
fn dump(truncated: bool, notes: &str, rows: &str, metadata: &str) -> String {
    format!(
        r#"{{"target":{{"name":"f","address":16}},"end":32,"truncated":{truncated},"notes":{notes},
        "rows":{rows},{metadata}}}"#
    )
}

const ABC: &str = r#"[{"address":16,"bytes":"414243","ascii":"ABC"}]"#;

#[test]
fn a_short_last_row_keeps_the_text_aligned() {
    let reply = hex_of(&dump(false, "[]", ABC, CLEAN));
    assert_eq!(reply.data, [format!("0x10  {:<47}  ABC", "41 42 43")]);
    assert_eq!(reply.notes, None);
}

#[test]
fn a_cut_hexdump_says_where_it_stops() {
    let reply = hex_of(&dump(true, r#"["partial"]"#, ABC, CLEAN));
    assert_eq!(
        reply.notes.as_deref(),
        Some("partial · the dump stops at 0x20")
    );
}

#[test]
fn hex_digits_and_text_that_are_not_are_shown_as_dots() {
    let rows = r#"[{"address":16,"bytes":"4g","ascii":"Aé"}]"#;
    let reply = hex_of(&dump(false, "[]", rows, CLEAN));
    assert_eq!(reply.data, [format!("0x10  {:<47}  A.", "4.")]);
}

#[test]
fn a_dump_with_no_rows_says_so() {
    let reply = hex_of(&dump(false, "[]", "[]", CLEAN));
    assert_eq!(reply.data, ["no bytes at 0x10"]);
}

#[test]
fn bytes_the_loader_changes_or_that_are_not_the_files_are_said() {
    let said = |metadata: &str| hex_of(&dump(false, "[]", ABC, metadata)).notes;
    let fixed = r#""fixups_coverage":"dynamic-relocations","fixups_reason":null,
        "fixups":[{"address":16}],"provenance":[{"source":"file","overlaps":[]}]"#;
    assert_eq!(
        said(fixed).as_deref(),
        Some("1 load-time fixups lie in these bytes: the loader changes them")
    );
    let partial = r#""fixups_coverage":"partial","fixups_reason":"some relocations unreadable",
        "fixups":[],"provenance":[{"source":"file","overlaps":[]}]"#;
    assert_eq!(
        said(partial).as_deref(),
        Some("fixups partial: some relocations unreadable")
    );
    let zero = r#""fixups_coverage":"none","fixups_reason":null,"fixups":[],
        "provenance":[{"source":"zero-fill","overlaps":[]},{"source":"file","overlaps":[{"index":1}]}]"#;
    assert_eq!(
        said(zero).as_deref(),
        Some("some bytes are zero-fill, not file content · segments overlap here")
    );
}

#[test]
fn p_code_is_the_instruction_then_one_indented_line_per_operation() {
    let reply = pcode_of(include_str!("../../tests/fixtures/raw-pcode-main.json"));
    assert_eq!(
        reply.data[..4],
        [
            "0x201140  PUSH RAX",
            "    (unique, 0x4f900, 8) = COPY (register, 0x0, 8)",
            "    (register, 0x20, 8) = INT_SUB (register, 0x20, 8), (const, 0x8, 8)",
            "    STORE (const, 0x3, 8), (register, 0x20, 8), (unique, 0x4f900, 8)"
        ]
    );
    assert_eq!(reply.notes, None);
}

#[test]
fn an_operation_with_no_output_and_no_inputs_has_no_trailing_space() {
    let reply = pcode_of(
        r#"{"target":{"name":"f","address":16},"end":17,"truncated":false,"notes":[],
        "stop_reason":"range_end","error":null,
        "instructions":[{"address":16,"mnemonic":"RET","operands":"",
        "ops":[{"opcode":"RETURN","output":null,"inputs":[]}]}]}"#,
    );
    assert_eq!(reply.data, ["0x10  RET", "    RETURN"]);
}

#[test]
fn a_listing_with_no_instructions_says_so() {
    let reply = pcode_of(
        r#"{"target":{"name":"f","address":16},"end":16,"truncated":false,"notes":[],
        "stop_reason":"range_end","error":null,"instructions":[]}"#,
    );
    assert_eq!(reply.data, ["no instructions at 0x10"]);
}

#[test]
fn a_lift_that_stopped_early_says_why() {
    let reply = pcode_of(
        r#"{"target":{"name":"f","address":16},"end":32,"truncated":false,"notes":[],
        "stop_reason":"decode_error","error":{"address":24,"message":"bad opcode at 0x18"},
        "instructions":[{"address":16,"mnemonic":"NOP","operands":"","ops":[]}]}"#,
    );
    assert_eq!(reply.data, ["0x10  NOP"]);
    assert_eq!(
        reply.notes.as_deref(),
        Some("the lift stops: decode_error · bad opcode at 0x18")
    );
}

#[test]
fn the_worker_answers_hex_and_p_code_on_request() {
    let (tx, rx) = mpsc::channel();
    let (requests, queue) = mpsc::channel();
    let worker = spawn(Arc::new(pinned_kuna()), hello(), queue, tx).expect("spawns");
    assert!(matches!(rx.recv(), Ok(Msg::Functions(Ok(_)))));
    requests.send(request(View::Hex, 0x201140)).expect("sends");
    let dump = next_lines(&rx, View::Hex, 0x201140);
    assert_eq!(dump.len(), 2, "{dump:?}");
    assert!(dump[0].starts_with("0x201140  50 bf 03"), "{dump:?}");
    requests
        .send(request(View::PCode, 0x201140))
        .expect("sends");
    let lifted = next_lines(&rx, View::PCode, 0x201140);
    assert_eq!(lifted[0], "0x201140  PUSH RAX", "{lifted:?}");
    drop(requests);
    worker.join().expect("ends when nobody can ask any more");
}

fn data() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/data.elf")
}

fn plain<T>(data: T) -> Reply<T> {
    Reply { data, notes: None }
}

const STRINGS: &str = include_str!("../../tests/fixtures/strings-data.json");
const CRYPTO: &str = include_str!("../../tests/fixtures/crypto-data.json");
const BLOBS: &str = include_str!("../../tests/fixtures/blobs-data.json");
const SUMMARY: &str = include_str!("../../tests/fixtures/summary-hello.json");

#[test]
fn strings_crypto_and_blobs_are_one_line_per_row_with_the_functions_that_use_them() {
    let strings = serde_json::from_str(STRINGS).expect("parses");
    assert_eq!(
        strings_reply(plain(strings)).data,
        [
            "0x200120  .rodata  hello from bonito  ← say",
            "0x200154  .rodata  ()-JUH"
        ]
    );
    let crypto = serde_json::from_str(CRYPTO).expect("parses");
    assert_eq!(
        crypto_reply(plain(crypto)).data,
        [
            "0x201192  .text  TEA/XTEA  delta 0x9E3779B9 (golden ratio; also RC5/RC6 Q32)  ← tea_step"
        ]
    );
    let blobs = serde_json::from_str(BLOBS).expect("parses");
    assert_eq!(
        blobs_reply(plain(blobs)).data,
        ["0x200140  .rodata  gzip  44 bytes  ← packed"]
    );
}

#[test]
fn a_string_used_by_many_functions_lists_five_and_counts_the_rest() {
    let users: Vec<String> = (1..=7)
        .map(|n| format!(r#"{{"name":"f{n}","address":{n}}}"#))
        .collect();
    let text = format!(
        r#"{{"scanned":"sections","strings":[{{"address":16,"text":"x","section":null,"functions":[{}]}}]}}"#,
        users.join(",")
    );
    let strings = serde_json::from_str(&text).expect("parses");
    assert_eq!(
        strings_reply(plain(strings)).data,
        ["0x10  ?  x  ← f1, f2, f3, f4, f5, …+2"]
    );
}

#[test]
fn a_binary_with_none_of_a_kind_says_so() {
    let none = |key: &str| format!(r#"{{"scanned":"sections","{key}":[]}}"#);
    let strings = serde_json::from_str(&none("strings")).expect("parses");
    assert_eq!(strings_reply(plain(strings)).data, ["no strings found"]);
    let crypto = serde_json::from_str(&none("constants")).expect("parses");
    assert_eq!(
        crypto_reply(plain(crypto)).data,
        ["no crypto constants found"]
    );
    let blobs = serde_json::from_str(&none("blobs")).expect("parses");
    assert_eq!(blobs_reply(plain(blobs)).data, ["no embedded files found"]);
}

#[test]
fn the_summary_lists_the_entry_the_counts_and_the_largest_functions() {
    let summary = serde_json::from_str(SUMMARY).expect("parses");
    let reply = summary_reply(plain(summary));
    assert_eq!(
        reply.data,
        [
            "entry: _start  0x201160",
            "main: main  0x201140",
            "functions: 4 (3 reachable from the entry, 2 with no callers)",
            "code: 98 bytes",
            "sizes: 16-63 bytes ×4",
            "",
            "largest functions:",
            "  0x201160  34  _start",
            "  0x201140  32  main",
            "  0x201120  16  square",
            "  0x201130  16  add"
        ]
    );
    assert_eq!(reply.notes, None);
}

#[test]
fn loader_repairs_lead_the_summary_and_reach_the_status_line() {
    let text = SUMMARY.replace(
        r#""warnings": []"#,
        r#""warnings": ["ELF section table unusable"]"#,
    );
    let summary = serde_json::from_str(&text).expect("parses");
    let reply = summary_reply(plain(summary));
    assert_eq!(reply.data[0], "warning: ELF section table unusable");
    assert_eq!(reply.notes.as_deref(), Some("ELF section table unusable"));
}

#[test]
fn a_summary_without_an_entry_point_says_none() {
    let text = SUMMARY.replace(r#""entry": {"#, r#""entry": null, "was_entry": {"#);
    let summary = serde_json::from_str(&text).expect("parses");
    assert_eq!(summary_reply(plain(summary)).data[0], "entry: none");
}

/// The lines and the generation of the next decompile answer.
fn next_decompile(rx: &Receiver<Msg>) -> (Vec<String>, u64) {
    match rx.recv().expect("an answer") {
        Msg::Code {
            view: View::Decompile,
            generation,
            answer: Ok(reply),
            ..
        } => (reply.data, generation),
        _ => panic!("expected a decompile answer"),
    }
}

#[test]
fn toggling_an_option_changes_what_the_worker_decompiles_and_says_which_generation_it_used() {
    let (tx, rx) = mpsc::channel();
    let (requests, queue) = mpsc::channel();
    let shared = Arc::new(Shared::default());
    let worker = spawn_with(
        Arc::new(pinned_kuna()),
        hello(),
        Arc::clone(&shared),
        queue,
        tx,
    )
    .expect("spawns");
    assert!(matches!(rx.recv(), Ok(Msg::Functions(Ok(_)))));
    let has = |lines: &[String], text: &str| lines.iter().any(|line| line.contains(text));
    requests
        .send(request(View::Decompile, 0x201140))
        .expect("sends");
    let (plain, first) = next_decompile(&rx);
    assert!(has(&plain, "v1") && !has(&plain, "xVar1"), "{plain:?}");
    let ghidra = Options::default()
        .with_option("namestyle", "ghidra")
        .expect("valid");
    let second = shared.replace(ghidra.clone());
    requests
        .send(request(View::Decompile, 0x201140))
        .expect("sends");
    let (renamed, generation) = next_decompile(&rx);
    assert!(
        has(&renamed, "xVar1") && !has(&renamed, "v1 "),
        "{renamed:?}"
    );
    assert_eq!((generation, generation > first), (second, true));
    let reliable = ghidra.with_mode("reliable").expect("valid");
    shared.replace(reliable);
    requests
        .send(request(View::Decompile, 0x201140))
        .expect("sends");
    let (both, _) = next_decompile(&rx);
    assert!(has(&both, "int4 main") && has(&both, "xVar1"), "{both:?}");
    drop(requests);
    worker.join().expect("ends when nobody can ask any more");
}

#[test]
fn a_scan_by_segments_is_said_because_the_section_table_was_unusable() {
    let said = |key: &str, scanned: &str| {
        let text = format!(r#"{{"scanned":"{scanned}","{key}":[]}}"#);
        match key {
            "strings" => strings_reply(plain(serde_json::from_str(&text).expect("parses"))).notes,
            "constants" => crypto_reply(plain(serde_json::from_str(&text).expect("parses"))).notes,
            _ => blobs_reply(plain(serde_json::from_str(&text).expect("parses"))).notes,
        }
    };
    for key in ["strings", "constants", "blobs"] {
        assert_eq!(said(key, "sections"), None, "{key}");
        let note = said(key, "segments").expect("a note");
        assert!(
            note.contains("segments") && note.contains("unpack"),
            "{key}: {note}"
        );
    }
}

#[test]
fn a_summary_of_an_image_without_an_entry_point_still_shows() {
    let text = SUMMARY.replace(
        r#""reachable_from_entry": 3"#,
        r#""reachable_from_entry": null"#,
    );
    let summary = serde_json::from_str(&text).expect("parses");
    assert_eq!(
        summary_reply(plain(summary)).data[2],
        "functions: 4 (reachable from the entry: unknown, 2 with no callers)"
    );
}

#[test]
fn a_summary_with_no_size_buckets_says_none() {
    let text = SUMMARY.replace(r#""count": 4"#, r#""count": 0"#);
    let summary = serde_json::from_str(&text).expect("parses");
    assert_eq!(summary_reply(plain(summary)).data[4], "sizes: none");
}

#[test]
fn a_warning_kuna_also_printed_on_stderr_is_said_once_and_a_flood_is_capped() {
    let text = SUMMARY.replace(
        r#""warnings": []"#,
        r#""warnings": ["ELF section table unusable"]"#,
    );
    let summary = serde_json::from_str(&text).expect("parses");
    let reply = summary_reply(Reply {
        data: summary,
        notes: Some("ELF section table unusable".to_string()),
    });
    assert_eq!(reply.notes.as_deref(), Some("ELF section table unusable"));
    let many: Vec<String> = (0..50).map(|n| format!(r#""repair {n}""#)).collect();
    let text = SUMMARY.replace(
        r#""warnings": []"#,
        &format!(r#""warnings": [{}]"#, many.join(",")),
    );
    let summary = serde_json::from_str(&text).expect("parses");
    let lines = summary_reply(plain(summary)).data;
    let warnings = lines
        .iter()
        .filter(|line| line.starts_with("warning: "))
        .count();
    assert_eq!(warnings, MAX_NOTES);
    assert!(
        lines.iter().any(|line| line.starts_with("entry: ")),
        "the overview survives"
    );
}

#[test]
fn the_worker_answers_the_binary_wide_views_at_address_zero() {
    let (tx, rx) = mpsc::channel();
    let (requests, queue) = mpsc::channel();
    let worker = spawn(Arc::new(pinned_kuna()), data(), queue, tx).expect("spawns");
    assert!(matches!(rx.recv(), Ok(Msg::Functions(Ok(_)))));
    for (view, expected) in [
        (View::Strings, "0x200120  .rodata  hello from bonito  ← say"),
        (View::Crypto, "0x201192  .text  TEA/XTEA"),
        (View::Blobs, "0x200140  .rodata  gzip  44 bytes  ← packed"),
        (View::Summary, "entry: _start  0x2011a0"),
    ] {
        requests.send(request(view, 0)).expect("sends");
        let lines = next_lines(&rx, view, 0);
        assert!(
            lines.iter().any(|line| line.starts_with(expected)),
            "{view:?}: {lines:?}"
        );
    }
    drop(requests);
    worker.join().expect("ends when nobody can ask any more");
}

#[test]
fn the_worker_lists_the_callers_of_a_function_on_request() {
    let (tx, rx) = mpsc::channel();
    let (requests, queue) = mpsc::channel();
    let worker = spawn(Arc::new(pinned_kuna()), hello(), queue, tx).expect("spawns");
    assert!(matches!(rx.recv(), Ok(Msg::Functions(Ok(_)))));
    requests
        .send(request(View::Xrefs, 0x201120))
        .expect("sends");
    let lines = next_lines(&rx, View::Xrefs, 0x201120);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].ends_with("main  CALL 0x201120"), "{lines:?}");
    drop(requests);
    worker.join().expect("ends when nobody can ask any more");
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
