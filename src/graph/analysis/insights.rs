//! Generate insights from graph analysis.
//!
//! Translates raw graph metrics into actionable data: bottlenecks,
//! keystones, influencers, hubs, authorities, etc.

use serde::Serialize;

use super::AnalysisEngine;

/// A single item in an insight list with its metric value.
#[derive(Debug, Clone, Serialize)]
pub struct InsightItem {
    pub id: String,
    pub value: f64,
}

/// High-level summary of graph analysis.
#[derive(Debug, Clone, Serialize)]
pub struct Insights {
    pub bottlenecks: Vec<InsightItem>,
    pub keystones: Vec<InsightItem>,
    pub influencers: Vec<InsightItem>,
    pub hubs: Vec<InsightItem>,
    pub authorities: Vec<InsightItem>,
    pub cores: Vec<InsightItem>,
    pub articulation: Vec<String>,
    pub slack: Vec<InsightItem>,
    pub orphans: Vec<String>,
    pub cycles: Vec<Vec<String>>,
    pub cluster_density: f64,
}

impl AnalysisEngine {
    /// Generate insights from the pre-computed graph metrics.
    pub fn generate_insights(&self, limit: usize) -> Insights {
        let n = self.graph().node_count();
        if n == 0 {
            return Insights {
                bottlenecks: Vec::new(),
                keystones: Vec::new(),
                influencers: Vec::new(),
                hubs: Vec::new(),
                authorities: Vec::new(),
                cores: Vec::new(),
                articulation: Vec::new(),
                slack: Vec::new(),
                orphans: Vec::new(),
                cycles: Vec::new(),
                cluster_density: 0.0,
            };
        }

        let m = self.metrics();

        let bottlenecks = top_items(&self.graph, &m.betweenness, limit);
        let keystones = top_items(&self.graph, &m.critical_path_heights, limit);
        let influencers = top_items(&self.graph, &m.eigenvector, limit);
        let hubs = top_items(&self.graph, &m.hubs, limit);
        let authorities = top_items(&self.graph, &m.authorities, limit);
        let cores = top_items_u32(&self.graph, &m.core_numbers, limit);
        let slack_items = top_items(&self.graph, &m.slack, limit);

        let art_pts = crate::graph::algorithms::articulation::articulation_points(&self.graph);
        let articulation: Vec<String> = art_pts
            .iter()
            .take(limit)
            .filter_map(|&i| self.graph().node_id(i))
            .collect();

        let orphans = find_orphans(&self.graph, &self.issues);

        let scc = crate::graph::algorithms::cycles::tarjan_scc(&self.graph);
        let cycles: Vec<Vec<String>> = scc
            .components
            .iter()
            .filter(|c| c.len() > 1)
            .take(limit)
            .map(|c| {
                let mut ids: Vec<String> =
                    c.iter().filter_map(|&i| self.graph().node_id(i)).collect();
                ids.sort();
                ids
            })
            .collect();

        Insights {
            bottlenecks,
            keystones,
            influencers,
            hubs,
            authorities,
            cores,
            articulation,
            slack: slack_items,
            orphans,
            cycles,
            cluster_density: self.graph().density(),
        }
    }
}

fn top_items(
    graph: &crate::graph::engine::graph::DiGraph,
    values: &[f64],
    limit: usize,
) -> Vec<InsightItem> {
    let mut items: Vec<(usize, f64)> = values
        .iter()
        .enumerate()
        .filter_map(|(i, &v)| if v > 0.0 { Some((i, v)) } else { None })
        .collect();

    items.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });

    items
        .into_iter()
        .take(limit)
        .filter_map(|(i, v)| graph.node_id(i).map(|id| InsightItem { id, value: v }))
        .collect()
}

fn top_items_u32(
    graph: &crate::graph::engine::graph::DiGraph,
    values: &[u32],
    limit: usize,
) -> Vec<InsightItem> {
    let mut items: Vec<(usize, u32)> = values
        .iter()
        .enumerate()
        .filter_map(|(i, &v)| if v > 0 { Some((i, v)) } else { None })
        .collect();

    items.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    items
        .into_iter()
        .take(limit)
        .filter_map(|(i, v)| {
            graph.node_id(i).map(|id| InsightItem {
                id,
                value: f64::from(v),
            })
        })
        .collect()
}

fn find_orphans(
    graph: &crate::graph::engine::graph::DiGraph,
    issues: &[crate::graph::types::Issue],
) -> Vec<String> {
    // Orphans = nodes with no incoming edges (no blockers, no dependencies).
    // With edge direction blocker -> dependent, in_degree == 0 means no one blocks this node.
    let mut orphans: Vec<String> = issues
        .iter()
        .filter(|issue| {
            graph
                .node_idx(&issue.id)
                .map(|idx| graph.in_degree(idx) == 0)
                .unwrap_or(false)
        })
        .map(|issue| issue.id.clone())
        .collect();
    orphans.sort();
    orphans
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
    fn test_insights_diamond() {
        let issues = vec![
            make_issue("top", vec!["left", "right"]),
            make_issue("left", vec!["bottom"]),
            make_issue("right", vec!["bottom"]),
            make_issue("bottom", vec![]),
        ];
        let engine = AnalysisEngine::new(&issues);
        let insights = engine.generate_insights(10);

        assert!(!insights.bottlenecks.is_empty());
        assert!(!insights.orphans.is_empty());
        assert!(insights.orphans.contains(&"bottom".to_string()));
        assert!(!engine.has_cycles());
    }

    #[test]
    fn test_insights_empty() {
        let engine = AnalysisEngine::new(&[]);
        let insights = engine.generate_insights(10);
        assert!(insights.bottlenecks.is_empty());
    }

    #[test]
    fn test_insights_cycle_detection() {
        let issues = vec![
            make_issue("a", vec!["b"]),
            make_issue("b", vec!["c"]),
            make_issue("c", vec!["a"]),
        ];
        let engine = AnalysisEngine::new(&issues);
        let insights = engine.generate_insights(10);
        assert!(!insights.cycles.is_empty());
        assert!(engine.has_cycles());
    }
}
