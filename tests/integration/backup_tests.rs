use crate::fixture::{TestFixture, TestSkill};

#[test]
#[cfg_attr(
    windows,
    ignore = "TODO(0.2.x): backup roundtrip relies on POSIX archive semantics; preexisting on v0.1.0"
)]
fn backup_create_and_restore_roundtrip() {
    let fixture = TestFixture::new("backup_roundtrip");
    let init = fixture.init();
    assert!(init.success, "init failed: {}", init.stderr);

    let original_config = std::fs::read_to_string(&fixture.config_path).expect("read config");

    let backup = fixture.run_ms(&["--robot", "backup", "create", "--id", "roundtrip"]);
    assert!(backup.success, "backup create failed: {}", backup.stderr);

    let backup_dir = fixture.ms_root.join("backups").join("roundtrip");
    assert!(backup_dir.exists(), "backup dir missing");

    std::fs::write(&fixture.config_path, "changed = true\n").expect("write config");

    let restore = fixture.run_ms(&["--robot", "backup", "restore", "roundtrip", "--approve"]);
    assert!(restore.success, "backup restore failed: {}", restore.stderr);

    let restored_config =
        std::fs::read_to_string(&fixture.config_path).expect("read restored config");
    assert_eq!(restored_config, original_config);
}

#[test]
fn backup_restore_missing_id_errors() {
    let fixture = TestFixture::new("backup_missing_id");
    let init = fixture.init();
    assert!(init.success, "init failed: {}", init.stderr);

    let restore = fixture.run_ms(&["--robot", "backup", "restore", "missing", "--approve"]);
    assert!(!restore.success, "restore should fail for missing backup");
    assert!(restore.stdout.contains("\"error\""));
}

#[test]
#[cfg_attr(
    windows,
    ignore = "TODO(0.2.x): backup roundtrip relies on POSIX archive semantics; preexisting on v0.1.0"
)]
fn backup_restore_replaces_archive_state() {
    let alpha = TestSkill::with_content(
        "alpha-skill",
        r#"---
name: Alpha Skill
description: First skill for backup testing
tags: [test, alpha]
---

# Alpha Skill

This is the original alpha skill.
"#,
    );
    let beta = TestSkill::with_content(
        "beta-skill",
        r#"---
name: Beta Skill
description: Second skill for backup testing
tags: [test, beta]
---

# Beta Skill

This is the beta skill.
"#,
    );

    let fixture = TestFixture::new("backup_restore_archive_state");
    let init = fixture.init();
    assert!(init.success, "init failed: {}", init.stderr);
    fixture.add_skill(&alpha);
    fixture.add_skill(&beta);

    let index = fixture.run_ms(&["--robot", "index"]);
    assert!(index.success, "index failed: {}", index.stderr);

    let backup = fixture.run_ms(&["--robot", "backup", "create", "--id", "original"]);
    assert!(backup.success, "backup create failed: {}", backup.stderr);

    let modified = r#"---
name: Alpha Skill Modified
description: Modified version of alpha skill
tags: [test, alpha, modified]
---

# Alpha Skill Modified

This is the MODIFIED alpha skill.
"#;
    std::fs::write(
        fixture.skills_dir.join("alpha-skill").join("SKILL.md"),
        modified,
    )
    .expect("write modified skill");

    let reindex = fixture.run_ms(&["--robot", "index"]);
    assert!(reindex.success, "reindex failed: {}", reindex.stderr);

    let search_before = fixture.run_ms(&["--robot", "search", "modified"]);
    assert!(
        search_before.success,
        "search before restore failed: {}",
        search_before.stderr
    );
    assert_eq!(search_before.json()["count"].as_u64(), Some(1));

    let restore = fixture.run_ms(&["--robot", "backup", "restore", "original", "--approve"]);
    assert!(restore.success, "backup restore failed: {}", restore.stderr);

    let search_after = fixture.run_ms(&["--robot", "search", "modified"]);
    assert!(
        search_after.success,
        "search after restore failed: {}",
        search_after.stderr
    );
    assert_eq!(search_after.json()["count"].as_u64(), Some(0));
}
