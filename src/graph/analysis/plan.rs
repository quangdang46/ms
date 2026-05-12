//! Execution plan generation with parallel tracks.

use serde::Serialize;

use super::AnalysisEngine;

/// A single actionable item in the execution plan.
#[derive(Debug, Clone, Serialize)]
pub struct PlanItem {
    pub id: String,
    pub title: String,
    pub priority: u8,
    pub status: String,
    pub unblocks: Vec<String>,
}

/// A group of related actionable items (work stream).
#[derive(Debug, Clone, Serialize)]
pub struct ExecutionTrack {
    pub track_id: String,
    pub items: Vec<PlanItem>,
    pub reason: String,
}

/// Quick insights about the plan.
#[derive(Debug, Clone, Serialize)]
pub struct PlanSummary {
    pub highest_impact: String,
    pub impact_reason: String,
    pub unblocks_count: i32,
}

/// Complete work plan with parallel tracks.
#[derive(Debug, Clone, Serialize)]
pub struct ExecutionPlan {
    pub tracks: Vec<ExecutionTrack>,
    pub total_actionable: usize,
    pub total_blocked: usize,
    pub summary: PlanSummary,
}

impl AnalysisEngine {
    /// Generate a dependency-respecting execution plan with parallel tracks.
    pub fn generate_plan(&self) -> ExecutionPlan {
        let actionable = self.actionable_ids();
        let actionable_set: std::collections::HashSet<&str> =
            actionable.iter().map(|s| s.as_str()).collect();

        let unblocks_map = self.compute_all_unblocks(&actionable);

        let components = self.find_connected_components();

        let tracks = self.build_tracks(&components, &actionable_set, &unblocks_map);

        let total_open: usize = self
            .issues()
            .iter()
            .filter(|i| !i.status.is_terminal())
            .count();

        let summary = compute_plan_summary(&actionable, &unblocks_map);

        ExecutionPlan {
            total_actionable: actionable.len(),
            total_blocked: total_open.saturating_sub(actionable.len()),
            tracks,
            summary,
        }
    }

    fn compute_all_unblocks(
        &self,
        actionable: &[String],
    ) -> std::collections::HashMap<String, Vec<String>> {
        let mut map = std::collections::HashMap::new();
        for id in actionable {
            map.insert(id.clone(), self.compute_unblocks(id));
        }
        map
    }

    fn compute_unblocks(&self, issue_id: &str) -> Vec<String> {
        let graph = self.graph();
        let Some(blocker_idx) = graph.node_idx(issue_id) else {
            return Vec::new();
        };

        let closed = self.closed_set();
        let mut unblocks = Vec::new();

        for &dep_idx in graph.predecessors_slice(blocker_idx) {
            if closed.get(dep_idx).copied().unwrap_or(false) {
                continue;
            }
            if let Some(dep_id) = graph.node_id(dep_idx) {
                let all_closed: bool = graph.successors_slice(dep_idx).iter().all(|&blocker| {
                    if blocker == blocker_idx {
                        return true;
                    }
                    closed.get(blocker).copied().unwrap_or(false)
                });
                if all_closed {
                    unblocks.push(dep_id);
                }
            }
        }

        unblocks.sort();
        unblocks
    }

    fn find_connected_components(&self) -> std::collections::HashMap<String, Vec<String>> {
        let mut parent: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();

        let mut ids: Vec<&str> = self.issues().iter().map(|i| i.id.as_str()).collect();
        ids.sort();

        for id in &ids {
            parent.insert(id.to_string(), id.to_string());
        }

        for issue in self.issues() {
            for dep in &issue.dependencies {
                if self.graph().node_idx(&dep.id).is_some() {
                    let px = find_root(&parent, &issue.id);
                    let py = find_root(&parent, &dep.id);
                    if px != py {
                        if px < py {
                            parent.insert(py, px);
                        } else {
                            parent.insert(px, py);
                        }
                    }
                }
            }
        }

        let mut components: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for id in &ids {
            let root = find_root(&parent, id);
            components.entry(root).or_default().push(id.to_string());
        }

        components
    }

    fn build_tracks(
        &self,
        components: &std::collections::HashMap<String, Vec<String>>,
        actionable_set: &std::collections::HashSet<&str>,
        unblocks_map: &std::collections::HashMap<String, Vec<String>>,
    ) -> Vec<ExecutionTrack> {
        let mut tracks = Vec::new();
        let mut roots: Vec<&str> = components.keys().map(|s| s.as_str()).collect();
        roots.sort();

        for (track_num, root) in roots.iter().enumerate() {
            let members = &components[*root];
            let mut actionable_members: Vec<_> = members
                .iter()
                .filter(|id| actionable_set.contains(id.as_str()))
                .collect();

            if actionable_members.is_empty() {
                continue;
            }

            actionable_members.sort_by(|a, b| {
                let issue_a = self.issues().iter().find(|i| i.id == **a);
                let issue_b = self.issues().iter().find(|i| i.id == **b);
                let pa = issue_a.map(|i| i.priority).unwrap_or(4);
                let pb = issue_b.map(|i| i.priority).unwrap_or(4);
                pa.cmp(&pb).then_with(|| a.cmp(b))
            });

            let items: Vec<PlanItem> = actionable_members
                .iter()
                .filter_map(|id| {
                    let issue = self.issues().iter().find(|i| i.id == **id)?;
                    Some(PlanItem {
                        id: issue.id.clone(),
                        title: issue.title.clone(),
                        priority: issue.priority,
                        status: issue.status.to_string(),
                        unblocks: unblocks_map.get(*id).cloned().unwrap_or_default(),
                    })
                })
                .collect();

            let reason = if items.len() == 1 {
                "Single actionable item".to_string()
            } else if components.len() == 1 {
                "All issues in connected graph".to_string()
            } else {
                "Independent work stream".to_string()
            };

            tracks.push(ExecutionTrack {
                track_id: generate_track_id(track_num + 1),
                items,
                reason,
            });
        }

        tracks
    }
}

fn find_root(parent: &std::collections::HashMap<String, String>, x: &str) -> String {
    let mut current = x;
    while let Some(p) = parent.get(current) {
        if p == current {
            return p.clone();
        }
        current = p.as_str();
    }
    x.to_string()
}

fn generate_track_id(n: usize) -> String {
    if n == 0 {
        return "track-?".to_string();
    }
    let mut n = n - 1;
    let mut letters = Vec::new();
    loop {
        letters.push(char::from(b'A' + (n % 26) as u8));
        if n < 26 {
            break;
        }
        n = n / 26 - 1;
    }
    letters.reverse();
    format!("track-{}", letters.into_iter().collect::<String>())
}

fn compute_plan_summary(
    actionable: &[String],
    unblocks_map: &std::collections::HashMap<String, Vec<String>>,
) -> PlanSummary {
    if actionable.is_empty() {
        return PlanSummary {
            highest_impact: String::new(),
            impact_reason: String::new(),
            unblocks_count: 0,
        };
    }

    let mut sorted = actionable.to_vec();
    sorted.sort();

    let mut highest_id = String::new();
    let mut highest_count = -1i32;

    for id in &sorted {
        let count = unblocks_map.get(id).map(|v| v.len()).unwrap_or(0) as i32;
        if count > highest_count {
            highest_count = count;
            highest_id = id.clone();
        }
    }

    let reason = if highest_count <= 0 {
        "No downstream dependencies".to_string()
    } else if highest_count == 1 {
        "Unblocks 1 task".to_string()
    } else {
        "Unblocks multiple tasks".to_string()
    };

    PlanSummary {
        highest_impact: highest_id,
        impact_reason: reason,
        unblocks_count: highest_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{Dependency, Issue, IssueStatus, IssueType};
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
    fn test_plan_simple() {
        let issues = vec![
            make_issue("a", IssueStatus::Closed, vec![]),
            make_issue("b", IssueStatus::Open, vec!["a"]),
        ];
        let engine = AnalysisEngine::new(&issues);
        let plan = engine.generate_plan();

        assert_eq!(plan.total_actionable, 1);
        assert!(!plan.tracks.is_empty());
    }

    #[test]
    fn test_plan_empty() {
        let engine = AnalysisEngine::new(&[]);
        let plan = engine.generate_plan();
        assert_eq!(plan.total_actionable, 0);
        assert!(plan.tracks.is_empty());
    }

    #[test]
    fn test_track_id_generation() {
        assert_eq!(generate_track_id(1), "track-A");
        assert_eq!(generate_track_id(2), "track-B");
        assert_eq!(generate_track_id(26), "track-Z");
        assert_eq!(generate_track_id(27), "track-AA");
    }
}
