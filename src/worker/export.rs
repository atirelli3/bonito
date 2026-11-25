//! The whole-binary export: kuna's export into the cache, made on a thread of its own, and how it is going.

use super::guarded;
use crate::app::{Export, Msg};
use crate::cache::Cache;
use crate::kuna::{self, Exported, Kuna, Progress, single_line};
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// How often the export's status file is read.
const POLL: Duration = Duration::from_secs(1);
/// How many characters of a failure the status line gets.
const FAILED_LIMIT: usize = 120;

/// Makes the export of the cache's binary on a thread of its own, and sends the app how it goes.
pub fn spawn(kuna: Arc<Kuna>, cache: Arc<Cache>, tx: Sender<Msg>) -> io::Result<JoinHandle<()>> {
    thread::Builder::new().spawn(move || run(&kuna, &cache, &tx))
}

/// Makes the export unless the cache has it, and sends how it ended; this session's folder is gone either way.
fn run(kuna: &Kuna, cache: &Cache, tx: &Sender<Msg>) {
    let status = match guarded(|| export(kuna, cache, tx)) {
        Ok(None) => return,
        Ok(Some(status)) => status,
        Err(error) => Export::Failed(error.to_string()),
    };
    // ponytail: a folder of a killed session stays until it is deleted by hand.
    let status = match (cache.clear_partial(), status) {
        (Err(error), Export::Failed(why)) => Export::Failed(format!("{why}; and {error}")),
        (Err(error), _) => Export::Failed(format!("cannot clear the export folder: {error}")),
        (Ok(()), status) => status,
    };
    // ponytail: a closed channel means the loop is gone; nobody is left to tell.
    let _ = tx.send(Msg::Export(shortened(status)));
}

/// `status`, a failure cut to what fits the status line.
fn shortened(status: Export) -> Export {
    match status {
        Export::Failed(why) => Export::Failed(single_line(&why, FAILED_LIMIT)),
        other => other,
    }
}

/// Exports into this session's folder and publishes it; none when the cache has the export already.
fn export(kuna: &Kuna, cache: &Cache, tx: &Sender<Msg>) -> Result<Option<Export>, kuna::Error> {
    if cache.complete().is_some() {
        return Ok(None);
    }
    let reported = |what: &str, error: io::Error| kuna::Error::Reported(format!("{what}: {error}"));
    cache
        .clear_partial()
        .map_err(|error| reported("cannot clear the export folder", error))?;
    let partial = cache.partial_dir();
    // ponytail: a closed channel means the loop is gone; nobody is left to tell.
    let _ = tx.send(Msg::Export(Export::Running {
        done: 0,
        total: None,
    }));
    let exported = watched(&partial, tx, || kuna.export(cache.binary(), &partial))?;
    if let Some(notes) = exported.notes {
        return Err(kuna::Error::Reported(format!(
            "kuna said {notes}: not cached"
        )));
    }
    if nothing_decompiled(&exported.data) {
        let message = "no function decompiled: not cached";
        return Err(kuna::Error::Reported(message.to_string()));
    }
    if !cache.unchanged() {
        let message = "the binary changed or cannot be read any more: not cached";
        return Err(kuna::Error::Reported(message.to_string()));
    }
    cache
        .publish()
        .map_err(|error| reported("cannot publish the export", error))?;
    Ok(Some(Export::Ready {
        functions: exported.data.functions,
        failed: exported.data.failed,
    }))
}

/// Whether the export holds no function that decompiled.
fn nothing_decompiled(exported: &Exported) -> bool {
    exported.failed >= exported.functions
}

/// Runs `work` while another thread reports the progress in the status file in `dir`; the poller ends with the work.
fn watched<T>(dir: &Path, tx: &Sender<Msg>, work: impl FnOnce() -> T) -> T {
    let (stop, stopped) = mpsc::channel::<()>();
    thread::scope(|scope| {
        scope.spawn(move || poll(dir, &stopped, tx));
        let result = work();
        drop(stop);
        result
    })
}

/// Reports the progress in `dir` every `POLL` until `stopped` is hung up.
fn poll(dir: &Path, stopped: &Receiver<()>, tx: &Sender<Msg>) {
    report(dir, tx);
    while let Err(RecvTimeoutError::Timeout) = stopped.recv_timeout(POLL) {
        report(dir, tx);
    }
}

/// Sends the progress in `dir`'s status file; nothing when there is none or it cannot be read.
fn report(dir: &Path, tx: &Sender<Msg>) {
    if let Some(Ok(progress)) = Progress::read(dir) {
        let running = Export::Running {
            done: progress.done,
            total: progress.total,
        };
        // ponytail: a closed channel means the loop is gone; nobody is left to tell.
        let _ = tx.send(Msg::Export(running));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::{Key, Served};
    use crate::kuna::Scratch;
    use crate::worker::tests::{hello, pinned_kuna};
    use std::fs;
    use std::io::Write;
    use std::sync::mpsc;

    fn cache_over(binary: &Path, root: &Path) -> Cache {
        Cache::open_in(root, Key::of(binary, "1.1005").expect("a key")).expect("opens")
    }

    /// What one `run` sent, in order.
    fn sent(kuna: &Kuna, cache: &Cache) -> Vec<Export> {
        let (tx, rx) = mpsc::channel();
        run(kuna, cache, &tx);
        drop(tx);
        rx.iter()
            .filter_map(|msg| match msg {
                Msg::Export(status) => Some(status),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_finished_export_is_published_and_serves_its_functions() {
        let scratch = Scratch::new().expect("a directory");
        let cache = cache_over(&hello(), scratch.path());
        let statuses = sent(&pinned_kuna(), &cache);
        assert_eq!(
            statuses.first(),
            Some(&Export::Running {
                done: 0,
                total: None
            })
        );
        assert_eq!(
            statuses.last(),
            Some(&Export::Ready {
                functions: 4,
                failed: 0
            })
        );
        assert!(cache.complete().is_some());
        assert!(!cache.partial_dir().exists(), "the folder became the entry");
        let dir = cache.serving_dir().expect("an export to read");
        let text = Served::default()
            .function(&dir, 0x201140)
            .expect("main is in it");
        assert!(text.starts_with("void main(void)"), "{text}");
    }

    #[test]
    fn an_export_that_is_already_in_the_cache_is_not_made_again() {
        let scratch = Scratch::new().expect("a directory");
        let cache = cache_over(&hello(), scratch.path());
        let kuna = pinned_kuna();
        assert!(!sent(&kuna, &cache).is_empty());
        kuna.cancel();
        assert!(
            sent(&kuna, &cache).is_empty(),
            "nothing to do, nothing said"
        );
    }

    #[test]
    fn a_leftover_folder_of_an_earlier_session_is_cleared_first() {
        let scratch = Scratch::new().expect("a directory");
        let cache = cache_over(&hello(), scratch.path());
        fs::create_dir(cache.partial_dir()).expect("a leftover");
        fs::write(cache.partial_dir().join("old.c"), "x").expect("writes");
        let statuses = sent(&pinned_kuna(), &cache);
        assert!(
            matches!(statuses.last(), Some(Export::Ready { .. })),
            "{statuses:?}"
        );
        let dir = cache.serving_dir().expect("an export to read");
        assert!(
            !dir.join("old.c").exists(),
            "the leftover is not in the cache"
        );
    }

    #[test]
    fn an_export_kuna_cannot_make_says_why_and_leaves_nothing() {
        let scratch = Scratch::new().expect("a directory");
        let junk = scratch.path().join("junk.bin");
        fs::write(&junk, b"this is not an executable").expect("writes");
        let cache = cache_over(&junk, &scratch.path().join("cache"));
        let statuses = sent(&pinned_kuna(), &cache);
        let Some(Export::Failed(why)) = statuses.last() else {
            panic!("expected a failure: {statuses:?}");
        };
        assert!(why.contains("the export failed"), "{why}");
        assert!(cache.complete().is_none());
        assert!(!cache.partial_dir().exists());
    }

    #[test]
    fn a_cancelled_export_leaves_nothing_behind() {
        let scratch = Scratch::new().expect("a directory");
        let cache = cache_over(&hello(), scratch.path());
        let kuna = pinned_kuna();
        kuna.cancel();
        let statuses = sent(&kuna, &cache);
        assert!(
            matches!(statuses.last(), Some(Export::Failed(_))),
            "{statuses:?}"
        );
        assert!(cache.complete().is_none());
        assert!(!cache.partial_dir().exists());
    }

    #[test]
    fn a_binary_that_changed_before_the_export_ended_is_not_cached() {
        let scratch = Scratch::new().expect("a directory");
        let copy = scratch.path().join("hello-copy.elf");
        fs::copy(hello(), &copy).expect("copies");
        let cache = cache_over(&copy, &scratch.path().join("cache"));
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&copy)
            .expect("opens");
        file.write_all(b"\0\0\0\0").expect("appends");
        drop(file);
        let statuses = sent(&pinned_kuna(), &cache);
        let Some(Export::Failed(why)) = statuses.last() else {
            panic!("expected a failure: {statuses:?}");
        };
        assert!(why.contains("changed"), "{why}");
        assert!(cache.complete().is_none());
    }

    #[test]
    fn while_the_work_runs_its_progress_is_reported_and_the_poller_ends_with_it() {
        let scratch = Scratch::new().expect("a directory");
        let text = r#"{"schema":1,"phase":"decompiling","functions_total":10,"functions_done":4,"functions_failed":0,"error":null}"#;
        fs::write(scratch.path().join(".streaming"), text).expect("writes");
        let (tx, rx) = mpsc::channel();
        let value = watched(scratch.path(), &tx, || {
            thread::sleep(Duration::from_millis(100));
            7
        });
        assert_eq!(value, 7);
        let Ok(Msg::Export(status)) = rx.try_recv() else {
            panic!("expected a status from the poller");
        };
        assert_eq!(
            status,
            Export::Running {
                done: 4,
                total: Some(10)
            }
        );
    }

    #[test]
    fn an_export_with_nothing_that_decompiled_is_not_worth_caching() {
        let none = Exported {
            functions: 0,
            failed: 0,
        };
        let all_failed = Exported {
            functions: 5,
            failed: 5,
        };
        let some = Exported {
            functions: 5,
            failed: 4,
        };
        assert!(nothing_decompiled(&none) && nothing_decompiled(&all_failed));
        assert!(!nothing_decompiled(&some));
    }

    #[test]
    fn a_failure_text_is_cut_so_it_cannot_push_the_rest_of_the_status_off_the_line() {
        let Export::Failed(text) = shortened(Export::Failed("x".repeat(1000))) else {
            panic!("still a failure");
        };
        assert!(text.chars().count() <= FAILED_LIMIT, "{}", text.len());
    }

    #[test]
    fn progress_in_the_status_file_is_reported_as_running() {
        let scratch = Scratch::new().expect("a directory");
        let text = r#"{"schema":1,"phase":"decompiling","functions_total":3153,"functions_done":1570,"functions_failed":1,"error":null}"#;
        fs::write(scratch.path().join(".streaming"), text).expect("writes");
        let (tx, rx) = mpsc::channel();
        report(scratch.path(), &tx);
        let Ok(Msg::Export(status)) = rx.try_recv() else {
            panic!("expected a status");
        };
        assert_eq!(
            status,
            Export::Running {
                done: 1570,
                total: Some(3153)
            }
        );
        report(&scratch.path().join("missing"), &tx);
        assert!(rx.try_recv().is_err(), "no status file, nothing to say");
    }
}
