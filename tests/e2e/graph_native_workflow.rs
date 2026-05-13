//! E2E tests for the native graph analysis engine.
//!
//! Tests verify all 8 `ms graph` subcommands produce valid output
//! using the locally built `./target/debug/ms` binary.

use std::fs;
use std::path::PathBuf;

struct GraphFixture {
    ms_root: PathBuf,
}

impl GraphFixture {
    fn setup(_name: &str) -> Self {
        let root = tempfile::tempdir().unwrap().keep();
        let ms_root = root.join(".ms");

        // Create the .ms directory structure
        fs::create_dir_all(&ms_root).expect("Failed to create .ms dir");
        fs::create_dir_all(ms_root.join("index")).expect("Failed to create index dir");
        fs::create_dir_all(ms_root.join("archive")).expect("Failed to create archive dir");

        // Create test skills with dependency chain: skill-a -> skill-b -> skill-c
        let skill_a_dir = root.join("skills").join("skill-a");
        let skill_b_dir = root.join("skills").join("skill-b");
        let skill_c_dir = root.join("skills").join("skill-c");
        fs::create_dir_all(&skill_a_dir).unwrap();
        fs::create_dir_all(&skill_b_dir).unwrap();
        fs::create_dir_all(&skill_c_dir).unwrap();

        fs::write(
            skill_a_dir.join("SKILL.md"),
            "---\nname: skill-a\ndescription: Base skill\ntags: [base]\n---\n# Skill A\nBase functionality.\n",
        )
        .unwrap();

        fs::write(
            skill_b_dir.join("SKILL.md"),
            "---\nname: skill-b\ndescription: Depends on skill-a\ntags: [dep]\nrequires: [skill-a]\n---\n# Skill B\nIntermediate.\n",
        )
        .unwrap();

        fs::write(
            skill_c_dir.join("SKILL.md"),
            "---\nname: skill-c\ndescription: Depends on a and b\ntags: [top]\nrequires: [skill-a, skill-b]\n---\n# Skill C\nAdvanced.\n",
        )
        .unwrap();

        Self { ms_root }
    }

    fn run_graph(&self, subcommand: &str, args: &[&str]) -> std::process::Output {
        let mut all_args = vec!["--robot", "graph", subcommand];
        all_args.extend_from_slice(args);

        std::process::Command::new(assert_cmd::cargo_bin!("ms"))
            .args(&all_args)
            .env("MS_ROOT", self.ms_root.to_str().unwrap())
            .output()
            .unwrap_or_else(|e| panic!("failed to run ms graph {}: {}", subcommand, e))
    }
}

fn parse_json(output: &std::process::Output) -> serde_json::Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "Invalid JSON output: {}\nstdout: {}\nstderr: {}",
            e,
            stdout,
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn assert_success(output: &std::process::Output, label: &str) {
    if !output.status.success() {
        panic!(
            "{} failed:\nstdout: {}\nstderr: {}",
            label,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

// ============================================================================
// Test all 8 graph subcommands
// ============================================================================

#[test]
fn graph_insights_returns_valid_json() {
    let fixture = GraphFixture::setup("insights");
    let output = fixture.run_graph("insights", &[]);
    assert_success(&output, "insights");

    let json = parse_json(&output);
    assert!(
        json.get("keystones").is_some() || json.get("bottlenecks").is_some(),
        "insights should contain keystones or bottlenecks"
    );
}

#[test]
fn graph_plan_returns_valid_json() {
    let fixture = GraphFixture::setup("plan");
    let output = fixture.run_graph("plan", &[]);
    assert_success(&output, "plan");

    let json = parse_json(&output);
    assert!(json.get("tracks").is_some(), "plan should contain tracks");
}

#[test]
fn graph_triage_returns_valid_json() {
    let fixture = GraphFixture::setup("triage");
    let output = fixture.run_graph("triage", &[]);
    assert_success(&output, "triage");

    let json = parse_json(&output);
    assert!(
        json.get("recommendations").is_some() || json.get("health").is_some(),
        "triage should contain recommendations or health"
    );
}

#[test]
fn graph_export_json_format() {
    let fixture = GraphFixture::setup("export-json");
    let output = fixture.run_graph("export", &[]);
    assert_success(&output, "export");

    let json = parse_json(&output);
    assert!(
        json.get("nodes").is_some() || json.get("data").is_some(),
        "export should contain graph data"
    );
}

#[test]
fn graph_export_dot_format() {
    let fixture = GraphFixture::setup("export-dot");
    let output = fixture.run_graph("export", &["--format", "dot"]);
    assert_success(&output, "export dot");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("digraph"),
        "DOT output should contain 'digraph'"
    );
}

#[test]
fn graph_export_mermaid_format() {
    let fixture = GraphFixture::setup("export-mermaid");
    let output = fixture.run_graph("export", &["--format", "mermaid"]);
    assert_success(&output, "export mermaid");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("graph"),
        "Mermaid output should contain 'graph'"
    );
}

#[test]
fn graph_cycles_returns_valid_json() {
    let fixture = GraphFixture::setup("cycles");
    let output = fixture.run_graph("cycles", &[]);
    assert_success(&output, "cycles");

    let json = parse_json(&output);
    assert!(
        json.get("cycles").is_some() || json.get("count").is_some(),
        "cycles should contain cycles or count"
    );
}

#[test]
fn graph_keystones_returns_valid_json() {
    let fixture = GraphFixture::setup("keystones");
    let output = fixture.run_graph("keystones", &[]);
    assert_success(&output, "keystones");

    let json = parse_json(&output);
    assert!(
        json.get("items").is_some() || json.get("count").is_some(),
        "keystones should contain items or count"
    );
}

#[test]
fn graph_bottlenecks_returns_valid_json() {
    let fixture = GraphFixture::setup("bottlenecks");
    let output = fixture.run_graph("bottlenecks", &[]);
    assert_success(&output, "bottlenecks");

    let json = parse_json(&output);
    assert!(
        json.get("items").is_some() || json.get("count").is_some(),
        "bottlenecks should contain items or count"
    );
}

#[test]
fn graph_health_returns_valid_json() {
    let fixture = GraphFixture::setup("health");
    let output = fixture.run_graph("health", &[]);
    assert_success(&output, "health");

    let json = parse_json(&output);
    assert!(json.get("labels").is_some(), "health should contain labels");
}
