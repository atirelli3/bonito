//! Text from kuna that traces back to a hostile binary, cleaned as it is parsed so nothing downstream can
//! print it as a terminal escape.

use serde::{Deserialize, Deserializer};
use std::ops::Deref;

/// The most characters a name or message keeps.
const TEXT_LIMIT: usize = 1024;

/// Whether a character could change what a terminal shows or hide what a name says: controls, line
/// separators, bidirectional overrides, and invisible, filler or tag characters.
pub fn is_unsafe(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{061C}'
                | '\u{115F}'
                | '\u{180E}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{3164}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}'
                | '\u{E0000}'..='\u{E007F}'
        )
}

/// `text` on one line: unsafe characters become spaces, and at most `limit` characters are kept.
pub fn single_line(text: &str, limit: usize) -> String {
    text.chars()
        .map(|c| if is_unsafe(c) { ' ' } else { c })
        .take(limit)
        .collect()
}

/// A name or message from kuna, safe to print.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Text(String);

impl Text {
    /// The cleaned text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Deref for Text {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl PartialEq<&str> for Text {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl<'de> Deserialize<'de> for Text {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(|raw| Self(single_line(&raw, TEXT_LIMIT)))
    }
}

/// C text from kuna: keeps lines and tabs, replaces every other unsafe character.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Code(String);

impl Code {
    /// The cleaned text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Deref for Code {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Code {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(Self(
            raw.chars()
                .map(|c| {
                    if is_unsafe(c) && c != '\n' && c != '\t' {
                        ' '
                    } else {
                        c
                    }
                })
                .collect(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_line_replaces_controls_and_bidi_overrides_with_spaces() {
        assert_eq!(single_line("a\u{1b}[31mb\nc\u{202e}d", 100), "a [31mb c d");
    }

    #[test]
    fn single_line_replaces_invisible_and_line_separator_characters() {
        let hidden = "a\u{2028}b\u{200b}c\u{feff}d\u{e0041}e\u{61c}f";
        assert_eq!(single_line(hidden, 100), "a b c d e f");
    }

    #[test]
    fn single_line_keeps_at_most_the_limit() {
        assert_eq!(single_line("abcdef", 3), "abc");
    }

    #[test]
    fn text_is_cleaned_while_parsing() {
        let text: Text = serde_json::from_str(r#""a\u001b]0;pwn\u0007b""#).expect("parses");
        assert_eq!(text.as_str(), "a ]0;pwn b");
    }

    #[test]
    fn text_is_capped() {
        let json = format!("\"{}\"", "x".repeat(TEXT_LIMIT + 500));
        let text: Text = serde_json::from_str(&json).expect("parses");
        assert_eq!(text.len(), TEXT_LIMIT);
    }

    #[test]
    fn code_keeps_lines_and_tabs_but_not_escapes() {
        let code: Code = serde_json::from_str(r#""a\n\tb\u001bc""#).expect("parses");
        assert_eq!(code.as_str(), "a\n\tb c");
    }
}
