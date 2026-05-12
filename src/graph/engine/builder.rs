//! Build a DiGraph from a collection of issues.

use super::graph::DiGraph;
use crate::graph::types::Issue;

/// Build a directed graph from issue dependencies.
///
/// Each issue becomes a node. Dependencies become edges from blocker to dependent:
/// blocker -> dependent (meaning "this blocker prevents the dependent from starting").
///
/// Edge direction: from blocker to the issue it blocks.
/// So if A depends on B, the edge is B -> A (B blocks A).
///
/// This convention matches the reachability module where `predecessors_slice(node)`
/// returns node's blockers (things that must be completed first).
pub fn build_graph(issues: &[Issue]) -> DiGraph {
    let mut graph = DiGraph::with_capacity(issues.len(), issues.len() * 2);

    // Add all nodes first
    for issue in issues {
        graph.add_node(&issue.id);
    }

    // Add edges from blocker -> dependent
    for issue in issues {
        if let Some(dependent_idx) = graph.node_idx(&issue.id) {
            for dep in &issue.dependencies {
                if let Some(blocker_idx) = graph.node_idx(&dep.id) {
                    // blocker -> dependent (blocker blocks dependent)
                    graph.add_edge(blocker_idx, dependent_idx);
                }
            }
        }
    }

    graph
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{Dependency, Issue, IssueStatus, IssueType};
    use std::collections::HashMap;

    fn make_issue(id: &str, deps: Vec<&str>) -> Issue {
        let dependencies = deps
            .into_iter()
            .map(|d| Dependency {
                id: d.to_string(),
                title: d.to_string(),
                status: None,
                dependency_type: None,
            })
            .collect();

        Issue {
            id: id.to_string(),
            title: id.to_string(),
            description: String::new(),
            status: IssueStatus::Open,
            priority: 2,
            issue_type: IssueType::Task,
            owner: None,
            assignee: None,
            labels: vec![],
            notes: None,
            created_at: None,
            created_by: None,
            updated_at: None,
            closed_at: None,
            dependencies,
            dependents: vec![],
            extra: HashMap::new(),
        }
    }

    #[test]
    fn test_build_graph_simple_chain() {
        let issues = vec![
            make_issue("a", vec!["b"]),
            make_issue("b", vec!["c"]),
            make_issue("c", vec![]),
        ];

        let graph = build_graph(&issues);
        assert_eq!(graph.node_count(), 3);
        assert_eq!(graph.edge_count(), 2);

        let a = graph.node_idx("a").unwrap();
        let b = graph.node_idx("b").unwrap();
        let c = graph.node_idx("c").unwrap();

        // Edge: b -> a (b blocks a, because a depends on b)
        assert_eq!(graph.out_degree(b), 1);
        assert!(graph.successors_slice(b).contains(&a));

        // Edge: c -> b (c blocks b, because b depends on c)
        assert_eq!(graph.out_degree(c), 1);
        assert!(graph.successors_slice(c).contains(&b));

        // a has nothing blocking it from downstream
        assert_eq!(graph.out_degree(a), 0);
    }

    #[test]
    fn test_build_graph_diamond() {
        let issues = vec![
            make_issue("top", vec!["left", "right"]),
            make_issue("left", vec!["bottom"]),
            make_issue("right", vec!["bottom"]),
            make_issue("bottom", vec![]),
        ];

        let graph = build_graph(&issues);
        assert_eq!(graph.node_count(), 4);
        assert_eq!(graph.edge_count(), 4);

        let left = graph.node_idx("left").unwrap();
        let right = graph.node_idx("right").unwrap();
        // left -> top and right -> top (both block top)
        assert!(
            graph
                .successors_slice(left)
                .contains(&graph.node_idx("top").unwrap())
        );
        assert!(
            graph
                .successors_slice(right)
                .contains(&graph.node_idx("top").unwrap())
        );
    }

    #[test]
    fn test_build_graph_missing_dependency() {
        // Issue references a dependency that doesn't exist in the issue list
        let issues = vec![make_issue("a", vec!["missing"])];

        let graph = build_graph(&issues);
        assert_eq!(graph.node_count(), 1);
        assert_eq!(graph.edge_count(), 0);
    }
}
