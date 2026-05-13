//! Analysis engine for graph-based insights.
//!
//! Orchestrates graph construction, algorithm execution, and result generation.
//! Ports of Go business logic from beads_viewer/pkg/analysis/.

pub mod health;
pub mod insights;
pub mod plan;
pub mod priority;
pub mod triage;

use crate::graph::algorithms::betweenness;
use crate::graph::algorithms::critical_path;
use crate::graph::algorithms::cycles;
use crate::graph::algorithms::eigenvector::{self, EigenvectorConfig};
use crate::graph::algorithms::hits::{self, HITSConfig};
use crate::graph::algorithms::kcore;
use crate::graph::algorithms::pagerank::{self, PageRankConfig};
use crate::graph::algorithms::slack;
use crate::graph::engine::builder::build_graph;
use crate::graph::engine::graph::DiGraph;
use crate::graph::engine::reachability;
use crate::graph::types::Issue;

/// Pre-computed graph metrics for all nodes.
#[derive(Debug, Clone)]
pub struct GraphMetrics {
    pub pagerank: Vec<f64>,
    pub betweenness: Vec<f64>,
    pub critical_path_heights: Vec<f64>,
    pub eigenvector: Vec<f64>,
    pub hubs: Vec<f64>,
    pub authorities: Vec<f64>,
    pub core_numbers: Vec<u32>,
    pub slack: Vec<f64>,
}

/// The main analysis engine.
///
/// Owns the graph, issues, and pre-computed metrics.
/// Provides high-level methods for insights, triage, plan, etc.
pub struct AnalysisEngine {
    graph: DiGraph,
    issues: Vec<Issue>,
    metrics: GraphMetrics,
}

impl AnalysisEngine {
    /// Create a new analysis engine from a slice of issues.
    ///
    /// Builds the graph and runs all graph algorithms to pre-compute metrics.
    pub fn new(issues: &[Issue]) -> Self {
        let graph = build_graph(issues);
        let n = graph.node_count();

        let metrics = if n == 0 {
            GraphMetrics {
                pagerank: Vec::new(),
                betweenness: Vec::new(),
                critical_path_heights: Vec::new(),
                eigenvector: Vec::new(),
                hubs: Vec::new(),
                authorities: Vec::new(),
                core_numbers: Vec::new(),
                slack: Vec::new(),
            }
        } else {
            let pr_config = PageRankConfig::default();
            let pr = pagerank::pagerank(&graph, &pr_config);

            let bw = if n > 500 {
                betweenness::betweenness_approx(&graph, 100, Some(42))
            } else {
                betweenness::betweenness(&graph)
            };

            let cp = critical_path::critical_path_heights(&graph);

            let ev_config = EigenvectorConfig::default();
            let ev = eigenvector::eigenvector(&graph, &ev_config);

            let hits_config = HITSConfig::default();
            let hits_result = hits::hits(&graph, &hits_config);

            let kc = kcore::kcore(&graph);
            let sl = slack::slack(&graph);

            GraphMetrics {
                pagerank: pr,
                betweenness: bw,
                critical_path_heights: cp,
                eigenvector: ev,
                hubs: hits_result.hubs,
                authorities: hits_result.authorities,
                core_numbers: kc,
                slack: sl,
            }
        };

        Self {
            graph,
            issues: issues.to_vec(),
            metrics,
        }
    }

    /// Reference to the underlying graph.
    pub fn graph(&self) -> &DiGraph {
        &self.graph
    }

    /// Reference to the pre-computed metrics.
    pub fn metrics(&self) -> &GraphMetrics {
        &self.metrics
    }

    /// Reference to the issues.
    pub fn issues(&self) -> &[Issue] {
        &self.issues
    }

    /// Get the closed set: boolean vector where true means the issue is closed/tombstone.
    pub fn closed_set(&self) -> Vec<bool> {
        let n = self.graph.node_count();
        let mut closed = vec![false; n];
        for issue in &self.issues {
            if let Some(idx) = self.graph.node_idx(&issue.id) {
                if idx < n {
                    closed[idx] = issue.status.is_terminal();
                }
            }
        }
        closed
    }

    /// Get actionable issue IDs (open issues with all blockers closed).
    pub fn actionable_ids(&self) -> Vec<String> {
        let closed = self.closed_set();
        let indices = reachability::actionable_nodes(&self.graph, &closed);
        indices
            .iter()
            .filter_map(|&i| self.graph.node_id(i))
            .collect()
    }

    /// Check if the graph has cycles.
    pub fn has_cycles(&self) -> bool {
        cycles::has_cycles(&self.graph)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{Dependency, IssueStatus, IssueType};
    use std::collections::HashMap;

    fn make_issue(id: &str, status: IssueStatus, deps: Vec<&str>) -> Issue {
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
            status,
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
    fn test_engine_simple_chain() {
        let issues = vec![
            make_issue("a", IssueStatus::Closed, vec![]),
            make_issue("b", IssueStatus::Open, vec!["a"]),
            make_issue("c", IssueStatus::Open, vec!["b"]),
        ];

        let engine = AnalysisEngine::new(&issues);
        assert_eq!(engine.graph().node_count(), 3);
        assert_eq!(engine.graph().edge_count(), 2);
        assert_eq!(engine.metrics().pagerank.len(), 3);
    }

    #[test]
    fn test_engine_empty() {
        let engine = AnalysisEngine::new(&[]);
        assert_eq!(engine.graph().node_count(), 0);
        assert!(!engine.has_cycles());
    }

    #[test]
    fn test_engine_actionable() {
        let issues = vec![
            make_issue("a", IssueStatus::Closed, vec![]),
            make_issue("b", IssueStatus::Open, vec!["a"]),
            make_issue("c", IssueStatus::Open, vec!["b"]),
        ];

        let engine = AnalysisEngine::new(&issues);
        let actionable = engine.actionable_ids();
        assert!(actionable.contains(&"b".to_string()));
    }

    #[test]
    fn test_engine_cycles() {
        let issues = vec![
            make_issue("a", IssueStatus::Open, vec!["b"]),
            make_issue("b", IssueStatus::Open, vec!["c"]),
            make_issue("c", IssueStatus::Open, vec!["a"]),
        ];

        let engine = AnalysisEngine::new(&issues);
        assert!(engine.has_cycles());
    }
}
