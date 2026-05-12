//! Core directed graph structure with adjacency lists.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Directed graph optimized for graph algorithms.
/// Uses adjacency lists for O(1) neighbor access.
#[derive(Clone)]
pub struct DiGraph {
    /// Node ID strings (issue IDs like "bv-123")
    nodes: Vec<String>,

    /// Reverse lookup: ID string -> index
    node_index: HashMap<String, usize>,

    /// Forward adjacency: adj[u] = vec of nodes that u points to
    /// (u depends on these nodes)
    adj: Vec<Vec<usize>>,

    /// Reverse adjacency: rev_adj[v] = vec of nodes pointing to v
    /// (these nodes depend on v)
    rev_adj: Vec<Vec<usize>>,

    /// Edge count (for density calculation)
    edge_count: usize,
}

/// Serializable graph snapshot for import/export.
#[derive(Serialize, Deserialize)]
pub struct GraphSnapshot {
    pub nodes: Vec<String>,
    pub edges: Vec<(usize, usize)>,
}

impl DiGraph {
    /// Create an empty graph.
    pub fn new() -> DiGraph {
        DiGraph {
            nodes: Vec::new(),
            node_index: HashMap::new(),
            adj: Vec::new(),
            rev_adj: Vec::new(),
            edge_count: 0,
        }
    }

    /// Create a graph with pre-allocated capacity.
    pub fn with_capacity(node_capacity: usize, _edge_capacity: usize) -> DiGraph {
        DiGraph {
            nodes: Vec::with_capacity(node_capacity),
            node_index: HashMap::with_capacity(node_capacity),
            adj: Vec::with_capacity(node_capacity),
            rev_adj: Vec::with_capacity(node_capacity),
            edge_count: 0,
        }
    }

    /// Add a node, returns its index. Idempotent - returns existing index if already present.
    pub fn add_node(&mut self, id: &str) -> usize {
        if let Some(&idx) = self.node_index.get(id) {
            return idx;
        }
        let idx = self.nodes.len();
        self.nodes.push(id.to_string());
        self.node_index.insert(id.to_string(), idx);
        self.adj.push(Vec::new());
        self.rev_adj.push(Vec::new());
        idx
    }

    /// Add a directed edge from -> to. Idempotent.
    pub fn add_edge(&mut self, from: usize, to: usize) {
        if from >= self.nodes.len() || to >= self.nodes.len() {
            return;
        }

        if self.adj[from].contains(&to) {
            return;
        }

        self.adj[from].push(to);
        self.rev_adj[to].push(from);
        self.edge_count += 1;
    }

    /// Number of nodes.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Number of edges.
    pub fn edge_count(&self) -> usize {
        self.edge_count
    }

    /// Graph density: edges / (nodes * (nodes - 1)).
    pub fn density(&self) -> f64 {
        let n = self.node_count() as f64;
        let e = self.edge_count as f64;
        if n <= 1.0 { 0.0 } else { e / (n * (n - 1.0)) }
    }

    /// Get node ID by index.
    pub fn node_id(&self, idx: usize) -> Option<String> {
        self.nodes.get(idx).cloned()
    }

    /// Get node index by ID.
    pub fn node_idx(&self, id: &str) -> Option<usize> {
        self.node_index.get(id).copied()
    }

    /// Get all node IDs.
    pub fn node_ids(&self) -> Vec<String> {
        self.nodes.clone()
    }

    /// Out-degree of a node (number of dependencies).
    pub fn out_degree(&self, node: usize) -> usize {
        self.adj.get(node).map_or(0, |v| v.len())
    }

    /// In-degree of a node (number of dependents).
    pub fn in_degree(&self, node: usize) -> usize {
        self.rev_adj.get(node).map_or(0, |v| v.len())
    }

    /// All out-degrees as a vector.
    pub fn out_degrees(&self) -> Vec<usize> {
        self.adj.iter().map(|v| v.len()).collect()
    }

    /// All in-degrees as a vector.
    pub fn in_degrees(&self) -> Vec<usize> {
        self.rev_adj.iter().map(|v| v.len()).collect()
    }

    /// Export graph as JSON snapshot.
    pub fn to_json(&self) -> String {
        let snapshot = GraphSnapshot {
            nodes: self.nodes.clone(),
            edges: self.edges_vec(),
        };
        serde_json::to_string(&snapshot).unwrap_or_default()
    }

    /// Import graph from JSON snapshot.
    pub fn from_json(json: &str) -> Result<DiGraph, String> {
        let snapshot: GraphSnapshot = serde_json::from_str(json).map_err(|e| e.to_string())?;

        let mut graph = DiGraph::with_capacity(snapshot.nodes.len(), snapshot.edges.len());
        for id in snapshot.nodes {
            graph.add_node(&id);
        }
        for (from, to) in snapshot.edges {
            graph.add_edge(from, to);
        }
        Ok(graph)
    }

    /// Get successors slice (internal use).
    pub(crate) fn successors_slice(&self, node: usize) -> &[usize] {
        self.adj.get(node).map_or(&[], |v| v.as_slice())
    }

    /// Get predecessors slice (internal use).
    pub(crate) fn predecessors_slice(&self, node: usize) -> &[usize] {
        self.rev_adj.get(node).map_or(&[], |v| v.as_slice())
    }

    /// Iterate over all edges (internal use).
    pub(crate) fn edges(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.adj
            .iter()
            .enumerate()
            .flat_map(|(from, tos)| tos.iter().map(move |&to| (from, to)))
    }

    /// Collect edges as vec (for serialization).
    fn edges_vec(&self) -> Vec<(usize, usize)> {
        self.edges().collect()
    }

    /// Get node count (internal, non-WASM).
    pub(crate) fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Check if graph is empty.
    #[expect(dead_code)]
    pub(crate) fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

impl Default for DiGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_graph() {
        let g = DiGraph::new();
        assert_eq!(g.node_count(), 0);
        assert_eq!(g.edge_count(), 0);
    }

    #[test]
    fn test_add_node_idempotent() {
        let mut g = DiGraph::new();
        let idx1 = g.add_node("bv-1");
        let idx2 = g.add_node("bv-1");
        assert_eq!(idx1, idx2);
        assert_eq!(g.node_count(), 1);
    }

    #[test]
    fn test_add_edge_idempotent() {
        let mut g = DiGraph::new();
        let a = g.add_node("a");
        let b = g.add_node("b");
        g.add_edge(a, b);
        g.add_edge(a, b); // Should be idempotent
        assert_eq!(g.edge_count(), 1);
    }

    #[test]
    fn test_degrees() {
        let mut g = DiGraph::new();
        let a = g.add_node("a");
        let b = g.add_node("b");
        let c = g.add_node("c");
        g.add_edge(a, b);
        g.add_edge(a, c);
        g.add_edge(b, c);

        assert_eq!(g.out_degree(a), 2);
        assert_eq!(g.out_degree(b), 1);
        assert_eq!(g.out_degree(c), 0);

        assert_eq!(g.in_degree(a), 0);
        assert_eq!(g.in_degree(b), 1);
        assert_eq!(g.in_degree(c), 2);
    }

    #[test]
    fn test_density() {
        let mut g = DiGraph::new();
        assert_eq!(g.density(), 0.0);

        g.add_node("a");
        assert_eq!(g.density(), 0.0);

        let a = 0;
        let b = g.add_node("b");
        g.add_edge(a, b);
        assert!((g.density() - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_json_roundtrip() {
        let mut g = DiGraph::new();
        let a = g.add_node("a");
        let b = g.add_node("b");
        g.add_edge(a, b);

        let json = g.to_json();
        let g2 = DiGraph::from_json(&json).unwrap();

        assert_eq!(g2.node_count(), 2);
        assert_eq!(g2.edge_count(), 1);
        assert_eq!(g2.node_id(0), Some("a".to_string()));
        assert_eq!(g2.node_id(1), Some("b".to_string()));
    }

    #[test]
    fn test_node_idx() {
        let mut g = DiGraph::new();
        g.add_node("issue-1");
        g.add_node("issue-2");

        assert_eq!(g.node_idx("issue-1"), Some(0));
        assert_eq!(g.node_idx("issue-2"), Some(1));
        assert_eq!(g.node_idx("missing"), None);
    }
}
