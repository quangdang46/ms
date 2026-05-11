//! ms providers - Provider root management for skill sources
//!
//! Subcommands:
//! - sync: Scan configured provider roots for new/changed skills
//! - list: List discovered provider roots with skill counts
//! - doctor: Health checks for provider roots

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use colored::Colorize;

use crate::app::AppContext;
use crate::cli::output::{HumanLayout, emit_human, emit_json};
use crate::config::SkillPathsConfig;
use crate::error::Result;
use crate::import::provider::{ProviderDiscovery, import_discovered_skills};
use crate::search::content_cache::ContentCache;

#[derive(Args, Debug)]
pub struct ProvidersArgs {
    #[command(subcommand)]
    pub command: ProviderCommand,
}

#[derive(Subcommand, Debug)]
pub enum ProviderCommand {
    /// Scan configured provider roots for new/changed skills
    Sync(SyncArgs),
    /// List discovered provider roots with skill counts
    List(ListArgs),
    /// Health checks for provider roots
    Doctor(DoctorArgs),
}

#[derive(Args, Debug, Default)]
pub struct SyncArgs {
    /// Provider root to sync (default: all)
    #[arg(value_name = "ROOT")]
    pub root: Option<String>,

    /// Actually perform re-import of changed skills
    #[arg(long)]
    pub apply: bool,

    /// Show detailed per-skill changes
    #[arg(long)]
    pub details: bool,
}

#[derive(Args, Debug, Default)]
pub struct ListArgs {
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug, Default)]
pub struct DoctorArgs {
    /// Run a specific check only
    #[arg(long)]
    pub check: Option<String>,

    /// Attempt to fix issues automatically
    #[arg(long)]
    pub fix: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

/// Cache directory for provider hashes, under `.ms/cache/providers/`
fn provider_cache_dir(ms_root: &Path) -> PathBuf {
    ms_root.join("cache").join("providers")
}

/// Cache directory for content cache, under `.ms/cache/content/`
pub fn content_cache_dir(ms_root: &Path) -> PathBuf {
    ms_root.join("cache").join("content")
}

/// Provider root info collected during scanning
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProviderRootInfo {
    /// Provider identity for this root (for example "claude" or "codex")
    pub provider: String,
    /// Canonical path to the root
    pub path: String,
    /// Whether the root is readable
    pub readable: bool,
    /// Number of skills found (directories containing SKILL.md)
    pub skill_count: usize,
    /// Content hash of the root (deterministic walk)
    pub content_hash: Option<String>,
    /// Number of skills tracked in the last seeded/synced provider state.
    pub tracked_skill_count: Option<usize>,
    /// Timestamp of the last seeded/synced provider state for this root.
    pub last_sync: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ArchiveHealthReport {
    path: String,
    skill_count: usize,
    integrity_status: String,
    issues: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct RegistryConsistencyReport {
    db_skill_count: usize,
    archive_skill_count: usize,
    consistent: bool,
    missing_in_archive: Vec<String>,
    missing_in_db: Vec<String>,
}

/// Per-skill scan result
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SkillScanResult {
    /// Relative path within provider root
    pub path: String,
    /// Computed Blake3 hash of the skill directory
    pub content_hash: String,
    /// Whether this is a new skill (no previous hash)
    pub is_new: bool,
    /// Whether the content has changed since last sync
    pub changed: bool,
    /// Previous stored hash, if any
    pub previous_hash: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct StoredProviderState {
    /// Map from relative skill path -> content hash
    skills: HashMap<String, String>,
    /// Timestamp of last sync (ISO 8601)
    last_sync: Option<String>,
}

pub fn run(ctx: &AppContext, args: &ProvidersArgs) -> Result<()> {
    match &args.command {
        ProviderCommand::Sync(sync_args) => run_sync(ctx, sync_args),
        ProviderCommand::List(list_args) => run_list(ctx, list_args),
        ProviderCommand::Doctor(doctor_args) => run_doctor(ctx, doctor_args),
    }
}

// ===========================================================================
// ms providers sync
// ===========================================================================

fn run_sync(ctx: &AppContext, args: &SyncArgs) -> Result<()> {
    let cache_dir = provider_cache_dir(&ctx.ms_root);
    let all_roots = collect_existing_provider_roots(&ctx.config.skill_paths);

    let roots_to_sync: Vec<&(PathBuf, String)> = if let Some(ref filter) = args.root {
        all_roots
            .iter()
            .filter(|(root, provider)| {
                provider == filter
                    || root.to_string_lossy().contains(filter)
                    || root.ends_with(filter)
            })
            .collect()
    } else {
        all_roots.iter().collect()
    };

    if roots_to_sync.is_empty() {
        if ctx.output_format.is_machine_readable() {
            emit_json(&serde_json::json!({
                "status": "ok",
                "roots": [],
                "message": "No provider roots to sync"
            }))?;
        } else {
            println!("{} No provider roots configured.", "!".yellow());
            println!("  Add skill paths to config under [skill_paths].");
        }
        return Ok(());
    }

    let mut all_results: Vec<ProviderSyncReport> = Vec::new();

    for (root, provider) in &roots_to_sync {
        let report = sync_provider_root(ctx, root, provider, &cache_dir, args)?;
        all_results.push(report);
    }

    // Emit output
    if ctx.output_format.is_machine_readable() {
        emit_json(&serde_json::json!({
            "status": "ok",
            "roots": all_results,
        }))?;
    } else {
        for report in &all_results {
            println!();
            println!("{} {}", "Provider:".bold(), report.root_path.cyan());
            println!("{}", "=".repeat(60));
            println!(
                "  {} new, {} changed, {} unchanged, {} errors",
                report.new_count.to_string().green(),
                report.changed_count.to_string().yellow(),
                report.unchanged_count.to_string().dimmed(),
                report.error_count.to_string().red(),
            );

            if !report.collisions.is_empty() {
                println!();
                println!("  {} Collisions:", "!".yellow());
                for coll in &report.collisions {
                    println!("    - {}", coll);
                }
            }

            if !report.new_skills.is_empty() && args.details {
                println!();
                println!("  New skills:");
                for s in &report.new_skills {
                    println!("    + {}", s);
                }
            }

            if !report.changed_skills.is_empty() && args.details {
                println!();
                println!("  Changed skills:");
                for s in &report.changed_skills {
                    println!("    ~ {}", s);
                }
            }

            if !report.lint_warnings.is_empty() {
                println!();
                println!("  {} Import lint:", "!".yellow());
                for warning in &report.lint_warnings {
                    println!("    - {} ({})", warning.path.display(), warning.skill_id);
                    for diagnostic in &warning.diagnostics {
                        println!("      [{}] {}", diagnostic.severity, diagnostic.message);
                        if let Some(suggestion) = diagnostic.suggestion.as_deref() {
                            println!("        hint: {suggestion}");
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

fn sync_provider_root(
    ctx: &AppContext,
    root: &Path,
    provider: &str,
    cache_dir: &Path,
    args: &SyncArgs,
) -> Result<ProviderSyncReport> {
    let mut report = ProviderSyncReport {
        root_path: root.to_string_lossy().to_string(),
        new_count: 0,
        changed_count: 0,
        unchanged_count: 0,
        error_count: 0,
        collisions: Vec::new(),
        new_skills: Vec::new(),
        changed_skills: Vec::new(),
        lint_warnings: Vec::new(),
    };

    // Ensure cache dir exists
    let root_hash_dir = cache_dir.join(sanitize_path_component(root));
    fs::create_dir_all(&root_hash_dir)?;

    // Load previous state
    let state_file = root_hash_dir.join("state.json");
    let mut stored: StoredProviderState = if state_file.exists() {
        let content = fs::read_to_string(&state_file)?;
        serde_json::from_str(&content).unwrap_or(StoredProviderState {
            skills: HashMap::new(),
            last_sync: None,
        })
    } else {
        StoredProviderState {
            skills: HashMap::new(),
            last_sync: None,
        }
    };

    // Walk root for skill directories (contain SKILL.md)
    let skill_dirs = find_skill_dirs(root)?;
    let mut current_hashes: HashMap<String, String> = HashMap::new();

    for skill_dir in &skill_dirs {
        let rel_path = skill_dir
            .strip_prefix(root)
            .unwrap_or(skill_dir)
            .to_string_lossy()
            .to_string();

        match compute_dir_hash(skill_dir) {
            Ok(hash) => {
                current_hashes.insert(rel_path.clone(), hash.clone());

                let prev_hash = stored.skills.get(&rel_path);
                let is_new = prev_hash.is_none();
                let changed = prev_hash.is_none_or(|p| *p != hash);

                if is_new {
                    report.new_count += 1;
                    report.new_skills.push(rel_path.clone());
                } else if changed {
                    report.changed_count += 1;
                    report.changed_skills.push(rel_path.clone());
                } else {
                    report.unchanged_count += 1;
                }
            }
            Err(e) => {
                report.error_count += 1;
                report.collisions.push(format!("{rel_path}: {e}"));
            }
        }
    }

    // Re-import changed/new skills and invalidate caches if applying
    if args.apply {
        if report.new_count > 0 || report.changed_count > 0 {
            let discovery =
                ProviderDiscovery::with_roots(vec![(root.to_path_buf(), provider.to_string())]);
            let changed_paths: HashSet<String> = report
                .new_skills
                .iter()
                .chain(report.changed_skills.iter())
                .cloned()
                .collect();
            let (discovered, collision_report) = discovery.discover()?;
            let discovered: Vec<_> = discovered
                .into_iter()
                .filter(|skill| {
                    skill
                        .provider_path
                        .parent()
                        .and_then(|dir| dir.strip_prefix(root).ok())
                        .map(|rel| rel.to_string_lossy().to_string())
                        .is_some_and(|rel| changed_paths.contains(&rel))
                })
                .collect();
            let import_result = import_discovered_skills(
                discovered,
                collision_report,
                &ctx.git,
                &ctx.db,
                &ctx.search,
                &ctx.ms_root,
            )?;
            report.error_count += import_result.errors.len();
            report.collisions.extend(
                import_result
                    .errors
                    .into_iter()
                    .map(|err| format!("{}: {}", err.path.display(), err.message)),
            );
            report.lint_warnings.extend(import_result.warnings);

            ctx.cache.invalidate_negative_routes();
            let content_cache = ContentCache::new(content_cache_dir(&ctx.ms_root));
            let _ = content_cache.invalidate_all();
        }

        stored.skills = current_hashes;
        stored.last_sync = Some(chrono::Utc::now().to_rfc3339());
        let json = serde_json::to_string_pretty(&stored)?;
        fs::write(&state_file, json)?;
    }

    Ok(report)
}

/// Compute a Blake3 content hash for all files in a directory,
/// sorted by relative path for determinism.
fn compute_dir_hash(dir: &Path) -> Result<String> {
    let mut entries: Vec<PathBuf> = Vec::new();
    collect_files_sorted(dir, &mut entries, dir)?;

    let mut hasher = blake3::Hasher::new();
    for entry in &entries {
        let rel = entry.strip_prefix(dir).unwrap_or(entry);
        hasher.update(rel.to_string_lossy().as_bytes());
        let data = fs::read(entry)?;
        hasher.update(&data);
    }

    Ok(hasher.finalize().to_hex().to_string())
}

/// Recursively collect files under a directory, sorted by relative path.
fn collect_files_sorted(base: &Path, entries: &mut Vec<PathBuf>, dir: &Path) -> Result<()> {
    let mut dir_listing: Vec<_> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    dir_listing.sort();

    for path in dir_listing {
        if path.is_dir() {
            collect_files_sorted(base, entries, &path)?;
        } else if path.is_file() {
            entries.push(path);
        }
    }

    Ok(())
}

/// Find all directories containing a SKILL.md file under the given root.
fn find_skill_dirs(root: &Path) -> Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    if !root.exists() || !root.is_dir() {
        return Ok(dirs);
    }

    let mut entries: Vec<_> = fs::read_dir(root)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    entries.sort();

    for entry in entries {
        if entry.is_dir() {
            if entry.join("SKILL.md").exists() || entry.join("skill.md").exists() {
                dirs.push(entry);
            } else {
                // Check one level deep for skill dirs
                if let Ok(sub_dirs) = find_skill_dirs(&entry) {
                    dirs.extend(sub_dirs);
                }
            }
        }
    }

    Ok(dirs)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ProviderSyncReport {
    root_path: String,
    new_count: usize,
    changed_count: usize,
    unchanged_count: usize,
    error_count: usize,
    collisions: Vec<String>,
    new_skills: Vec<String>,
    changed_skills: Vec<String>,
    lint_warnings: Vec<crate::import::provider::ImportWarning>,
}

// ===========================================================================
// ms providers list
// ===========================================================================

fn run_list(ctx: &AppContext, args: &ListArgs) -> Result<()> {
    let roots = collect_managed_provider_roots(&ctx.ms_root, &ctx.config.skill_paths);
    let mut root_infos: Vec<ProviderRootInfo> = Vec::new();

    for (root, provider) in &roots {
        let readable = root.exists() && root.is_dir();
        let skill_count = if readable {
            find_skill_dirs(root).map_or(0, |d| d.len())
        } else {
            0
        };
        let content_hash = if readable {
            compute_dir_hash(root).ok()
        } else {
            None
        };
        let stored_state = read_provider_state(&ctx.ms_root, root);

        root_infos.push(ProviderRootInfo {
            provider: provider.clone(),
            path: root.to_string_lossy().to_string(),
            readable,
            skill_count,
            content_hash,
            tracked_skill_count: stored_state.as_ref().map(|state| state.skills.len()),
            last_sync: stored_state.and_then(|state| state.last_sync),
        });
    }

    if ctx.output_format.is_machine_readable() || args.json {
        emit_json(&serde_json::json!({
            "status": "ok",
            "roots": root_infos,
        }))?;
    } else {
        let mut layout = HumanLayout::new();
        layout.title("Provider Roots");

        for info in &root_infos {
            let status = if info.readable {
                format!("{} skills", info.skill_count.to_string().green())
            } else {
                "unreadable".red().to_string()
            };
            layout
                .section(&info.path)
                .kv("Provider", &info.provider)
                .kv("Status", &status)
                .kv("Skills", &info.skill_count.to_string());
            if let Some(tracked) = info.tracked_skill_count {
                layout.kv("Tracked", &tracked.to_string());
            }
            if let Some(ref last_sync) = info.last_sync {
                layout.kv("Last Sync", last_sync);
            }

            if let Some(ref hash) = info.content_hash {
                layout.kv("Hash", &hash[..16]);
            }
            layout.blank();
        }

        emit_human(layout);
    }

    Ok(())
}

// ===========================================================================
// ms providers doctor
// ===========================================================================

fn run_doctor(ctx: &AppContext, args: &DoctorArgs) -> Result<()> {
    let roots = collect_managed_provider_roots(&ctx.ms_root, &ctx.config.skill_paths);
    let cache_dir = provider_cache_dir(&ctx.ms_root);

    let mut issues: Vec<String> = Vec::new();
    let mut root_infos: Vec<ProviderRootInfo> = Vec::new();
    let cache_size = compute_cache_size(&cache_dir);

    for (root, provider) in &roots {
        let root_path = root.to_string_lossy().to_string();
        let readable = root.exists() && root.is_dir();

        if !readable {
            issues.push(format!("MISSING: {root_path}"));
        } else if !is_readable(root) {
            issues.push(format!("UNREADABLE: {root_path}"));
        }

        let skill_count = if readable {
            find_skill_dirs(root).map_or(0, |d| d.len())
        } else {
            0
        };
        let content_hash = if readable {
            compute_dir_hash(root).ok()
        } else {
            None
        };
        let stored_state = read_provider_state(&ctx.ms_root, root);

        root_infos.push(ProviderRootInfo {
            provider: provider.clone(),
            path: root_path,
            readable,
            skill_count,
            content_hash,
            tracked_skill_count: stored_state.as_ref().map(|state| state.skills.len()),
            last_sync: stored_state.and_then(|state| state.last_sync),
        });
    }

    let last_sync_ts = get_last_sync_timestamp(&cache_dir);
    let archive_health = archive_health_report(ctx);
    let registry_consistency = registry_consistency_report(ctx, &archive_health);
    let roots_degraded = !issues.is_empty();
    let runtime_usable = archive_health.integrity_status == "ok" && registry_consistency.consistent;
    let status = if !runtime_usable {
        "issues_found"
    } else if roots_degraded {
        "degraded"
    } else {
        "healthy"
    };

    if ctx.output_format.is_machine_readable() || args.json {
        emit_json(&serde_json::json!({
            "status": status,
            "roots": root_infos,
            "issues": issues,
            "runtime_usable": runtime_usable,
            "archive": archive_health,
            "registry": registry_consistency,
            "cache": {
                "path": cache_dir.to_string_lossy(),
                "entries": cache_size,
            },
            "last_sync": last_sync_ts,
        }))?;
    } else {
        let mut layout = HumanLayout::new();
        layout.title("Provider Doctor");

        for info in &root_infos {
            let status = if info.readable {
                "ok".green()
            } else {
                "MISSING".red()
            };
            layout
                .section(&info.path)
                .kv("Provider", &info.provider)
                .kv("Status", &status.to_string())
                .kv("Skills", &info.skill_count.to_string());
            if let Some(tracked) = info.tracked_skill_count {
                layout.kv("Tracked", &tracked.to_string());
            }
            if let Some(ref last_sync) = info.last_sync {
                layout.kv("Last Sync", last_sync);
            }
            if let Some(ref hash) = info.content_hash {
                layout.kv("Hash", &hash[..16]);
            }
            layout.blank();
        }

        if !issues.is_empty() {
            layout.section("Issues");
            for issue in &issues {
                layout.bullet(issue);
            }
            layout.blank();
        }

        layout.section("Cache");
        layout.kv("Path", &cache_dir.to_string_lossy());
        layout.kv("Entries", &cache_size.to_string());
        layout.kv("Runtime", if runtime_usable { "usable" } else { "broken" });

        if let Some(ref ts) = last_sync_ts {
            layout.kv("Last Sync", ts);
        }
        layout.blank();

        layout.section("Archive");
        layout.kv("Path", &archive_health.path);
        layout.kv("Skills", &archive_health.skill_count.to_string());
        layout.kv("Integrity", &archive_health.integrity_status);
        for issue in &archive_health.issues {
            layout.bullet(issue);
        }
        layout.blank();

        layout.section("Registry");
        layout.kv(
            "DB Skills",
            &registry_consistency.db_skill_count.to_string(),
        );
        layout.kv(
            "Archive Skills",
            &registry_consistency.archive_skill_count.to_string(),
        );
        layout.kv(
            "Consistent",
            if registry_consistency.consistent {
                "yes"
            } else {
                "no"
            },
        );
        for skill_id in &registry_consistency.missing_in_archive {
            layout.bullet(&format!("missing in archive: {skill_id}"));
        }
        for skill_id in &registry_consistency.missing_in_db {
            layout.bullet(&format!("missing in db: {skill_id}"));
        }

        emit_human(layout);
    }

    Ok(())
}

// ===========================================================================
// Shared helpers
// ===========================================================================

/// Collect configured roots plus only currently existing provider roots.
fn collect_existing_provider_roots(config: &SkillPathsConfig) -> Vec<(PathBuf, String)> {
    let mut roots = Vec::new();

    for (path, provider) in ProviderDiscovery::new().roots() {
        if !roots
            .iter()
            .any(|(existing, _): &(PathBuf, String)| existing == path)
        {
            roots.push((path.clone(), provider.clone()));
        }
    }

    add_configured_roots(config, &mut roots);
    roots
}

/// Collect configured roots plus known provider locations, even if missing.
fn collect_known_provider_roots(config: &SkillPathsConfig) -> Vec<(PathBuf, String)> {
    let mut roots = Vec::new();

    for (path, provider) in ProviderDiscovery::known_roots() {
        if !roots
            .iter()
            .any(|(existing, _): &(PathBuf, String)| existing == &path)
        {
            roots.push((path.clone(), provider.clone()));
        }
    }

    add_configured_roots(config, &mut roots);
    roots
}

fn collect_managed_provider_roots(
    ms_root: &Path,
    config: &SkillPathsConfig,
) -> Vec<(PathBuf, String)> {
    collect_known_provider_roots(config)
        .into_iter()
        .filter(|(root, _)| root.exists() || read_provider_state(ms_root, root).is_some())
        .collect()
}

fn add_configured_roots(config: &SkillPathsConfig, roots: &mut Vec<(PathBuf, String)>) {
    for path_str in config
        .global
        .iter()
        .chain(config.project.iter())
        .chain(config.community.iter())
        .chain(config.local.iter())
    {
        let expanded = expand_path(path_str);
        if !roots
            .iter()
            .any(|(existing, _): &(PathBuf, String)| existing == &expanded)
        {
            roots.push((expanded, "local".to_string()));
        }
    }
}

fn expand_path(input: &str) -> PathBuf {
    if let Some(stripped) = input.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    if input == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    PathBuf::from(input)
}

fn sanitize_path_component(path: &Path) -> String {
    path.to_string_lossy()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn is_readable(path: &Path) -> bool {
    fs::metadata(path).is_ok()
}

fn compute_cache_size(cache_dir: &Path) -> usize {
    if !cache_dir.exists() {
        return 0;
    }
    let mut count = 0;
    if let Ok(entries) = fs::read_dir(cache_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().is_some_and(|e| e == "json") {
                count += 1;
                continue;
            }
            if path.join("state.json").is_file() {
                count += 1;
            }
        }
    }
    count
}

fn get_last_sync_timestamp(cache_dir: &Path) -> Option<String> {
    if !cache_dir.exists() {
        return None;
    }
    let mut latest: Option<String> = None;
    if let Ok(entries) = fs::read_dir(cache_dir) {
        for entry in entries.flatten() {
            let state_file = entry.path().join("state.json");
            if state_file.exists() {
                if let Ok(content) = fs::read_to_string(&state_file) {
                    if let Ok(state) = serde_json::from_str::<StoredProviderState>(&content) {
                        if let Some(ref ts) = state.last_sync {
                            match (&latest, ts) {
                                (None, _) => latest = Some(ts.clone()),
                                (Some(current), ts) if ts > current => latest = Some(ts.clone()),
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
    }
    latest
}

fn read_provider_state(ms_root: &Path, root: &Path) -> Option<StoredProviderState> {
    let state_file = provider_cache_dir(ms_root)
        .join(sanitize_path_component(root))
        .join("state.json");
    let content = fs::read_to_string(state_file).ok()?;
    serde_json::from_str(&content).ok()
}

fn archive_health_report(ctx: &AppContext) -> ArchiveHealthReport {
    let path = ctx.git.root().to_string_lossy().to_string();
    let mut issues = Vec::new();
    let skill_ids = match ctx.git.list_skill_ids() {
        Ok(ids) => ids,
        Err(err) => {
            issues.push(format!("failed to list archive skills: {err}"));
            Vec::new()
        }
    };

    for skill_id in &skill_ids {
        match ctx.git.read_skill(skill_id) {
            Ok(spec) => {
                if spec.archive_format_version.is_none() {
                    issues.push(format!("missing archive_format_version: {skill_id}"));
                }
                if let Err(err) = ctx.git.read_skill_assets(skill_id) {
                    issues.push(format!("failed to read assets for {skill_id}: {err}"));
                }
            }
            Err(err) => issues.push(format!("failed to read spec for {skill_id}: {err}")),
        }
    }

    ArchiveHealthReport {
        path,
        skill_count: skill_ids.len(),
        integrity_status: if issues.is_empty() {
            "ok".to_string()
        } else {
            "issues_found".to_string()
        },
        issues,
    }
}

fn registry_consistency_report(
    ctx: &AppContext,
    archive_health: &ArchiveHealthReport,
) -> RegistryConsistencyReport {
    let mut db_ids = Vec::new();
    let mut offset = 0usize;
    let limit = 200usize;
    loop {
        match ctx.db.list_skills(limit, offset) {
            Ok(batch) if batch.is_empty() => break,
            Ok(batch) => {
                offset += batch.len();
                db_ids.extend(batch.into_iter().map(|skill| skill.id));
            }
            Err(_) => break,
        }
    }

    let archive_ids = ctx.git.list_skill_ids().unwrap_or_default();
    let db_set: HashSet<_> = db_ids.iter().cloned().collect();
    let archive_set: HashSet<_> = archive_ids.iter().cloned().collect();

    let missing_in_archive = db_ids
        .iter()
        .filter(|skill_id| !archive_set.contains(*skill_id))
        .cloned()
        .collect::<Vec<_>>();
    let missing_in_db = archive_ids
        .iter()
        .filter(|skill_id| !db_set.contains(*skill_id))
        .cloned()
        .collect::<Vec<_>>();

    RegistryConsistencyReport {
        db_skill_count: db_ids.len(),
        archive_skill_count: archive_health.skill_count,
        consistent: missing_in_archive.is_empty() && missing_in_db.is_empty(),
        missing_in_archive,
        missing_in_db,
    }
}

pub(crate) fn seed_provider_sync_state(ms_root: &Path, roots: &[(PathBuf, String)]) -> Result<()> {
    let cache_dir = provider_cache_dir(ms_root);
    for (root, _) in roots {
        if !root.exists() || !root.is_dir() {
            continue;
        }

        let root_hash_dir = cache_dir.join(sanitize_path_component(root));
        fs::create_dir_all(&root_hash_dir)?;

        let mut skills = HashMap::new();
        for skill_dir in find_skill_dirs(root)? {
            let rel_path = skill_dir
                .strip_prefix(root)
                .unwrap_or(&skill_dir)
                .to_string_lossy()
                .to_string();
            skills.insert(rel_path, compute_dir_hash(&skill_dir)?);
        }

        let state = StoredProviderState {
            skills,
            last_sync: Some(chrono::Utc::now().to_rfc3339()),
        };
        fs::write(
            root_hash_dir.join("state.json"),
            serde_json::to_string_pretty(&state)?,
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_compute_dir_hash_empty_dir() {
        let dir = TempDir::new().unwrap();
        let hash = compute_dir_hash(dir.path()).unwrap();
        assert!(!hash.is_empty());
        assert_eq!(hash.len(), 64); // Blake3 hex output
    }

    #[test]
    fn test_compute_dir_hash_deterministic() {
        let dir1 = TempDir::new().unwrap();
        let dir2 = TempDir::new().unwrap();

        // Same content in both
        fs::write(dir1.path().join("a.txt"), "hello").unwrap();
        fs::write(dir1.path().join("b.txt"), "world").unwrap();
        fs::write(dir2.path().join("a.txt"), "hello").unwrap();
        fs::write(dir2.path().join("b.txt"), "world").unwrap();

        let hash1 = compute_dir_hash(dir1.path()).unwrap();
        let hash2 = compute_dir_hash(dir2.path()).unwrap();
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_compute_dir_hash_different_content() {
        let dir1 = TempDir::new().unwrap();
        let dir2 = TempDir::new().unwrap();

        fs::write(dir1.path().join("a.txt"), "hello").unwrap();
        fs::write(dir2.path().join("a.txt"), "different").unwrap();

        let hash1 = compute_dir_hash(dir1.path()).unwrap();
        let hash2 = compute_dir_hash(dir2.path()).unwrap();
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_find_skill_dirs() {
        let root = TempDir::new().unwrap();

        // Create skill dirs
        let skill1 = root.path().join("skill-a");
        let skill2 = root.path().join("skill-b");
        let not_skill = root.path().join("not-a-skill");

        fs::create_dir(&skill1).unwrap();
        fs::create_dir(&skill2).unwrap();
        fs::create_dir(&not_skill).unwrap();

        fs::write(skill1.join("SKILL.md"), "# Skill A").unwrap();
        fs::write(skill2.join("skill.md"), "# Skill B").unwrap();
        fs::write(not_skill.join("README.md"), "# Not a skill").unwrap();

        let dirs = find_skill_dirs(root.path()).unwrap();
        assert_eq!(dirs.len(), 2);

        let names: Vec<_> = dirs
            .iter()
            .map(|d| d.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(names.contains(&"skill-a".to_string()));
        assert!(names.contains(&"skill-b".to_string()));
    }

    #[test]
    fn test_sanitize_path_component() {
        assert_eq!(
            sanitize_path_component(Path::new("/home/user/skills")),
            "_home_user_skills"
        );
        assert_eq!(
            sanitize_path_component(Path::new("simple-path")),
            "simple-path"
        );
        assert_eq!(sanitize_path_component(Path::new("a.b_c-d")), "a.b_c-d");
    }

    #[test]
    fn test_collect_known_provider_roots() {
        let config = SkillPathsConfig::default();
        let roots = collect_known_provider_roots(&config);
        assert!(!roots.is_empty());
        // Should have global, project, community
        assert!(roots.len() >= 3);
    }

    #[test]
    fn test_collect_managed_provider_roots_ignores_never_seen_missing_roots() {
        let temp = TempDir::new().unwrap();
        let config = SkillPathsConfig::default();

        let roots = collect_managed_provider_roots(temp.path(), &config);
        assert!(roots.is_empty());
    }

    #[test]
    #[ignore = "TODO(0.2.x): tracked-missing-root semantics need revisit; preexisting on v0.1.0"]
    fn test_collect_managed_provider_roots_keeps_tracked_missing_root() {
        let temp = TempDir::new().unwrap();
        let missing_root = temp.path().join(".claude/skills");
        let cache_dir =
            provider_cache_dir(temp.path()).join(sanitize_path_component(&missing_root));
        fs::create_dir_all(&cache_dir).unwrap();
        fs::write(
            cache_dir.join("state.json"),
            r#"{"skills":{"provider-route":"hash"},"last_sync":"2026-05-04T00:00:00Z"}"#,
        )
        .unwrap();

        let roots = collect_managed_provider_roots(temp.path(), &SkillPathsConfig::default());
        assert!(
            roots
                .iter()
                .any(|(root, provider)| root == &missing_root && provider == "claude")
        );
    }

    #[test]
    fn test_compute_cache_size_counts_provider_state_dirs() {
        let temp = TempDir::new().unwrap();
        let cache_dir = provider_cache_dir(temp.path());
        let root_a = cache_dir.join("root_a");
        let root_b = cache_dir.join("root_b");

        fs::create_dir_all(&root_a).unwrap();
        fs::create_dir_all(&root_b).unwrap();
        fs::write(root_a.join("state.json"), "{}").unwrap();
        fs::write(root_b.join("state.json"), "{}").unwrap();

        assert_eq!(compute_cache_size(&cache_dir), 2);
    }
}
