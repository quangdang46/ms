//! Priority scoring and impact analysis.

use serde::Serialize;

use super::AnalysisEngine;

/// Impact score breakdown for a single issue.
#[derive(Debug, Clone, Serialize)]
pub struct ImpactScore {
    pub id: String,
    pub title: String,
    pub score: f64,
    pub breakdown: ScoreBreakdown,
}

/// Breakdown of the impact score by factor.
#[derive(Debug, Clone, Serialize)]
pub struct ScoreBreakdown {
    pub pagerank: f64,
    pub betweenness: f64,
    pub blocker_ratio: f64,
    pub critical_path: f64,
    pub priority_boost: f64,
}

/// Weighted factors for impact scoring.
struct Weights {
    pagerank: f64,
    betweenness: f64,
    blocker_ratio: f64,
    critical_path: f64,
    priority_boost: f64,
}

const DEFAULT_WEIGHTS: Weights = Weights {
    pagerank: 0.22,
    betweenness: 0.20,
    blocker_ratio: 0.13,
    critical_path: 0.25,
    priority_boost: 0.20,
};

impl AnalysisEngine {
    /// Compute impact scores for all active issues.
    pub fn compute_impact_scores(&self) -> Vec<ImpactScore> {
        let n = self.graph().node_count();
        if n == 0 {
            return Vec::new();
        }

        let m = self.metrics();

        let pr_max = m
            .pagerank
            .iter()
            .cloned()
            .fold(0.0_f64, f64::max)
            .max(1e-10);
        let bw_max = m
            .betweenness
            .iter()
            .cloned()
            .fold(0.0_f64, f64::max)
            .max(1e-10);
        let cp_max = m
            .critical_path_heights
            .iter()
            .cloned()
            .fold(0.0_f64, f64::max)
            .max(1e-10);

        let mut scores: Vec<ImpactScore> = self
            .issues()
            .iter()
            .filter(|issue| issue.is_active())
            .filter_map(|issue| {
                let idx = self.graph().node_idx(&issue.id)?;
                if idx >= n {
                    return None;
                }

                let pr = m.pagerank.get(idx).copied().unwrap_or(0.0) / pr_max;
                let bw = m.betweenness.get(idx).copied().unwrap_or(0.0) / bw_max;
                let cp = m.critical_path_heights.get(idx).copied().unwrap_or(0.0) / cp_max;

                let in_deg = self.graph().in_degree(idx) as f64;
                let max_in = self
                    .graph()
                    .in_degrees()
                    .iter()
                    .max()
                    .copied()
                    .unwrap_or(1)
                    .max(1) as f64;
                let blocker_ratio = in_deg / max_in;

                let priority_boost = (4 - issue.priority) as f64 / 4.0;

                let w = &DEFAULT_WEIGHTS;
                let score = w.pagerank * pr
                    + w.betweenness * bw
                    + w.blocker_ratio * blocker_ratio
                    + w.critical_path * cp
                    + w.priority_boost * priority_boost;

                Some(ImpactScore {
                    id: issue.id.clone(),
                    title: issue.title.clone(),
                    score,
                    breakdown: ScoreBreakdown {
                        pagerank: pr,
                        betweenness: bw,
                        blocker_ratio,
                        critical_path: cp,
                        priority_boost,
                    },
                })
            })
            .collect();

        scores.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scores
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{Dependency, Issue, IssueStatus, IssueType};
    use std::collections::HashMap;

    fn make_issue(id: &str, priority: u8, deps: Vec<&str>) -> Issue {
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
            priority,
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
    fn test_impact_scores_diamond() {
        let issues = vec![
            make_issue("top", 0, vec!["left", "right"]),
            make_issue("left", 1, vec!["bottom"]),
            make_issue("right", 1, vec!["bottom"]),
            make_issue("bottom", 2, vec![]),
        ];
        let engine = AnalysisEngine::new(&issues);
        let scores = engine.compute_impact_scores();
        assert_eq!(scores.len(), 4);
        assert!(scores[0].score > 0.0);
    }

    #[test]
    fn test_impact_scores_empty() {
        let engine = AnalysisEngine::new(&[]);
        let scores = engine.compute_impact_scores();
        assert!(scores.is_empty());
    }
}
