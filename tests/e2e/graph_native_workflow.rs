//! E2E tests for the native graph analysis engine.
//!
//! Tests verify all 8 `ms graph` subcommands produce valid output.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

struct GraphFixture {
    root: PathBuf,
    ms_root: PathBuf,
}

impl GraphFixture {
    fn setup(name: &str) -> Self {
        let root = tempfile::tempdir().unwrap().into_path();
        let ms_root = root.join(".ms");

        // Initialize ms workspace
        let output = Command::new("cargo")
            .args(["run", "--", "init", "--path", root.to_str().unwrap()])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("failed to init ms workspace");

        assert!(output.status.success(), "ms init failed: {}", String::from_utf8_lossy(&output.stderr));

        // Create test skills with dependencies
        let skill_a_dir = root.join("skills").join("skill-a");
        let skill_b_dir = root.join("skills").join("skill-b");
        let skill_c_dir = root.join("skills").join("skill-c");
        fs::create_dir_all(&skill_a_dir).unwrap();
        fs::create_dir_all(&skill_b_dir).unwrap();
        fs::create_dir_all(&skill_c_dir).unwrap();

        fs::write(skill_a_dir.join("SKILL.md"),
            "---\nname: skill-a\ndescription: Base skill\ntags: [base]\n---\n# Skill A\nBase functionality.\n"
        ).unwrap();

        fs::write(skill_b_dir.join("SKILL.md"),
            "---\nname: skill-b\ndescription: Depends on skill-a\ntags: [dep]\nrequires: [skill-a]\n---\n# Skill B\nIntermediate.\n"
        ).unwrap();

        fs::write(skill_c_dir.join("SKILL.md"),
            "---\nname: skill-c\ndescription: Depends on a and b\ntags: [top]\nrequires: [skill-a, skill-b]\n---\n# Skill C\nAdvanced.\n"
        ).unwrap();

        // Index skills
        let output = Command::new("cargo")
            .args(["run", "--", "index", "add", root.join("skills").to_str().unwrap()])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .env("MS_ROOT", ms_root.to_str().unwrap())
            .output()
            .expect("failed to index skills");

        assert!(output.status.success(), "ms index add failed: {}", String::from_utf8_lossy(&output.stderr));

        Self { root, ms_root }
    }

    fn run_graph(&self, subcommand: &str, args: &[&str]) -> CommandOutput {
        let mut all_args = vec!["--robot", "graph", subcommand];
        all_args.extend_from_slice(args);

        let output = Command::new("cargo")
            .args(["run", "--"])
            .args(&all_args)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .env("MS_ROOT", self.ms_root.to_str().unwrap())
            .output()
            .expect(&format!("failed to run ms graph {}", subcommand));

        CommandOutput {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        }
    }
}

struct CommandOutput {
    success: bool,
    stdout: String,
    stderr: String,
}

impl CommandOutput {
    fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.stdout).unwrap_or_else(|e| {
            panic!("Invalid JSON output: {}\nstdout: {}", e, self.stdout)
        })
    }
}

// ============================================================================
// Test all 8 graph subcommands
// ============================================================================

#[test]
fn graph_insights_returns_valid_json() {
    let fixture = GraphFixture::setup("insights");
    let output = fixture.run_graph("insights", &[]);
    assert!(output.success, "insights failed: {}", output.stderr);

    let json = output.json();
    // Should have at least keystones or bottlenecks arrays
    assert!(json.get("keystones").is_some() || json.get("bottlenecks").is_some(),
        "insights should contain keystones or bottlenecks");
}

#[test]
fn graph_plan_returns_valid_json() {
    let fixture = GraphFixture::setup("plan");
    let output = fixture.run_graph("plan", &[]);
    assert!(output.success, "plan failed: {}", output.stderr);

    let json = output.json();
    assert!(json.get("tracks").is_some(), "plan should contain tracks");
}

#[test]
fn graph_triage_returns_valid_json() {
    let fixture = GraphFixture::setup("triage");
    let output = fixture.run_graph("triage", &[]);
    assert!(output.success, "triage failed: {}", output.stderr);

    let json = output.json();
    // Triage should have recommendations or health
    assert!(json.get("recommendations").is_some() || json.get("health").is_some(),
        "triage should contain recommendations or health");
}

#[test]
fn graph_export_json_format() {
    let fixture = GraphFixture::setup("export-json");
    let output = fixture.run_graph("export", &[]);
    assert!(output.success, "export failed: {}", output.stderr);

    let json = output.json();
    assert!(json.get("nodes").is_some(), "export should contain nodes");
    assert!(json.get("edges").is_some(), "export should contain edges");
}

#[test]
fn graph_export_dot_format() {
    let fixture = GraphFixture::setup("export-dot");
    let output = fixture.run_graph("export", &["--format", "dot"]);
    assert!(output.success, "export dot failed: {}", output.stderr);

    assert!(output.stdout.contains("digraph"), "DOT output should contain 'digraph'");
    assert!(output.stdout.contains("skill-a"), "DOT should reference skill-a");
}

#[test]
fn graph_export_mermaid_format() {
    let fixture = GraphFixture::setup("export-mermaid");
    let output = fixture.run_graph("export", &["--format", "mermaid"]);
    assert!(output.success, "export mermaid failed: {}", output.stderr);

    assert!(output.stdout.contains("graph"), "Mermaid output should contain 'graph'");
    assert!(output.stdout.contains("skill-a"), "Mermaid should reference skill-a");
}

#[test]
fn graph_cycles_returns_valid_json() {
    let fixture = GraphFixture::setup("cycles");
    let output = fixture.run_graph("cycles", &[]);
    assert!(output.success, "cycles failed: {}", output.stderr);

    let json = output.json();
    assert!(json.get("cycles").is_some(), "cycles should contain cycles array");
}

#[test]
fn graph_keystones_returns_valid_json() {
    let fixture = GraphFixture::setup("keystones");
    let output = fixture.run_graph("keystones", &[]);
    assert!(output.success, "keystones failed: {}", output.stderr);

    let json = output.json();
    assert!(json.get("items").is_some(), "keystones should contain items array");
    // skill-a is a root, should be a keystone
    let items = json["items"].as_array().expect("items should be array");
    let ids: Vec<&str> = items.iter().filter_map(|i| i["id"].as_str()).collect();
    assert!(ids.contains(&"skill-a"), "skill-a should be a keystone");
}

#[test]
fn graph_bottlenecks_returns_valid_json() {
    let fixture = GraphFixture::setup("bottlenecks");
    let output = fixture.run_graph("bottlenecks", &[]);
    assert!(output.success, "bottlenecks failed: {}", output.stderr);

    let json = output.json();
    assert!(json.get("items").is_some(), "bottlenecks should contain items array");
}

#[test]
fn graph_health_returns_valid_json() {
    let fixture = GraphFixture::setup("health");
    let output = fixture.run_graph("health", &[]);
    assert!(output.success, "health failed: {}", output.stderr);

    let json = output.json();
    assert!(json.get("labels").is_some(), "health should contain labels");
}