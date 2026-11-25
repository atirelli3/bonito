//! Typed views of the JSON kuna prints: unknown fields are ignored, the fields bonito relies on are required,
//! and text that comes from the analysed binary is cleaned as it is parsed.
// ponytail: the fields are kuna's contract; the views read them as they land.
#![allow(dead_code)]

use super::text::{Code, Text};
use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};

/// One function kuna found in the binary.
#[derive(Debug, Deserialize, Serialize)]
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

/// What `kuna read --json` prints: the bytes of a range, sixteen to a row.
#[derive(Debug, Deserialize)]
pub struct Hexdump {
    pub target: Target,
    /// One past the last address listed: where to resume when `truncated`.
    pub end: u64,
    /// Whether kuna stopped before the end of the range.
    pub truncated: bool,
    /// What kuna wants the user to know about the dump.
    pub notes: Vec<Text>,
    pub rows: Vec<HexRow>,
    /// The relocation metadata the fixups came from, or `partial` when some of it could not be read.
    pub fixups_coverage: Text,
    /// Why the fixups are incomplete, if they are.
    #[serde(deserialize_with = "Option::deserialize")]
    pub fixups_reason: Option<Text>,
    /// Slots the loader rewrites, so the bytes shown are the file's, not what runs; only how many matters.
    pub fixups: Vec<IgnoredAny>,
    /// Where the bytes came from: the file, zero fill or a repaired header.
    pub provenance: Vec<Provenance>,
}

/// A span of a dump and where its bytes came from.
#[derive(Debug, Deserialize)]
pub struct Provenance {
    /// `file`, `zero-fill`, `repaired` or `unknown`.
    pub source: Text,
    /// Other segments that declare the same range; only how many matters.
    #[serde(default)]
    pub overlaps: Vec<IgnoredAny>,
}

/// Up to sixteen bytes.
#[derive(Debug, Deserialize)]
pub struct HexRow {
    pub address: u64,
    /// The bytes as hex digits, two to a byte.
    pub bytes: Text,
    /// The bytes as characters, `.` for those that do not print.
    pub ascii: Text,
}

/// What `kuna raw-pcode --json` prints: each instruction with its SLEIGH p-code.
#[derive(Debug, Deserialize)]
pub struct Pcode {
    pub target: Target,
    /// One past the last address listed: where to resume when `truncated`.
    pub end: u64,
    /// Whether kuna stopped before the end of the range.
    pub truncated: bool,
    /// What kuna wants the user to know about the listing.
    pub notes: Vec<Text>,
    /// Why the walk ended: `range_end` when it reached the end of the range.
    #[serde(deserialize_with = "Option::deserialize")]
    pub stop_reason: Option<Text>,
    /// The instruction that did not lift, if one did not.
    #[serde(deserialize_with = "Option::deserialize")]
    pub error: Option<LiftError>,
    pub instructions: Vec<Lifted>,
}

/// Why a walk ended early.
#[derive(Debug, Deserialize)]
pub struct LiftError {
    pub message: Text,
}

/// One instruction and the operations it lifts to.
#[derive(Debug, Deserialize)]
pub struct Lifted {
    pub address: u64,
    pub mnemonic: Text,
    pub operands: Text,
    pub ops: Vec<Op>,
}

/// One p-code operation.
#[derive(Debug, Deserialize)]
pub struct Op {
    pub opcode: Text,
    /// Where the result goes; null for a store or a branch.
    #[serde(deserialize_with = "Option::deserialize")]
    pub output: Option<Varnode>,
    pub inputs: Vec<Varnode>,
}

/// A p-code operand: `size` bytes at `offset` of an address space.
#[derive(Debug, Deserialize)]
pub struct Varnode {
    /// `register`, `unique`, `const`, `ram`…
    pub space: Text,
    pub offset: u64,
    pub size: u64,
}

/// What `kuna strings --json` prints.
#[derive(Debug, Deserialize)]
pub struct Strings {
    /// `sections`, or `segments` when the section table was unusable and the scan covers whatever is mapped.
    pub scanned: Text,
    pub strings: Vec<FoundString>,
}

/// One string and the functions that use it.
#[derive(Debug, Deserialize)]
pub struct FoundString {
    pub address: u64,
    pub text: Text,
    /// The section it lies in, null when kuna could not tell.
    #[serde(deserialize_with = "Option::deserialize")]
    pub section: Option<Text>,
    pub functions: Vec<Located>,
}

/// What `kuna crypto --json` prints.
#[derive(Debug, Deserialize)]
pub struct Crypto {
    /// `sections`, or `segments` when the section table was unusable.
    pub scanned: Text,
    pub constants: Vec<Constant>,
}

/// One well-known crypto constant and the functions that use it.
#[derive(Debug, Deserialize)]
pub struct Constant {
    pub address: u64,
    /// `SHA-256`, `TEA/XTEA`…
    pub algorithm: Text,
    /// Which constant of the algorithm: `K`, `delta 0x9E3779B9`…
    pub name: Text,
    #[serde(deserialize_with = "Option::deserialize")]
    pub section: Option<Text>,
    pub functions: Vec<Located>,
}

/// What `kuna blobs --json` prints.
#[derive(Debug, Deserialize)]
pub struct Blobs {
    /// `sections`, or `segments` when the section table was unusable.
    pub scanned: Text,
    pub blobs: Vec<Blob>,
}

/// One whole file stored in the image (`png`, `gzip`, `zip`…) and the functions that use it.
#[derive(Debug, Deserialize)]
pub struct Blob {
    pub address: u64,
    pub kind: Text,
    /// Its exact length in bytes.
    pub length: u64,
    #[serde(deserialize_with = "Option::deserialize")]
    pub section: Option<Text>,
    pub functions: Vec<Located>,
}

/// What `kuna functions --summary --json` prints: where to start.
#[derive(Debug, Deserialize)]
pub struct Summary {
    /// Why discovery failed, if it did.
    pub error: Option<Text>,
    /// The loader's header repairs: the answer is degraded.
    pub warnings: Vec<Text>,
    /// How many functions kuna found.
    pub total: u64,
    pub summary: Overview,
}

/// The numbers of a summary.
#[derive(Debug, Deserialize)]
pub struct Overview {
    #[serde(deserialize_with = "Option::deserialize")]
    pub entry: Option<Located>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub main: Option<Located>,
    /// How many functions the entry point reaches; null when there is no entry point or the image is packed.
    #[serde(deserialize_with = "Option::deserialize")]
    pub reachable_from_entry: Option<u64>,
    /// Functions nothing calls.
    pub no_callers: u64,
    pub code_bytes: u64,
    pub size_buckets: Vec<SizeBucket>,
    /// The largest functions, largest first.
    pub largest: Vec<Largest>,
}

/// How many functions have a size in a range: `16-63`, `4096+`.
#[derive(Debug, Deserialize)]
pub struct SizeBucket {
    pub bucket: Text,
    pub count: u64,
}

/// One of the largest functions.
#[derive(Debug, Deserialize)]
pub struct Largest {
    pub name: Text,
    pub address: u64,
    pub size: u64,
}

/// The address a disassembly, a dump or an xrefs query started from.
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

    const HEXDUMP: &str = include_str!("../../tests/fixtures/read-main.json");
    const PCODE: &str = include_str!("../../tests/fixtures/raw-pcode-main.json");

    #[test]
    fn parses_the_real_hexdump() {
        let dump: Hexdump = serde_json::from_str(HEXDUMP).expect("parses");
        assert_eq!(
            (dump.target.address, dump.end, dump.truncated),
            (0x201140, 0x201160, false)
        );
        assert_eq!(dump.rows.len(), 2);
        let row = &dump.rows[0];
        assert_eq!(row.address, 0x201140);
        assert_eq!(row.bytes.as_str(), "50bf03000000e8d5ffffff89c7be0400");
        assert_eq!(row.ascii.as_str(), "P...............");
        assert!(dump.notes.is_empty());
    }

    #[test]
    fn a_disassembly_is_not_a_hexdump() {
        assert!(serde_json::from_str::<Hexdump>(DISASSEMBLE).is_err());
    }

    #[test]
    fn parses_the_real_pcode_listing() {
        let listing: Pcode = serde_json::from_str(PCODE).expect("parses");
        assert_eq!(listing.instructions.len(), 8);
        assert_eq!(listing.stop_reason.as_deref(), Some("range_end"));
        assert!(listing.error.is_none() && !listing.truncated);
        let push = &listing.instructions[0];
        assert_eq!(
            (push.mnemonic.as_str(), push.operands.as_str()),
            ("PUSH", "RAX")
        );
        assert_eq!(push.ops.len(), 3);
        let copy = &push.ops[0];
        let out = copy.output.as_ref().expect("COPY has an output");
        assert_eq!(copy.opcode.as_str(), "COPY");
        assert_eq!(
            (out.space.as_str(), out.offset, out.size),
            ("unique", 0x4f900, 8)
        );
        assert_eq!(copy.inputs.len(), 1);
        assert!(push.ops[2].output.is_none() && push.ops[2].inputs.len() == 3);
    }

    #[test]
    fn a_disassembly_is_not_a_pcode_listing() {
        assert!(serde_json::from_str::<Pcode>(DISASSEMBLE).is_err());
    }

    /// `text` as JSON with `key` removed from the top-level object, or from `inside` it.
    fn without(text: &str, key: &str, inside: Option<&str>) -> String {
        let mut value: serde_json::Value = serde_json::from_str(text).expect("json");
        let object = match inside {
            Some(name) => value[name].as_object_mut(),
            None => value.as_object_mut(),
        };
        object.expect("an object").remove(key);
        value.to_string()
    }

    #[test]
    fn keys_that_may_be_null_must_still_be_there() {
        for key in ["error", "stop_reason"] {
            let text = without(PCODE, key, None);
            assert!(serde_json::from_str::<Pcode>(&text).is_err(), "{key}");
        }
        let mut value: serde_json::Value = serde_json::from_str(PCODE).expect("json");
        value["instructions"][0]["ops"][0]
            .as_object_mut()
            .expect("an op")
            .remove("output");
        assert!(serde_json::from_str::<Pcode>(&value.to_string()).is_err());
        let text = without(HEXDUMP, "fixups_reason", None);
        assert!(serde_json::from_str::<Hexdump>(&text).is_err());
    }

    #[test]
    fn a_hexdump_without_its_load_metadata_is_rejected() {
        for key in ["fixups_coverage", "fixups", "provenance"] {
            let text = without(HEXDUMP, key, None);
            assert!(serde_json::from_str::<Hexdump>(&text).is_err(), "{key}");
        }
    }

    const STRINGS: &str = include_str!("../../tests/fixtures/strings-data.json");
    const CRYPTO: &str = include_str!("../../tests/fixtures/crypto-data.json");
    const BLOBS: &str = include_str!("../../tests/fixtures/blobs-data.json");
    const SUMMARY: &str = include_str!("../../tests/fixtures/summary-hello.json");

    #[test]
    fn parses_the_real_strings_crypto_and_blobs() {
        let strings: Strings = serde_json::from_str(STRINGS).expect("parses");
        let greeting = &strings.strings[0];
        assert_eq!(
            (greeting.address, greeting.text.as_str()),
            (0x200120, "hello from bonito")
        );
        assert_eq!(greeting.section.as_deref(), Some(".rodata"));
        assert_eq!(greeting.functions[0].name.as_str(), "say");
        let crypto: Crypto = serde_json::from_str(CRYPTO).expect("parses");
        let delta = &crypto.constants[0];
        assert_eq!(
            (delta.algorithm.as_str(), delta.functions[0].name.as_str()),
            ("TEA/XTEA", "tea_step")
        );
        let blobs: Blobs = serde_json::from_str(BLOBS).expect("parses");
        let gzip = &blobs.blobs[0];
        assert_eq!(
            (
                gzip.kind.as_str(),
                gzip.length,
                gzip.functions[0].name.as_str()
            ),
            ("gzip", 44, "packed")
        );
    }

    #[test]
    fn parses_the_real_summary() {
        let document: Summary = serde_json::from_str(SUMMARY).expect("parses");
        assert_eq!((document.total, document.warnings.len()), (4, 0));
        let overview = &document.summary;
        let entry = overview.entry.as_ref().expect("an entry point");
        assert_eq!((entry.name.as_str(), entry.address), ("_start", 0x201160));
        assert_eq!(
            overview.main.as_ref().map(|main| main.address),
            Some(0x201140)
        );
        assert_eq!(
            (
                overview.reachable_from_entry,
                overview.no_callers,
                overview.code_bytes
            ),
            (Some(3), 2, 98)
        );
        let buckets: Vec<(&str, u64)> = overview
            .size_buckets
            .iter()
            .map(|b| (b.bucket.as_str(), b.count))
            .collect();
        assert!(buckets.contains(&("16-63", 4)), "{buckets:?}");
        assert_eq!(overview.largest[0].name.as_str(), "_start");
        assert_eq!(overview.largest[0].size, 34);
    }

    #[test]
    fn a_summary_of_an_image_without_an_entry_point_parses_but_must_still_say_so() {
        let text = SUMMARY.replace(
            r#""reachable_from_entry": 3"#,
            r#""reachable_from_entry": null"#,
        );
        let document: Summary = serde_json::from_str(&text).expect("parses");
        assert_eq!(document.summary.reachable_from_entry, None);
        let mut value: serde_json::Value = serde_json::from_str(SUMMARY).expect("json");
        value["summary"]
            .as_object_mut()
            .expect("an object")
            .remove("reachable_from_entry");
        assert!(serde_json::from_str::<Summary>(&value.to_string()).is_err());
        let strings = without(STRINGS, "scanned", None);
        assert!(serde_json::from_str::<Strings>(&strings).is_err());
        let crypto = without(CRYPTO, "scanned", None);
        assert!(serde_json::from_str::<Crypto>(&crypto).is_err());
        let blobs = without(BLOBS, "scanned", None);
        assert!(serde_json::from_str::<Blobs>(&blobs).is_err());
    }

    #[test]
    fn a_row_without_its_functions_or_a_summary_without_its_overview_is_rejected() {
        let strings = without(STRINGS, "strings", None);
        assert!(serde_json::from_str::<Strings>(&strings).is_err());
        let mut value: serde_json::Value = serde_json::from_str(STRINGS).expect("json");
        value["strings"][0]
            .as_object_mut()
            .expect("a row")
            .remove("functions");
        assert!(serde_json::from_str::<Strings>(&value.to_string()).is_err());
        let summary = without(SUMMARY, "summary", None);
        assert!(serde_json::from_str::<Summary>(&summary).is_err());
        let warnings = without(SUMMARY, "warnings", None);
        assert!(serde_json::from_str::<Summary>(&warnings).is_err());
    }

    #[test]
    fn text_in_strings_is_cleaned_while_parsing() {
        let text = r#"{"scanned":"sections","strings":[{"address":1,"text":"a\u001b[2Jb","section":null,"functions":[]}]}"#;
        let strings: Strings = serde_json::from_str(text).expect("parses");
        assert_eq!(strings.strings[0].text.as_str(), "a [2Jb");
        assert!(strings.strings[0].section.is_none());
    }

    #[test]
    fn a_lift_error_is_an_object_with_a_message() {
        let text = PCODE.replace(
            r#""error": null"#,
            r#""error": {"address": 1, "message": "bad\u001b[2J"}"#,
        );
        let listing: Pcode = serde_json::from_str(&text).expect("parses");
        let error = listing.error.expect("an error");
        assert_eq!(error.message.as_str(), "bad [2J");
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
