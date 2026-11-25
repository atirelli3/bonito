//! Typed views of the JSON kuna prints: unknown fields are ignored, the fields bonito relies on are required,
//! and text that comes from the analysed binary is cleaned as it is parsed.
// ponytail: the fields are kuna's contract; the views of #51, #52 and #9 read them as they land.
#![allow(dead_code)]

use super::text::{Code, Text};
use serde::Deserialize;
use serde::de::IgnoredAny;

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

/// What `kuna xrefs --json` prints.
#[derive(Debug, Deserialize)]
pub struct Xrefs {
    /// What was asked for.
    pub target: Target,
    pub coverage: Coverage,
    /// The references in code: calls, jumps, reads and writes.
    pub xrefs: Vec<Xref>,
    /// How much of the image's relocation metadata kuna read: `partial` and `unsupported` mean `data_refs` is short.
    pub data_refs_coverage: Text,
    pub data_refs_reason: Option<Text>,
    /// Words the image stores that hold the target's address.
    pub data_refs: Vec<DataRef>,
}

/// What kuna decoded for an xrefs query.
#[derive(Debug, Deserialize)]
pub struct Coverage {
    /// Whether kuna decoded the target: not so for data and unmapped addresses, which no code reference names.
    pub decoded: bool,
    /// Code the walk did not decode, so references may be missing; only how many matters.
    #[serde(default)]
    pub unwalked: Vec<IgnoredAny>,
    /// What kuna wants the user to know about the walk.
    pub notes: Vec<Text>,
}

/// One reference in code.
#[derive(Debug, Deserialize)]
pub struct Xref {
    /// Where the referencing instruction is.
    pub from_address: u64,
    /// `call`, `jump`, `data`, `read`, `write` or `pointer`.
    pub kind: Text,
    /// The function the instruction is in, if it is in one.
    pub from_function: Option<Located>,
    pub instruction: Text,
}

/// A function by name and entry address.
#[derive(Debug, Deserialize)]
pub struct Located {
    pub name: Text,
    pub address: u64,
}

/// A word the image stores that holds an address.
#[derive(Debug, Deserialize)]
pub struct DataRef {
    pub kind: Text,
    /// Where the word is.
    pub from_address: u64,
    pub from_name: Option<Text>,
}

/// The address a disassembly or an xrefs query started from.
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

    const XREFS: &str = include_str!("../../tests/fixtures/xrefs-square.json");

    #[test]
    fn parses_the_real_xrefs_output() {
        let document: Xrefs = serde_json::from_str(XREFS).expect("parses");
        assert_eq!(document.target.name.as_deref(), Some("square"));
        assert_eq!(document.xrefs.len(), 2);
        let first = &document.xrefs[0];
        assert_eq!(first.from_address, 0x201146);
        assert_eq!(
            (first.kind.as_str(), first.instruction.as_str()),
            ("call", "CALL 0x201120")
        );
        let from = first.from_function.as_ref().expect("inside a function");
        assert_eq!((from.name.as_str(), from.address), ("main", 0x201140));
        assert!(document.data_refs.is_empty() && document.coverage.notes.is_empty());
        assert_eq!(document.data_refs_coverage.as_str(), "none");
    }

    #[test]
    fn an_xrefs_document_without_its_references_is_rejected() {
        let text = r#"{"target":{"name":"t","address":1},"coverage":{"decoded":true,"notes":[]},"data_refs":[],"data_refs_coverage":"none"}"#;
        assert!(serde_json::from_str::<Xrefs>(text).is_err());
    }

    #[test]
    fn coverage_without_decoded_is_rejected() {
        let text = r#"{"target":{"name":"t","address":1},"coverage":{"notes":[]},"data_refs_coverage":"none","xrefs":[],"data_refs":[]}"#;
        assert!(serde_json::from_str::<Xrefs>(text).is_err());
    }

    #[test]
    fn text_in_xrefs_is_cleaned_while_parsing() {
        let text = r#"{"target":{"name":null,"address":1},"coverage":{"decoded":true,"notes":[]},"data_refs_coverage":"none",
            "xrefs":[{"from_address":2,"kind":"call","instruction":"CALL\u001b[2J","from_function":null}],
            "data_refs":[{"kind":"pointer","from_address":3,"from_name":"evil\u001b[2J"}]}"#;
        let document: Xrefs = serde_json::from_str(text).expect("parses");
        assert_eq!(document.xrefs[0].instruction.as_str(), "CALL [2J");
        assert!(document.xrefs[0].from_function.is_none());
        assert_eq!(document.data_refs[0].from_name.as_deref(), Some("evil [2J"));
    }
}
