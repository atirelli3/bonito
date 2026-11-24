//! Typed, lenient views of the JSON kuna prints: unknown fields are ignored and optional ones default.

use serde::Deserialize;

/// One function kuna found in the binary.
#[derive(Debug, Deserialize)]
pub struct Function {
    /// A symbol name, or kuna's generated `sub_<addr>`.
    pub name: String,
    /// The entry address.
    pub address: u64,
    /// The extent in bytes.
    pub size: u64,
    /// Other names for the same address.
    #[serde(default)]
    pub aliases: Vec<String>,
}

/// What `kuna functions --json` prints.
#[derive(Debug, Deserialize)]
pub struct Functions {
    /// Why discovery failed, if it did.
    pub error: Option<String>,
    pub functions: Vec<Function>,
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
}
