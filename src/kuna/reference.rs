//! What kuna says about itself, none of it about the analysed binary: the option catalog, the mode presets and its
//! embedded docs.
// ponytail: a catalog summary longer than `Text`'s 1024 characters is cut without a mark: the view is a browse line.

use super::text::{self, Text};
use super::{Error, Kuna, Limits, Reply, parse, run};
use serde::Deserialize;
use serde::de::DeserializeOwned;

/// One settable option of the phase model.
#[derive(Debug, Deserialize)]
pub struct CatalogRow {
    pub option: Text,
    /// What it accepts: words, or a placeholder such as `<directory>` for free text.
    pub values: Vec<Text>,
    pub default: Text,
    /// `core`, `analysis` or `transform`.
    pub tier: Text,
    pub summary: Text,
}

/// What `kuna modes --json` prints.
#[derive(Debug, Deserialize)]
struct Modes {
    modes: Vec<Mode>,
}

/// A named preset over the options.
#[derive(Debug, Deserialize)]
pub struct Mode {
    pub name: Text,
    pub summary: Text,
    /// Whether `auto` picks it by input size.
    pub automatic: bool,
    pub overrides: Vec<Override>,
}

/// One option a mode sets.
#[derive(Debug, Deserialize)]
pub struct Override {
    pub option: Text,
    pub value: Text,
}

/// One embedded document.
#[derive(Debug, Deserialize)]
pub struct Topic {
    pub topic: Text,
    pub title: Text,
    // ponytail: no view shows a topic's summary yet.
    #[allow(dead_code)]
    pub summary: Text,
    pub bytes: u64,
}

/// The catalog, the modes and the topics, loaded together.
#[derive(Debug)]
pub struct Reference {
    pub catalog: Vec<CatalogRow>,
    pub modes: Vec<Mode>,
    pub topics: Vec<Topic>,
}

impl Kuna {
    /// The settable options, in kuna's order.
    pub fn catalog(&self) -> Result<Reply<Vec<CatalogRow>>, Error> {
        self.ask(&["catalog", "--json"])
    }

    /// The mode presets.
    pub fn modes(&self) -> Result<Reply<Vec<Mode>>, Error> {
        let reply: Reply<Modes> = self.ask(&["modes", "--json"])?;
        reply.try_map(|document| Ok(document.modes))
    }

    /// The embedded documents.
    pub fn topics(&self) -> Result<Reply<Vec<Topic>>, Error> {
        self.ask(&["docs", "--json"])
    }

    /// The catalog, the modes and the topics; any one failing fails all, and the notes are joined.
    pub fn reference(&self) -> Result<Reply<Reference>, Error> {
        let catalog = self.catalog().map_err(|error| named("catalog", error))?;
        let modes = self.modes().map_err(|error| named("modes", error))?;
        let topics = self.topics().map_err(|error| named("docs", error))?;
        Ok(Reply {
            data: Reference {
                catalog: catalog.data,
                modes: modes.data,
                topics: topics.data,
            },
            notes: merged([catalog.notes, modes.notes, topics.notes]),
        })
    }

    /// The text of the embedded document `topic`, which must be one `topics` listed.
    pub fn doc(&self, topic: &str) -> Result<Reply<String>, Error> {
        if topic.is_empty() || topic.starts_with('-') {
            return Err(Error::Reported(format!("not a topic: {topic:?}")));
        }
        let limits = Limits::new(&self.stop, self.scratch.path());
        let output = run(&self.path, ["docs", topic], &limits)?;
        Ok(Reply {
            data: text::multiline(&output.stdout),
            notes: output.notes,
        })
    }

    /// Runs `kuna <args>` (`--json` among them): these commands take no binary.
    fn ask<T: DeserializeOwned>(&self, args: &[&str]) -> Result<Reply<T>, Error> {
        let limits = Limits::new(&self.stop, self.scratch.path());
        let output = run(&self.path, args.iter().copied(), &limits)?;
        Ok(Reply {
            data: parse(&output.stdout)?,
            notes: output.notes,
        })
    }
}

/// `error` said to come from `kuna <command>`; a cancel stays a cancel, whatever it interrupted.
fn named(command: &str, error: Error) -> Error {
    match error {
        Error::Cancelled => Error::Cancelled,
        Error::Reported(message) => Error::Reported(format!("{command}: {message}")),
        other => Error::Reported(format!("{command}: {other}")),
    }
}

/// The notes of several calls as one; none when none of them had any.
fn merged(notes: impl IntoIterator<Item = Option<String>>) -> Option<String> {
    let notes: Vec<String> = notes.into_iter().flatten().collect();
    (!notes.is_empty()).then(|| notes.join(" · "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failure_names_the_call_that_failed_but_a_cancel_stays_a_cancel() {
        let failed = named("modes", Error::Reported("bad shape".to_string()));
        assert_eq!(failed.to_string(), "kuna reported: modes: bad shape");
        assert!(matches!(
            named("catalog", Error::Cancelled),
            Error::Cancelled
        ));
    }

    #[test]
    fn the_notes_of_several_calls_are_one_in_order_and_none_when_all_are_silent() {
        let said = |text: &str| Some(text.to_string());
        assert_eq!(merged([None, said("a"), said("b")]), said("a · b"));
        assert_eq!(merged([None, None, None]), None);
    }

    const CATALOG: &str = include_str!("../../tests/fixtures/catalog-sample.json");
    const MODES: &str = include_str!("../../tests/fixtures/modes.json");
    const TOPICS: &str = include_str!("../../tests/fixtures/docs-topics.json");

    #[test]
    fn parses_the_real_catalog_rows() {
        let rows: Vec<CatalogRow> = serde_json::from_str(CATALOG).expect("parses");
        let names: Vec<&str> = rows.iter().map(|row| row.option.as_str()).collect();
        assert_eq!(names, ["compareform", "namestyle", "objcsdk"]);
        let compare = &rows[0];
        assert_eq!(compare.values, ["canonical", "original"]);
        assert_eq!(
            (compare.default.as_str(), compare.tier.as_str()),
            ("original", "core")
        );
        assert!(compare.summary.starts_with("Whether comparisons"));
        assert_eq!(rows[2].values, ["<directory>", "off"]);
    }

    #[test]
    fn parses_the_real_modes() {
        let document: Modes = serde_json::from_str(MODES).expect("parses");
        let names: Vec<&str> = document
            .modes
            .iter()
            .map(|mode| mode.name.as_str())
            .collect();
        assert_eq!(names, ["auto", "reliable", "aggressive", "fast"]);
        assert!(document.modes[0].automatic && !document.modes[1].automatic);
        assert_eq!(document.modes[0].overrides[0].option.as_str(), "ctypes");
        assert_eq!(document.modes[0].overrides[0].value.as_str(), "on");
        assert!(document.modes[1].overrides.is_empty());
    }

    #[test]
    fn parses_the_real_topics() {
        let topics: Vec<Topic> = serde_json::from_str(TOPICS).expect("parses");
        let names: Vec<&str> = topics.iter().map(|topic| topic.topic.as_str()).collect();
        assert_eq!(names, ["cli", "options", "agents", "phases", "modes"]);
        assert_eq!(topics[1].bytes, 905_570);
    }

    #[test]
    fn rows_without_their_keys_are_rejected_and_hostile_text_is_cleaned() {
        assert!(serde_json::from_str::<Vec<CatalogRow>>(r#"[{"option":"a"}]"#).is_err());
        assert!(serde_json::from_str::<Modes>(r#"{"presets":[]}"#).is_err());
        let text = r#"[{"topic":"a\u001b[2J","title":"t","summary":"s","bytes":1}]"#;
        let topics: Vec<Topic> = serde_json::from_str(text).expect("parses");
        assert_eq!(topics[0].topic.as_str(), "a [2J");
    }
}
