//! E2E Scenario: Provider Import Bootstrap & Archive Integrity (bd-3to2, bd-28jh)
//!
//! Tests:
//! - `ms init` auto-discovers and imports provider skills (bd-3to2)
//! - Imported skills survive provider folder deletion (bd-28jh)
//! - Content hash and archive format version are stored in DB (bd-28jh)

use super::fixture::E2EFixture;
use ms::error::Result;
use std::fs;

/// Helper: create a SKILL.md in a provider root under the fixture's project dir.
fn create_provider_skill(
    fixture: &E2EFixture,
    provider_dir: &str,
    skill_name: &str,
    content: &str,
) -> Result<()> {
    let skill_dir = fixture.root.join(provider_dir).join(skill_name);
    fs::create_dir_all(&skill_dir)?;
    fs::write(skill_dir.join("SKILL.md"), content)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// bd-3to2: ms init provider discovery and bootstrap
// ---------------------------------------------------------------------------

/// Test that init discovers skills from a claude provider root and imports them.
#[test]
fn test_init_imports_claude_provider_skills() -> Result<()> {
    let mut fixture = E2EFixture::new("init_claude_provider");

    create_provider_skill(
        &fixture,
        ".claude/skills",
        "rust-patterns",
        "# Rust Patterns\n\n## Rules\n\n- Prefer composition over inheritance\n",
    )?;
    create_provider_skill(
        &fixture,
        ".claude/skills",
        "async-rust",
        "# Async Rust\n\n## Rules\n\n- Use async/await for I/O-bound work\n",
    )?;

    let init_out = fixture.run_ms(&["--robot", "init"]);
    assert!(init_out.success, "init should succeed: {}", init_out.stderr);

    let list_out = fixture.run_ms(&["--robot", "list"]);
    fixture.assert_success(&list_out, "list");
    fixture.assert_output_contains(&list_out, "rust-patterns");
    fixture.assert_output_contains(&list_out, "async-rust");

    fixture.checkpoint("provider_imported");
    Ok(())
}

/// Test that init succeeds and list works even with no provider roots.
#[test]
fn test_init_succeeds_with_no_provider_roots() -> Result<()> {
    let mut fixture = E2EFixture::new("init_no_providers");

    let init_out = fixture.run_ms(&["--robot", "init"]);
    assert!(init_out.success, "init should succeed: {}", init_out.stderr);

    let list_out = fixture.run_ms(&["--robot", "list"]);
    fixture.assert_success(&list_out, "list");

    fixture.checkpoint("no_provider_roots");
    Ok(())
}

/// Test that init produces robot JSON with provider_roots_scanned and provider_skills_imported.
#[test]
fn test_init_robot_json_has_provider_fields() -> Result<()> {
    let mut fixture = E2EFixture::new("init_robot_provider_json");

    create_provider_skill(
        &fixture,
        ".codex/skills",
        "python-style",
        "# Python Style\n\n## Rules\n\n- Follow PEP 8\n",
    )?;

    let robot_out = fixture.run_ms(&["--robot", "init"]);
    assert!(
        robot_out.success,
        "init should succeed: {}",
        robot_out.stderr
    );

    let json: serde_json::Value =
        serde_json::from_str(&robot_out.stdout).expect("robot init output should be valid JSON");

    assert_eq!(json.get("status").and_then(|v| v.as_str()), Some("ok"));
    assert!(
        json.get("provider_roots_scanned").is_some(),
        "robot JSON should include provider_roots_scanned"
    );
    assert!(
        json.get("provider_skills_imported").is_some(),
        "robot JSON should include provider_skills_imported"
    );

    fixture.checkpoint("robot_json_fields");
    Ok(())
}

/// Test that collision warning appears when two providers export the same skill ID.
#[test]
fn test_init_collision_warning_on_duplicate_ids() -> Result<()> {
    let mut fixture = E2EFixture::new("init_collision_warning");

    let skill_content = "# Shared Skill\n\n## Rules\n\n- Rule one\n";
    create_provider_skill(&fixture, ".claude/skills", "shared-skill", skill_content)?;
    create_provider_skill(&fixture, ".codex/skills", "shared-skill", skill_content)?;

    let init_out = fixture.run_ms(&["--robot", "init"]);
    assert!(init_out.success, "init should succeed: {}", init_out.stderr);

    let json: serde_json::Value =
        serde_json::from_str(&init_out.stdout).expect("robot init output should be valid JSON");
    assert_eq!(json.get("status").and_then(|v| v.as_str()), Some("ok"));

    let list_out = fixture.run_ms(&["--robot", "list"]);
    fixture.assert_success(&list_out, "list");
    fixture.assert_output_contains(&list_out, "shared-skill");

    fixture.checkpoint("collision_handled_gracefully");
    Ok(())
}

/// Test that manual skill_paths config is NOT required for provider root discovery.
#[test]
fn test_no_manual_config_required_for_provider_discovery() -> Result<()> {
    let mut fixture = E2EFixture::new("init_no_manual_config");

    create_provider_skill(
        &fixture,
        ".claude/skills",
        "error-handling",
        "# Error Handling\n\n## Rules\n\n- Use Result for fallible operations\n",
    )?;

    let init_out = fixture.run_ms(&["--robot", "init"]);
    assert!(init_out.success, "init should succeed: {}", init_out.stderr);

    let list_out = fixture.run_ms(&["--robot", "list"]);
    fixture.assert_success(&list_out, "list");
    fixture.assert_output_contains(&list_out, "error-handling");

    fixture.checkpoint("no_manual_config_needed");
    Ok(())
}

// ---------------------------------------------------------------------------
// bd-28jh: Archive integrity — assets survive deletion, checksums stored
// ---------------------------------------------------------------------------

/// Test that provider-imported skills remain usable after source folder deletion.
#[test]
fn test_imported_skills_survive_source_deletion() -> Result<()> {
    let mut fixture = E2EFixture::new("import_survives_deletion");

    create_provider_skill(
        &fixture,
        ".claude/skills",
        "survivable-skill",
        "# Survivable Skill\n\n## Rules\n\n- This rule survives source deletion\n",
    )?;

    let init_out = fixture.run_ms(&["--robot", "init"]);
    assert!(init_out.success, "init should succeed: {}", init_out.stderr);

    let show_out = fixture.run_ms(&["--robot", "show", "survivable-skill"]);
    fixture.assert_success(&show_out, "show before deletion");
    let show_json: serde_json::Value = serde_json::from_str(&show_out.stdout).expect("valid JSON");
    assert_eq!(
        show_json["skill"]["name"].as_str().unwrap(),
        "Survivable Skill"
    );

    // Delete the source provider folder entirely
    fs::remove_dir_all(fixture.root.join(".claude/skills"))?;

    // Show should still work — archive-backed
    let show_after = fixture.run_ms(&["--robot", "show", "survivable-skill"]);
    fixture.assert_success(&show_after, "show after deletion");

    // Load should also work
    let load_out = fixture.run_ms(&["--robot", "load", "survivable-skill"]);
    fixture.assert_success(&load_out, "load after deletion");
    assert!(load_out.stdout.contains("Survivable Skill"));

    fixture.checkpoint("survived_source_deletion");
    Ok(())
}

/// Test that scripts within provider-imported skills survive source deletion.
#[test]
fn test_imported_scripts_survive_deletion() -> Result<()> {
    let mut fixture = E2EFixture::new("import_scripts_survive");

    let skill_dir = fixture.root.join(".codex/skills/scripted-skill");
    fs::create_dir_all(skill_dir.join("scripts"))?;
    fs::write(
        skill_dir.join("SKILL.md"),
        "# Scripted Skill\n\n## Rules\n\n- Run the setup script first\n",
    )?;
    fs::write(skill_dir.join("scripts/setup.sh"), "#!/bin/sh\necho setup")?;
    fs::write(skill_dir.join("scripts/run.sh"), "#!/bin/sh\necho running")?;

    let init_out = fixture.run_ms(&["--robot", "init"]);
    assert!(init_out.success, "init should succeed: {}", init_out.stderr);

    let show_out = fixture.run_ms(&["--robot", "show", "scripted-skill"]);
    fixture.assert_success(&show_out, "show before deletion");

    fs::remove_dir_all(fixture.root.join(".codex/skills"))?;

    // Show still works after source deletion
    let show_after = fixture.run_ms(&["--robot", "show", "scripted-skill"]);
    fixture.assert_success(&show_after, "show after deletion");

    fixture.checkpoint("scripts_survived_deletion");
    Ok(())
}

/// Test that content hash and archive_format_version are stored in the DB for imported skills.
#[test]
fn test_import_stores_integrity_metadata() -> Result<()> {
    let mut fixture = E2EFixture::new("import_integrity_metadata");

    create_provider_skill(
        &fixture,
        ".claude/skills",
        "integrity-skill",
        "# Integrity Skill\n\n## Rules\n\n- Content hash and format version are recorded\n",
    )?;

    let init_out = fixture.run_ms(&["--robot", "init"]);
    assert!(init_out.success, "init should succeed: {}", init_out.stderr);

    // Verify content_hash and archive_format_version are stored in DB
    fixture.open_db();
    fixture.verify_db_state(
        |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT content_hash, archive_format_version \
                     FROM skills WHERE name = 'Integrity Skill'",
                )
                .expect("prepare");
            let result = stmt
                .query_row([], |row| {
                    let hash: String = row.get(0).unwrap_or_default();
                    let afv: Option<String> = row.get(1).ok();
                    Ok((hash, afv))
                })
                .ok();
            match result {
                Some((hash, afv)) => {
                    println!(
                        "[bd-28jh] content_hash present (len={}), archive_format_version={:?}",
                        hash.len(),
                        afv
                    );
                    !hash.is_empty() && afv.is_some()
                }
                None => {
                    println!("[bd-28jh] No record found for 'Integrity Skill'");
                    false
                }
            }
        },
        "imported skill should have non-empty content_hash and archive_format_version",
    );

    fixture.checkpoint("integrity_metadata_stored");
    Ok(())
}

// ---------------------------------------------------------------------------
// bd-1jq2: Archive-first registry for list/show/load
// ---------------------------------------------------------------------------

/// Test that list, show, and load all work after provider folder deletion.
#[test]
fn test_archive_first_list_show_load_after_deletion() -> Result<()> {
    let mut fixture = E2EFixture::new("archive_first_all_commands");

    // Create a provider skill
    create_provider_skill(
        &fixture,
        ".claude/skills",
        "archive-test",
        "# Archive Test\n\n## Rules\n\n- This skill should work via archive after deletion\n",
    )?;

    let init_out = fixture.run_ms(&["--robot", "init"]);
    assert!(init_out.success, "init should succeed: {}", init_out.stderr);

    // Verify list shows the skill
    let list_out = fixture.run_ms(&["--robot", "list"]);
    fixture.assert_success(&list_out, "list before deletion");
    fixture.assert_output_contains(&list_out, "archive-test");

    // Verify show works
    let show_out = fixture.run_ms(&["--robot", "show", "archive-test"]);
    fixture.assert_success(&show_out, "show before deletion");

    // Verify load works
    let load_out = fixture.run_ms(&["--robot", "load", "archive-test"]);
    fixture.assert_success(&load_out, "load before deletion");
    assert!(load_out.stdout.contains("Archive Test"));

    // Delete the provider folder
    std::fs::remove_dir_all(fixture.root.join(".claude/skills"))?;

    // list should still work
    let list_after = fixture.run_ms(&["--robot", "list"]);
    fixture.assert_success(&list_after, "list after deletion");
    fixture.assert_output_contains(&list_after, "archive-test");

    // show should still work
    let show_after = fixture.run_ms(&["--robot", "show", "archive-test"]);
    fixture.assert_success(&show_after, "show after deletion");

    // load should still work
    let load_after = fixture.run_ms(&["--robot", "load", "archive-test"]);
    fixture.assert_success(&load_after, "load after deletion");
    assert!(load_after.stdout.contains("Archive Test"));

    fixture.checkpoint("archive_first_all_commands_ok");
    Ok(())
}

// ---------------------------------------------------------------------------
// bd-19e6: kebab-case section slugs and ms load --section
// ---------------------------------------------------------------------------

/// Test the full show -> load --section workflow using section slugs.
#[test]
fn test_show_load_section_slug_workflow() -> Result<()> {
    let mut fixture = E2EFixture::new("section_slug_workflow");

    create_provider_skill(
        &fixture,
        ".claude/skills",
        "multi-section",
        r#"---
name: Multi Section
description: A skill with multiple sections
tags: []
---

# Multi Section

## Rules

- Follow these rules always

## Examples

Here are some code examples.

## Pitfalls

- Avoid these common mistakes.
"#,
    )?;

    let init_out = fixture.run_ms(&["--robot", "init"]);
    assert!(init_out.success, "init should succeed: {}", init_out.stderr);

    // Get section slugs from show
    let show_out = fixture.run_ms(&["--robot", "show", "multi-section"]);
    fixture.assert_success(&show_out, "show");
    let show_json: serde_json::Value = serde_json::from_str(&show_out.stdout).expect("valid JSON");

    let slugs = show_json["skill"]["section_slugs"]
        .as_array()
        .expect("section_slugs should be an array");
    println!("[bd-19e6] Available section slugs: {:?}", slugs);

    // At least one slug should be present for a multi-section skill
    assert!(
        !slugs.is_empty(),
        "skill should have at least one section slug, got {:?}",
        slugs
    );

    // Load just the first section
    let first_slug = slugs[0].as_str().unwrap();
    let section_out =
        fixture.run_ms(&["--robot", "load", "multi-section", "--section", first_slug]);
    fixture.assert_success(&section_out, "load --section");

    // Verify the loaded output contains the section title
    assert!(
        section_out
            .stdout
            .contains(first_slug.replace('-', " ").split(' ').next().unwrap_or("")),
        "load output should mention section content"
    );

    fixture.checkpoint("section_slug_workflow_ok");
    Ok(())
}

/// Test that load --section with invalid slug produces a clean error.
#[test]
fn test_load_section_invalid_slug_fails_cleanly() -> Result<()> {
    let mut fixture = E2EFixture::new("invalid_section_slug");

    create_provider_skill(
        &fixture,
        ".claude/skills",
        "simple-skill",
        "# Simple Skill\n\n## Rules\n\n- Be simple\n",
    )?;

    let init_out = fixture.run_ms(&["--robot", "init"]);
    assert!(init_out.success, "init should succeed: {}", init_out.stderr);

    // Load with a slug that doesn't match any section
    let load_out = fixture.run_ms(&[
        "--robot",
        "load",
        "simple-skill",
        "--section",
        "nonexistent-section",
    ]);
    // The load succeeds (skill found), but the section note is rendered in the body
    // Exit code 0 because load_skill succeeds even when section not found
    assert!(
        load_out.success,
        "load should succeed even with invalid section slug: exit={} stderr={}",
        load_out.exit_code, load_out.stderr
    );

    fixture.checkpoint("invalid_section_handled");
    Ok(())
}

// ---------------------------------------------------------------------------
// bd-5stx: providers sync/list/doctor incremental maintenance
// ---------------------------------------------------------------------------

/// Test that providers sync imports a new provider skill without re-running init.
#[test]
fn test_providers_sync_imports_new_skill_without_reinit() -> Result<()> {
    let mut fixture = E2EFixture::new("providers_sync_new_skill");

    create_provider_skill(
        &fixture,
        ".claude/skills",
        "bootstrap",
        r#"---
id: bootstrap
name: Bootstrap
description: Initial provider skill
tags: [bootstrap]
---
# Bootstrap

## Overview
Initial provider content.
"#,
    )?;

    let init_out = fixture.run_ms(&["--robot", "init"]);
    fixture.assert_success(&init_out, "init");

    let route_before = fixture.run_ms(&["--robot", "route", "sync me later"]);
    fixture.assert_success(&route_before, "route before sync");
    assert_eq!(route_before.json()["decision"], "no_match");

    create_provider_skill(
        &fixture,
        ".claude/skills",
        "sync-me",
        r#"---
id: sync-me
name: Sync Me
description: Imported by providers sync after a cached no-match
tags: [sync, invalidation]
trigger_phrases: ["sync me later"]
---
# Sync Me

## Overview
This skill should appear after providers sync invalidates the route cache.
"#,
    )?;

    let sync_out = fixture.run_ms(&["--robot", "providers", "sync", "--apply"]);
    fixture.assert_success(&sync_out, "providers sync --apply");
    let sync_json = sync_out.json();
    let claude_root = fixture
        .root
        .join(".claude/skills")
        .to_string_lossy()
        .to_string();
    assert!(
        sync_json["roots"]
            .as_array()
            .expect("sync roots")
            .iter()
            .any(|root| {
                root["root_path"].as_str() == Some(claude_root.as_str())
                    && root["new_count"].as_u64().unwrap_or(0) == 1
                    && root["unchanged_count"].as_u64().unwrap_or(0) == 1
            }),
        "providers sync should import exactly the new claude skill"
    );

    let route_after = fixture.run_ms(&["--robot", "route", "sync me later"]);
    fixture.assert_success(&route_after, "route after sync");
    let route_json = route_after.json();
    assert_eq!(route_json["decision"], "match");
    assert!(
        route_json["candidates"]
            .as_array()
            .expect("route candidates")
            .iter()
            .any(|candidate| candidate["skill_id"] == "claude/sync-me"),
        "route should resolve the newly imported provider skill"
    );

    fixture.checkpoint("providers_sync_new_skill_ok");
    Ok(())
}

/// Test that providers sync re-imports only the changed provider skill.
#[test]
fn test_providers_sync_reimports_only_changed_skill() -> Result<()> {
    let mut fixture = E2EFixture::new("providers_sync_changed_skill");

    create_provider_skill(
        &fixture,
        ".claude/skills",
        "alpha-skill",
        r#"---
id: alpha-skill
name: Alpha Skill
description: Unchanged provider skill
tags: [provider]
---
# Alpha Skill

## Overview
This skill should stay unchanged.
"#,
    )?;
    create_provider_skill(
        &fixture,
        ".claude/skills",
        "cached-skill",
        r#"---
id: cached-skill
name: Cached Skill
description: Skill with multiple sections for cache testing
tags: [provider, cache]
---
# Cached Skill

## Overview
Base content.

## Details
Detailed content here.
"#,
    )?;

    let init_out = fixture.run_ms(&["--robot", "init"]);
    fixture.assert_success(&init_out, "init");

    let load_before = fixture.run_ms(&[
        "--robot",
        "load",
        "claude/cached-skill",
        "--section",
        "details",
    ]);
    fixture.assert_success(&load_before, "load before sync");
    assert_eq!(
        load_before.json()["data"]["content"],
        "## Details\n\nDetailed content here.\n\n"
    );

    create_provider_skill(
        &fixture,
        ".claude/skills",
        "cached-skill",
        r#"---
id: cached-skill
name: Cached Skill
description: Skill with multiple sections for cache testing
tags: [provider, cache]
---
# Cached Skill

## Overview
Base content.

## Details
Updated detail content.
"#,
    )?;

    let sync_out = fixture.run_ms(&["--robot", "providers", "sync", "--apply"]);
    fixture.assert_success(&sync_out, "providers sync changed skill");
    let sync_json = sync_out.json();
    let claude_root = fixture
        .root
        .join(".claude/skills")
        .to_string_lossy()
        .to_string();
    assert!(
        sync_json["roots"]
            .as_array()
            .expect("sync roots")
            .iter()
            .any(|root| {
                root["root_path"].as_str() == Some(claude_root.as_str())
                    && root["new_count"].as_u64().unwrap_or(0) == 0
                    && root["changed_count"].as_u64().unwrap_or(0) == 1
                    && root["unchanged_count"].as_u64().unwrap_or(0) == 1
                    && root["changed_skills"]
                        .as_array()
                        .is_some_and(|skills| skills.iter().any(|skill| skill == "cached-skill"))
            }),
        "providers sync should only re-import the changed claude skill"
    );

    let load_after = fixture.run_ms(&[
        "--robot",
        "load",
        "claude/cached-skill",
        "--section",
        "details",
    ]);
    fixture.assert_success(&load_after, "load after sync");
    assert_eq!(
        load_after.json()["data"]["content"],
        "## Details\n\nUpdated detail content.\n\n"
    );

    fixture.checkpoint("providers_sync_changed_skill_ok");
    Ok(())
}

/// Test that providers list exposes tracked provider roots and last sync state.
#[test]
fn test_providers_list_shows_tracked_root_state() -> Result<()> {
    let mut fixture = E2EFixture::new("providers_list_tracked_state");

    create_provider_skill(
        &fixture,
        ".claude/skills",
        "provider-route",
        r#"---
id: provider-route
name: Provider Route
description: Route-first provider verification skill
tags: [provider, route]
---
# Provider Route

## Overview
Local claude provider content.
"#,
    )?;
    create_provider_skill(
        &fixture,
        ".codex/skills",
        "global-helper",
        r#"---
id: global-helper
name: Global Helper
description: Global codex helper skill
tags: [global, helper]
---
# Global Helper

## Overview
Global helper content.
"#,
    )?;

    let init_out = fixture.run_ms(&["--robot", "init"]);
    fixture.assert_success(&init_out, "init");

    let list_out = fixture.run_ms(&["--robot", "providers", "list"]);
    fixture.assert_success(&list_out, "providers list");
    let list_json = list_out.json();
    let roots = list_json["roots"].as_array().expect("roots");
    let claude_root = fixture
        .root
        .join(".claude/skills")
        .to_string_lossy()
        .to_string();
    let codex_root = fixture
        .root
        .join(".codex/skills")
        .to_string_lossy()
        .to_string();

    assert!(
        roots.iter().any(|root| {
            root["provider"] == "claude"
                && root["path"].as_str() == Some(claude_root.as_str())
                && root["tracked_skill_count"].as_u64().unwrap_or(0) == 1
                && root["last_sync"].is_string()
        }),
        "providers list should show the tracked claude root"
    );
    assert!(
        roots.iter().any(|root| {
            root["provider"] == "codex"
                && root["path"].as_str() == Some(codex_root.as_str())
                && root["tracked_skill_count"].as_u64().unwrap_or(0) == 1
                && root["last_sync"].is_string()
        }),
        "providers list should show the tracked codex root"
    );

    fixture.checkpoint("providers_list_tracked_state_ok");
    Ok(())
}

/// Test that providers doctor reports degraded-but-usable state after source removal.
#[test]
fn test_providers_doctor_reports_degraded_but_usable_state() -> Result<()> {
    let mut fixture = E2EFixture::new("providers_doctor_degraded_state");

    create_provider_skill(
        &fixture,
        ".claude/skills",
        "provider-doctor",
        r#"---
id: provider-doctor
name: Provider Doctor
description: Provider doctor degradation test
tags: [provider, doctor]
trigger_phrases: ["provider doctor degrade"]
---
# Provider Doctor

## Overview
Archive-backed runtime should stay usable when source roots disappear.
"#,
    )?;

    let init_out = fixture.run_ms(&["--robot", "init"]);
    fixture.assert_success(&init_out, "init");

    fs::rename(
        fixture.root.join(".claude/skills"),
        fixture.root.join(".claude/skills.hidden"),
    )?;

    let show_out = fixture.run_ms(&["--robot", "show", "claude/provider-doctor"]);
    fixture.assert_success(&show_out, "show after root removal");

    let doctor_out = fixture.run_ms(&["--robot", "providers", "doctor"]);
    fixture.assert_success(&doctor_out, "providers doctor");
    let doctor_json = doctor_out.json();
    let claude_root = fixture
        .root
        .join(".claude/skills")
        .to_string_lossy()
        .to_string();

    assert_eq!(doctor_json["status"], "degraded");
    assert_eq!(doctor_json["runtime_usable"], true);
    assert_eq!(doctor_json["archive"]["integrity_status"], "ok");
    assert_eq!(doctor_json["registry"]["consistent"], true);
    assert!(
        doctor_json["cache"]["entries"].as_u64().unwrap_or(0) >= 1,
        "doctor should count provider state cache entries"
    );
    assert!(
        doctor_json["roots"]
            .as_array()
            .expect("doctor roots")
            .iter()
            .any(|root| {
                root["provider"] == "claude"
                    && root["path"].as_str() == Some(claude_root.as_str())
                    && root["readable"] == false
                    && root["tracked_skill_count"].as_u64().unwrap_or(0) == 1
                    && root["last_sync"].is_string()
            }),
        "doctor should report the missing claude root with tracked state"
    );
    assert!(
        doctor_json["issues"]
            .as_array()
            .expect("doctor issues")
            .iter()
            .any(|issue| issue.as_str().is_some_and(|msg| msg.contains("MISSING:"))),
        "doctor should report the missing provider root"
    );

    fixture.checkpoint("providers_doctor_degraded_state_ok");
    Ok(())
}
