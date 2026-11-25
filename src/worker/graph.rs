//! The call graph view: the callers and callees of a function as two indented trees.

use super::{MAX_LINES, bounded, join_notes};
use crate::kuna::{self, Graph, Node, Reply};
use std::collections::{HashMap, HashSet};

/// How many levels a row is indented by; deeper rows line up with this one.
// ponytail: the depth is lost past this.
const MAX_INDENT: usize = 40;
/// How many bytes of rows a tree may hold.
const MAX_BYTES: usize = 16 * 1024 * 1024;

/// A reference to the node at `node`, of edge kind `kind`, sorted by `key`.
#[derive(Clone)]
struct Link<'a> {
    key: u64,
    node: usize,
    kind: &'a str,
}

/// The edges of a graph as one list of links per node, by direction.
struct Links<'a> {
    callers: Vec<Vec<Link<'a>>>,
    callees: Vec<Vec<Link<'a>>>,
    /// How many edges of the function asked for name a function the graph does not list.
    lost: usize,
}

/// The rows of a tree so far, and what was left out of them.
#[derive(Default)]
struct Rows {
    lines: Vec<String>,
    bytes: usize,
    /// Some row sits deeper than `MAX_INDENT`.
    flat: bool,
}

impl Rows {
    fn push(&mut self, line: String) {
        self.bytes += line.len();
        self.lines.push(line);
    }

    /// Whether no more rows are wanted: one past `MAX_LINES` lets `bounded` report the cut.
    fn is_full(&self) -> bool {
        self.lines.len() > MAX_LINES || self.bytes > MAX_BYTES
    }
}

/// The tree lines for the function at `address`. The graph is loaded once and kept in `cache`; a failed load is
/// not kept, and what kuna said while loading is told once, with the answer that loaded it.
pub fn graph_lines(
    cache: &mut Option<Graph>,
    load: impl FnOnce() -> Result<Reply<Graph>, kuna::Error>,
    address: u64,
) -> Result<Reply<Vec<String>>, kuna::Error> {
    let (graph, loaded) = match cache.take() {
        Some(graph) => (graph, None),
        None => {
            let Reply { data, notes } = load()?;
            (data, notes)
        }
    };
    let answer = tree(&graph, loaded, address);
    *cache = Some(graph);
    answer
}

/// The function at `address` with its callers and then its callees, each as a tree that expands a function once.
fn tree(
    graph: &Graph,
    loaded: Option<String>,
    address: u64,
) -> Result<Reply<Vec<String>>, kuna::Error> {
    let nodes = &graph.nodes;
    let at: HashMap<u64, usize> = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.address, index))
        .collect();
    let root = *at.get(&address).ok_or_else(|| {
        kuna::Error::Reported(format!("no function at {address:#x} in kuna's call graph"))
    })?;
    let links = link_edges(graph, &at, address);
    let mut rows = Rows::default();
    rows.push(describe(&nodes[root], None, false));
    let mut none = false;
    for (title, mut lists) in [("callers", links.callers), ("callees", links.callees)] {
        lists
            .iter_mut()
            .for_each(|list| list.sort_by_key(|link| link.key));
        none |= lists[root].is_empty();
        if lists[root].is_empty() {
            rows.push(format!("{title}: none direct"));
        } else {
            rows.push(title.to_string());
            walk(nodes, &lists, root, &mut rows);
        }
    }
    let said = loaded.into_iter().chain(caveats(&rows, none, links.lost));
    let (lines, cut) = bounded(rows.lines.into_iter());
    Ok(Reply {
        data: lines,
        notes: join_notes(said, cut),
    })
}

/// What the user should know about a tree besides its rows.
fn caveats(rows: &Rows, none: bool, lost: usize) -> Vec<String> {
    [
        none.then(|| "calls through pointers are not edges: none direct can miss some".to_string()),
        (lost > 0)
            .then(|| format!("{lost} edge(s) of this function name a function kuna did not list")),
        (rows.bytes > MAX_BYTES).then(|| format!("the tree is cut at {} MiB", MAX_BYTES >> 20)),
        rows.flat
            .then(|| format!("rows deeper than {MAX_INDENT} levels are not indented further")),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// The edges of `graph` as links between the nodes at `at`; those of the function at `address` that name an unknown
/// function are counted, not shown.
fn link_edges<'a>(graph: &'a Graph, at: &HashMap<u64, usize>, address: u64) -> Links<'a> {
    let empty = vec![Vec::new(); graph.nodes.len()];
    let mut links = Links {
        callers: empty.clone(),
        callees: empty,
        lost: 0,
    };
    for edge in &graph.edges {
        let ends = (at.get(&edge.caller_address), at.get(&edge.callee_address));
        let (Some(&from), Some(&to)) = ends else {
            let ours = edge.caller_address == address || edge.callee_address == address;
            links.lost += usize::from(ours);
            continue;
        };
        let kind = edge.kind.as_str();
        links.callees[from].push(Link {
            key: edge.callee_order,
            node: to,
            kind,
        });
        links.callers[to].push(Link {
            key: edge.caller_address,
            node: from,
            kind,
        });
    }
    links
}

/// Appends the rows below `root` to `rows`, depth first; a node already shown is marked, not expanded.
fn walk(nodes: &[Node], links: &[Vec<Link>], root: usize, rows: &mut Rows) {
    let mut seen = HashSet::from([root]);
    let below = |node: usize, depth: usize| {
        links[node]
            .iter()
            .rev()
            .map(move |link| (link.node, link.kind, depth))
    };
    let mut stack: Vec<(usize, &str, usize)> = below(root, 1).collect();
    while let Some((node, kind, depth)) = stack.pop() {
        if rows.is_full() {
            break;
        }
        let above = !seen.insert(node);
        rows.flat |= depth > MAX_INDENT;
        let indent = "  ".repeat(depth.min(MAX_INDENT));
        rows.push(format!(
            "{indent}{}",
            describe(&nodes[node], Some(kind), above)
        ));
        if !above {
            stack.extend(below(node, depth + 1));
        }
    }
}

/// `main  0x201140`, then a jump or data edge, `entry`, a thunk or import, `indirect calls`, `(above)`.
fn describe(node: &Node, edge: Option<&str>, above: bool) -> String {
    let mut parts = vec![format!("{}  {:#x}", node.name.as_str(), node.address)];
    parts.extend(edge.filter(|kind| *kind != "call").map(str::to_string));
    parts.extend(node.is_entry_point.then(|| "entry".to_string()));
    parts.extend((node.kind.as_str() != "normal").then(|| node.kind.as_str().to_string()));
    parts.extend(
        node.has_indirect_calls
            .then(|| "indirect calls".to_string()),
    );
    parts.extend(above.then(|| "(above)".to_string()));
    parts.join("  ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const HELLO: &str = include_str!("../../tests/fixtures/graph-hello.json");

    fn hello() -> Graph {
        serde_json::from_str(HELLO).expect("parses")
    }

    /// A graph of `(address, name, kind)` nodes and `(caller, callee, kind)` edges, in the order given.
    fn graph(nodes: &[(u64, &str, &str)], edges: &[(u64, u64, &str)]) -> Graph {
        let nodes: Vec<_> = nodes
            .iter()
            .map(|(address, name, kind)| {
                json!({"address": address, "name": name, "kind": kind,
                       "isEntryPoint": false, "hasIndirectCalls": false})
            })
            .collect();
        let edges: Vec<_> = edges
            .iter()
            .enumerate()
            .map(|(order, (from, to, kind))| {
                json!({"callerAddress": from, "calleeAddress": to, "kind": kind, "calleeOrder": order})
            })
            .collect();
        serde_json::from_value(json!({"functions": nodes, "edges": edges})).expect("a graph")
    }

    fn answer(graph: &Graph, address: u64) -> Reply<Vec<String>> {
        tree(graph, None, address).expect("a tree")
    }

    fn lines(graph: &Graph, address: u64) -> Vec<String> {
        answer(graph, address).data
    }

    fn notes(graph: &Graph, address: u64) -> String {
        answer(graph, address).notes.unwrap_or_default()
    }

    #[test]
    fn main_of_hello_has_no_callers_and_two_callees_in_call_order() {
        assert_eq!(
            lines(&hello(), 0x201140),
            [
                "main  0x201140",
                "callers: none direct",
                "callees",
                "  square  0x201120",
                "  add  0x201130  jump",
            ]
        );
    }

    #[test]
    fn square_of_hello_lists_its_callers_by_address() {
        assert_eq!(
            lines(&hello(), 0x201120),
            [
                "square  0x201120",
                "callers",
                "  main  0x201140",
                "  _start  0x201160  entry",
                "callees: none direct",
            ]
        );
    }

    #[test]
    fn an_empty_list_says_calls_through_pointers_are_not_edges() {
        assert!(notes(&hello(), 0x201140).contains("through pointers"));
    }

    #[test]
    fn a_function_reached_twice_is_expanded_once() {
        let reply = graph(
            &[
                (1, "a", "normal"),
                (2, "b", "normal"),
                (3, "c", "normal"),
                (4, "d", "normal"),
            ],
            &[
                (1, 2, "call"),
                (1, 3, "call"),
                (2, 4, "call"),
                (3, 4, "call"),
            ],
        );
        assert_eq!(
            lines(&reply, 1),
            [
                "a  0x1",
                "callers: none direct",
                "callees",
                "  b  0x2",
                "    d  0x4",
                "  c  0x3",
                "    d  0x4  (above)",
            ]
        );
    }

    #[test]
    fn a_cycle_and_a_self_call_end_at_the_function_already_shown() {
        let reply = graph(
            &[(1, "a", "normal"), (2, "b", "normal")],
            &[(1, 2, "call"), (2, 1, "call"), (1, 1, "call")],
        );
        let tree = answer(&reply, 1);
        assert_eq!(
            tree.data,
            [
                "a  0x1",
                "callers",
                "  a  0x1  (above)",
                "  b  0x2",
                "    a  0x1  (above)",
                "callees",
                "  b  0x2",
                "    a  0x1  (above)",
                "  a  0x1  (above)",
            ]
        );
        assert_eq!(tree.notes, None);
    }

    #[test]
    fn a_node_says_what_kuna_knows_about_it() {
        let mut reply = graph(
            &[
                (1, "f", "normal"),
                (2, "puts", "import"),
                (3, "stub", "thunk"),
            ],
            &[(1, 2, "call"), (1, 3, "data")],
        );
        reply.nodes[0].has_indirect_calls = true;
        assert_eq!(
            lines(&reply, 1),
            [
                "f  0x1  indirect calls",
                "callers: none direct",
                "callees",
                "  puts  0x2  import",
                "  stub  0x3  data  thunk",
            ]
        );
    }

    #[test]
    fn a_function_that_is_not_in_the_graph_is_an_error() {
        let error = tree(&hello(), None, 0x1).expect_err("not a node");
        assert!(error.to_string().contains("0x1"), "{error}");
    }

    #[test]
    fn only_an_edge_of_the_function_asked_for_is_reported_as_unlisted() {
        let reply = graph(
            &[(1, "a", "normal"), (2, "b", "normal")],
            &[(1, 2, "call"), (1, 99, "call")],
        );
        assert!(notes(&reply, 1).contains("1 edge(s)"));
        assert!(!notes(&reply, 2).contains("did not list"));
    }

    #[test]
    fn a_chain_deeper_than_the_indent_says_so() {
        let names: Vec<String> = (0..60).map(|at| format!("f{at}")).collect();
        let nodes: Vec<(u64, &str, &str)> = names
            .iter()
            .enumerate()
            .map(|(at, name)| (at as u64, name.as_str(), "normal"))
            .collect();
        let edges: Vec<(u64, u64, &str)> = (0..59).map(|at| (at, at + 1, "call")).collect();
        assert!(notes(&graph(&nodes, &edges), 0).contains("not indented further"));
    }

    #[test]
    fn a_very_deep_chain_is_cut_and_stays_small() {
        let count = 200_000_u64;
        let nodes: Vec<(u64, String, &str)> = (0..count)
            .map(|at| (at, format!("f{at}"), "normal"))
            .collect();
        let nodes: Vec<(u64, &str, &str)> = nodes
            .iter()
            .map(|(at, name, kind)| (*at, name.as_str(), *kind))
            .collect();
        let edges: Vec<(u64, u64, &str)> = (0..count - 1).map(|at| (at, at + 1, "call")).collect();
        let reply = answer(&graph(&nodes, &edges), 0);
        assert_eq!(reply.data.len(), MAX_LINES);
        assert!(reply.notes.is_some_and(|note| note.contains("cut")));
        let size: usize = reply.data.iter().map(String::len).sum();
        assert!(size < MAX_LINES * 200, "{size} bytes");
    }

    #[test]
    fn the_walk_stops_one_row_past_the_limit() {
        let count = MAX_LINES as u64 + 50_000;
        let names: Vec<String> = (0..=count).map(|at| format!("f{at}")).collect();
        let nodes: Vec<(u64, &str, &str)> = names
            .iter()
            .enumerate()
            .map(|(at, name)| (at as u64, name.as_str(), "normal"))
            .collect();
        let edges: Vec<(u64, u64, &str)> = (1..=count).map(|leaf| (0, leaf, "call")).collect();
        let hub = graph(&nodes, &edges);
        let at: HashMap<u64, usize> = (0..=count).map(|at| (at, at as usize)).collect();
        let links = link_edges(&hub, &at, 0);
        let mut rows = Rows::default();
        walk(&hub.nodes, &links.callees, 0, &mut rows);
        assert_eq!(rows.lines.len(), MAX_LINES + 1);
    }

    #[test]
    fn a_long_name_repeated_on_every_row_is_cut_by_size() {
        let name = "😀".repeat(1000);
        let edges: Vec<(u64, u64, &str)> = (0..5000).map(|_| (1, 2, "call")).collect();
        let hostile = graph(&[(1, "r", "normal"), (2, &name, "normal")], &edges);
        let reply = answer(&hostile, 1);
        assert!(reply.notes.is_some_and(|note| note.contains("16 MiB")));
        let size: usize = reply.data.iter().map(String::len).sum();
        assert!(size < MAX_BYTES + 10_000, "{size} bytes");
    }

    #[test]
    fn the_graph_is_loaded_once_and_what_kuna_said_is_told_once() {
        let mut cache = None;
        let mut loads = 0;
        let mut ask = |outcome: Result<Reply<Graph>, kuna::Error>| {
            let load = || {
                loads += 1;
                outcome
            };
            graph_lines(&mut cache, load, 0x201140)
        };
        let failed = kuna::Error::Reported("no graph".to_string());
        assert!(ask(Err(failed)).is_err(), "a failed load is not kept");
        let said = Reply {
            data: hello(),
            notes: Some("kuna said so".to_string()),
        };
        let first = ask(Ok(said)).expect("loads");
        assert!(
            first
                .notes
                .is_some_and(|note| note.contains("kuna said so"))
        );
        let second = ask(Err(kuna::Error::Cancelled)).expect("kept, not reloaded");
        assert!(
            second
                .notes
                .is_none_or(|note| !note.contains("kuna said so"))
        );
        assert_eq!(loads, 2);
    }
}
