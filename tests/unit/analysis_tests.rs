//! Unit tests for the graph analysis engine (insights, triage, priority, plan).

use std::collections::HashMap;

use ms::graph::analysis::AnalysisEngine;
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
        labels: vec!["layer:project".to_string()],
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

fn closed_issue(id: &str) -> Issue {
    let mut issue = sample_issue(id);
    issue.status = IssueStatus::Closed;
    issue
}

fn high_priority_issue(id: &str) -> Issue {
    let mut issue = sample_issue(id);
    issue.priority = 0;
    issue
}

// ============================================================================
// AnalysisEngine Construction Tests
// ============================================================================

#[test]
fn analysis_engine_empty() {
    let engine = AnalysisEngine::new(&[]);
    assert!(!engine.has_cycles());
    assert!(engine.issues().is_empty());
}

#[test]
fn analysis_engine_single_issue() {
    let issues = vec![sample_issue("a")];
    let engine = AnalysisEngine::new(&issues);
    assert!(!engine.has_cycles());
    assert_eq!(engine.issues().len(), 1);
}

#[test]
fn analysis_engine_closed_issue_counted() {
    let issues = vec![closed_issue("a")];
    let engine = AnalysisEngine::new(&issues);
    assert_eq!(engine.issues().len(), 1);
    assert_eq!(engine.closed_set().len(), 1);
}

#[test]
fn analysis_engine_chain() {
    // a -> b -> c (a depends on b, b depends on c)
    let issues = vec![
        issue_with_deps("a", vec!["b"]),
        issue_with_deps("b", vec!["c"]),
        sample_issue("c"),
    ];
    let engine = AnalysisEngine::new(&issues);
    assert!(!engine.has_cycles());
    assert_eq!(engine.issues().len(), 3);
}

// ============================================================================
// Impact Score Tests
// ============================================================================

#[test]
fn impact_scores_returns_all_nodes() {
    let issues = vec![
        issue_with_deps("a", vec!["b"]),
        issue_with_deps("b", vec!["c"]),
        sample_issue("c"),
    ];
    let engine = AnalysisEngine::new(&issues);
    let scores = engine.compute_impact_scores();

    assert_eq!(scores.len(), 3);
    let ids: Vec<&str> = scores.iter().map(|s| s.id.as_str()).collect();
    assert!(ids.contains(&"a"));
    assert!(ids.contains(&"b"));
    assert!(ids.contains(&"c"));
}

#[test]
fn impact_scores_empty() {
    let engine = AnalysisEngine::new(&[]);
    let scores = engine.compute_impact_scores();
    assert!(scores.is_empty());
}

#[test]
fn impact_scores_normalized() {
    let issues = vec![
        issue_with_deps("a", vec!["b"]),
        issue_with_deps("b", vec!["c"]),
        sample_issue("c"),
    ];
    let engine = AnalysisEngine::new(&issues);
    let scores = engine.compute_impact_scores();

    for score in &scores {
        assert!(
            score.score >= 0.0 && score.score <= 1.0,
            "Score for {} should be 0-1 normalized, got {}",
            score.id,
            score.score
        );
    }
}

#[test]
fn impact_scores_priority_boost() {
    let issues = vec![high_priority_issue("critical"), sample_issue("normal")];
    let engine = AnalysisEngine::new(&issues);
    let scores = engine.compute_impact_scores();

    let critical = scores.iter().find(|s| s.id == "critical").unwrap();
    let normal = scores.iter().find(|s| s.id == "normal").unwrap();
    assert!(
        critical.score >= normal.score,
        "High priority issue should have >= impact score"
    );
}

// ============================================================================
// Insights Tests
// ============================================================================

#[test]
fn insights_empty() {
    let engine = AnalysisEngine::new(&[]);
    let insights = engine.generate_insights(10);
    assert!(insights.keystones.is_empty());
    assert!(insights.bottlenecks.is_empty());
    assert!(insights.orphans.is_empty());
}

#[test]
fn insights_chain_finds_keystones() {
    let issues: Vec<Issue> = (0..5)
        .map(|i| {
            if i == 0 {
                sample_issue(&format!("s{}", i))
            } else {
                issue_with_deps(&format!("s{}", i), vec![&format!("s{}", i - 1)])
            }
        })
        .collect();
    let engine = AnalysisEngine::new(&issues);
    let insights = engine.generate_insights(5);

    assert!(
        !insights.keystones.is_empty(),
        "Chain should have keystones"
    );
    let keystone_ids: Vec<&str> = insights.keystones.iter().map(|k| k.id.as_str()).collect();
    assert!(
        keystone_ids.contains(&"s0"),
        "Root of chain should be a keystone"
    );
}

#[test]
fn insights_respects_limit() {
    let issues: Vec<Issue> = (0..20).map(|i| sample_issue(&format!("s{}", i))).collect();
    let engine = AnalysisEngine::new(&issues);
    let insights = engine.generate_insights(5);

    assert!(
        insights.keystones.len() <= 5,
        "Keystones should respect limit"
    );
}

// ============================================================================
// Triage Tests
// ============================================================================

#[test]
fn triage_empty() {
    let engine = AnalysisEngine::new(&[]);
    let triage = engine.compute_triage();
    assert!(triage.recommendations.is_empty());
    assert!(triage.quick_wins.is_empty());
    assert!(triage.blockers.is_empty());
}

#[test]
fn triage_with_issues() {
    let issues = vec![issue_with_deps("a", vec!["b"]), sample_issue("b")];
    let engine = AnalysisEngine::new(&issues);
    let triage = engine.compute_triage();

    assert!(triage.health.total >= 2);
}

// ============================================================================
// Plan Tests
// ============================================================================

#[test]
fn plan_empty() {
    let engine = AnalysisEngine::new(&[]);
    let plan = engine.generate_plan();
    assert!(plan.tracks.is_empty());
}

#[test]
fn plan_chain_produces_tracks() {
    let issues = vec![
        issue_with_deps("a", vec!["b"]),
        issue_with_deps("b", vec!["c"]),
        sample_issue("c"),
    ];
    let engine = AnalysisEngine::new(&issues);
    let plan = engine.generate_plan();

    assert!(
        !plan.tracks.is_empty(),
        "Chain should produce at least one track"
    );
}

// ============================================================================
// Health Tests
// ============================================================================

#[test]
fn health_label_distribution() {
    let issues = vec![sample_issue("a"), sample_issue("b")];
    let engine = AnalysisEngine::new(&issues);
    let health = engine.compute_label_health();

    assert!(!health.labels.is_empty(), "Should have label health data");
}

// ============================================================================
// Cycle Detection Integration
// ============================================================================

#[test]
fn cycle_detection_no_cycles() {
    let issues = vec![issue_with_deps("a", vec!["b"]), sample_issue("b")];
    let engine = AnalysisEngine::new(&issues);
    assert!(!engine.has_cycles());
}
