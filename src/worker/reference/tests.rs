//! Tests of the reference views and of the thread that loads them for the app.

use super::*;
use crate::app::Request;
use crate::keymap::View;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

const CATALOG: &str = include_str!("../../../tests/fixtures/catalog-sample.json");
const MODES: &str = include_str!("../../../tests/fixtures/modes.json");
const TOPICS: &str = include_str!("../../../tests/fixtures/docs-topics.json");

fn plain<T>(data: T) -> Reply<T> {
    Reply { data, notes: None }
}

fn modes() -> Vec<Mode> {
    let document: serde_json::Value = serde_json::from_str(MODES).expect("json");
    serde_json::from_value(document["modes"].clone()).expect("modes")
}

fn pinned_kuna() -> Arc<Kuna> {
    let exe = if cfg!(windows) { "kuna.exe" } else { "kuna" };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let sample = root.join("tests/fixtures/hello.elf");
    let path = root.join("target/kuna").join(exe);
    Arc::new(kuna::locate(Some(&path), &sample).expect("the pinned kuna (scripts/kuna.sh)"))
}

fn hello() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hello.elf")
}

#[test]
fn the_catalog_is_one_line_per_option_with_its_tier_default_values_and_summary() {
    let rows = serde_json::from_str(CATALOG).expect("parses");
    let reply = catalog_reply(plain(rows));
    assert_eq!(reply.data.len(), 3);
    assert!(
        reply.data[0].starts_with(
            "core       compareform = original  [canonical|original]  Whether comparisons keep"
        ),
        "{:?}",
        reply.data[0]
    );
    assert!(
        reply.data[1]
            .starts_with("core       namestyle = angr  [angr|ghidra]  Master default-name")
    );
    assert!(
        reply.data[2].starts_with("analysis   objcsdk = off  [<directory>|off]  Read Objective-C")
    );
    assert_eq!(reply.notes, None);
}

#[test]
fn the_modes_list_each_preset_with_the_options_it_sets() {
    let reply = modes_reply(plain(modes()));
    let lines = &reply.data;
    assert!(
        lines[0].starts_with("auto (automatic)  Select aggressive below"),
        "{lines:?}"
    );
    assert_eq!(lines[1], "    ctypes = on");
    assert!(lines[2].starts_with("reliable  The shipped, well-tested defaults"));
    assert!(lines[3].starts_with("aggressive  Maximum recovery"));
    assert_eq!(lines[4], "    switchmodbound = on");
    assert_eq!(
        lines.iter().filter(|line| line.starts_with("    ")).count(),
        1 + 46 + 5
    );
}

#[test]
fn the_topics_are_listed_with_their_size_and_a_document_is_its_lines() {
    let topics = serde_json::from_str(TOPICS).expect("parses");
    let reply = topics_reply(plain(topics));
    assert_eq!(
        reply.data[1],
        "options   kuna option catalog  (905570 bytes)"
    );
    let doc = doc_reply(plain("# Title\n\n\ta tab\nlast".to_string()));
    assert_eq!(doc.data, ["# Title", "", "    a tab", "last"]);
}

#[test]
fn a_document_with_no_text_says_so() {
    assert_eq!(
        doc_reply(plain(String::new())).data,
        ["kuna printed no text for this topic"]
    );
}

#[test]
fn nothing_listed_says_so() {
    assert_eq!(
        catalog_reply(plain(Vec::new())).data,
        ["kuna lists no options"]
    );
    assert_eq!(modes_reply(plain(Vec::new())).data, ["kuna lists no modes"]);
    assert_eq!(
        topics_reply(plain(Vec::new())).data,
        ["kuna has no embedded docs"]
    );
}

#[test]
fn the_docs_view_lists_the_topics_at_zero_and_reads_the_nth_at_n() {
    let kuna = pinned_kuna();
    let list = docs_reply(&kuna, 0).expect("lists").data;
    assert!(
        list.iter().any(|line| line.starts_with("options ")),
        "{list:?}"
    );
    let modes = docs_reply(&kuna, 5).expect("reads").data;
    assert_eq!(
        modes[0],
        "# Decompiler modes (`auto` | `reliable` | `aggressive` | `fast`)"
    );
    let error = docs_reply(&kuna, 6).expect_err("there are five");
    assert!(error.to_string().contains("no topic number 6"), "{error}");
    assert!(docs_reply(&kuna, u64::MAX).is_err());
}

#[test]
fn the_reference_thread_sends_what_kuna_knows_once() {
    let (tx, rx) = mpsc::channel();
    spawn(pinned_kuna(), tx)
        .expect("spawns")
        .join()
        .expect("finishes");
    let Ok(Msg::Reference(Ok(reply))) = rx.recv() else {
        panic!("expected the reference");
    };
    assert!(reply.data.catalog.len() > 200 && reply.data.modes.len() == 4);
    assert!(rx.try_recv().is_err(), "sent once");
}

#[test]
fn a_cancelled_kuna_sends_the_failure_not_silence() {
    let kuna = pinned_kuna();
    kuna.cancel();
    let (tx, rx) = mpsc::channel();
    spawn(kuna, tx).expect("spawns").join().expect("finishes");
    assert!(matches!(
        rx.recv(),
        Ok(Msg::Reference(Err(kuna::Error::Cancelled)))
    ));
}

#[test]
fn the_worker_answers_the_catalog_the_modes_and_the_docs_views() {
    let (tx, rx) = mpsc::channel();
    let (requests, queue) = mpsc::channel();
    let worker = super::super::spawn(pinned_kuna(), hello(), queue, tx).expect("spawns");
    assert!(matches!(rx.recv(), Ok(Msg::Functions(Ok(_)))));
    for (view, address, expected) in [
        (View::Catalog, 0, "core       compareform = original"),
        (View::Modes, 0, "auto (automatic)"),
        (View::Docs, 0, "cli "),
        (View::Docs, 5, "# Decompiler modes"),
    ] {
        requests.send(Request { view, address }).expect("sends");
        let Ok(Msg::Code {
            answer: Ok(reply), ..
        }) = rx.recv()
        else {
            panic!("expected an answer for {view:?}");
        };
        let found = reply.data.iter().any(|line| line.starts_with(expected));
        assert!(
            found,
            "{view:?} {address}: {:?}",
            &reply.data[..3.min(reply.data.len())]
        );
    }
    drop(requests);
    worker.join().expect("ends when nobody can ask any more");
}
