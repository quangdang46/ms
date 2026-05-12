use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use rusqlite::Connection;
use tempfile::TempDir;

fn cass_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cass")
}

fn copy_cass_fixture(relative_path: &str, destination: &Path) -> std::io::Result<()> {
    let source = cass_fixture_root().join(relative_path);
    std::fs::copy(&source, destination).map(|_| ())
}

fn copy_dir_contents(source: &Path, destination: &Path) -> std::io::Result<()> {
    if !source.exists() {
        return Ok(());
    }

    std::fs::create_dir_all(destination)?;

    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());

        if source_path.is_dir() {
            copy_dir_contents(&source_path, &destination_path)?;
        } else {
            if let Some(parent) = destination_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&source_path, &destination_path)?;
        }
    }

    Ok(())
}

// =============================================================================
// Assertion Macros
// =============================================================================

/// Assert that a file exists at the given path
#[macro_export]
macro_rules! assert_file_exists {
    ($path:expr) => {
        assert!(
            std::path::Path::new($path).exists(),
            "Expected file to exist: {:?}",
            $path
        );
    };
    ($path:expr, $msg:expr) => {
        assert!(std::path::Path::new($path).exists(), $msg);
    };
}

/// Assert that a file contains expected content
#[macro_export]
macro_rules! assert_file_contains {
    ($path:expr, $expected:expr) => {{
        let content = std::fs::read_to_string($path).expect(&format!("Failed to read {:?}", $path));
        assert!(
            content.contains($expected),
            "File {:?} does not contain '{}'\nActual content:\n{}",
            $path,
            $expected,
            &content[..std::cmp::min(content.len(), 500)]
        );
    }};
}

/// Assert that a JSON file matches expected structure
#[macro_export]
macro_rules! assert_json_matches {
    ($path:expr, $expected:expr) => {{
        let content = std::fs::read_to_string($path).expect(&format!("Failed to read {:?}", $path));
        let actual: serde_json::Value =
            serde_json::from_str(&content).expect(&format!("Invalid JSON in {:?}", $path));
        let expected: serde_json::Value = serde_json::json!($expected);
        assert_eq!(
            actual, expected,
            "JSON mismatch in {:?}\nActual: {}\nExpected: {}",
            $path, actual, expected
        );
    }};
}

/// Assert command exit code
#[macro_export]
macro_rules! assert_exit_code {
    ($output:expr, $code:expr) => {
        assert_eq!(
            $output.exit_code, $code,
            "Expected exit code {} but got {}\nstdout: {}\nstderr: {}",
            $code, $output.exit_code, $output.stdout, $output.stderr
        );
    };
}

/// Assert stdout contains expected text
#[macro_export]
macro_rules! assert_stdout_contains {
    ($output:expr, $expected:expr) => {
        assert!(
            $output.stdout.contains($expected),
            "stdout does not contain '{}'\nActual stdout:\n{}",
            $expected,
            $output.stdout
        );
    };
}

/// Assert stderr contains expected text
#[macro_export]
macro_rules! assert_stderr_contains {
    ($output:expr, $expected:expr) => {
        assert!(
            $output.stderr.contains($expected),
            "stderr does not contain '{}'\nActual stderr:\n{}",
            $expected,
            $output.stderr
        );
    };
}

/// Assert command succeeded
#[macro_export]
macro_rules! assert_command_success {
    ($output:expr) => {
        assert!(
            $output.success,
            "Command failed with exit code {}\nstdout: {}\nstderr: {}",
            $output.exit_code, $output.stdout, $output.stderr
        );
    };
    ($output:expr, $msg:expr) => {
        assert!(
            $output.success,
            "{}: exit code {}\nstdout: {}\nstderr: {}",
            $msg, $output.exit_code, $output.stdout, $output.stderr
        );
    };
}

/// Integration test fixture providing isolated environment
#[allow(dead_code)]
pub struct TestFixture {
    /// Root temp directory
    pub temp_dir: TempDir,
    /// Project root (`temp_dir` path)
    pub root: PathBuf,
    /// ms root directory (./.ms)
    pub ms_root: PathBuf,
    /// Config file path
    pub config_path: PathBuf,
    /// Skills directory (project-local ./skills)
    pub skills_dir: PathBuf,
    /// Search index path
    pub index_path: PathBuf,
    /// Database connection for state verification
    pub db: Option<Connection>,
    /// Test start time for timing
    start_time: std::time::Instant,
    /// Test name for logging
    test_name: String,
    /// Number of attempts for command retries on infra-style failures.
    retry_attempts: u32,
}

#[allow(dead_code)]
impl TestFixture {
    /// Create a fresh test fixture
    pub fn new(test_name: &str) -> Self {
        let start_time = std::time::Instant::now();
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let root = temp_dir.path().to_path_buf();
        let ms_root = root.join(".ms");
        let config_path = ms_root.join("config.toml");
        let skills_dir = root.join("skills");
        let index_path = ms_root.join("index");

        std::fs::create_dir_all(&skills_dir).expect("Failed to create skills dir");

        println!("\n{}", "=".repeat(70));
        println!("[FIXTURE] Test: {test_name}");
        println!("[FIXTURE] Root: {root:?}");
        println!("[FIXTURE] MS Root: {ms_root:?}");
        println!("[FIXTURE] Config: {config_path:?}");
        println!("[FIXTURE] Skills: {skills_dir:?}");
        println!("[FIXTURE] Index: {index_path:?}");
        println!("{}", "=".repeat(70));

        Self {
            temp_dir,
            root,
            ms_root,
            config_path,
            skills_dir,
            index_path,
            db: None,
            start_time,
            test_name: test_name.to_string(),
            retry_attempts: 1,
        }
    }

    /// Create fixture with pre-indexed skills
    pub fn with_indexed_skills(test_name: &str, skills: &[TestSkill]) -> Self {
        let mut fixture = Self::new(test_name);
        let init = fixture.init();
        assert!(init.success, "init failed: {}", init.stderr);

        for skill in skills {
            fixture.add_skill(skill);
        }

        let output = fixture.run_ms(&["--robot", "index"]);
        assert!(output.success, "Failed to index skills: {}", output.stderr);

        fixture.open_db();
        fixture
    }

    /// Create fixture with mock CASS integration
    pub fn with_mock_cass(test_name: &str) -> Self {
        let fixture = Self::new(test_name);

        let cass_dir = fixture.root.join("mock_cass");
        let sessions_dir = cass_dir.join("sessions");
        let extractions_dir = cass_dir.join("extractions");
        std::fs::create_dir_all(&sessions_dir).expect("Failed to create CASS sessions dir");
        std::fs::create_dir_all(&extractions_dir).expect("Failed to create CASS extractions dir");

        for session_name in [
            "session-001.jsonl",
            "session-002.jsonl",
            "session-003.jsonl",
        ] {
            copy_cass_fixture(
                &format!("sessions/{session_name}"),
                &sessions_dir.join(session_name),
            )
            .expect("Failed to copy CASS session fixture");
        }
        copy_cass_fixture(
            "extractions/debugging-skill.json",
            &extractions_dir.join("debugging-skill.json"),
        )
        .expect("Failed to copy CASS extraction fixture");

        println!("[FIXTURE] Fixture-backed CASS configured at: {cass_dir:?}");

        fixture
    }

    /// Create fixture with pre-populated sample bundles
    pub fn with_sample_bundles(test_name: &str) -> Self {
        let fixture = Self::new(test_name);
        let init = fixture.init();
        assert!(init.success, "init failed: {}", init.stderr);

        // Add sample bundles
        fixture.add_bundle(&sample_bundles::rust_patterns());
        fixture.add_bundle(&sample_bundles::testing_patterns());

        println!("[FIXTURE] Sample bundles added");

        fixture
    }

    /// Create fixture with pre-populated sample skills
    pub fn with_sample_skills(test_name: &str) -> Self {
        let mut fixture = Self::new(test_name);
        let init = fixture.init();
        assert!(init.success, "init failed: {}", init.stderr);

        // Add all sample skills from the sample_skills module
        for skill in sample_skills::all() {
            fixture.add_skill(&skill);
        }

        // Index everything
        let output = fixture.run_ms(&["--robot", "index"]);
        assert!(output.success, "Failed to index skills: {}", output.stderr);

        fixture.open_db();
        println!("[FIXTURE] Sample skills setup complete");

        fixture
    }

    /// Create fixture with both skills and bundles
    pub fn with_full_setup(test_name: &str) -> Self {
        let mut fixture = Self::new(test_name);
        let init = fixture.init();
        assert!(init.success, "init failed: {}", init.stderr);

        // Add all sample skills from the sample_skills module
        for skill in sample_skills::all() {
            fixture.add_skill(&skill);
        }

        // Add sample bundles from the sample_bundles module
        fixture.add_bundle(&sample_bundles::rust_patterns());
        fixture.add_bundle(&sample_bundles::testing_patterns());

        // Index everything
        let output = fixture.run_ms(&["--robot", "index"]);
        assert!(output.success, "Failed to index: {}", output.stderr);

        fixture.open_db();
        println!("[FIXTURE] Full setup complete");

        fixture
    }

    fn clone_with_suffix(&self, suffix: &str) -> Self {
        let mut clone = Self::new(&format!("{}-{suffix}", self.test_name));

        copy_dir_contents(&self.skills_dir, &clone.skills_dir)
            .expect("Failed to copy skills into cloned fixture");

        let source_bundles = self.root.join("bundles");
        let destination_bundles = clone.root.join("bundles");
        copy_dir_contents(&source_bundles, &destination_bundles)
            .expect("Failed to copy bundles into cloned fixture");

        let source_mock_cass = self.root.join("mock_cass");
        let destination_mock_cass = clone.root.join("mock_cass");
        copy_dir_contents(&source_mock_cass, &destination_mock_cass)
            .expect("Failed to copy mock CASS data into cloned fixture");

        if self.config_path.exists() {
            let init = clone.init();
            assert!(
                init.success,
                "Failed to init cloned fixture: {}",
                init.stderr
            );

            if self.db_path().exists() || self.index_path.exists() {
                let index = clone.run_ms(&["--robot", "index"]);
                assert!(
                    index.success,
                    "Failed to re-index cloned fixture: {}",
                    index.stderr
                );
                clone.open_db();
            }
        }

        clone.retry_attempts = self.retry_attempts;
        clone
    }

    /// Rebuild this fixture into a fresh temp root with the same inputs and indexed state.
    pub fn with_isolated_env(&self) -> Self {
        self.clone_with_suffix("isolated")
    }

    /// Rebuild this fixture with a custom retry budget for command execution.
    pub fn with_retry(&self, max_attempts: u32) -> Self {
        let mut clone = self.clone_with_suffix("retry");
        clone.retry_attempts = max_attempts.max(1);
        clone
    }

    /// Verify temporary fixture cleanup by dropping a cloned isolated environment.
    pub fn verify_cleanup(&self) -> anyhow::Result<()> {
        let clone = self.with_isolated_env();
        let clone_root = clone.root.clone();
        drop(clone);

        if clone_root.exists() {
            return Err(anyhow::anyhow!(
                "isolated fixture root still exists after drop: {}",
                clone_root.display()
            ));
        }

        Ok(())
    }

    /// Count rows in common fixture tables for concise database assertions.
    pub fn db_row_count(&self, table: &str) -> Option<i64> {
        let sql = match table {
            "skills" => "SELECT COUNT(*) FROM skills",
            "skills_fts" => "SELECT COUNT(*) FROM skills_fts",
            "skill_aliases" => "SELECT COUNT(*) FROM skill_aliases",
            _ => return None,
        };

        self.db
            .as_ref()
            .and_then(|db| db.query_row::<i64, _, _>(sql, [], |row| row.get(0)).ok())
    }

    /// Compare two test skills and summarize the meaningful differences.
    pub fn diff_skills(&self, a: &TestSkill, b: &TestSkill) -> SkillDiff {
        a.diff(b)
    }

    /// Build a ready Rust-oriented fixture with repo signals and indexed sample skills.
    pub fn factory_rust_project() -> Self {
        let fixture = Self::with_sample_skills("factory_rust_project");

        std::fs::create_dir_all(fixture.root.join("src"))
            .expect("Failed to create Rust src directory");
        std::fs::write(
            fixture.root.join("Cargo.toml"),
            "[package]\nname = \"fixture-rust-project\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .expect("Failed to write Cargo.toml");
        std::fs::write(fixture.root.join("src/main.rs"), "fn main() {}\n")
            .expect("Failed to write Rust main.rs");

        fixture
    }

    /// Build a ready Node-oriented fixture with repo signals and indexed sample skills.
    pub fn factory_node_project() -> Self {
        let mut fixture = Self::new("factory_node_project");
        let init = fixture.init();
        assert!(init.success, "init failed: {}", init.stderr);

        std::fs::create_dir_all(fixture.root.join("src"))
            .expect("Failed to create Node src directory");
        std::fs::write(
            fixture.root.join("package.json"),
            "{\n  \"name\": \"fixture-node-project\",\n  \"version\": \"1.0.0\",\n  \"type\": \"module\"\n}\n",
        )
        .expect("Failed to write package.json");
        std::fs::write(
            fixture.root.join("src/index.ts"),
            "export function main(): void {\n  console.log('fixture-node-project');\n}\n",
        )
        .expect("Failed to write index.ts");

        fixture.add_skill(&TestSkill::with_content(
            "node-testing",
            "# Node Testing\n\nUse a fast Node.js test runner and keep I/O isolated.\n",
        ));
        let index = fixture.run_ms(&["--robot", "index"]);
        assert!(
            index.success,
            "Failed to index Node fixture: {}",
            index.stderr
        );
        fixture.open_db();

        fixture
    }

    /// Add a skill to the test environment
    pub fn add_skill(&self, skill: &TestSkill) {
        let skill_dir = self.skills_dir.join(&skill.name);
        std::fs::create_dir_all(&skill_dir).expect("Failed to create skill dir");

        let skill_file = skill_dir.join("SKILL.md");
        std::fs::write(&skill_file, skill.to_markdown()).expect("Failed to write skill");

        println!(
            "[FIXTURE] Added skill: {} ({} bytes)",
            skill.name,
            skill.content.len()
        );
    }

    /// Run ms CLI command and capture output
    pub fn run_ms(&self, args: &[&str]) -> CommandOutput {
        self.run_ms_with_timeout(args, Duration::from_secs(30))
    }

    /// Run ms CLI command with custom timeout
    pub fn run_ms_with_timeout(&self, args: &[&str], timeout: Duration) -> CommandOutput {
        let attempts = self.retry_attempts.max(1);

        for attempt in 1..=attempts {
            let output = self.run_ms_once(args, timeout);
            let retriable = !output.success
                && output.exit_code == -1
                && (output.stderr.contains("Command timed out")
                    || output.stderr.contains("Failed to spawn")
                    || output.stderr.contains("Error waiting"));

            if !retriable || attempt == attempts {
                return output;
            }

            println!(
                "[RETRY] Attempt {attempt}/{attempts} failed with retriable infra error, retrying..."
            );
            std::thread::sleep(Duration::from_millis(100 * u64::from(attempt)));
        }

        unreachable!("run_ms_with_timeout should always return from the retry loop")
    }

    fn run_ms_once(&self, args: &[&str], timeout: Duration) -> CommandOutput {
        use std::io::Read;
        use std::process::Stdio;

        let start = std::time::Instant::now();
        println!("\n[CMD] ms {} (timeout: {:?})", args.join(" "), timeout);

        let mut child = Command::new(env!("CARGO_BIN_EXE_ms"))
            .args(args)
            .env("HOME", &self.root)
            .env("MS_ROOT", &self.ms_root)
            .env("MS_CONFIG", &self.config_path)
            .current_dir(&self.root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("Failed to spawn ms command");

        // Wait with timeout
        let result: Result<std::process::ExitStatus, String> = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => {
                    if start.elapsed() > timeout {
                        let _ = child.kill();
                        break Err("Command timed out".to_string());
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => break Err(format!("Error waiting: {e}")),
            }
        };

        let elapsed = start.elapsed();

        let (success, exit_code, stdout, stderr) = match result {
            Ok(status) => {
                let mut stdout_str = String::new();
                let mut stderr_str = String::new();
                if let Some(mut out) = child.stdout.take() {
                    let _ = out.read_to_string(&mut stdout_str);
                }
                if let Some(mut err) = child.stderr.take() {
                    let _ = err.read_to_string(&mut stderr_str);
                }
                (
                    status.success(),
                    status.code().unwrap_or(-1),
                    stdout_str,
                    stderr_str,
                )
            }
            Err(msg) => (false, -1, String::new(), msg),
        };

        println!("[CMD] Exit code: {exit_code}");
        println!("[CMD] Timing: {elapsed:?}");

        // Warn about slow operations (threshold: 5 seconds)
        const SLOW_THRESHOLD: Duration = Duration::from_secs(5);
        if elapsed > SLOW_THRESHOLD {
            println!("[SLOW] ⚠ Command took {elapsed:?} (threshold: {SLOW_THRESHOLD:?})");
        }

        if !stdout.is_empty() {
            println!("[STDOUT]\n{stdout}");
        }
        if !stderr.is_empty() {
            println!("[STDERR]\n{stderr}");
        }

        CommandOutput {
            success,
            exit_code,
            stdout,
            stderr,
            elapsed,
        }
    }

    /// Run ms CLI command with environment variables
    pub fn run_ms_with_env(&self, args: &[&str], env_vars: &[(&str, &str)]) -> CommandOutput {
        let start = std::time::Instant::now();
        println!(
            "\n[CMD] ms {} (with {} env vars)",
            args.join(" "),
            env_vars.len()
        );

        let mut cmd = Command::new(env!("CARGO_BIN_EXE_ms"));
        cmd.args(args)
            .env("HOME", &self.root)
            .env("MS_ROOT", &self.ms_root)
            .env("MS_CONFIG", &self.config_path)
            .current_dir(&self.root);

        for (key, value) in env_vars {
            cmd.env(key, value);
        }

        let output = cmd.output().expect("Failed to execute ms command");

        let elapsed = start.elapsed();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        println!("[CMD] Exit code: {}", output.status.code().unwrap_or(-1));
        println!("[CMD] Timing: {elapsed:?}");

        // Warn about slow operations (threshold: 5 seconds)
        const SLOW_THRESHOLD: Duration = Duration::from_secs(5);
        if elapsed > SLOW_THRESHOLD {
            println!("[SLOW] ⚠ Command took {elapsed:?} (threshold: {SLOW_THRESHOLD:?})");
        }

        if !stdout.is_empty() {
            println!("[STDOUT]\n{stdout}");
        }
        if !stderr.is_empty() {
            println!("[STDERR]\n{stderr}");
        }

        CommandOutput {
            success: output.status.success(),
            exit_code: output.status.code().unwrap_or(-1),
            stdout,
            stderr,
            elapsed,
        }
    }

    pub fn init(&self) -> CommandOutput {
        self.run_ms(&["--robot", "init"])
    }

    /// Add a bundle to the test environment
    pub fn add_bundle(&self, bundle: &TestBundle) {
        let bundle_dir = self.root.join("bundles").join(&bundle.name);
        std::fs::create_dir_all(&bundle_dir).expect("Failed to create bundle dir");

        // Write manifest
        let manifest_path = bundle_dir.join("bundle.json");
        std::fs::write(&manifest_path, &bundle.manifest).expect("Failed to write bundle manifest");

        // Write skills if any
        for (skill_name, skill_content) in &bundle.skills {
            let skill_dir = bundle_dir.join("skills").join(skill_name);
            std::fs::create_dir_all(&skill_dir).expect("Failed to create bundle skill dir");
            let skill_file = skill_dir.join("SKILL.md");
            std::fs::write(&skill_file, skill_content).expect("Failed to write bundle skill");
        }

        println!(
            "[FIXTURE] Added bundle: {} ({} skills)",
            bundle.name,
            bundle.skills.len()
        );
    }

    pub fn db_path(&self) -> PathBuf {
        self.ms_root.join("ms.db")
    }

    /// Verify database state
    pub fn verify_db_state(&self, check: impl FnOnce(&Connection) -> bool, description: &str) {
        if let Some(ref db) = self.db {
            let db_state = self.dump_db_state(db);
            println!("[DB STATE] {db_state}");

            let result = check(db);
            assert!(result, "Database state check failed: {description}");

            println!("[DB CHECK] {description} - PASSED");
        } else {
            println!("[DB CHECK] Skipped (no database connection): {description}");
        }
    }

    pub fn open_db(&mut self) {
        let db_path = self.ms_root.join("ms.db");
        if db_path.exists() {
            self.db = Some(Connection::open(&db_path).expect("Failed to open db"));
            println!("[FIXTURE] Database opened: {db_path:?}");
        }
    }

    /// Dump database state for logging
    fn dump_db_state(&self, db: &Connection) -> String {
        let mut state = String::new();

        if let Ok(count) =
            db.query_row::<i64, _, _>("SELECT COUNT(*) FROM skills", [], |r| r.get(0))
        {
            state.push_str(&format!("skills={count} "));
        }
        if let Ok(count) =
            db.query_row::<i64, _, _>("SELECT COUNT(*) FROM skills_fts", [], |r| r.get(0))
        {
            state.push_str(&format!("fts={count} "));
        }

        state
    }

    /// Dump directory tree for debugging
    pub fn dump_directory_tree(&self) -> String {
        self.dump_directory_tree_at(&self.root)
    }

    /// Dump directory tree starting from a specific path
    pub fn dump_directory_tree_at(&self, path: &Path) -> String {
        let mut output = String::new();
        self.build_tree(&mut output, path, "", true);
        output
    }

    fn build_tree(&self, output: &mut String, path: &Path, prefix: &str, is_last: bool) {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let connector = if is_last { "└── " } else { "├── " };

        if prefix.is_empty() {
            output.push_str(&format!("{name}\n"));
        } else {
            output.push_str(&format!("{prefix}{connector}{name}\n"));
        }

        if path.is_dir() {
            let mut entries: Vec<_> = std::fs::read_dir(path)
                .ok()
                .map(|rd| rd.filter_map(std::result::Result::ok).collect())
                .unwrap_or_default();
            entries.sort_by_key(std::fs::DirEntry::file_name);

            let new_prefix = if prefix.is_empty() {
                String::new()
            } else if is_last {
                format!("{prefix}    ")
            } else {
                format!("{prefix}│   ")
            };

            for (i, entry) in entries.iter().enumerate() {
                let is_last_entry = i == entries.len() - 1;
                self.build_tree(output, &entry.path(), &new_prefix, is_last_entry);
            }
        }
    }

    /// Dump index state for debugging
    pub fn dump_index_state(&self) -> String {
        let index_path = &self.index_path;
        let mut state = String::new();

        state.push_str(&format!("Index path: {index_path:?}\n"));

        if !index_path.exists() {
            state.push_str("  Index directory does not exist\n");
            return state;
        }

        // Count files in index directory
        let file_count = std::fs::read_dir(index_path)
            .ok()
            .map_or(0, |rd| rd.filter_map(std::result::Result::ok).count());

        state.push_str(&format!("  Files: {file_count}\n"));

        // Get total size
        let total_size: u64 = walkdir::WalkDir::new(index_path)
            .into_iter()
            .filter_map(std::result::Result::ok)
            .filter_map(|e| e.metadata().ok())
            .filter(std::fs::Metadata::is_file)
            .map(|m| m.len())
            .sum();

        state.push_str(&format!("  Total size: {total_size} bytes\n"));

        // List index files
        state.push_str("  Contents:\n");
        if let Ok(entries) = std::fs::read_dir(index_path) {
            for entry in entries.filter_map(std::result::Result::ok) {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                state.push_str(&format!(
                    "    {} ({} bytes)\n",
                    entry.file_name().to_string_lossy(),
                    size
                ));
            }
        }

        state
    }

    /// Get elapsed time since fixture creation
    pub fn elapsed(&self) -> std::time::Duration {
        self.start_time.elapsed()
    }

    /// Log timing information
    pub fn log_timing(&self, operation: &str) {
        println!("[TIMING] {} completed at {:?}", operation, self.elapsed());
    }
}

impl Drop for TestFixture {
    fn drop(&mut self) {
        let elapsed = self.start_time.elapsed();
        println!("\n{}", "=".repeat(70));
        println!("[FIXTURE] Test complete: {}", self.test_name);
        println!("[FIXTURE] Total time: {elapsed:?}");
        println!("[FIXTURE] Cleaning up: {:?}", self.temp_dir.path());
        println!("{}\n", "=".repeat(70));
    }
}

/// Test skill definition
pub struct TestSkill {
    pub name: String,
    pub content: String,
    pub tags: Vec<String>,
    pub layer: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillDiff {
    pub name_changed: bool,
    pub content_changed: bool,
    pub tags_added: Vec<String>,
    pub tags_removed: Vec<String>,
    pub layer_changed: bool,
}

impl SkillDiff {
    pub fn has_changes(&self) -> bool {
        self.name_changed
            || self.content_changed
            || self.layer_changed
            || !self.tags_added.is_empty()
            || !self.tags_removed.is_empty()
    }
}

impl TestSkill {
    pub fn new(name: &str, description: &str) -> Self {
        let content = format!("# {name}\n\n{description}\n\n## Overview\n\n{description}\n");

        Self {
            name: name.to_string(),
            content,
            tags: Vec::new(),
            layer: None,
        }
    }

    pub fn with_content(name: &str, content: &str) -> Self {
        Self {
            name: name.to_string(),
            content: content.to_string(),
            tags: Vec::new(),
            layer: None,
        }
    }

    #[allow(dead_code)]
    pub fn with_tags(mut self, tags: Vec<&str>) -> Self {
        self.tags = tags.into_iter().map(String::from).collect();
        self
    }

    #[allow(dead_code)]
    pub fn with_layer(mut self, layer: &str) -> Self {
        self.layer = Some(layer.to_string());
        self
    }

    pub fn to_markdown(&self) -> String {
        let mut md = String::new();

        // Add frontmatter if we have metadata
        if !self.tags.is_empty() || self.layer.is_some() {
            md.push_str("---\n");
            md.push_str(&format!("name: {}\n", self.name));
            if !self.tags.is_empty() {
                md.push_str(&format!("tags: [{}]\n", self.tags.join(", ")));
            }
            // Note: layer is usually determined by directory structure in ms,
            // but for tests we might want to simulate it or put it in frontmatter
            // if ms supports overriding it via frontmatter (which it generally doesn't for security/structure reasons,
            // but let's check. Actually ms determines layer by path).
            //
            // However, we can still put it in metadata for 'ms list' to pick up if it parses it?
            // Re-reading list.rs: it filters by s.source_layer.
            // s.source_layer comes from where the file was found.
            // So to test layer filtering, we need to place the file in the correct directory structure.
            md.push_str("---\n\n");
        }

        md.push_str(&self.content);
        md
    }

    pub fn diff(&self, other: &Self) -> SkillDiff {
        let self_tags: std::collections::BTreeSet<_> = self.tags.iter().cloned().collect();
        let other_tags: std::collections::BTreeSet<_> = other.tags.iter().cloned().collect();

        SkillDiff {
            name_changed: self.name != other.name,
            content_changed: self.content != other.content,
            tags_added: other_tags.difference(&self_tags).cloned().collect(),
            tags_removed: self_tags.difference(&other_tags).cloned().collect(),
            layer_changed: self.layer != other.layer,
        }
    }
}

/// Test bundle definition
#[allow(dead_code)]
pub struct TestBundle {
    pub name: String,
    pub manifest: String,
    pub skills: Vec<(String, String)>,
}

#[allow(dead_code)]
impl TestBundle {
    /// Create a minimal test bundle
    pub fn new(name: &str, description: &str) -> Self {
        let manifest = format!(
            r#"{{
  "name": "{name}",
  "version": "1.0.0",
  "description": "{description}",
  "skills": []
}}"#
        );

        Self {
            name: name.to_string(),
            manifest,
            skills: Vec::new(),
        }
    }

    /// Create a bundle with skills
    pub fn with_skills(name: &str, description: &str, skills: Vec<(&str, &str)>) -> Self {
        let skill_names: Vec<_> = skills.iter().map(|(n, _)| format!(r#""{n}""#)).collect();
        let manifest = format!(
            r#"{{
  "name": "{}",
  "version": "1.0.0",
  "description": "{}",
  "skills": [{}]
}}"#,
            name,
            description,
            skill_names.join(", ")
        );

        Self {
            name: name.to_string(),
            manifest,
            skills: skills
                .into_iter()
                .map(|(n, c)| (n.to_string(), c.to_string()))
                .collect(),
        }
    }

    /// Add a skill to this bundle
    pub fn add_skill(mut self, name: &str, content: &str) -> Self {
        self.skills.push((name.to_string(), content.to_string()));
        self
    }
}

/// Sample bundles for testing
#[allow(dead_code)]
pub mod sample_bundles {
    use super::TestBundle;

    /// A bundle for Rust development patterns
    pub fn rust_patterns() -> TestBundle {
        TestBundle::with_skills(
            "rust-patterns",
            "Common Rust development patterns",
            vec![
                (
                    "error-handling",
                    "# Error Handling\n\nPatterns for Rust error handling with Result and ?",
                ),
                (
                    "async-patterns",
                    "# Async Patterns\n\nAsync/await patterns for Tokio and async-std",
                ),
            ],
        )
    }

    /// A bundle for testing patterns
    pub fn testing_patterns() -> TestBundle {
        TestBundle::with_skills(
            "testing-patterns",
            "Testing patterns and best practices",
            vec![(
                "unit-testing",
                "# Unit Testing\n\nUnit testing patterns with #[test] and assertions",
            )],
        )
    }

    /// An empty bundle for edge case testing
    pub fn empty_bundle() -> TestBundle {
        TestBundle::new("empty-bundle", "An empty bundle for testing")
    }
}

/// Sample skills for testing
#[allow(dead_code)]
pub mod sample_skills {
    use super::TestSkill;

    /// A comprehensive Rust error handling skill
    pub fn rust_error_handling() -> TestSkill {
        TestSkill::with_content(
            "rust-error-handling",
            r"# Rust Error Handling

Use Result<T, E> for recoverable errors and panic! for unrecoverable ones.

## Overview

Rust's error handling is based on the Result type for operations that can fail.

## Rules

- Use `?` operator to propagate errors
- Define custom error types with thiserror
- Use anyhow for application errors
- Reserve panic! for programming errors

## Examples

```rust
fn read_file(path: &str) -> Result<String, std::io::Error> {
    std::fs::read_to_string(path)
}
```
",
        )
    }

    /// A Git workflow skill
    pub fn git_workflow() -> TestSkill {
        TestSkill::with_content(
            "git-workflow",
            r"# Git Workflow

Standard Git workflow patterns for feature development.

## Overview

Use feature branches with meaningful commit messages.

## Rules

- Create feature branches from main
- Write descriptive commit messages
- Squash commits before merging
- Use conventional commit format
",
        )
    }

    /// A testing best practices skill
    pub fn testing_best_practices() -> TestSkill {
        TestSkill::with_content(
            "testing-best-practices",
            r"# Testing Best Practices

Guidelines for writing effective tests.

## Overview

Good tests are fast, isolated, and readable.

## Rules

- Test one thing per test
- Use descriptive test names
- Arrange-Act-Assert pattern
- Mock external dependencies
",
        )
    }

    /// A minimal skill for edge case testing
    pub fn minimal() -> TestSkill {
        TestSkill::new("minimal-skill", "A minimal skill for testing")
    }

    /// All sample skills as a vector
    pub fn all() -> Vec<TestSkill> {
        vec![
            rust_error_handling(),
            git_workflow(),
            testing_best_practices(),
        ]
    }
}

/// Command output structure
#[allow(dead_code)]
pub struct CommandOutput {
    pub success: bool,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub elapsed: std::time::Duration,
}

#[allow(dead_code)]
impl CommandOutput {
    /// Check if stdout contains expected text
    pub fn stdout_contains(&self, expected: &str) -> bool {
        self.stdout.contains(expected)
    }

    /// Check if stderr contains expected text
    pub fn stderr_contains(&self, expected: &str) -> bool {
        self.stderr.contains(expected)
    }

    /// Parse stdout as JSON
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.stdout).expect("stdout should be valid JSON")
    }

    /// Try to parse stdout as JSON
    pub fn try_json(&self) -> Option<serde_json::Value> {
        serde_json::from_str(&self.stdout).ok()
    }

    /// Assert this command succeeded
    pub fn assert_success(&self) {
        assert!(
            self.success,
            "Expected success but got exit code {}\nstdout: {}\nstderr: {}",
            self.exit_code, self.stdout, self.stderr
        );
    }

    /// Assert this command failed
    pub fn assert_failure(&self) {
        assert!(
            !self.success,
            "Expected failure but command succeeded\nstdout: {}\nstderr: {}",
            self.stdout, self.stderr
        );
    }

    /// Assert exit code
    pub fn assert_exit_code(&self, expected: i32) {
        assert_eq!(
            self.exit_code, expected,
            "Expected exit code {} but got {}\nstdout: {}\nstderr: {}",
            expected, self.exit_code, self.stdout, self.stderr
        );
    }
}

#[allow(dead_code)]
fn ensure_parent(path: &Path) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
}

#[cfg(test)]
mod enhancement_tests {
    use super::{SkillDiff, TestFixture, TestSkill, sample_skills};

    #[test]
    fn test_with_isolated_env_preserves_indexed_state() {
        let fixture = TestFixture::with_sample_skills("with_isolated_env_preserves_indexed_state");
        let isolated = fixture.with_isolated_env();

        assert_ne!(fixture.root, isolated.root);
        assert!(isolated.config_path.exists());
        assert_eq!(isolated.db_row_count("skills"), Some(3));
        assert!(
            isolated
                .skills_dir
                .join("rust-error-handling")
                .join("SKILL.md")
                .exists(),
            "isolated fixture should copy project skills"
        );
    }

    #[test]
    fn test_with_retry_sets_retry_budget_on_isolated_clone() {
        let fixture = TestFixture::with_sample_skills("with_retry_sets_retry_budget");
        let retried = fixture.with_retry(3);

        assert_ne!(fixture.root, retried.root);
        assert_eq!(retried.retry_attempts, 3);
        assert_eq!(retried.db_row_count("skills"), Some(3));
    }

    #[test]
    #[cfg_attr(
        windows,
        ignore = "TODO(0.2.x): TempDir cleanup races with file-handle release on Windows; preexisting on v0.1.0"
    )]
    fn test_verify_cleanup_drops_cloned_fixture_root() {
        let fixture = TestFixture::with_sample_skills("verify_cleanup_drops_clone");
        fixture.verify_cleanup().expect("cleanup verification");
    }

    #[test]
    fn test_skill_diff_reports_content_tags_and_layer_changes() {
        let base = sample_skills::rust_error_handling()
            .with_tags(vec!["rust", "errors"])
            .with_layer("project");
        let updated = TestSkill::with_content(
            "rust-error-handling",
            "# Rust Error Handling\n\nUpdated guidance.\n",
        )
        .with_tags(vec!["rust", "advanced"])
        .with_layer("global");

        let diff = base.diff(&updated);

        assert_eq!(
            diff,
            SkillDiff {
                name_changed: false,
                content_changed: true,
                tags_added: vec!["advanced".to_string()],
                tags_removed: vec!["errors".to_string()],
                layer_changed: true,
            }
        );
        assert!(diff.has_changes());
    }

    #[test]
    fn test_factory_rust_project_creates_repo_signals() {
        let fixture = TestFixture::factory_rust_project();

        assert!(fixture.root.join("Cargo.toml").exists());
        assert!(fixture.root.join("src/main.rs").exists());
        assert_eq!(fixture.db_row_count("skills"), Some(3));
    }

    #[test]
    fn test_factory_node_project_creates_repo_signals() {
        let fixture = TestFixture::factory_node_project();

        assert!(fixture.root.join("package.json").exists());
        assert!(fixture.root.join("src/index.ts").exists());
        assert_eq!(fixture.db_row_count("skills"), Some(1));
    }
}
