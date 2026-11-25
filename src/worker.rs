//! The kuna worker: lists the functions, then answers the requests for views.

use crate::app::{Msg, Request};
use crate::keymap::View;
use crate::kuna::{self, Decompiled, Disassembly, Kuna, Reply, Xrefs};
use std::io;
use std::iter::once;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// What a tab becomes in a code pane.
// ponytail: four spaces, not tab stops.
const TAB: &str = "    ";
/// How many lines a view keeps: a hostile binary can make kuna print far more.
const MAX_LINES: usize = 100_000;
/// How many characters of a line a view keeps.
const MAX_LINE_CHARS: usize = 4096;
/// How many notes a reply keeps: a hostile binary can make kuna print thousands.
const MAX_NOTES: usize = 10;
/// The `data_refs_coverage` values that mean kuna read all of the image's relocation metadata.
const COMPLETE_POINTERS: [&str; 5] = [
    "none",
    "chained-fixups",
    "rebase-opcodes",
    "dynamic-relocations",
    "base-relocations",
];
/// How many characters of a panic message are kept.
const PANIC_LIMIT: usize = 200;
/// How often an idle worker checks whether kuna was cancelled.
const IDLE: Duration = Duration::from_millis(100);

/// Lists the functions of `binary` on a worker thread, then answers `requests` until every sender is gone or
/// kuna is cancelled. Of the requests waiting, only the newest of each view is answered: the user has moved on
/// from the rest. A panic in a kuna call is sent as an error.
// ponytail: a call in flight is not cancelled by a newer request, only by `Kuna::cancel` (#11 caches instead).
pub fn spawn(
    kuna: Arc<Kuna>,
    binary: PathBuf,
    requests: Receiver<Request>,
    tx: Sender<Msg>,
) -> io::Result<JoinHandle<()>> {
    thread::Builder::new().spawn(move || {
        let functions = guarded(|| kuna.functions(&binary));
        // ponytail: a closed channel means the loop is gone; nobody is left to tell.
        if tx.send(Msg::Functions(functions)).is_err() {
            return;
        }
        loop {
            let oldest = match requests.recv_timeout(IDLE) {
                Ok(request) => request,
                Err(RecvTimeoutError::Timeout) if !kuna.is_cancelled() => continue,
                Err(_) => return,
            };
            for Request { view, address } in newest_per_view(oldest, &requests) {
                let answer = guarded(|| lines_of(&kuna, &binary, view, address));
                let msg = Msg::Code {
                    view,
                    address,
                    answer,
                };
                if tx.send(msg).is_err() {
                    return;
                }
            }
        }
    })
}

/// `first` and the requests waiting, in order, without those a later request for the same view replaces.
fn newest_per_view(first: Request, waiting: &Receiver<Request>) -> Vec<Request> {
    let all: Vec<Request> = once(first).chain(waiting.try_iter()).collect();
    all.iter()
        .enumerate()
        .filter(|(at, request)| all[at + 1..].iter().all(|later| later.view != request.view))
        .map(|(_, request)| *request)
        .collect()
}

/// Runs `work`; a panic in it becomes an error that keeps the panic's message.
fn guarded<T>(work: impl FnOnce() -> Result<T, kuna::Error>) -> Result<T, kuna::Error> {
    panic::catch_unwind(AssertUnwindSafe(work)).unwrap_or_else(|payload| {
        let reason = match (
            payload.downcast_ref::<&str>(),
            payload.downcast_ref::<String>(),
        ) {
            (Some(text), _) => (*text).to_string(),
            (None, Some(text)) => text.clone(),
            (None, None) => "no message".to_string(),
        };
        let reason = kuna::single_line(&reason, PANIC_LIMIT);
        Err(kuna::Error::Control(io::Error::other(format!(
            "the worker thread panicked: {reason}"
        ))))
    })
}

/// The lines of `view` of the function at `address`, with what kuna said while making them.
fn lines_of(
    kuna: &Kuna,
    binary: &Path,
    view: View,
    address: u64,
) -> Result<Reply<Vec<String>>, kuna::Error> {
    match view {
        View::Decompile => decompile_reply(kuna.decompile(binary, address)?),
        View::Disassembly => Ok(disassembly_reply(kuna.disassemble(binary, address)?)),
        View::Xrefs => Ok(xrefs_reply(kuna.xrefs(binary, address)?)),
        // ponytail: #9 adds these; nothing asks for them yet.
        View::Strings | View::CallGraph | View::Hex | View::PCode | View::Summary => Err(
            kuna::Error::Reported(format!("the {view:?} view is not available in bonito yet")),
        ),
    }
}

/// At most `MAX_LINES` lines of at most `MAX_LINE_CHARS` characters, and whether anything was cut.
fn bounded(mut lines: impl Iterator<Item = String>) -> (Vec<String>, bool) {
    let mut kept: Vec<String> = lines.by_ref().take(MAX_LINES).collect();
    let mut cut = lines.next().is_some();
    for line in &mut kept {
        if let Some((end, _)) = line.char_indices().nth(MAX_LINE_CHARS) {
            line.truncate(end);
            cut = true;
        }
    }
    (kept, cut)
}

/// The notes of a reply, joined for the status line; none when there are none. Past `MAX_NOTES` they are dropped.
fn join_notes(notes: impl IntoIterator<Item = String>, cut: bool) -> Option<String> {
    let mut notes = notes.into_iter();
    let mut kept: Vec<String> = notes.by_ref().take(MAX_NOTES).collect();
    let more = notes.next().is_some();
    kept.extend(more.then(|| format!("more than {MAX_NOTES} notes, the rest are not shown")));
    kept.extend(cut.then(|| format!("cut to {MAX_LINES} lines of {MAX_LINE_CHARS} characters")));
    (!kept.is_empty()).then(|| kept.join(" · "))
}

/// The C text of a function, one string per line; a function kuna gave no code for is an error.
fn decompile_reply(reply: Reply<Decompiled>) -> Result<Reply<Vec<String>>, kuna::Error> {
    let Reply { data, notes } = reply;
    let code = data.code.as_ref().ok_or_else(|| {
        kuna::Error::Reported("kuna returned no code for this function".to_string())
    })?;
    let (lines, cut) = bounded(code.as_str().lines().map(str::to_string));
    Ok(Reply {
        data: lines
            .into_iter()
            .map(|line| line.replace('\t', TAB))
            .collect(),
        notes: join_notes(notes, cut),
    })
}

/// A disassembly as `0x201140  PUSH RAX` lines; kuna's notes and a cut listing join the reply's notes.
fn disassembly_reply(reply: Reply<Disassembly>) -> Reply<Vec<String>> {
    let Reply { data, notes } = reply;
    let (lines, cut) =
        bounded(data.instructions.iter().map(|instruction| {
            format!("{:#x}  {}", instruction.address, instruction.text.as_str())
        }));
    let stopped = data
        .truncated
        .then(|| format!("the listing stops at {:#x}", data.end));
    let said = notes
        .into_iter()
        .chain(data.notes.iter().map(|note| note.as_str().to_string()))
        .chain(stopped);
    Reply {
        data: lines,
        notes: join_notes(said, cut),
    }
}

/// The references to a function as `0x201146  call  main  CALL 0x201120` lines, the stored pointers after them;
/// kuna's notes, what it did not decode and a short pointer list join the reply's notes. An answer that may be
/// incomplete never says "no references": a missing caller is worse than none.
fn xrefs_reply(reply: Reply<Xrefs>) -> Reply<Vec<String>> {
    let Reply { data, notes } = reply;
    let (mut lines, cut) = bounded(xref_rows(&data));
    let short = short_pointers(&data);
    let coverage = &data.coverage;
    let walked = coverage.decoded && coverage.unwalked.is_empty();
    if lines.is_empty() {
        let target = data.target.name.as_ref().map_or_else(
            || format!("{:#x}", data.target.address),
            |name| name.as_str().to_string(),
        );
        lines.push(if walked && short.is_none() {
            format!("no references to {target}")
        } else {
            format!("no references found to {target} (see the status line)")
        });
    }
    let stderr = notes.clone().unwrap_or_default();
    let unwalked = coverage.unwalked.len();
    let walk = [
        (!coverage.decoded)
            .then(|| "the target was not decoded: no code reference can be found".to_string()),
        (unwalked > 0)
            .then(|| format!("{unwalked} regions were not decoded: references may be missing")),
    ];
    let said = notes
        .into_iter()
        .chain(
            coverage
                .notes
                .iter()
                .map(|note| note.as_str())
                .filter(|note| !stderr.contains(note))
                .map(str::to_string),
        )
        .chain(walk.into_iter().flatten())
        .chain(short);
    Reply {
        data: lines,
        notes: join_notes(said, cut),
    }
}

/// The rows of an xrefs answer: the references in code, then the stored pointers.
fn xref_rows(data: &Xrefs) -> impl Iterator<Item = String> {
    let calls = data.xrefs.iter().map(|xref| {
        let from = xref
            .from_function
            .as_ref()
            .map_or("?", |function| function.name.as_str());
        let (kind, instruction) = (xref.kind.as_str(), xref.instruction.as_str());
        format!("{:#x}  {kind}  {from}  {instruction}", xref.from_address)
    });
    let pointers = data.data_refs.iter().map(|pointer| {
        let from = pointer.from_name.as_ref().map_or("?", |name| name.as_str());
        let kind = pointer.kind.as_str();
        format!("{:#x}  {kind}  {from}", pointer.from_address)
    });
    calls.chain(pointers)
}

/// Says so when kuna did not read all of the stored pointers (a value it does not know counts as short).
fn short_pointers(data: &Xrefs) -> Option<String> {
    let coverage = data.data_refs_coverage.as_str();
    (!COMPLETE_POINTERS.contains(&coverage)).then(|| match &data.data_refs_reason {
        Some(reason) => format!("stored pointers {coverage}: {}", reason.as_str()),
        None => format!("stored pointers {coverage}"),
    })
}

#[cfg(test)]
mod tests;
