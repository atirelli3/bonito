//! Typed views of the JSON kuna prints: unknown fields are ignored, the fields bonito relies on are required,
//! and text that comes from the analysed binary is cleaned as it is parsed.

use super::text::{Code, Text};
use serde::Deserialize;

/// One function kuna found in the binary.
#[derive(Debug, Deserialize)]
pub struct Function {
    /// A symbol name, or kuna's generated `sub_<addr>`.
    pub name: Text,
    /// The entry address.
    pub address: u64,
    /// The extent in bytes.
    pub size: u64,
    /// Other names for the same address.
    #[serde(default)]
    pub aliases: Vec<Text>,
}

/// What `kuna functions --json` prints.
#[derive(Debug, Deserialize)]
pub struct Functions {
    /// Why discovery failed, if it did.
    pub error: Option<Text>,
    pub functions: Vec<Function>,
}

/// What `kuna decompile --json` prints.
#[derive(Debug, Deserialize)]
pub struct Decompilation {
    /// Why decompilation failed, if it did.
    pub error: Option<Text>,
    pub functions: Vec<Decompiled>,
}

/// One decompiled function.
#[derive(Debug, Deserialize)]
pub struct Decompiled {
    pub name: Text,
    pub address: u64,
    /// The C text.
    pub code: Option<Code>,
    /// Why this function failed, if it did.
    pub error: Option<Text>,
    pub warnings: Vec<Warning>,
}

/// Something kuna noticed while decompiling.
#[derive(Debug, Deserialize)]
pub struct Warning {
    /// A stable code such as `tail-call`.
    pub kind: Text,
    pub message: Text,
    /// The line of the C text it is about.
    pub line: Option<u32>,
    pub address: Option<u64>,
}

/// What `kuna disassemble --as code --json` prints.
#[derive(Debug, Deserialize)]
pub struct Disassembly {
    /// What was asked for; `target.name` is none when the address is not a function start.
    pub target: Target,
    /// The first address listed.
    pub start: u64,
    /// One past the last address listed: where to resume when `truncated`.
    pub end: u64,
    /// Whether kuna stopped before the end of the range.
    pub truncated: bool,
    /// What kuna wants the user to know about the listing.
    pub notes: Vec<Text>,
    pub instructions: Vec<Instruction>,
}

/// The address a disassembly started from.
#[derive(Debug, Deserialize)]
pub struct Target {
    /// The function's name, when a function starts there.
    pub name: Option<Text>,
    pub address: u64,
}

/// One decoded instruction.
#[derive(Debug, Deserialize)]
pub struct Instruction {
    pub address: u64,
    /// The encoded bytes, as hex.
    pub bytes: Text,
    pub mnemonic: Text,
    pub operands: Text,
    /// `mnemonic operands`, ready to show.
    pub text: Text,
}

#[cfg(test)]
mod tests {
    use super::*;

    const FUNCTIONS: &str = include_str!("../../tests/fixtures/functions.json");

    #[test]
    fn parses_the_real_functions_output() {
        let document: Functions = serde_json::from_str(FUNCTIONS).expect("parses");
        let names: Vec<&str> = document.functions.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["square", "add", "main", "_start"]);
        let square = &document.functions[0];
        assert_eq!((square.address, square.size), (0x201120, 16));
        assert_eq!(square.aliases, ["sub_201120"]);
        assert_eq!(document.error, None);
    }

    #[test]
    fn ignores_unknown_fields_and_defaults_optional_ones() {
        let text =
            r#"{"future":1,"functions":[{"name":"f","address":16,"size":4,"novel":{"a":[1]}}]}"#;
        let document: Functions = serde_json::from_str(text).expect("parses");
        let function = &document.functions[0];
        assert_eq!((function.size, function.aliases.len()), (4, 0));
    }

    #[test]
    fn a_function_without_a_size_is_rejected() {
        let text = r#"{"functions":[{"name":"f","address":16}]}"#;
        assert!(serde_json::from_str::<Functions>(text).is_err());
    }

    #[test]
    fn a_document_without_functions_is_rejected() {
        assert!(serde_json::from_str::<Functions>(r#"{"count":0}"#).is_err());
    }

    #[test]
    fn a_function_without_an_address_is_rejected() {
        let text = r#"{"functions":[{"name":"f"}]}"#;
        assert!(serde_json::from_str::<Functions>(text).is_err());
    }

    #[test]
    fn keeps_the_error_kuna_reports() {
        let text = r#"{"error":"packed image","functions":[]}"#;
        let document: Functions = serde_json::from_str(text).expect("parses");
        assert_eq!(document.error.as_deref(), Some("packed image"));
    }

    const DECOMPILE: &str = include_str!("../../tests/fixtures/decompile-main.json");
    const DISASSEMBLE: &str = include_str!("../../tests/fixtures/disassemble-main.json");

    #[test]
    fn parses_the_real_decompile_output() {
        let document: Decompilation = serde_json::from_str(DECOMPILE).expect("parses");
        let main = &document.functions[0];
        assert_eq!((main.name.as_str(), main.address), ("main", 0x201140));
        assert!(main.code.as_deref().expect("code").contains("square(3)"));
        let warning = &main.warnings[0];
        assert_eq!(
            (warning.kind.as_str(), warning.line),
            ("tail-call", Some(6))
        );
    }

    #[test]
    fn a_decompiled_function_may_have_no_code() {
        let text = r#"{"error":null,"functions":[{"name":"f","address":16,"code":null,"error":"too big","warnings":[]}]}"#;
        let document: Decompilation = serde_json::from_str(text).expect("parses");
        let function = &document.functions[0];
        assert_eq!(
            (function.code.as_deref(), function.error.as_deref()),
            (None, Some("too big"))
        );
    }

    #[test]
    fn parses_the_real_disassembly() {
        let disassembly: Disassembly = serde_json::from_str(DISASSEMBLE).expect("parses");
        assert_eq!(disassembly.instructions.len(), 8);
        let first = &disassembly.instructions[0];
        assert_eq!((first.address, first.mnemonic.as_str()), (0x201140, "PUSH"));
        assert_eq!(
            (first.operands.as_str(), first.text.as_str()),
            ("RAX", "PUSH RAX")
        );
        assert!(!disassembly.truncated && disassembly.notes.is_empty());
    }

    #[test]
    fn a_disassembly_keeps_what_it_started_from() {
        let disassembly: Disassembly = serde_json::from_str(DISASSEMBLE).expect("parses");
        assert_eq!(disassembly.target.name.as_deref(), Some("main"));
        assert_eq!(disassembly.target.address, 0x201140);
        assert_eq!((disassembly.start, disassembly.end), (0x201140, 0x201160));
    }

    #[test]
    fn names_from_the_binary_are_cleaned_while_parsing() {
        let text = format!(
            r#"{{"functions":[{{"name":"evil\u001b[2J","address":1,"size":1,"aliases":["a{}b"]}}]}}"#,
            '\u{202e}'
        );
        let document: Functions = serde_json::from_str(&text).expect("parses");
        let function = &document.functions[0];
        assert_eq!(function.name.as_str(), "evil [2J");
        assert_eq!(function.aliases, ["a b"]);
    }

    #[test]
    fn a_hexdump_is_not_a_disassembly() {
        let text = r#"{"kind":"data","truncated":false,"notes":[],"rows":[]}"#;
        assert!(serde_json::from_str::<Disassembly>(text).is_err());
    }
}
