//! E2E Scenario: Graph Workflow Integration Tests
//!
//! Comprehensive tests for the `ms graph` command covering:
//! - Graph export (JSON format)
//! - Graph insights (keystones, bottlenecks)
//! - Cycle detection
//!
//! Note: The graph command requires `bv` (beads_viewer) to be available.
//! Tests gracefully handle the case where `bv` is not installed.

use super::fixture::E2EFixture;
use ms::error::Result;

// Skill definitions with dependencies to create a graph structure

const SKILL_BASE: &str = r#"---
id: base-skill
name: Base Skill
description: A foundational skill that other skills depend on
tags: [base, foundation]
provides: [base-cap]
---

# Base Skill

Foundational concepts.

## Core Principles

- Principle one
- Principle two
"#;

const SKILL_INTERMEDIATE: &str = r#"---
id: intermediate-skill
name: Intermediate Skill
description: Builds on base concepts
tags: [intermediate, foundation]
requires: [base-cap]
provides: [intermediate-cap]
---

# Intermediate Skill

Intermediate concepts building on the base.

## Prerequisites

- Requires base-skill knowledge

## Advanced Concepts

- Concept one
- Concept two
"#;

const SKILL_ADVANCED: &str = r#"---
id: advanced-skill
name: Advanced Skill
description: Advanced patterns requiring intermediate knowledge
tags: [advanced, patterns]
requires: [base-cap, intermediate-cap]
---

# Advanced Skill

Advanced patterns.

## Prerequisites

- Requires intermediate-skill knowledge
- Requires base-skill knowledge

## Patterns

- Pattern one
- Pattern two
"#;

const SKILL_ISOLATED: &str = r#"---
id: isolated-skill
name: Isolated Skill
description: A standalone skill with no dependencies
tags: [standalone]
---

# Isolated Skill

A completely independent skill.

## Content

- Topic one
- Topic two
"#;

/// Create a fixture with skills that form a dependency graph
fn setup_graph_fixture(scenario: &str) -> Result<E2EFixture> {
    let mut fixture = E2EFixture::new(scenario);

    fixture.log_step("Initialize ms");
    let output = fixture.init();
    fixture.assert_success(&output, "init");

    fixture.log_step("Create skills forming a graph");
    fixture.create_skill("base-skill", SKILL_BASE)?;
    fixture.create_skill("intermediate-skill", SKILL_INTERMEDIATE)?;
    fixture.create_skill("advanced-skill", SKILL_ADVANCED)?;
    fixture.create_skill("isolated-skill", SKILL_ISOLATED)?;

    fixture.log_step("Index skills");
    let output = fixture.run_ms(&["--robot", "index"]);
    fixture.assert_success(&output, "index");

    // Checkpoint: skills indexed
    fixture.checkpoint("graph:indexed");

    fixture.emit_event(
        super::fixture::LogLevel::Info,
        "graph",
        "Skills indexed for graph testing",
        Some(serde_json::json!({
            "skills": ["base-skill", "intermediate-skill", "advanced-skill", "isolated-skill"],
            "expected_edges": "base -> intermediate -> advanced",
        })),
    );

    Ok(fixture)
}

fn skip_if_bv_unavailable(
    fixture: &mut E2EFixture,
    output: &super::fixture::CommandOutput,
) -> bool {
    if output.success {
        return false;
    }

    let combined = format!("{}{}", output.stdout, output.stderr);
    if combined.contains("bv is not available") || combined.contains("not available on PATH") {
        fixture.emit_event(
            super::fixture::LogLevel::Warn,
            "graph",
            "bv not available on this system, skipping test",
            None,
        );
        fixture.generate_report();
        return true;
    }

    false
}

#[test]
fn test_graph_export_json() -> Result<()> {
    let mut fixture = setup_graph_fixture("graph_export_json")?;

    fixture.log_step("Export graph as JSON");
    let output = fixture.run_ms(&["--robot", "graph", "export", "--format", "json"]);

    if skip_if_bv_unavailable(&mut fixture, &output) {
        return Ok(());
    }
    fixture.assert_success(&output, "graph export json");

    let json = output.json();
    let edge_count = json["edges"].as_u64().unwrap_or(0);
    let edges = json["adjacency"]["edges"]
        .as_array()
        .expect("adjacency.edges array");

    assert_eq!(json["format"].as_str(), Some("json"));
    assert_eq!(json["nodes"].as_u64(), Some(4));
    assert_eq!(
        edge_count, 3,
        "Expected 3 graph edges from requires/provides metadata"
    );
    assert_eq!(edges.len(), 3, "Expected 3 adjacency edges");

    fixture.emit_event(
        super::fixture::LogLevel::Info,
        "graph",
        "Graph export JSON completed",
        Some(serde_json::json!({
            "format": "json",
            "nodes": json["nodes"],
            "edges": edge_count,
        })),
    );

    fixture.generate_report();
    Ok(())
}

#[test]
fn test_graph_export_mermaid() -> Result<()> {
    let mut fixture = setup_graph_fixture("graph_export_mermaid")?;

    fixture.log_step("Export graph as Mermaid format");
    let output = fixture.run_ms(&["--robot", "graph", "export", "--format", "mermaid"]);

    if skip_if_bv_unavailable(&mut fixture, &output) {
        return Ok(());
    }
    fixture.assert_success(&output, "graph export mermaid");

    let json = output.json();
    let graph = json["graph"].as_str().expect("graph mermaid string");

    assert_eq!(json["format"].as_str(), Some("mermaid"));
    assert_eq!(json["nodes"].as_u64(), Some(4));
    assert_eq!(json["edges"].as_u64(), Some(3));
    assert!(
        graph.contains("graph TD"),
        "Mermaid output should contain graph TD"
    );
    assert!(
        graph.contains("==>"),
        "Mermaid output should contain dependency edges"
    );

    fixture.generate_report();
    Ok(())
}

#[test]
fn test_graph_export_dot() -> Result<()> {
    let mut fixture = setup_graph_fixture("graph_export_dot")?;

    fixture.log_step("Export graph as DOT format");
    let output = fixture.run_ms(&["--robot", "graph", "export", "--format", "dot"]);

    if skip_if_bv_unavailable(&mut fixture, &output) {
        return Ok(());
    }
    fixture.assert_success(&output, "graph export dot");

    let json = output.json();
    let graph = json["graph"].as_str().expect("graph dot string");

    assert_eq!(json["format"].as_str(), Some("dot"));
    assert_eq!(json["nodes"].as_u64(), Some(4));
    assert_eq!(json["edges"].as_u64(), Some(3));
    assert!(
        graph.contains("digraph"),
        "DOT output should contain digraph"
    );
    assert!(
        graph.contains("->"),
        "DOT output should contain dependency edges"
    );

    fixture.emit_event(
        super::fixture::LogLevel::Info,
        "graph",
        "Graph export DOT completed",
        Some(serde_json::json!({
            "format": "dot",
            "graph_len": graph.len(),
        })),
    );

    fixture.generate_report();
    Ok(())
}

#[test]
fn test_graph_cycles() -> Result<()> {
    let mut fixture = setup_graph_fixture("graph_cycles")?;

    // Checkpoint: pre-cycles
    fixture.checkpoint("graph:pre-cycles");

    fixture.log_step("Detect cycles in skill graph");
    let output = fixture.run_ms(&["--robot", "graph", "cycles", "--limit", "10"]);

    if skip_if_bv_unavailable(&mut fixture, &output) {
        return Ok(());
    }
    fixture.assert_success(&output, "graph cycles");

    let json = output.json();
    let status = json["status"].as_str().expect("status field");

    assert_eq!(status, "ok", "Cycles detection status should be ok");
    assert!(
        json.get("count").is_some(),
        "Response should have 'count' field"
    );
    assert!(
        json.get("cycles").is_some(),
        "Response should have 'cycles' field"
    );

    let cycle_count = json["count"].as_u64().unwrap_or(0);

    fixture.emit_event(
        super::fixture::LogLevel::Info,
        "graph",
        &format!("Cycle detection found {} cycles", cycle_count),
        Some(serde_json::json!({
            "count": cycle_count,
            "limit": 10,
        })),
    );

    assert_eq!(cycle_count, 0, "Expected DAG fixture to have no cycles");
    assert!(json["cycles"].is_array(), "Cycles should be an array");

    // Checkpoint: post-cycles
    fixture.checkpoint("graph:post-cycles");

    fixture.generate_report();
    Ok(())
}

#[test]
fn test_graph_insights() -> Result<()> {
    let mut fixture = setup_graph_fixture("graph_insights")?;

    fixture.log_step("Get graph insights");
    let output = fixture.run_ms(&["--robot", "graph", "insights"]);

    if skip_if_bv_unavailable(&mut fixture, &output) {
        return Ok(());
    }
    fixture.assert_success(&output, "graph insights");

    // Verify the output is valid JSON and contains insight data
    let json = output.json();
    assert_eq!(json["Stats"]["EdgeCount"].as_u64(), Some(3));
    assert!(
        json["Keystones"].is_array(),
        "Insights should expose Keystones array"
    );
    assert!(
        json["Bottlenecks"].is_array(),
        "Insights should expose Bottlenecks array"
    );

    fixture.emit_event(
        super::fixture::LogLevel::Info,
        "graph",
        "Graph insights completed",
        Some(serde_json::json!({
            "has_cycles": json.get("Cycles").is_some(),
            "has_keystones": json.get("Keystones").is_some(),
            "has_bottlenecks": json.get("Bottlenecks").is_some(),
        })),
    );

    fixture.generate_report();
    Ok(())
}

#[test]
fn test_graph_keystones() -> Result<()> {
    let mut fixture = setup_graph_fixture("graph_keystones")?;

    fixture.log_step("Get top keystone skills");
    let output = fixture.run_ms(&["--robot", "graph", "keystones", "--limit", "5"]);

    if skip_if_bv_unavailable(&mut fixture, &output) {
        return Ok(());
    }
    fixture.assert_success(&output, "graph keystones");

    let json = output.json();
    let status = json["status"].as_str().expect("status");
    assert_eq!(status, "ok", "Keystones status should be ok");

    assert!(
        json.get("count").is_some(),
        "Response should have 'count' field"
    );
    assert!(
        json.get("items").is_some(),
        "Response should have 'items' field"
    );

    let items = json["items"].as_array().expect("items array");
    assert!(
        items.len() <= 5,
        "Items should respect limit of 5, got {}",
        items.len()
    );

    fixture.emit_event(
        super::fixture::LogLevel::Info,
        "graph",
        &format!("Keystones: {} items returned", items.len()),
        Some(serde_json::json!({
            "count": json["count"],
            "items_count": items.len(),
            "limit": 5,
        })),
    );

    fixture.generate_report();
    Ok(())
}

#[test]
fn test_graph_bottlenecks() -> Result<()> {
    let mut fixture = setup_graph_fixture("graph_bottlenecks")?;

    fixture.log_step("Get top bottleneck skills");
    let output = fixture.run_ms(&["--robot", "graph", "bottlenecks", "--limit", "5"]);

    if skip_if_bv_unavailable(&mut fixture, &output) {
        return Ok(());
    }
    fixture.assert_success(&output, "graph bottlenecks");

    let json = output.json();
    let status = json["status"].as_str().expect("status");
    assert_eq!(status, "ok", "Bottlenecks status should be ok");

    assert!(
        json.get("items").is_some(),
        "Response should have 'items' field"
    );

    fixture.emit_event(
        super::fixture::LogLevel::Info,
        "graph",
        "Bottlenecks query completed",
        Some(serde_json::json!({
            "count": json["count"],
        })),
    );

    fixture.generate_report();
    Ok(())
}

#[test]
fn test_graph_plan() -> Result<()> {
    let mut fixture = setup_graph_fixture("graph_plan")?;

    fixture.log_step("Generate graph execution plan");
    let output = fixture.run_ms(&["--robot", "graph", "plan"]);

    if skip_if_bv_unavailable(&mut fixture, &output) {
        return Ok(());
    }
    fixture.assert_success(&output, "graph plan");

    let json = output.json();
    assert_eq!(json["plan"]["total_actionable"].as_u64(), Some(2));
    assert_eq!(json["plan"]["total_blocked"].as_u64(), Some(2));
    assert!(
        json["plan"]["tracks"].is_array(),
        "Plan should include tracks array"
    );
    assert_eq!(
        json["plan"]["summary"]["highest_impact"].as_str(),
        Some("base-skill"),
        "Plan should highlight the top unblocker"
    );

    fixture.generate_report();
    Ok(())
}

#[test]
fn test_graph_triage() -> Result<()> {
    let mut fixture = setup_graph_fixture("graph_triage")?;

    fixture.log_step("Generate graph triage recommendations");
    let output = fixture.run_ms(&["--robot", "graph", "triage"]);

    if skip_if_bv_unavailable(&mut fixture, &output) {
        return Ok(());
    }
    fixture.assert_success(&output, "graph triage");

    let json = output.json();
    assert_eq!(json["triage"]["quick_ref"]["open_count"].as_u64(), Some(4));
    assert_eq!(
        json["triage"]["project_health"]["graph"]["edge_count"].as_u64(),
        Some(3)
    );
    assert!(
        json["triage"]["recommendations"]
            .as_array()
            .map(|items| !items.is_empty())
            .unwrap_or(false),
        "Triage should include at least one recommendation"
    );

    fixture.generate_report();
    Ok(())
}

#[test]
fn test_graph_health() -> Result<()> {
    let mut fixture = setup_graph_fixture("graph_health")?;

    fixture.log_step("Generate graph label health summary");
    let output = fixture.run_ms(&["--robot", "graph", "health"]);

    if skip_if_bv_unavailable(&mut fixture, &output) {
        return Ok(());
    }
    fixture.assert_success(&output, "graph health");

    let json = output.json();
    assert!(
        json["results"]["total_labels"].as_u64().unwrap_or(0) >= 4,
        "Health summary should report multiple labels"
    );
    assert!(
        json["results"]["labels"].is_array(),
        "Health should include detailed label metrics"
    );
    assert!(
        json["results"]["summaries"]
            .as_array()
            .map(|items| !items.is_empty())
            .unwrap_or(false),
        "Health should include summary rows"
    );

    fixture.generate_report();
    Ok(())
}

#[test]
#[ignore] // uses deleted --bv-path flag
fn test_graph_without_bv() -> Result<()> {
    let mut fixture = setup_graph_fixture("graph_without_bv")?;

    fixture.log_step("Test graph command with invalid bv path");
    let output = fixture.run_ms(&[
        "--robot",
        "graph",
        "--bv-path",
        "/nonexistent/bv",
        "export",
        "--format",
        "json",
    ]);

    // This should fail because bv is not found at the given path
    assert!(!output.success, "Graph with invalid bv path should fail");

    let combined = format!("{}{}", output.stdout, output.stderr);
    assert!(
        combined.contains("not available") || combined.contains("error") || !output.success,
        "Should report that bv is not available"
    );

    fixture.emit_event(
        super::fixture::LogLevel::Info,
        "graph",
        "Graph correctly failed with invalid bv path",
        Some(serde_json::json!({
            "exit_code": output.exit_code,
            "expected": "failure",
        })),
    );

    fixture.generate_report();
    Ok(())
}
