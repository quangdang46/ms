//! Triage scoring and recommendations.
//!
//! Provides a comprehensive triage result with recommendations,
//! quick wins, blockers, and project health assessment.

use serde::Serialize;

use super::AnalysisEngine;
use super::priority::ImpactScore;
use crate::graph::types::IssueStatus;

/// Full triage result.
#[derive(Debug, Clone, Serialize)]
pub struct TriageResult {
    pub recommendations: Vec<Recommendation>,
    pub quick_wins: Vec<QuickWin>,
    pub blockers: Vec<BlockerItem>,
    pub health: ProjectHealth,
    pub impact_scores: Vec<ImpactScore>,
    pub total_issues: usize,
    pub active_issues: usize,
}

/// A recommended action item.
#[derive(Debug, Clone, Serialize)]
pub struct Recommendation {
    pub id: String,
    pub title: String,
    pub reason: String,
    pub score: f64,
}

/// A quick win (low effort, high impact).
#[derive(Debug, Clone, Serialize)]
pub struct QuickWin {
    pub id: String,
    pub title: String,
    pub reason: String,
}

/// A critical blocker item.
#[derive(Debug, Clone, Serialize)]
pub struct BlockerItem {
    pub id: String,
    pub title: String,
    pub blocking_count: usize,
    pub reason: String,
}

/// Project health assessment.
#[derive(Debug, Clone, Serialize)]
pub struct ProjectHealth {
    pub total: usize,
    pub open: usize,
    pub in_progress: usize,
    pub closed: usize,
    pub blocked: usize,
    pub has_cycles: bool,
    pub cycle_count: usize,
    pub density: f64,
}

impl AnalysisEngine {
    /// Compute full triage result.
    pub fn compute_triage(&self) -> TriageResult {
        let impact_scores = self.compute_impact_scores();

        let total = self.issues().len();
        let open = self
            .issues()
            .iter()
            .filter(|i| i.status == IssueStatus::Open)
            .count();
        let in_progress = self
            .issues()
            .iter()
            .filter(|i| i.status == IssueStatus::InProgress)
            .count();
        let closed = self
            .issues()
            .iter()
            .filter(|i| i.status.is_terminal())
            .count();
        let blocked = self
            .issues()
            .iter()
            .filter(|i| i.status == IssueStatus::Blocked)
            .count();
        let active = self.issues().iter().filter(|i| i.is_active()).count();

        let scc = crate::graph::algorithms::cycles::tarjan_scc(self.graph());
        let cycle_count = scc.cycle_count;

        let recommendations: Vec<Recommendation> = impact_scores
            .iter()
            .take(10)
            .map(|s| Recommendation {
                id: s.id.clone(),
                title: s.title.clone(),
                reason: format!("Impact score: {:.3}", s.score),
                score: s.score,
            })
            .collect();

        let quick_wins: Vec<QuickWin> = impact_scores
            .iter()
            .filter(|s| s.score > 0.3)
            .take(5)
            .map(|s| QuickWin {
                id: s.id.clone(),
                title: s.title.clone(),
                reason: "High impact relative to complexity".to_string(),
            })
            .collect();

        let blockers: Vec<BlockerItem> = self
            .issues()
            .iter()
            .filter(|i| i.is_active())
            .filter_map(|i| {
                let idx = self.graph().node_idx(&i.id)?;
                let blocking_count = self.graph().out_degree(idx);
                if blocking_count > 0 {
                    Some(BlockerItem {
                        id: i.id.clone(),
                        title: i.title.clone(),
                        blocking_count,
                        reason: format!("Blocks {blocking_count} downstream issues"),
                    })
                } else {
                    None
                }
            })
            .take(10)
            .collect();

        TriageResult {
            recommendations,
            quick_wins,
            blockers,
            health: ProjectHealth {
                total,
                open,
                in_progress,
                closed,
                blocked,
                has_cycles: self.has_cycles(),
                cycle_count,
                density: self.graph().density(),
            },
            impact_scores,
            total_issues: total,
            active_issues: active,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{Dependency, Issue, IssueType};
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
    fn test_triage_basic() {
        let issues = vec![
            make_issue("a", IssueStatus::Closed, vec![]),
            make_issue("b", IssueStatus::Open, vec!["a"]),
            make_issue("c", IssueStatus::Open, vec!["b"]),
        ];
        let engine = AnalysisEngine::new(&issues);
        let triage = engine.compute_triage();

        assert_eq!(triage.total_issues, 3);
        assert_eq!(triage.health.open, 2);
        assert_eq!(triage.health.closed, 1);
    }
}
