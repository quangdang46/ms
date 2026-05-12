//! Unit tests for the graph module.
//!
//! Tests cover:
//! - skills_to_issues conversion
//! - Dependency resolution (direct and capability-based)
//! - Priority mapping from quality scores
//! - Label normalization
//! - BvClient construction and configuration
//! - JSONL file writing

use std::collections::HashMap;

use serde_json::Value as JsonValue;

use ms::graph::types::{Dependency, Issue, IssueStatus, IssueType};
use ms::graph::skills::skills_to_issues;
use ms::storage::sqlite::SkillRecord;

// ============================================================================
// Test Fixtures
// ============================================================================

fn base_skill_record(id: &str) -> SkillRecord {
    SkillRecord {
        id: id.to_string(),
        name: format!("Skill {}", id),
        description: String::new(),
        version: Some("0.1.0".to_string()),
        author: None,
        provider: None,
        source_path: String::new(),
        source_layer: "project".to_string(),
        git_remote: None,
        git_commit: None,
        content_hash: "hash".to_string(),
        body: String::new(),
        metadata_json: "{}".to_string(),
        assets_json: "{}".to_string(),
        token_count: 0,
        quality_score: 0.5,
        indexed_at: String::new(),
        modified_at: String::new(),
        is_deprecated: false,
        deprecation_reason: None,
        archive_format_version: None,
        provenance_json: "{}".to_string(),
    }
}

fn skill_with_meta(id: &str, meta: &serde_json::Value) -> SkillRecord {
    let mut skill = base_skill_record(id);
    skill.metadata_json = meta.to_string();
    skill
}

fn skill_with_quality(id: &str, quality_score: f64) -> SkillRecord {
    let mut skill = base_skill_record(id);
    skill.quality_score = quality_score;
    skill
}

fn skill_with_layer(id: &str, layer: &str) -> SkillRecord {
    let mut skill = base_skill_record(id);
    skill.source_layer = layer.to_string();
    skill
}

fn deprecated_skill(id: &str) -> SkillRecord {
    let mut skill = base_skill_record(id);
    skill.is_deprecated = true;
    skill.deprecation_reason = Some("Superseded".to_string());
    skill
}

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
// skills_to_issues Tests - Basic Conversion
// ============================================================================

#[test]
fn skills_to_issues_empty_slice() {
    let skills: Vec<SkillRecord> = vec![];
    let issues = skills_to_issues(&skills).unwrap();
    assert!(issues.is_empty());
}

#[test]
fn skills_to_issues_single_skill_minimal() {
    let skill = base_skill_record("test-skill");
    let issues = skills_to_issues(&[skill]).unwrap();

    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].id, "test-skill");
    assert_eq!(issues[0].title, "Skill test-skill");
    assert_eq!(issues[0].status, IssueStatus::Open);
    assert_eq!(issues[0].issue_type, IssueType::Task);
}

#[test]
fn skills_to_issues_preserves_description() {
    let mut skill = base_skill_record("desc-skill");
    skill.description = "This is a detailed description".to_string();

    let issues = skills_to_issues(&[skill]).unwrap();
    assert_eq!(issues[0].description, "This is a detailed description");
}

#[test]
fn skills_to_issues_preserves_owner_from_author() {
    let mut skill = base_skill_record("author-skill");
    skill.author = Some("alice@example.com".to_string());

    let issues = skills_to_issues(&[skill]).unwrap();
    assert_eq!(issues[0].owner, Some("alice@example.com".to_string()));
}

#[test]
fn skills_to_issues_sets_version_in_extra() {
    let mut skill = base_skill_record("ver-skill");
    skill.version = Some("1.2.3".to_string());

    let issues = skills_to_issues(&[skill]).unwrap();
    let extra = &issues[0].extra;
    assert_eq!(
        extra.get("skill_version"),
        Some(&JsonValue::String("1.2.3".to_string()))
    );
}

#[test]
fn skills_to_issues_default_version_in_extra() {
    let mut skill = base_skill_record("no-ver-skill");
    skill.version = None;

    let issues = skills_to_issues(&[skill]).unwrap();
    let extra = &issues[0].extra;
    assert_eq!(
        extra.get("skill_version"),
        Some(&JsonValue::String("0.1.0".to_string()))
    );
}

#[test]
fn skills_to_issues_sets_quality_score_in_extra() {
    let skill = skill_with_quality("qual-skill", 0.87);

    let issues = skills_to_issues(&[skill]).unwrap();
    let extra = &issues[0].extra;
    assert_eq!(extra.get("quality_score"), Some(&JsonValue::from(0.87)));
}

// ============================================================================
// skills_to_issues Tests - Status Mapping
// ============================================================================

#[test]
fn skills_to_issues_active_skill_is_open() {
    let skill = base_skill_record("active-skill");
    let issues = skills_to_issues(&[skill]).unwrap();
    assert_eq!(issues[0].status, IssueStatus::Open);
}

#[test]
fn skills_to_issues_deprecated_skill_is_closed() {
    let skill = deprecated_skill("old-skill");
    let issues = skills_to_issues(&[skill]).unwrap();
    assert_eq!(issues[0].status, IssueStatus::Closed);
}

#[test]
fn skills_to_issues_mixed_deprecated_and_active() {
    let skills = vec![base_skill_record("active"), deprecated_skill("deprecated")];

    let issues = skills_to_issues(&skills).unwrap();
    let active_issue = issues.iter().find(|i| i.id == "active").unwrap();
    let dep_issue = issues.iter().find(|i| i.id == "deprecated").unwrap();

    assert_eq!(active_issue.status, IssueStatus::Open);
    assert_eq!(dep_issue.status, IssueStatus::Closed);
}

// ============================================================================
// skills_to_issues Tests - Priority Mapping from Quality
// ============================================================================

#[test]
fn skills_to_issues_quality_0_95_priority_0() {
    let skill = skill_with_quality("high-qual", 0.95);
    let issues = skills_to_issues(&[skill]).unwrap();
    assert_eq!(issues[0].priority, 0);
}

#[test]
fn skills_to_issues_quality_0_90_priority_0() {
    let skill = skill_with_quality("qual-90", 0.90);
    let issues = skills_to_issues(&[skill]).unwrap();
    assert_eq!(issues[0].priority, 0);
}

#[test]
fn skills_to_issues_quality_0_89_priority_1() {
    let skill = skill_with_quality("qual-89", 0.89);
    let issues = skills_to_issues(&[skill]).unwrap();
    assert_eq!(issues[0].priority, 1);
}

#[test]
fn skills_to_issues_quality_0_70_priority_1() {
    let skill = skill_with_quality("qual-70", 0.70);
    let issues = skills_to_issues(&[skill]).unwrap();
    assert_eq!(issues[0].priority, 1);
}

#[test]
fn skills_to_issues_quality_0_69_priority_2() {
    let skill = skill_with_quality("qual-69", 0.69);
    let issues = skills_to_issues(&[skill]).unwrap();
    assert_eq!(issues[0].priority, 2);
}

#[test]
fn skills_to_issues_quality_0_50_priority_2() {
    let skill = skill_with_quality("qual-50", 0.50);
    let issues = skills_to_issues(&[skill]).unwrap();
    assert_eq!(issues[0].priority, 2);
}

#[test]
fn skills_to_issues_quality_0_49_priority_3() {
    let skill = skill_with_quality("qual-49", 0.49);
    let issues = skills_to_issues(&[skill]).unwrap();
    assert_eq!(issues[0].priority, 3);
}

#[test]
fn skills_to_issues_quality_0_30_priority_3() {
    let skill = skill_with_quality("qual-30", 0.30);
    let issues = skills_to_issues(&[skill]).unwrap();
    assert_eq!(issues[0].priority, 3);
}

#[test]
fn skills_to_issues_quality_0_29_priority_4() {
    let skill = skill_with_quality("qual-29", 0.29);
    let issues = skills_to_issues(&[skill]).unwrap();
    assert_eq!(issues[0].priority, 4);
}

#[test]
fn skills_to_issues_quality_0_0_priority_4() {
    let skill = skill_with_quality("qual-zero", 0.0);
    let issues = skills_to_issues(&[skill]).unwrap();
    assert_eq!(issues[0].priority, 4);
}

// ============================================================================
// skills_to_issues Tests - Labels
// ============================================================================

#[test]
fn skills_to_issues_layer_label_added() {
    let skill = skill_with_layer("layer-skill", "project");
    let issues = skills_to_issues(&[skill]).unwrap();
    assert!(issues[0].labels.contains(&"layer:project".to_string()));
}

#[test]
fn skills_to_issues_different_layers() {
    let skills = vec![
        skill_with_layer("proj-skill", "project"),
        skill_with_layer("user-skill", "user"),
        skill_with_layer("sys-skill", "system"),
    ];

    let issues = skills_to_issues(&skills).unwrap();

    let proj_issue = issues.iter().find(|i| i.id == "proj-skill").unwrap();
    let user_issue = issues.iter().find(|i| i.id == "user-skill").unwrap();
    let sys_issue = issues.iter().find(|i| i.id == "sys-skill").unwrap();

    assert!(proj_issue.labels.contains(&"layer:project".to_string()));
    assert!(user_issue.labels.contains(&"layer:user".to_string()));
    assert!(sys_issue.labels.contains(&"layer:system".to_string()));
}

#[test]
fn skills_to_issues_tags_as_labels() {
    let skill = skill_with_meta(
        "tagged-skill",
        &serde_json::json!({
            "tags": ["rust", "cli", "testing"]
        }),
    );

    let issues = skills_to_issues(&[skill]).unwrap();
    let labels = &issues[0].labels;

    assert!(labels.contains(&"rust".to_string()));
    assert!(labels.contains(&"cli".to_string()));
    assert!(labels.contains(&"testing".to_string()));
}

#[test]
fn skills_to_issues_tags_lowercased() {
    let skill = skill_with_meta(
        "case-skill",
        &serde_json::json!({
            "tags": ["Rust", "CLI", "TESTING"]
        }),
    );

    let issues = skills_to_issues(&[skill]).unwrap();
    let labels = &issues[0].labels;

    // Should be lowercased
    assert!(labels.contains(&"rust".to_string()));
    assert!(labels.contains(&"cli".to_string()));
    assert!(labels.contains(&"testing".to_string()));

    // Uppercase versions should not be present
    assert!(!labels.contains(&"Rust".to_string()));
    assert!(!labels.contains(&"CLI".to_string()));
}

#[test]
fn skills_to_issues_tags_deduped() {
    let skill = skill_with_meta(
        "dup-skill",
        &serde_json::json!({
            "tags": ["rust", "Rust", "RUST"]
        }),
    );

    let issues = skills_to_issues(&[skill]).unwrap();
    let rust_count = issues[0].labels.iter().filter(|l| *l == "rust").count();
    assert_eq!(rust_count, 1, "Duplicate tags should be deduped");
}

#[test]
fn skills_to_issues_labels_sorted() {
    let skill = skill_with_meta(
        "sort-skill",
        &serde_json::json!({
            "tags": ["zebra", "alpha", "mango"]
        }),
    );

    let issues = skills_to_issues(&[skill]).unwrap();
    let labels = &issues[0].labels;

    // Labels should be sorted
    let mut sorted_labels = labels.clone();
    sorted_labels.sort();
    assert_eq!(labels, &sorted_labels);
}

// ============================================================================
// skills_to_issues Tests - Dependencies
// ============================================================================

#[test]
fn skills_to_issues_no_dependencies() {
    let skill = skill_with_meta(
        "no-deps",
        &serde_json::json!({
            "requires": [],
            "provides": []
        }),
    );

    let issues = skills_to_issues(&[skill]).unwrap();
    assert!(issues[0].dependencies.is_empty());
}

#[test]
fn skills_to_issues_direct_id_dependency() {
    let skill_a = skill_with_meta(
        "skill-a",
        &serde_json::json!({
            "requires": ["skill-b"]
        }),
    );
    let skill_b = skill_with_meta("skill-b", &serde_json::json!({}));

    let issues = skills_to_issues(&[skill_a, skill_b]).unwrap();
    let issue_a = issues.iter().find(|i| i.id == "skill-a").unwrap();

    assert_eq!(issue_a.dependencies.len(), 1);
    assert_eq!(issue_a.dependencies[0].id, "skill-b");
}

#[test]
fn skills_to_issues_capability_dependency() {
    let skill_a = skill_with_meta(
        "skill-a",
        &serde_json::json!({
            "requires": ["database"]
        }),
    );
    let skill_b = skill_with_meta(
        "skill-b",
        &serde_json::json!({
            "provides": ["database"]
        }),
    );

    let issues = skills_to_issues(&[skill_a, skill_b]).unwrap();
    let issue_a = issues.iter().find(|i| i.id == "skill-a").unwrap();

    assert_eq!(issue_a.dependencies.len(), 1);
    assert_eq!(issue_a.dependencies[0].id, "skill-b");
}

#[test]
fn skills_to_issues_case_insensitive_capability() {
    let skill_a = skill_with_meta(
        "skill-a",
        &serde_json::json!({
            "requires": ["DATABASE"]
        }),
    );
    let skill_b = skill_with_meta(
        "skill-b",
        &serde_json::json!({
            "provides": ["database"]
        }),
    );

    let issues = skills_to_issues(&[skill_a, skill_b]).unwrap();
    let issue_a = issues.iter().find(|i| i.id == "skill-a").unwrap();

    assert_eq!(
        issue_a.dependencies.len(),
        1,
        "Capability lookup should be case-insensitive"
    );
}

#[test]
fn skills_to_issues_multiple_providers_same_capability() {
    let skill_a = skill_with_meta(
        "skill-a",
        &serde_json::json!({
            "requires": ["storage"]
        }),
    );
    let skill_b = skill_with_meta(
        "skill-b",
        &serde_json::json!({
            "provides": ["storage"]
        }),
    );
    let skill_c = skill_with_meta(
        "skill-c",
        &serde_json::json!({
            "provides": ["storage"]
        }),
    );

    let issues = skills_to_issues(&[skill_a, skill_b, skill_c]).unwrap();
    let issue_a = issues.iter().find(|i| i.id == "skill-a").unwrap();

    // Both providers should be dependencies
    assert_eq!(issue_a.dependencies.len(), 2);
    let dep_ids: Vec<&str> = issue_a.dependencies.iter().map(|d| d.id.as_str()).collect();
    assert!(dep_ids.contains(&"skill-b"));
    assert!(dep_ids.contains(&"skill-c"));
}

#[test]
fn skills_to_issues_no_self_dependency() {
    let skill = skill_with_meta(
        "self-ref",
        &serde_json::json!({
            "requires": ["self-ref"],
            "provides": ["self-ref"]
        }),
    );

    let issues = skills_to_issues(&[skill]).unwrap();
    assert!(
        issues[0].dependencies.is_empty(),
        "Skill should not depend on itself"
    );
}

#[test]
fn skills_to_issues_unresolved_dependency() {
    let skill = skill_with_meta(
        "orphan",
        &serde_json::json!({
            "requires": ["nonexistent-capability"]
        }),
    );

    let issues = skills_to_issues(&[skill]).unwrap();
    // Unresolved dependency is simply not added
    assert!(issues[0].dependencies.is_empty());
}

#[test]
fn skills_to_issues_dependency_includes_title() {
    let mut skill_a = skill_with_meta(
        "skill-a",
        &serde_json::json!({
            "requires": ["skill-b"]
        }),
    );
    skill_a.name = "Skill A".to_string();

    let mut skill_b = skill_with_meta("skill-b", &serde_json::json!({}));
    skill_b.name = "Skill B with Custom Name".to_string();

    let issues = skills_to_issues(&[skill_a, skill_b]).unwrap();
    let issue_a = issues.iter().find(|i| i.id == "skill-a").unwrap();

    assert_eq!(issue_a.dependencies[0].title, "Skill B with Custom Name");
}

#[test]
fn skills_to_issues_dependency_includes_status() {
    let skill_a = skill_with_meta(
        "skill-a",
        &serde_json::json!({
            "requires": ["skill-b"]
        }),
    );
    let skill_b = deprecated_skill("skill-b");

    let issues = skills_to_issues(&[skill_a, skill_b]).unwrap();
    let issue_a = issues.iter().find(|i| i.id == "skill-a").unwrap();

    assert_eq!(issue_a.dependencies[0].status, Some(IssueStatus::Closed));
}

#[test]
fn skills_to_issues_dependencies_sorted() {
    let skill_a = skill_with_meta(
        "skill-a",
        &serde_json::json!({
            "requires": ["zebra", "alpha", "mango"]
        }),
    );
    let skill_z = skill_with_meta("zebra", &serde_json::json!({}));
    let skill_m = skill_with_meta("mango", &serde_json::json!({}));
    let skill_al = skill_with_meta("alpha", &serde_json::json!({}));

    let issues = skills_to_issues(&[skill_a, skill_z, skill_m, skill_al]).unwrap();
    let issue_a = issues.iter().find(|i| i.id == "skill-a").unwrap();

    let dep_ids: Vec<&str> = issue_a.dependencies.iter().map(|d| d.id.as_str()).collect();
    let mut sorted_ids = dep_ids.clone();
    sorted_ids.sort();
    assert_eq!(dep_ids, sorted_ids, "Dependencies should be sorted by ID");
}

// ============================================================================
// skills_to_issues Tests - Metadata Parsing Edge Cases
// ============================================================================

#[test]
fn skills_to_issues_empty_metadata_json() {
    let skill = skill_with_meta("empty-meta", &serde_json::json!({}));
    let issues = skills_to_issues(&[skill]).unwrap();

    // Should work with empty metadata
    assert_eq!(issues.len(), 1);
    assert!(issues[0].dependencies.is_empty());
    // Only layer label
    assert_eq!(issues[0].labels.len(), 1);
}

#[test]
fn skills_to_issues_null_metadata_fields() {
    let skill = skill_with_meta(
        "null-fields",
        &serde_json::json!({
            "tags": null,
            "requires": null,
            "provides": null
        }),
    );

    let issues = skills_to_issues(&[skill]).unwrap();
    assert!(issues[0].dependencies.is_empty());
    // Only layer label when tags are null
    assert_eq!(issues[0].labels.len(), 1);
}

#[test]
fn skills_to_issues_invalid_json_metadata() {
    let mut skill = base_skill_record("bad-json");
    skill.metadata_json = "not valid json at all".to_string();

    let issues = skills_to_issues(&[skill]).unwrap();
    // Should handle gracefully, treating as empty metadata
    assert_eq!(issues.len(), 1);
    assert!(issues[0].dependencies.is_empty());
}

#[test]
fn skills_to_issues_numeric_tags_ignored() {
    let skill = skill_with_meta(
        "numeric-tags",
        &serde_json::json!({
            "tags": [1, 2, "valid", 3.14, true]
        }),
    );

    let issues = skills_to_issues(&[skill]).unwrap();
    let non_layer_labels: Vec<_> = issues[0]
        .labels
        .iter()
        .filter(|l| !l.starts_with("layer:"))
        .collect();

    // Only string tags should be included
    assert_eq!(non_layer_labels.len(), 1);
    assert!(non_layer_labels.contains(&&"valid".to_string()));
}

// ============================================================================
// Issue Type Tests
// ============================================================================

#[test]
fn skills_to_issues_all_are_task_type() {
    let skills = vec![
        base_skill_record("skill-1"),
        base_skill_record("skill-2"),
        base_skill_record("skill-3"),
    ];

    let issues = skills_to_issues(&skills).unwrap();
    for issue in issues {
        assert_eq!(issue.issue_type, IssueType::Task);
    }
}

// ============================================================================
// Large Dataset Tests
// ============================================================================

#[test]
fn skills_to_issues_many_skills() {
    let skills: Vec<SkillRecord> = (0..100)
        .map(|i| base_skill_record(&format!("skill-{}", i)))
        .collect();

    let issues = skills_to_issues(&skills).unwrap();
    assert_eq!(issues.len(), 100);
}

#[test]
fn skills_to_issues_complex_dependency_graph() {
    // Create a chain: skill-0 -> skill-1 -> skill-2 -> ... -> skill-9
    let skills: Vec<SkillRecord> = (0..10)
        .map(|i| {
            if i == 0 {
                skill_with_meta(&format!("skill-{}", i), &serde_json::json!({}))
            } else {
                skill_with_meta(
                    &format!("skill-{}", i),
                    &serde_json::json!({
                        "requires": [format!("skill-{}", i - 1)]
                    }),
                )
            }
        })
        .collect();

    let issues = skills_to_issues(&skills).unwrap();

    // Verify chain
    let skill_5 = issues.iter().find(|i| i.id == "skill-5").unwrap();
    assert_eq!(skill_5.dependencies.len(), 1);
    assert_eq!(skill_5.dependencies[0].id, "skill-4");

    let skill_0 = issues.iter().find(|i| i.id == "skill-0").unwrap();
    assert!(skill_0.dependencies.is_empty());
}
