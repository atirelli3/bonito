//! The call graph: what `kuna decompile-graph` prints (schema 4), as nodes and edges.
// ponytail: #80 reads the graph; remove this `allow` then.
#![allow(dead_code)]

use super::text::Text;
use super::{Error, Kuna, Reply, hex, parse, query_args};
use serde::Deserialize;
use std::path::Path;

/// The schema of `decompile-graph` bonito reads.
const SCHEMA: u32 = 4;
/// The seconds kuna may spend decompiling the one function it is asked to.
const DECOMPILE_SECONDS: &str = "1";

/// One function of the program, by entry address.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    pub address: u64,
    pub name: Text,
    /// `normal`, `thunk` or `import`.
    pub kind: Text,
    pub is_entry_point: bool,
    /// Calls through a pointer: they are not edges.
    pub has_indirect_calls: bool,
}

/// A reference from one function to another.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Edge {
    pub caller_address: u64,
    pub callee_address: u64,
    /// `call`, `jump` or `data`.
    pub kind: Text,
    /// The position among the caller's callees.
    pub callee_order: u64,
}

/// Every function of the program and the edges between them.
#[derive(Debug)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

/// The version a `decompile-graph` document says it is.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Header {
    schema_version: u32,
}

/// The nodes and edges of a `decompile-graph` document.
#[derive(Debug, Deserialize)]
struct Body {
    #[serde(rename = "functions")]
    nodes: Vec<Node>,
    edges: Vec<Edge>,
}

impl Kuna {
    /// The call graph of `binary`; kuna also decompiles the function at `address`, for at most a second.
    pub fn graph(&self, binary: &Path, address: u64) -> Result<Reply<Graph>, Error> {
        let target = hex(address);
        let extra = ["--addr", &target, "--max-fn-seconds", DECOMPILE_SECONDS];
        let args = query_args("decompile-graph", binary, &extra)?;
        let output = self.output(args)?;
        Ok(Reply {
            data: read(&output.stdout)?,
            notes: output.notes,
        })
    }
}

/// The graph in `text`: its schema is checked before the rest is read, so a newer one is named as such.
fn read(text: &str) -> Result<Graph, Error> {
    let header: Header = parse(text)?;
    if header.schema_version != SCHEMA {
        return Err(Error::Reported(format!(
            "kuna's call graph is schema {}, bonito reads schema {SCHEMA}",
            header.schema_version
        )));
    }
    let body: Body = parse(text)?;
    Ok(Graph {
        nodes: body.nodes,
        edges: body.edges,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELLO: &str = include_str!("../../tests/fixtures/graph-hello.json");

    fn graph(text: &str) -> Result<Graph, Error> {
        read(text)
    }

    #[test]
    fn reads_the_recorded_graph_of_hello() {
        let graph = graph(HELLO).expect("schema 4");
        let names: Vec<&str> = graph.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["square", "add", "main", "_start"]);
        assert_eq!(graph.edges.len(), 4);
        let main = &graph.nodes[2];
        assert_eq!((main.address, main.kind.as_str()), (0x201140, "normal"));
        assert!(graph.nodes[3].is_entry_point);
        let jump = &graph.edges[1];
        assert_eq!(
            (jump.caller_address, jump.callee_address, jump.kind.as_str()),
            (0x201140, 0x201130, "jump")
        );
        assert_eq!(jump.callee_order, 1);
    }

    #[test]
    fn another_schema_version_is_an_error_naming_both() {
        let text = HELLO.replace("\"schemaVersion\": 4", "\"schemaVersion\": 5");
        let message = graph(&text).expect_err("schema 5").to_string();
        assert!(message.contains('5') && message.contains('4'), "{message}");
    }

    #[test]
    fn a_newer_schema_that_renames_keys_still_says_it_is_the_schema() {
        let text = HELLO
            .replace("\"schemaVersion\": 4", "\"schemaVersion\": 5")
            .replace("\"edges\"", "\"links\"");
        let message = graph(&text).expect_err("schema 5").to_string();
        assert!(message.contains("schema 5"), "{message}");
    }

    #[test]
    fn a_renamed_key_bonito_relies_on_fails_loudly() {
        for key in ["edges", "calleeAddress", "isEntryPoint", "schemaVersion"] {
            let text = HELLO.replace(&format!("\"{key}\""), "\"renamed\"");
            assert!(graph(&text).is_err(), "{key} is required");
        }
    }

    #[test]
    fn a_hostile_name_is_cleaned() {
        let text = HELLO.replace("\"main\"", "\"ma\\u001b[2Jin\\u202e\"");
        let graph = graph(&text).expect("parses");
        assert!(!graph.nodes[2].name.contains(['\u{1b}', '\u{202e}']));
    }
}
