//! The reference views (the option catalog, the mode presets, kuna's embedded docs) and the thread that loads the
//! same data once for the app.

use super::{TAB, bounded, guarded, join_notes, rows_reply};
use crate::app::Msg;
use crate::kuna::{self, CatalogRow, Kuna, Mode, Reply, Topic};
use std::io;
use std::iter::once;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};

/// Loads the catalog, the modes and the topics once and sends them to the app as `Msg::Reference`.
/// A panic or a failure is sent as an error: `:set` then says the catalog could not be loaded.
pub fn spawn(kuna: Arc<Kuna>, tx: Sender<Msg>) -> io::Result<JoinHandle<()>> {
    thread::Builder::new().spawn(move || {
        let reference = guarded(|| kuna.reference());
        // ponytail: a closed channel means the loop is gone; nobody is left to tell.
        let _ = tx.send(Msg::Reference(reference));
    })
}

/// The catalog as `core       compareform = original  [canonical|original]  Whether comparisons…` lines.
pub(super) fn catalog_reply(reply: Reply<Vec<CatalogRow>>) -> Reply<Vec<String>> {
    let Reply { data, notes } = reply;
    let rows = data.iter().map(|row| {
        let values: Vec<&str> = row.values.iter().map(|value| value.as_str()).collect();
        format!(
            "{:<9}  {} = {}  [{}]  {}",
            row.tier.as_str(),
            row.option.as_str(),
            row.default.as_str(),
            values.join("|"),
            row.summary.as_str()
        )
    });
    rows_reply(notes, rows, "kuna lists no options")
}

/// The modes as `reliable  summary` lines, each followed by the options it sets.
pub(super) fn modes_reply(reply: Reply<Vec<Mode>>) -> Reply<Vec<String>> {
    let Reply { data, notes } = reply;
    let rows = data.iter().flat_map(|mode| {
        let automatic = if mode.automatic { " (automatic)" } else { "" };
        let head = format!(
            "{}{automatic}  {}",
            mode.name.as_str(),
            mode.summary.as_str()
        );
        let overrides = mode
            .overrides
            .iter()
            .map(|set| format!("    {} = {}", set.option.as_str(), set.value.as_str()));
        once(head).chain(overrides)
    });
    rows_reply(notes, rows, "kuna lists no modes")
}

/// `options   <title>  (905570 bytes)` lines.
fn topics_reply(reply: Reply<Vec<Topic>>) -> Reply<Vec<String>> {
    let Reply { data, notes } = reply;
    let rows = data.iter().map(|topic| {
        format!(
            "{:<8}  {}  ({} bytes)",
            topic.topic.as_str(),
            topic.title.as_str(),
            topic.bytes
        )
    });
    rows_reply(notes, rows, "kuna has no embedded docs")
}

/// A document as its lines, tabs made wide; at most `MAX_LINES` of them.
fn doc_reply(reply: Reply<String>) -> Reply<Vec<String>> {
    let Reply { data, notes } = reply;
    let (mut lines, cut) = bounded(data.lines().map(|line| line.replace('\t', TAB)));
    if lines.is_empty() {
        lines.push("kuna printed no text for this topic".to_string());
    }
    Reply {
        data: lines,
        notes: join_notes(notes, cut),
    }
}

/// The docs view: topic number 0 is the list of topics, number n the n-th of them.
// ponytail: the app counts topics in its own snapshot and this lists them again to turn the number back into a name
// (a request carries a number, not a name); the two agree unless kuna is replaced during the session.
pub(super) fn docs_reply(kuna: &Kuna, number: u64) -> Result<Reply<Vec<String>>, kuna::Error> {
    let topics = kuna.topics()?;
    if number == 0 {
        return Ok(topics_reply(topics));
    }
    let found = usize::try_from(number - 1)
        .ok()
        .and_then(|at| topics.data.get(at));
    let topic =
        found.ok_or_else(|| kuna::Error::Reported(format!("kuna has no topic number {number}")))?;
    Ok(doc_reply(kuna.doc(topic.topic.as_str())?))
}

#[cfg(test)]
mod tests;
