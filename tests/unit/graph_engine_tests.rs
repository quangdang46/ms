//! Unit tests for the graph engine (DiGraph construction and algorithm integration).

use std::collections::HashMap;

use ms::graph::engine::builder::build_graph;
use ms::graph::types::{Dependency, Issue, IssueStatus, IssueType};

fn sample_issue(id: &str) -> Issue {
    Issue {
        id: id.to_string(),
        title: format!("Issue {}", id),
        description: String::new(),
        status: IssueStatus::Open,
        priority: 2,
        issue_type: IssueType::Task,
        owner: None,
        assignee: None,
        labels: Vec::new(),
        notes: None,
        created_at: None,
        created_by: None,
        updated_at: None,
        closed_at: None,
        dependencies: Vec::new(),
        dependents: Vec::new(),
        extra: HashMap::new(),
    }
}

fn issue_with_deps(id: &str, deps: Vec<&str>) -> Issue {
    let mut issue = sample_issue(id);
    issue.dependencies = deps
        .into_iter()
        .map(|d| Dependency {
            id: d.to_string(),
            title: format!("Dep {}", d),
            status: Some(IssueStatus::Open),
            dependency_type: None,
        })
        .collect();
    issue
}

// ============================================================================
// DiGraph Construction Tests
// ============================================================================

#[test]
fn graph_engine_empty_issues() {
    let issues: Vec<Issue> = vec![];
    let graph = build_graph(&issues);
    assert_eq!(graph.node_count(), 0);
}

#[test]
fn graph_engine_single_node() {
    let issues = vec![sample_issue("a")];
    let graph = build_graph(&issues);
    assert_eq!(graph.node_count(), 1);
    assert_eq!(graph.edge_count(), 0);
}

#[test]
fn graph_engine_two_node_chain() {
    // a -> b (a depends on b, so edge b -> a)
    let issues = vec![issue_with_deps("a", vec!["b"]), sample_issue("b")];
    let graph = build_graph(&issues);
    assert_eq!(graph.node_count(), 2);
    assert_eq!(graph.edge_count(), 1);
    // Edge direction: blocker -> dependent
    let a_idx = graph.node_idx("a").unwrap();
    let b_idx = graph.node_idx("b").unwrap();
    // b -> a (b blocks a, since a depends on b)
    // b has out_degree 1 (points to dependent a)
    assert_eq!(graph.out_degree(b_idx), 1, "b should have one dependent");
    // a has in_degree 1 (has one blocker: b)
    assert_eq!(graph.in_degree(a_idx), 1, "a should have one blocker");
    // b should have no blockers (in_degree 0)
    assert_eq!(graph.in_degree(b_idx), 0, "b should have no blockers");
}

#[test]
fn graph_engine_diamond() {
    // a depends on b and c
    // b depends on d
    // c depends on d
    // d is root
    let issues = vec![
        issue_with_deps("a", vec!["b", "c"]),
        issue_with_deps("b", vec!["d"]),
        issue_with_deps("c", vec!["d"]),
        sample_issue("d"),
    ];
    let graph = build_graph(&issues);
    assert_eq!(graph.node_count(), 4);
    assert!(
        graph.edge_count() >= 3,
        "Diamond should have at least 3 edges"
    );
}

#[test]
fn graph_engine_json_roundtrip() {
    let issues = vec![issue_with_deps("a", vec!["b"]), sample_issue("b")];
    let graph = build_graph(&issues);
    let json = graph.to_json();
    let restored = ms::graph::engine::graph::DiGraph::from_json(&json).unwrap();

    assert_eq!(graph.node_count(), restored.node_count());
    assert_eq!(graph.edge_count(), restored.edge_count());
    assert!(restored.node_idx("a").is_some());
    assert!(restored.node_idx("b").is_some());
}

#[test]
fn graph_engine_orphan_detection() {
    // a depends on b, c is standalone (orphan in terms of no blockers)
    let issues = vec![
        issue_with_deps("a", vec!["b"]),
        sample_issue("b"),
        sample_issue("c"), // orphan
    ];
    let graph = build_graph(&issues);

    let c_idx = graph.node_idx("c").unwrap();
    assert_eq!(graph.in_degree(c_idx), 0, "c should have no blockers");
}

#[test]
fn graph_engine_self_loop_ignored() {
    let issues = vec![issue_with_deps("a", vec!["a"])];
    let graph = build_graph(&issues);
    assert_eq!(graph.node_count(), 1);
}

#[test]
fn graph_engine_missing_dependency_not_created() {
    // a depends on b, but b is not in the issue set
    let issues = vec![issue_with_deps("a", vec!["b"])];
    let graph = build_graph(&issues);
    // Only existing issues are nodes
    assert_eq!(graph.node_count(), 1);
}

#[test]
fn graph_engine_large_dataset() {
    let issues: Vec<Issue> = (0..1000)
        .map(|i| {
            if i == 0 {
                sample_issue("skill-0")
            } else {
                issue_with_deps(&format!("skill-{}", i), vec![&format!("skill-{}", i - 1)])
            }
        })
        .collect();

    let graph = build_graph(&issues);
    assert_eq!(graph.node_count(), 1000);
    assert_eq!(graph.edge_count(), 999);
}
