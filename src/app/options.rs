//! The commands about kuna itself: its option catalog, its modes and its docs, and the options a decompile runs with.

use super::{App, Request};
use crate::keymap::View;
use crate::kuna::{self, Reference, Reply};
use crate::options::{Options, Shared, check_name};
use std::sync::Arc;

/// The title of a view that is about kuna, not about a function of the binary.
const KUNA: &str = "kuna";

/// What kuna says about itself (options, modes, docs), as far as it is known.
pub enum Knowledge {
    /// The request is still running.
    Loading,
    /// The data, and what kuna said while still succeeding.
    Ready(Reference, Option<String>),
    /// The request failed: why.
    Failed(String),
}

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

    /// An answer of the worker. A decompile made with options that are no longer the current ones is dropped (it was
    /// asked before `:set`, and the one shown was asked again); any other answer stands, whatever the options became.
    pub(super) fn on_code(
        &mut self,
        request: Request,
        generation: u64,
        answer: Result<Reply<Vec<String>>, kuna::Error>,
    ) {
        if request.view == View::Decompile && generation != self.shared.generation() {
            return;
        }
        self.answered(request, answer);
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
        let reference = match self.known() {
            Ok(reference) => reference,
            Err(why) => return self.say(&why),
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

    /// The options a decompile runs with, shared with the worker: `main` gives the worker the app's handle.
    pub fn shared(&self) -> Arc<Shared> {
        Arc::clone(&self.shared)
    }

    /// The options a decompile runs with.
    pub fn options(&self) -> Options {
        self.shared.options()
    }

    /// `decompile options: mode=reliable namestyle=ghidra` while any is set, for the status line.
    pub fn options_note(&self) -> Option<String> {
        let options = self.options();
        (!options.is_default()).then(|| format!("decompile options: {options}"))
    }

    /// The reference data, or why it is not there (still loading, or failed).
    fn known(&self) -> Result<&Reference, String> {
        match &self.reference {
            Knowledge::Ready(reference, _) => Ok(reference),
            Knowledge::Loading => Err("the reference is still loading".to_string()),
            Knowledge::Failed(why) => Err(format!("the reference could not be loaded: {why}")),
        }
    }

    /// `:set` lists, `:set NAME` shows, `:set NAME&` resets, `:set NAME VALUE` sets; `force` (`:set!`) does not check
    /// the catalog, so an option that is not one of its rows can be set. The characters are checked either way.
    pub(super) fn set_command(&mut self, args: &str, force: bool) {
        let words: Vec<&str> = args.split_whitespace().collect();
        match words.as_slice() {
            [] => {
                let note = self.options_note();
                self.say(note.as_deref().unwrap_or("no options set"));
            }
            [name] => match name.strip_suffix('&') {
                Some(name) => self.reset_option(name),
                None => self.show_option(name),
            },
            [name, value] => self.assign_option(name, value, force),
            _ => self.say("usage: :set NAME VALUE, :set NAME&, :set NAME"),
        }
    }

    /// `:set NAME`: its value, or the catalog's default for it.
    fn show_option(&mut self, name: &str) {
        if let Err(refused) = check_name(name) {
            return self.say(&refused.to_string());
        }
        let said = match self.options().get(name) {
            Some(value) => format!("{name} = {value}"),
            None => {
                let row = self
                    .known()
                    .ok()
                    .and_then(|reference| reference.catalog.iter().find(|row| row.option == name));
                match row {
                    Some(row) => format!("{name} is {} (default)", row.default.as_str()),
                    None => format!("{name} is not set"),
                }
            }
        };
        self.say(&said);
    }

    /// `:set NAME&`: kuna's default for `name` again.
    fn reset_option(&mut self, name: &str) {
        if let Err(refused) = check_name(name) {
            return self.say(&refused.to_string());
        }
        let options = self.options();
        if options.get(name).is_none() {
            return self.say(&format!("{name} is not set"));
        }
        self.use_options(
            options.without_option(name),
            format!("{name} reset to its default"),
        );
    }

    /// `:set NAME VALUE`.
    fn assign_option(&mut self, name: &str, value: &str, force: bool) {
        let next = match self.options().with_option(name, value) {
            Ok(next) => next,
            Err(refused) => return self.say(&refused.to_string()),
        };
        if !force {
            let reference = match self.known() {
                Ok(reference) => reference,
                Err(why) => return self.say(&format!("{why} (:set! sends it anyway)")),
            };
            if let Err(why) = in_catalog(reference, name, value) {
                return self.say(&why);
            }
        }
        self.use_options(next, format!("{name} = {value}"));
    }

    /// `:mode` shows the mode, `:mode NAME` sets one of kuna's modes (`auto` is kuna's default and clears it);
    /// `force` (`:mode!`) does not check the list of modes.
    pub(super) fn mode_command(&mut self, name: &str, force: bool) {
        if name.is_empty() {
            let said = match self.options().mode() {
                Some(mode) => format!("mode: {mode}"),
                None => "mode: auto (kuna picks by input size)".to_string(),
            };
            return self.say(&said);
        }
        let next = match self.options().with_mode(name) {
            Ok(next) => next,
            Err(refused) => return self.say(&refused.to_string()),
        };
        if !force {
            let reference = match self.known() {
                Ok(reference) => reference,
                Err(why) => return self.say(&format!("{why} (:mode! sends it anyway)")),
            };
            if let Err(why) = in_modes(reference, name) {
                return self.say(&why);
            }
        }
        self.use_options(next, format!("mode = {name}"));
    }

    /// Makes `options` the ones to decompile with, if they differ: the decompiles kept are made with the old ones
    /// and are dropped, and the one shown is asked for again. Says `said` either way.
    fn use_options(&mut self, options: Options, said: String) {
        if options != self.options() {
            self.shared.replace(options);
            self.views
                .retain(|request, _| request.view != View::Decompile);
            let shown = self.shown.as_ref().map(|shown| shown.request);
            if let Some(request) = shown.filter(|request| request.view == View::Decompile) {
                self.ask(request);
            }
        }
        self.say(&said);
    }

    /// Answers the last key or command.
    fn say(&mut self, message: &str) {
        self.message = Some(message.to_string());
    }
}

/// `value` for `name` if the catalog has such an option and accepts the value (any value when it takes free text,
/// which its values list shows as a placeholder like `<directory>`); otherwise what is wrong.
fn in_catalog(reference: &Reference, name: &str, value: &str) -> Result<(), String> {
    let Some(row) = reference.catalog.iter().find(|row| row.option == name) else {
        return Err(format!(
            "no such option: {name} (see :catalog; :set! sends it anyway)"
        ));
    };
    let free_text = row.values.iter().any(|accepted| accepted.starts_with('<'));
    if free_text || row.values.iter().any(|accepted| accepted.as_str() == value) {
        return Ok(());
    }
    let accepted: Vec<&str> = row
        .values
        .iter()
        .map(|accepted| accepted.as_str())
        .collect();
    Err(format!(
        "{name} must be one of: {} (:set! sends it anyway)",
        accepted.join(", ")
    ))
}

/// `Ok` if kuna has a mode called `name`; otherwise the modes it has.
fn in_modes(reference: &Reference, name: &str) -> Result<(), String> {
    if reference.modes.iter().any(|mode| mode.name == name) {
        return Ok(());
    }
    let known: Vec<&str> = reference
        .modes
        .iter()
        .map(|mode| mode.name.as_str())
        .collect();
    Err(format!(
        "no such mode: {name} (known: {}; :mode! sends it anyway)",
        known.join(", ")
    ))
}
