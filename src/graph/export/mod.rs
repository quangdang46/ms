//! Export formats for graph analysis results.
//!
//! Supports DOT, Mermaid, and JSON output formats.

use crate::graph::engine::graph::DiGraph;
use crate::graph::types::Issue;

/// Export graph as DOT format for Graphviz.
pub fn to_dot(graph: &DiGraph, _issues: &[Issue]) -> String {
    let mut lines = vec!["digraph skills {".to_string()];
    let n = graph.node_count();

    for i in 0..n {
        if let Some(id) = graph.node_id(i) {
            lines.push(format!("  \"{}\";", id));
        }
    }

    for i in 0..n {
        for &j in graph.successors_slice(i) {
            if let (Some(from), Some(to)) = (graph.node_id(i), graph.node_id(j)) {
                lines.push(format!("  \"{}\" -> \"{}\";", from, to));
            }
        }
    }

    lines.push("}".to_string());
    lines.join("\n")
}

/// Export graph as Mermaid diagram.
pub fn to_mermaid(graph: &DiGraph, _issues: &[Issue]) -> String {
    let mut lines = vec!["graph TD;".to_string()];
    let n = graph.node_count();

    // Emit all nodes first so the diagram still renders even when there
    // are no edges. Mermaid allows bare node ids on their own line and
    // will render them as standalone boxes.
    for i in 0..n {
        if let Some(id) = graph.node_id(i) {
            lines.push(format!("  {};", sanitize_mermaid_id(&id)));
        }
    }

    for i in 0..n {
        for &j in graph.successors_slice(i) {
            if let (Some(from), Some(to)) = (graph.node_id(i), graph.node_id(j)) {
                lines.push(format!(
                    "  {} --> {};",
                    sanitize_mermaid_id(&from),
                    sanitize_mermaid_id(&to)
                ));
            }
        }
    }

    lines.join("\n")
}

/// Mermaid node ids must be alphanumeric/underscore. Replace anything else
/// with `_` so provider-qualified ids (e.g. `local/rust-error-handling`)
/// produce valid mermaid output.
fn sanitize_mermaid_id(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Export graph as JSON.
pub fn to_json(graph: &DiGraph, _issues: &[Issue]) -> String {
    let mut nodes: Vec<serde_json::Value> = Vec::new();
    let mut edges: Vec<serde_json::Value> = Vec::new();
    let n = graph.node_count();

    for i in 0..n {
        if let Some(id) = graph.node_id(i) {
            nodes.push(serde_json::json!({"id": id}));
        }
    }

    for i in 0..n {
        for &j in graph.successors_slice(i) {
            if let (Some(from), Some(to)) = (graph.node_id(i), graph.node_id(j)) {
                edges.push(serde_json::json!({
                    "from": from,
                    "to": to
                }));
            }
        }
    }

    serde_json::to_string_pretty(&serde_json::json!({
        "nodes": nodes,
        "edges": edges,
        "node_count": n,
        "edge_count": graph.edge_count(),
        "density": graph.density(),
    }))
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_dot() {
        let mut graph = DiGraph::new();
        let a = graph.add_node("a");
        let b = graph.add_node("b");
        graph.add_edge(a, b);

        let dot = to_dot(&graph, &[]);
        assert!(dot.contains("digraph skills"));
        assert!(dot.contains("\"a\" -> \"b\";"));
    }

    #[test]
    fn test_to_mermaid() {
        let mut graph = DiGraph::new();
        let a = graph.add_node("a");
        let b = graph.add_node("b");
        graph.add_edge(a, b);

        let mermaid = to_mermaid(&graph, &[]);
        assert!(mermaid.contains("graph TD;"));
        assert!(mermaid.contains("a --> b;"));
    }

    #[test]
    fn test_to_json() {
        let mut graph = DiGraph::new();
        let a = graph.add_node("a");
        let b = graph.add_node("b");
        graph.add_edge(a, b);

        let json = to_json(&graph, &[]);
        assert!(json.contains("\"nodes\""));
        assert!(json.contains("\"edges\""));
        assert!(json.contains("\"node_count\": 2"));
    }
}
