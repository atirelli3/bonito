//! The options a decompile runs with: kuna's `--mode` and `--option NAME VALUE`, chosen by the user.

use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};

/// The longest option name, value or mode that is accepted.
const MAX_LEN: usize = 100;
/// How many different options can be set: each one is three arguments of kuna's command line.
const MAX_OPTIONS: usize = 64;

/// Why a name, a value or a mode was refused; the text is ready for the status line.
#[derive(Debug, PartialEq, Eq)]
pub struct Refused(String);

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refused {}

/// The mode and the options of a decompile. Every change returns a new value, so one handed to the worker never
/// changes under it. Text is checked on the way in, so `args` can go to kuna's argv as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    mode: Option<String>,
    set: Vec<(String, String)>,
}

impl Options {
    /// The mode; `auto` (kuna picks one by input size) is the same as none.
    pub fn with_mode(&self, mode: &str) -> Result<Self, Refused> {
        check(
            "mode",
            mode,
            is_word,
            "only letters, digits and _ are allowed",
        )?;
        let mode = (mode != "auto").then(|| mode.to_string());
        Ok(Self {
            mode,
            set: self.set.clone(),
        })
    }

    /// `name` set to `value`: a name set before keeps its place.
    pub fn with_option(&self, name: &str, value: &str) -> Result<Self, Refused> {
        check_name(name)?;
        if name == "mode" {
            return Err(Refused(format!(
                "option name {name:?}: set the mode with :mode"
            )));
        }
        check(
            "option value",
            value,
            is_value,
            "only letters, digits and _ . : / + - are allowed",
        )?;
        if value.starts_with("//") || value.contains("://") {
            return Err(Refused(format!(
                "option value {value:?}: must not name a network location"
            )));
        }
        let mut set = self.set.clone();
        match set.iter().position(|(known, _)| known == name) {
            Some(at) => set[at].1 = value.to_string(),
            None if set.len() >= MAX_OPTIONS => {
                return Err(Refused(format!("too many options: at most {MAX_OPTIONS}")));
            }
            None => set.push((name.to_string(), value.to_string())),
        }
        Ok(Self {
            mode: self.mode.clone(),
            set,
        })
    }

    /// Without `name`: kuna's default for it applies again.
    pub fn without_option(&self, name: &str) -> Self {
        Self {
            mode: self.mode.clone(),
            set: self
                .set
                .iter()
                .filter(|(known, _)| known != name)
                .cloned()
                .collect(),
        }
    }

    /// The mode, if one is set; none is kuna's `auto`.
    pub fn mode(&self) -> Option<&str> {
        self.mode.as_deref()
    }

    /// The value `name` was set to, if it was.
    pub fn get(&self, name: &str) -> Option<&str> {
        let found = self.set.iter().find(|(known, _)| known == name);
        found.map(|(_, value)| value.as_str())
    }

    /// Whether nothing is set: kuna runs as if bonito were not there.
    pub fn is_default(&self) -> bool {
        self.mode.is_none() && self.set.is_empty()
    }

    /// The arguments kuna gets: the mode, then `--option NAME VALUE` for each option in the order it was set.
    pub fn args(&self) -> Vec<String> {
        let mode = self
            .mode
            .iter()
            .flat_map(|mode| ["--mode".to_string(), mode.clone()]);
        let set = self
            .set
            .iter()
            .flat_map(|(name, value)| ["--option".to_string(), name.clone(), value.clone()]);
        mode.chain(set).collect()
    }
}

/// The options the next decompile runs with, shared by the app (which changes them) and the worker (which reads them
/// before each decompile), and a generation that counts the changes: an answer made with an older one is stale.
/// The options and their generation sit under one lock, so a reader never sees new options with an old generation.
#[derive(Debug, Default)]
pub struct Shared {
    state: Mutex<(Options, u64)>,
}

impl Shared {
    /// The options now, and the generation they belong to.
    pub fn current(&self) -> (Options, u64) {
        let state = self.lock();
        (state.0.clone(), state.1)
    }

    /// The options now.
    pub fn options(&self) -> Options {
        self.lock().0.clone()
    }

    /// The generation of the options now.
    pub fn generation(&self) -> u64 {
        self.lock().1
    }

    /// Makes `options` the ones to use; returns the new generation.
    pub fn replace(&self, options: Options) -> u64 {
        let mut state = self.lock();
        *state = (options, state.1.saturating_add(1));
        state.1
    }

    /// The state; a panic elsewhere cannot leave it half written, so a poisoned lock is still good.
    fn lock(&self) -> MutexGuard<'_, (Options, u64)> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// `mode=reliable namestyle=ghidra`, for the status line; nothing when nothing is set.
impl fmt::Display for Options {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mode = self.mode.iter().map(|mode| format!("mode={mode}"));
        let set = self
            .set
            .iter()
            .map(|(name, value)| format!("{name}={value}"));
        let parts: Vec<String> = mode.chain(set).collect();
        f.write_str(&parts.join(" "))
    }
}

/// Letters, digits and `_`: an option name or a mode.
fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// What an option value may hold: a word, a number, a path-like name.
fn is_value(c: char) -> bool {
    is_word(c) || matches!(c, '.' | ':' | '/' | '+' | '-')
}

/// Whether `name` could be the name of an option: what `with_option` asks of it before anything else.
pub fn check_name(name: &str) -> Result<(), Refused> {
    check(
        "option name",
        name,
        is_word,
        "only letters, digits and _ are allowed",
    )
}

/// `text` as a `kind` (`option name`…): not empty, not long, not starting with `-` (kuna would read a flag) and
/// made of what `allowed` accepts.
fn check(kind: &str, text: &str, allowed: fn(char) -> bool, rule: &str) -> Result<(), Refused> {
    if text.is_empty() {
        return Err(Refused(format!("{kind} is empty")));
    }
    if text.chars().count() > MAX_LEN {
        return Err(Refused(format!(
            "{kind} is longer than {MAX_LEN} characters"
        )));
    }
    if text.starts_with('-') {
        return Err(Refused(format!("{kind} {text:?}: must not start with -")));
    }
    if !text.chars().all(allowed) {
        return Err(Refused(format!("{kind} {text:?}: {rule}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
