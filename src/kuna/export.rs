//! The whole-binary export: `kuna decompile-project --stream` into a folder, and the progress it reports.
// ponytail: #88 runs the export; remove this `allow` then.
#![allow(dead_code)]

use super::text::Text;
use super::{Error, Kuna, Reply, query_args};
use serde::Deserialize;
use serde::de::IgnoredAny;
use std::io::Read;
use std::path::Path;
use std::time::Duration;
use std::{fs, io};

/// How long an export may run before it is killed.
const EXPORT_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);
/// The `.streaming` schema bonito reads.
const SCHEMA: u32 = 1;
/// The file in the export folder that exists only while it runs.
const STATUS_FILE: &str = ".streaming";
/// The file in the export folder that lists each finished function.
const INDEX_FILE: &str = "index.jsonl";
/// How many bytes of `.streaming` are read.
const STATUS_LIMIT: u64 = 64 * 1024;
/// How many bytes of `index.jsonl` are read.
const INDEX_LIMIT: u64 = 64 * 1024 * 1024;
/// What kuna says on stderr of every serial export; it is not a problem.
const SERIAL_NOTE: &str = "[kuna --stream] serial run; --jobs auto uses every core";

/// The schema a `.streaming` file says it has.
#[derive(Deserialize)]
struct Header {
    schema: u32,
}

/// What kuna's `.streaming` file says of a running export; unknown keys are ignored, a null is as required as a value.
#[derive(Debug, Deserialize)]
pub struct Progress {
    /// `loading`, `decompiling`, `finalizing` or `failed`.
    pub phase: Text,
    /// Null until the load has counted the functions.
    #[serde(rename = "functions_total", deserialize_with = "Option::deserialize")]
    pub total: Option<u64>,
    #[serde(rename = "functions_done")]
    pub done: u64,
    #[serde(rename = "functions_failed")]
    pub failed: u64,
    /// Why it failed, when it did.
    #[serde(deserialize_with = "Option::deserialize")]
    pub error: Option<Text>,
}

impl Progress {
    /// The status in `text`, its schema checked first.
    pub fn parse(text: &str) -> Result<Self, String> {
        let unreadable = |why: String| format!("unreadable export status: {why}");
        let header: Header =
            serde_json::from_str(text).map_err(|error| unreadable(error.to_string()))?;
        if header.schema != SCHEMA {
            return Err(unreadable(format!(
                "schema {}, bonito reads schema {SCHEMA}",
                header.schema
            )));
        }
        serde_json::from_str(text).map_err(|error| unreadable(error.to_string()))
    }

    /// The status of the export in `dir`; none when there is no `.streaming` file (not started, or finished).
    pub fn read(dir: &Path) -> Option<Result<Self, String>> {
        let mut text = String::new();
        let read = fs::File::open(dir.join(STATUS_FILE))
            .and_then(|file| file.take(STATUS_LIMIT).read_to_string(&mut text));
        match read {
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => Some(Err(format!("cannot read the export status: {error}"))),
            Ok(_) => Some(Self::parse(&text)),
        }
    }
}

/// What a finished export holds.
#[derive(Debug, PartialEq, Eq)]
pub struct Exported {
    pub functions: u64,
    /// How many of them kuna could not decompile: their entries say why.
    pub failed: u64,
}

/// One line of `index.jsonl`; unknown keys are ignored.
#[derive(Deserialize)]
struct Row {
    #[serde(deserialize_with = "Option::deserialize")]
    error: Option<IgnoredAny>,
}

impl Kuna {
    /// Exports every function of `binary` into the absolute folder `dir`, which must be empty or not there yet,
    /// returning when kuna is done and has left a finished export.
    pub fn export(&self, binary: &Path, dir: &Path) -> Result<Reply<Exported>, Error> {
        if !dir.is_absolute() {
            let message = format!("the export folder {dir:?} is not an absolute path");
            return Err(Error::Reported(message));
        }
        if fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_some()) {
            return Err(Error::Reported(format!(
                "the export folder {dir:?} is not empty"
            )));
        }
        let mut args = query_args("decompile-project", binary, &["--stream", "-o"])?;
        args.push(dir.as_os_str().to_owned());
        let output = self
            .output_for(args, EXPORT_TIMEOUT)
            .map_err(|error| why_it_failed(error, dir))?;
        Ok(Reply {
            data: summarize(dir)?,
            notes: without_serial_note(output.notes),
        })
    }
}

/// `error` with the reason kuna wrote to `.streaming`, when it says the export failed.
fn why_it_failed(error: Error, dir: &Path) -> Error {
    if !matches!(error, Error::Failed { .. }) {
        return error;
    }
    match Progress::read(dir) {
        Some(Ok(Progress {
            phase,
            error: Some(why),
            ..
        })) if phase == "failed" => Error::Reported(format!("the export failed: {}", why.as_str())),
        _ => error,
    }
}

/// What the finished export in `dir` holds; one that kuna left unfinished, or an index that is not one, is an error.
fn summarize(dir: &Path) -> Result<Exported, Error> {
    let unfinished = || Error::Reported("kuna ended but left the export unfinished".to_string());
    if dir.join(STATUS_FILE).exists() {
        return Err(unfinished());
    }
    let file = fs::File::open(dir.join(INDEX_FILE)).map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => unfinished(),
        _ => Error::Read(error),
    })?;
    let mut text = String::new();
    file.take(INDEX_LIMIT + 1)
        .read_to_string(&mut text)
        .map_err(Error::Read)?;
    if text.len() as u64 > INDEX_LIMIT {
        return Err(Error::Reported(format!(
            "{INDEX_FILE} is longer than {INDEX_LIMIT} bytes"
        )));
    }
    let mut exported = Exported {
        functions: 0,
        failed: 0,
    };
    for line in text.lines() {
        let row: Row = serde_json::from_str(line).map_err(|_| {
            Error::Reported(format!("{INDEX_FILE} has a line that is not an entry"))
        })?;
        exported.functions += 1;
        exported.failed += u64::from(row.error.is_some());
    }
    Ok(exported)
}

/// `notes` without the routine serial-run line, which is on every export; none when nothing else is left.
fn without_serial_note(notes: Option<String>) -> Option<String> {
    let rest = notes?.replace(SERIAL_NOTE, "");
    let rest = rest.trim();
    (!rest.is_empty()).then(|| rest.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kuna::Scratch;

    const LOADING: &str = r#"{"schema":1,"phase":"loading","pid":9041,"started_at":1795620699,"updated_at":1795620699,"elapsed_s":0,"jobs":1,"seeds":0,"functions_total":null,"functions_done":0,"functions_failed":0,"seconds_since_last_result":0,"c_bytes":0,"asm":"pending","error":null}"#;
    const DECOMPILING: &str = r#"{"schema":1,"phase":"decompiling","pid":2927817,"started_at":1789162729,"updated_at":1789162747,"elapsed_s":17,"jobs":4,"seeds":1,"functions_total":3153,"functions_done":1570,"functions_failed":1,"seconds_since_last_result":0,"c_bytes":2316041,"asm":"complete","error":null}"#;
    const FINALIZING: &str = r#"{"schema":1,"phase":"finalizing","pid":9041,"started_at":1795620699,"updated_at":1795620700,"elapsed_s":0,"jobs":1,"seeds":1,"functions_total":134,"functions_done":134,"functions_failed":0,"seconds_since_last_result":0,"c_bytes":76532,"asm":"complete","error":null}"#;

    #[test]
    fn a_decompiling_status_has_its_counts() {
        let progress = Progress::parse(DECOMPILING).expect("parses");
        assert_eq!(progress.phase, "decompiling");
        assert_eq!(
            (progress.total, progress.done, progress.failed),
            (Some(3153), 1570, 1)
        );
        assert_eq!(progress.error, None);
    }

    #[test]
    fn a_loading_status_has_a_null_total() {
        let progress = Progress::parse(LOADING).expect("parses");
        assert_eq!(progress.phase, "loading");
        assert_eq!(
            (progress.total, progress.done, progress.failed),
            (None, 0, 0)
        );
    }

    #[test]
    fn a_finalizing_status_has_done_equal_to_total() {
        let progress = Progress::parse(FINALIZING).expect("parses");
        assert_eq!(progress.phase, "finalizing");
        assert_eq!((progress.total, progress.done), (Some(134), 134));
    }

    #[test]
    fn a_failed_status_carries_its_message_cleaned() {
        let text = r#"{"schema":1,"phase":"failed","functions_total":null,"functions_done":0,"functions_failed":0,"error":"unloadable image\u001b[2J"}"#;
        let progress = Progress::parse(text).expect("parses");
        assert_eq!(progress.phase, "failed");
        let message = progress.error.expect("a message");
        assert!(message.as_str().starts_with("unloadable image"));
        assert!(!message.contains('\u{1b}'));
    }

    #[test]
    fn a_status_bonito_cannot_read_is_an_error_that_says_why() {
        let newer = DECOMPILING.replace("\"schema\":1", "\"schema\":2");
        let message = Progress::parse(&newer).expect_err("schema 2");
        assert!(message.contains("schema 2"), "{message}");
        let both = newer.replace("functions_done", "done");
        let message = Progress::parse(&both).expect_err("schema 2, keys renamed");
        assert!(message.contains("schema 2"), "{message}");
        let renamed = DECOMPILING.replace("functions_done", "done");
        assert!(Progress::parse(&renamed).is_err(), "a count was renamed");
        for text in ["", "not json", r#"{"schema":1}"#] {
            assert!(Progress::parse(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn a_renamed_total_or_error_key_is_an_error_not_a_missing_value() {
        let no_total = DECOMPILING.replace("functions_total", "total");
        assert!(
            Progress::parse(&no_total).is_err(),
            "functions_total is required"
        );
        let no_error = DECOMPILING.replace("\"error\"", "\"why\"");
        assert!(Progress::parse(&no_error).is_err(), "error is required");
    }

    #[test]
    fn a_status_file_that_cannot_be_read_is_an_error_and_a_missing_one_is_none() {
        let scratch = Scratch::new().expect("a directory");
        assert!(
            Progress::read(scratch.path()).is_none(),
            "not started or finished"
        );
        fs::create_dir(scratch.path().join(STATUS_FILE)).expect("a folder in its place");
        assert!(matches!(Progress::read(scratch.path()), Some(Err(_))));
    }

    #[test]
    fn an_index_row_without_an_error_key_is_not_counted_as_a_success() {
        let scratch = Scratch::new().expect("a directory");
        let rows = "{\"seq\":0,\"name\":\"f\",\"error\":null}\n{\"seq\":1,\"name\":\"g\",\"error\":\"timed out\"}\n";
        fs::write(scratch.path().join(INDEX_FILE), rows).expect("writes");
        let exported = summarize(scratch.path()).expect("counts");
        assert_eq!((exported.functions, exported.failed), (2, 1));
        let renamed = rows.replace("\"error\"", "\"status\"");
        fs::write(scratch.path().join(INDEX_FILE), renamed).expect("writes");
        assert!(summarize(scratch.path()).is_err());
    }

    #[test]
    fn an_index_that_cannot_be_opened_is_not_called_unfinished() {
        let scratch = Scratch::new().expect("a directory");
        assert!(
            matches!(summarize(scratch.path()), Err(Error::Reported(m)) if m.contains("unfinished"))
        );
        fs::create_dir(scratch.path().join(INDEX_FILE)).expect("a folder in its place");
        assert!(matches!(summarize(scratch.path()), Err(Error::Read(_))));
    }

    #[test]
    fn the_routine_serial_note_is_dropped_and_anything_else_stays() {
        let serial = "[kuna --stream] serial run; --jobs auto uses every core".to_string();
        assert_eq!(without_serial_note(Some(serial.clone())), None);
        assert_eq!(without_serial_note(None), None);
        let repair = "ELF section table unusable".to_string();
        assert_eq!(
            without_serial_note(Some(repair.clone())),
            Some(repair.clone())
        );
        let joined = without_serial_note(Some(format!("{repair} {serial}")));
        assert_eq!(joined, Some(repair));
    }
}
