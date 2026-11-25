//! The kuna worker: lists the functions, then answers the requests for views.

use crate::app::{Msg, Request};
use crate::keymap::View;
use crate::kuna::{
    self, Blobs, Crypto, Decompiled, Disassembly, Hexdump, Kuna, Located, Op, Pcode, Reply,
    Strings, Summary, Varnode, Xrefs,
};
use crate::options::{Options, Shared};
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

/// `spawn_with` and no options: a worker for the tests.
#[cfg(test)]
pub fn spawn(
    kuna: Arc<Kuna>,
    binary: PathBuf,
    requests: Receiver<Request>,
    tx: Sender<Msg>,
) -> io::Result<JoinHandle<()>> {
    spawn_with(kuna, binary, Arc::default(), requests, tx)
}

/// Lists the functions of `binary` on a worker thread, then answers `requests` until every sender is gone or
/// kuna is cancelled. Of the requests waiting, only the newest of each view is answered: the user has moved on
/// from the rest. A panic in a kuna call is sent as an error. A decompile runs with the options `shared` holds when
/// its request is taken up, and its answer says which generation of them that was.
// ponytail: a call in flight is not cancelled by a newer request, only by `Kuna::cancel` (#11 caches instead).
pub fn spawn_with(
    kuna: Arc<Kuna>,
    binary: PathBuf,
    shared: Arc<Shared>,
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
                let (options, generation) = shared.current();
                let answer = guarded(|| lines_of(&kuna, &binary, view, address, &options));
                let msg = Msg::Code {
                    view,
                    address,
                    generation,
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
    options: &Options,
) -> Result<Reply<Vec<String>>, kuna::Error> {
    match view {
        View::Decompile => decompile_reply(kuna.decompile(binary, address, options)?),
        View::Disassembly => Ok(disassembly_reply(kuna.disassemble(binary, address)?)),
        View::Xrefs => Ok(xrefs_reply(kuna.xrefs(binary, address)?)),
        View::Hex => Ok(hex_reply(kuna.hexdump(binary, address)?)),
        View::PCode => Ok(pcode_reply(kuna.pcode(binary, address)?)),
        View::Strings => Ok(strings_reply(kuna.strings(binary)?)),
        View::Crypto => Ok(crypto_reply(kuna.crypto(binary)?)),
        View::Blobs => Ok(blobs_reply(kuna.blobs(binary)?)),
        View::Summary => Ok(summary_reply(kuna.summary(binary)?)),
        View::Catalog => Ok(catalog_reply(kuna.catalog()?)),
        View::Modes => Ok(modes_reply(kuna.modes()?)),
        View::Docs => docs_reply(kuna, address),
        // ponytail: #12 adds the call graph; nothing asks for it yet.
        View::CallGraph => Err(kuna::Error::Reported(format!(
            "the {view:?} view is not available in bonito yet"
        ))),
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

/// How many functions a row names before it says how many more.
const MAX_USERS: usize = 5;

/// `  ← main, sub_1…` for the functions that use something; nothing when none does.
fn callers(functions: &[Located]) -> String {
    let names: Vec<&str> = functions
        .iter()
        .take(MAX_USERS)
        .map(|function| function.name.as_str())
        .collect();
    if names.is_empty() {
        return String::new();
    }
    let more = functions.len().saturating_sub(MAX_USERS);
    let tail = if more > 0 {
        format!(", …+{more}")
    } else {
        String::new()
    };
    format!("  ← {}{tail}", names.join(", "))
}

/// The section a row lies in, `?` when kuna could not tell.
fn in_section(section: &Option<kuna::Text>) -> &str {
    section.as_ref().map_or("?", |section| section.as_str())
}

/// `rows` as the lines of a reply, `empty` when there are none; `notes` and a cut listing are the reply's notes.
fn rows_reply(
    notes: impl IntoIterator<Item = String>,
    rows: impl Iterator<Item = String>,
    empty: &str,
) -> Reply<Vec<String>> {
    let (mut lines, cut) = bounded(rows);
    if lines.is_empty() {
        lines.push(empty.to_string());
    }
    Reply {
        data: lines,
        notes: join_notes(notes, cut),
    }
}

/// Says so when kuna scanned segments, not sections: the section table was unusable, so the scan covers whatever
/// is mapped (a packer's own data, say) and a listing, an empty one included, may not be the program's.
fn scan_note(scanned: &kuna::Text) -> Option<String> {
    (scanned.as_str() != "sections").then(|| {
        format!(
            "scanned by {}, not by sections: the section table is unusable, unpack the binary first",
            scanned.as_str()
        )
    })
}

/// Strings as `0x200120  .rodata  hello  ← say` lines.
fn strings_reply(reply: Reply<Strings>) -> Reply<Vec<String>> {
    let Reply { data, notes } = reply;
    let rows = data.strings.iter().map(|row| {
        let (section, text) = (in_section(&row.section), row.text.as_str());
        format!(
            "{:#x}  {section}  {text}{}",
            row.address,
            callers(&row.functions)
        )
    });
    let said = notes.into_iter().chain(scan_note(&data.scanned));
    rows_reply(said, rows, "no strings found")
}

/// Crypto constants as `0x201192  .text  TEA/XTEA  delta 0x9E3779B9  ← tea_step` lines.
fn crypto_reply(reply: Reply<Crypto>) -> Reply<Vec<String>> {
    let Reply { data, notes } = reply;
    let rows = data.constants.iter().map(|row| {
        let (section, algorithm) = (in_section(&row.section), row.algorithm.as_str());
        let name = row.name.as_str();
        format!(
            "{:#x}  {section}  {algorithm}  {name}{}",
            row.address,
            callers(&row.functions)
        )
    });
    let said = notes.into_iter().chain(scan_note(&data.scanned));
    rows_reply(said, rows, "no crypto constants found")
}

/// Embedded files as `0x200140  .rodata  gzip  44 bytes  ← packed` lines.
fn blobs_reply(reply: Reply<Blobs>) -> Reply<Vec<String>> {
    let Reply { data, notes } = reply;
    let rows = data.blobs.iter().map(|row| {
        let (section, kind) = (in_section(&row.section), row.kind.as_str());
        let length = row.length;
        format!(
            "{:#x}  {section}  {kind}  {length} bytes{}",
            row.address,
            callers(&row.functions)
        )
    });
    let said = notes.into_iter().chain(scan_note(&data.scanned));
    rows_reply(said, rows, "no embedded files found")
}

/// The summary as lines: the loader's warnings first (at most `MAX_NOTES`; they also reach the status line, unless
/// kuna printed them on stderr too), then the entry point, the counts and the largest functions.
fn summary_reply(reply: Reply<Summary>) -> Reply<Vec<String>> {
    let Reply { data, notes } = reply;
    let warnings = data
        .warnings
        .iter()
        .take(MAX_NOTES)
        .map(|warning| format!("warning: {}", warning.as_str()));
    let (lines, cut) = bounded(warnings.chain(overview_rows(&data)));
    let stderr = notes.clone().unwrap_or_default();
    let said = notes.into_iter().chain(
        data.warnings
            .iter()
            .map(|warning| warning.as_str())
            .filter(|warning| !stderr.contains(warning))
            .map(str::to_string),
    );
    Reply {
        data: lines,
        notes: join_notes(said, cut),
    }
}

/// The entry point, the counts, the size buckets and the largest functions of a summary.
fn overview_rows(data: &Summary) -> impl Iterator<Item = String> {
    let overview = &data.summary;
    let spot = |label: &str, at: &Option<Located>| match at {
        Some(function) => format!(
            "{label}: {}  {:#x}",
            function.name.as_str(),
            function.address
        ),
        None => format!("{label}: none"),
    };
    let buckets: Vec<String> = overview
        .size_buckets
        .iter()
        .filter(|bucket| bucket.count > 0)
        .map(|bucket| format!("{} bytes ×{}", bucket.bucket.as_str(), bucket.count))
        .collect();
    let sizes = if buckets.is_empty() {
        "none".to_string()
    } else {
        buckets.join(", ")
    };
    let reach = match overview.reachable_from_entry {
        Some(reached) => format!("{reached} reachable from the entry"),
        None => "reachable from the entry: unknown".to_string(),
    };
    let counts = format!(
        "functions: {} ({reach}, {} with no callers)",
        data.total, overview.no_callers
    );
    let largest: Vec<String> = overview
        .largest
        .iter()
        .map(|function| {
            format!(
                "  {:#x}  {}  {}",
                function.address,
                function.size,
                function.name.as_str()
            )
        })
        .collect();
    [
        spot("entry", &overview.entry),
        spot("main", &overview.main),
        counts,
        format!("code: {} bytes", overview.code_bytes),
        format!("sizes: {sizes}"),
        String::new(),
        "largest functions:".to_string(),
    ]
    .into_iter()
    .chain(largest)
}

/// A hexdump as `0x201140  50 bf 03 …  P...` lines. The digits and the text are forced to hex digits and printable
/// ASCII (kuna's are from the binary). Kuna's notes, a cut dump and what the load metadata says about the bytes
/// join the reply's notes.
fn hex_reply(reply: Reply<Hexdump>) -> Reply<Vec<String>> {
    let Reply { data, notes } = reply;
    let (mut lines, cut) = bounded(data.rows.iter().map(|row| {
        let digits = dotted(row.bytes.as_str(), |c| c.is_ascii_hexdigit());
        let text = dotted(row.ascii.as_str(), |c| c == ' ' || c.is_ascii_graphic());
        format!("{:#x}  {:<47}  {text}", row.address, spaced(&digits))
    }));
    if lines.is_empty() {
        lines.push(format!("no bytes at {:#x}", data.target.address));
    }
    let stopped = data
        .truncated
        .then(|| format!("the dump stops at {:#x}", data.end));
    let said = notes
        .into_iter()
        .chain(data.notes.iter().map(|note| note.as_str().to_string()))
        .chain(stopped)
        .chain(load_caveats(&data));
    Reply {
        data: lines,
        notes: join_notes(said, cut),
    }
}

/// `text` with every character `keep` rejects shown as a dot.
fn dotted(text: &str, keep: impl Fn(char) -> bool) -> String {
    text.chars()
        .map(|c| if keep(c) { c } else { '.' })
        .collect()
}

/// `50bf03` as `50 bf 03`.
fn spaced(digits: &str) -> String {
    let mut spaced = String::new();
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && at % 2 == 0 {
            spaced.push(' ');
        }
        spaced.push(digit);
    }
    spaced
}

/// What the load metadata says about the bytes of a dump: fixups the loader applies, metadata kuna could not
/// read, and bytes that are not the file's.
fn load_caveats(data: &Hexdump) -> Vec<String> {
    let fixups = data.fixups.len();
    let other = data
        .provenance
        .iter()
        .find(|part| part.source.as_str() != "file");
    let overlap = data.provenance.iter().any(|part| !part.overlaps.is_empty());
    [
        (fixups > 0).then(|| {
            format!("{fixups} load-time fixups lie in these bytes: the loader changes them")
        }),
        data.fixups_reason.as_ref().map(|reason| {
            format!(
                "fixups {}: {}",
                data.fixups_coverage.as_str(),
                reason.as_str()
            )
        }),
        other.map(|part| format!("some bytes are {}, not file content", part.source.as_str())),
        overlap.then(|| "segments overlap here".to_string()),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Each instruction as `0x201140  PUSH RAX`, then one indented line per p-code operation; kuna's notes, a cut
/// listing and a walk that stopped early join the reply's notes.
fn pcode_reply(reply: Reply<Pcode>) -> Reply<Vec<String>> {
    let Reply { data, notes } = reply;
    let (mut lines, cut) = bounded(data.instructions.iter().flat_map(|instruction| {
        let head = format!(
            "{:#x}  {} {}",
            instruction.address,
            instruction.mnemonic.as_str(),
            instruction.operands.as_str()
        );
        let ops = instruction
            .ops
            .iter()
            .map(|op| format!("    {}", operation(op)));
        once(head.trim_end().to_string()).chain(ops)
    }));
    let stopped = data
        .truncated
        .then(|| format!("the listing stops at {:#x}", data.end));
    if lines.is_empty() {
        lines.push(format!("no instructions at {:#x}", data.target.address));
    }
    let reason = data
        .stop_reason
        .as_ref()
        .filter(|reason| reason.as_str() != "range_end")
        .map(|reason| format!("the lift stops: {}", reason.as_str()));
    let error = data
        .error
        .as_ref()
        .map(|error| error.message.as_str().to_string());
    let said = notes
        .into_iter()
        .chain(data.notes.iter().map(|note| note.as_str().to_string()))
        .chain(stopped)
        .chain(reason)
        .chain(error);
    Reply {
        data: lines,
        notes: join_notes(said, cut),
    }
}

/// `(unique, 0x4f900, 8) = COPY (register, 0x0, 8)`: a p-code operation, without the `=` when it has no output.
fn operation(op: &Op) -> String {
    let inputs: Vec<String> = op.inputs.iter().map(varnode).collect();
    let call = format!("{} {}", op.opcode.as_str(), inputs.join(", "));
    match &op.output {
        Some(output) => format!("{} = {}", varnode(output), call.trim_end()),
        None => call.trim_end().to_string(),
    }
}

/// `(register, 0x20, 8)`.
fn varnode(node: &Varnode) -> String {
    format!(
        "({}, {:#x}, {})",
        node.space.as_str(),
        node.offset,
        node.size
    )
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

mod reference;

pub use reference::spawn as spawn_reference;
use reference::{catalog_reply, docs_reply, modes_reply};

#[cfg(test)]
mod tests;
