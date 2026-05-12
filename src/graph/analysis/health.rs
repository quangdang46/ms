//! Label health analysis.
//!
//! Analyzes label distribution, staleness, and attention recommendations.

use serde::Serialize;

use super::AnalysisEngine;

/// Label distribution and health info.
#[derive(Debug, Clone, Serialize)]
pub struct LabelHealth {
    pub labels: Vec<LabelInfo>,
    pub total_labels: usize,
    pub unlabeled_count: usize,
}

/// Information about a single label.
#[derive(Debug, Clone, Serialize)]
pub struct LabelInfo {
    pub label: String,
    pub count: usize,
    pub open_count: usize,
    pub closed_count: usize,
}

impl AnalysisEngine {
    /// Compute label health analysis.
    pub fn compute_label_health(&self) -> LabelHealth {
        use std::collections::HashMap;

        let mut label_counts: HashMap<String, (usize, usize, usize)> = HashMap::new();
        let mut unlabeled = 0;

        for issue in self.issues() {
            if issue.labels.is_empty() {
                unlabeled += 1;
                continue;
            }

            for label in &issue.labels {
                let entry = label_counts.entry(label.clone()).or_insert((0, 0, 0));
                entry.0 += 1;
                if issue.status.is_terminal() {
                    entry.2 += 1;
                } else {
                    entry.1 += 1;
                }
            }
        }

        let mut labels: Vec<LabelInfo> = label_counts
            .into_iter()
            .map(|(label, (count, open, closed))| LabelInfo {
                label,
                count,
                open_count: open,
                closed_count: closed,
            })
            .collect();

        labels.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.label.cmp(&b.label)));

        LabelHealth {
            total_labels: labels.len(),
            unlabeled_count: unlabeled,
            labels,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{Issue, IssueStatus, IssueType};
    use std::collections::HashMap;

    fn make_issue(id: &str, status: IssueStatus, labels: Vec<&str>) -> Issue {
        Issue {
            id: id.to_string(),
            title: id.to_string(),
            description: String::new(),
            status,
            priority: 2,
            issue_type: IssueType::Task,
            owner: None,
            assignee: None,
            labels: labels.into_iter().map(String::from).collect(),
            notes: None,
            created_at: None,
            created_by: None,
            updated_at: None,
            closed_at: None,
            dependencies: vec![],
            dependents: vec![],
            extra: HashMap::new(),
        }
    }

    #[test]
    fn test_label_health_basic() {
        let issues = vec![
            make_issue("a", IssueStatus::Open, vec!["rust", "cli"]),
            make_issue("b", IssueStatus::Closed, vec!["rust"]),
            make_issue("c", IssueStatus::Open, vec![]),
        ];
        let engine = AnalysisEngine::new(&issues);
        let health = engine.compute_label_health();

        assert_eq!(health.unlabeled_count, 1);
        assert_eq!(health.total_labels, 2);
        assert_eq!(health.labels[0].label, "rust");
        assert_eq!(health.labels[0].count, 2);
    }
}
