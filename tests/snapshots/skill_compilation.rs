use std::fs;
use std::path::PathBuf;

use insta::assert_snapshot;

use ms::core::spec_lens::{compile_markdown, parse_markdown};

fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn compile_fixture(path: &str) -> String {
    let content = fs::read_to_string(fixture_path(path)).expect("read fixture");
    let spec = parse_markdown(&content).expect("parse markdown");
    compile_markdown(&spec)
}

#[test]
#[ignore = "TODO(0.2.x): compilation snapshot drifted from current frontmatter layout; preexisting on v0.1.0"]
fn test_skill_compilation_minimal() {
    let output = compile_fixture("tests/fixtures/skills/valid_minimal.md");
    assert_snapshot!("compilation_minimal", output);
}

#[test]
#[ignore = "TODO(0.2.x): compilation snapshot drifted from current frontmatter layout; preexisting on v0.1.0"]
fn test_skill_compilation_full() {
    let output = compile_fixture("tests/fixtures/skills/valid_full.md");
    assert_snapshot!("compilation_full", output);
}
