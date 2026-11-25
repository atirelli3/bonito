//! The commands about kuna itself: its option catalog, its modes and its docs.

use super::{App, Knowledge, Request};
use crate::keymap::View;
use crate::kuna::{self, Reference, Reply};

/// The title of a view that is about kuna, not about a function of the binary.
const KUNA: &str = "kuna";

impl App {
    /// The reference data arrived, or failed: `:docs TOPIC` says which when it is asked before it is known.
    pub(super) fn on_reference(&mut self, answer: Result<Reply<Reference>, kuna::Error>) {
        self.reference = match answer {
            Ok(reply) => Knowledge::Ready(reply.data, reply.notes),
            Err(error) => Knowledge::Failed(error.to_string()),
        };
    }

    /// What kuna said loading its reference data, or why it could not: for the status line.
    pub fn reference_note(&self) -> Option<String> {
        match &self.reference {
            Knowledge::Loading => None,
            Knowledge::Ready(_, notes) => notes.clone(),
            Knowledge::Failed(why) => Some(format!(
                "kuna's options and docs could not be loaded: {why}"
            )),
        }
    }

    /// `:catalog` and `:modes`: kuna's own answer, asked of the worker like any view.
    pub(super) fn open_reference(&mut self, view: View) {
        self.jump(Request { view, address: 0 }, KUNA.to_string());
    }

    /// `:docs`: the list of topics; `:docs TOPIC`: that document.
    pub(super) fn open_docs(&mut self, topic: &str) {
        if topic.is_empty() {
            return self.open_reference(View::Docs);
        }
        let reference = match &self.reference {
            Knowledge::Ready(reference, _) => reference,
            Knowledge::Loading => return self.say("the reference is still loading"),
            Knowledge::Failed(why) => {
                let message = format!("the reference could not be loaded: {why}");
                return self.say(&message);
            }
        };
        let Some(at) = reference
            .topics
            .iter()
            .position(|known| known.topic == topic)
        else {
            let known: Vec<&str> = reference
                .topics
                .iter()
                .map(|known| known.topic.as_str())
                .collect();
            let message = format!("no topic: {topic} (known: {})", known.join(", "));
            return self.say(&message);
        };
        let Ok(number) = u64::try_from(at + 1) else {
            return self.say("no such topic");
        };
        self.jump(
            Request {
                view: View::Docs,
                address: number,
            },
            topic.to_string(),
        );
    }

    /// Answers the last key or command.
    fn say(&mut self, message: &str) {
        self.message = Some(message.to_string());
    }
}
