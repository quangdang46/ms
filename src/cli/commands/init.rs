//! ms init - Initialize ms in current directory or globally

use std::fs;
use std::path::{Path, PathBuf};

use clap::Args;
use colored::Colorize;

use crate::cli::commands::providers::seed_provider_sync_state;
use crate::cli::output::OutputFormat;
use crate::error::{MsError, Result};
use crate::import::provider::{ProviderDiscovery, import_discovered_skills};
use crate::search::SearchIndex;
use crate::storage::{Database, GitArchive};

#[derive(Args, Debug)]
pub struct InitArgs {
    /// Initialize globally (~/.local/share/ms) instead of locally (.ms/)
    #[arg(long)]
    pub global: bool,

    /// Force initialization even if already initialized
    #[arg(long, short)]
    pub force: bool,
}

pub fn run(ctx: &crate::app::AppContext, args: &InitArgs) -> Result<()> {
    run_with_robot(ctx.output_format != OutputFormat::Human, args)
}

pub fn run_without_context(robot_mode: bool, args: &InitArgs) -> Result<()> {
    run_with_robot(robot_mode, args)
}

fn run_with_robot(robot_mode: bool, args: &InitArgs) -> Result<()> {
    let target = if args.global {
        global_ms_root()?
    } else {
        local_ms_root()?
    };
    let config_path = config_path_for(&target, args.global)?;

    // Check if already initialized
    if args.global {
        if config_path.exists() && !args.force {
            return Err(MsError::Config(format!(
                "already initialized at {}",
                config_path.display()
            )));
        }
    } else if local_target_initialized(&target, &config_path) && !args.force {
        return Err(MsError::Config(format!(
            "already initialized at {}",
            target.display()
        )));
    }

    if args.global {
        return if robot_mode {
            init_robot_global(&config_path, args)
        } else {
            init_human_global(&config_path, args)
        };
    }

    if robot_mode {
        return init_robot(&target, args);
    }

    init_human(&target, args)
}

fn init_human_global(config_path: &Path, args: &InitArgs) -> Result<()> {
    println!("{}", "Initializing global ms configuration...".bold());
    println!();

    print!("Creating default configuration... ");
    create_default_config(config_path, true, args.force)?;
    println!("{}", "OK".green());

    println!();
    println!(
        "{} Initialized at {}",
        "✓".green().bold(),
        config_path.display()
    );
    println!();
    println!("Add skill paths with:");
    println!("  ms config skill_paths.global '[\"~/my-skills\"]'");

    Ok(())
}

fn init_robot_global(config_path: &Path, args: &InitArgs) -> Result<()> {
    create_default_config(config_path, true, args.force)?;
    println!(
        "{}",
        serde_json::json!({
            "status": "ok",
            "config": config_path.display().to_string(),
        })
    );
    Ok(())
}

fn init_human(target: &Path, args: &InitArgs) -> Result<()> {
    println!("{}", "Initializing ms...".bold());
    println!();

    // Create directory structure
    print!("Creating directory structure... ");
    create_directories(target)?;
    println!("{}", "OK".green());

    // Create SQLite database
    print!("Initializing database... ");
    let db_path = target.join("ms.db");
    Database::open(&db_path)?;
    println!("{}", "OK".green());

    // Create Git archive
    print!("Initializing Git archive... ");
    let archive_path = target.join("archive");
    GitArchive::open(&archive_path)?;
    println!("{}", "OK".green());

    // Create search index
    print!("Initializing search index... ");
    let index_path = target.join("index");
    SearchIndex::open(&index_path)?;
    println!("{}", "OK".green());

    // Create default config
    print!("Creating default configuration... ");
    let config_path = config_path_for(target, args.global)?;
    create_default_config(&config_path, args.global, args.force)?;
    println!("{}", "OK".green());

    // Discover and import provider skills
    let db = Database::open(&db_path)?;
    let archive = GitArchive::open(&archive_path)?;
    let search = SearchIndex::open(&index_path)?;
    let discovery = ProviderDiscovery::new();
    let roots = discovery.roots();
    let existing_roots: Vec<_> = roots.iter().filter(|(p, _)| p.is_dir()).collect();
    print!("Discovering provider skills... ");
    if existing_roots.is_empty() {
        println!("{}", "no provider roots found".yellow());
    } else {
        println!(
            "{} scanning {} provider root(s)",
            "OK".green(),
            existing_roots.len()
        );
        for (root, provider) in &existing_roots {
            println!("  {} ({})", root.display(), provider);
        }
    }
    let (discovered, collision_report) = discovery.discover()?;
    if !discovered.is_empty() {
        let result =
            import_discovered_skills(discovered, collision_report, &archive, &db, &search, target)?;
        println!(
            "{} imported {} skills from providers",
            "OK".green(),
            result.imported.len()
        );
        if result.collision_report.has_collisions {
            println!(
                "{} {} skill(s) from different providers share the same name — \
                 using provider prefix to keep them distinct (e.g. agents/dcg vs claude/dcg)",
                "NOTE:".green(),
                result.collision_report.len()
            );
        }
        if !result.errors.is_empty() {
            for err in &result.errors {
                eprintln!(
                    "  warning: failed to import {}: {}",
                    err.path.display(),
                    err.message
                );
            }
        }
        if !result.warnings.is_empty() {
            println!(
                "{}: {} provider import lint warning(s)",
                "WARNING".yellow(),
                result.warnings.len()
            );
            for warning in &result.warnings {
                println!("  {} ({})", warning.path.display(), warning.skill_id);
                for diagnostic in &warning.diagnostics {
                    println!("    - [{}] {}", diagnostic.severity, diagnostic.message);
                    if let Some(suggestion) = diagnostic.suggestion.as_deref() {
                        println!("      hint: {suggestion}");
                    }
                }
            }
        }
        seed_provider_sync_state(target, discovery.roots())?;
    }

    // Sync provider roots to update cache state (disabled - ctx not available in init_human)
    // TODO: re-add sync if needed with proper context passing
    // let sync_args = SyncArgs {
    //     apply: true,
    //     root: None,
    //     verbose: false,
    // };
    // if let Err(e) = run_sync(ctx, &sync_args) {
    //     eprintln!("warning: provider sync failed: {}", e);
    // }

    println!();
    println!("{} Initialized at {}", "✓".green().bold(), target.display());

    println!();
    println!("Add skill paths with:");
    println!("  ms config skill_paths.project '[\"./skills\"]'");

    Ok(())
}

fn init_robot(target: &Path, args: &InitArgs) -> Result<()> {
    // Create everything silently
    create_directories(target)?;

    let db_path = target.join("ms.db");
    Database::open(&db_path)?;

    let archive_path = target.join("archive");
    GitArchive::open(&archive_path)?;

    let index_path = target.join("index");
    SearchIndex::open(&index_path)?;

    let config_path = config_path_for(target, args.global)?;
    create_default_config(&config_path, args.global, args.force)?;

    // Discover and import provider skills
    let db = Database::open(&db_path)?;
    let archive = GitArchive::open(&archive_path)?;
    let search = SearchIndex::open(&index_path)?;
    let discovery = ProviderDiscovery::new();
    let provider_roots: Vec<_> = discovery
        .roots()
        .iter()
        .filter(|(p, _)| p.is_dir())
        .map(|(p, name)| serde_json::json!({"path": p.display().to_string(), "provider": name}))
        .collect();
    let (discovered, collision_report) = discovery.discover()?;
    let (provider_count, provider_import_warnings) = if discovered.is_empty() {
        (0, Vec::new())
    } else {
        let result =
            import_discovered_skills(discovered, collision_report, &archive, &db, &search, target)?;
        seed_provider_sync_state(target, discovery.roots())?;
        (result.imported.len(), result.warnings)
    };

    // Sync provider roots to update cache state (disabled - ctx not available in init_robot)
    // TODO: re-add sync if needed with proper context passing
    // let sync_args = SyncArgs {
    //     apply: true,
    //     root: None,
    //     verbose: false,
    // };
    // if let Err(e) = run_sync(ctx, &sync_args) {
    //     eprintln!("warning: provider sync failed: {}", e);
    // }

    println!(
        "{}",
        serde_json::json!({
            "status": "ok",
            "path": target.display().to_string(),
            "db": db_path.display().to_string(),
            "archive": archive_path.display().to_string(),
            "index": index_path.display().to_string(),
            "config": config_path.display().to_string(),
            "provider_roots_scanned": provider_roots.len(),
            "provider_skills_imported": provider_count,
            "provider_import_warnings": provider_import_warnings,
        })
    );

    Ok(())
}

fn create_directories(target: &Path) -> Result<()> {
    fs::create_dir_all(target)?;
    fs::create_dir_all(target.join("tx"))?;
    Ok(())
}

fn create_default_config(config_path: &Path, global: bool, force: bool) -> Result<()> {
    // Create parent directory if needed
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Don't overwrite existing config
    if config_path.exists() && !force {
        return Ok(());
    }

    let default_config = if global {
        r#"# ms configuration

[skill_paths]
# Global skill repositories
global = []

# Community skill repositories
community = []

[search]
# Embedding backend configuration
use_embeddings = true
embedding_backend = "hash"
embedding_dims = 384
bm25_weight = 0.5
semantic_weight = 0.5

[cm]
# cass-memory (cm) integration
enabled = true
# cm_path = "cm"
# default_flags = []

[robot]
# Default robot mode format
format = "json"
include_metadata = true

[safety]
# Destructive Command Guard configuration
dcg_bin = "dcg"
dcg_packs = []
dcg_explain_format = "json"
require_verbatim_approval = true
# When `true`, block commands if DCG is unavailable. When `false` (default),
# fail open with a warning so optional DCG installs do not break ms edit/simulate.
require_dcg = false
"#
    } else {
        r#"# ms configuration (project-local)

[skill_paths]
# Project-local skill paths
project = ["./skills"]

# Local overrides
local = []

[search]
# Embedding backend configuration
use_embeddings = true
embedding_backend = "hash"
embedding_dims = 384
bm25_weight = 0.5
semantic_weight = 0.5

[cm]
# cass-memory (cm) integration
enabled = true
# cm_path = "cm"
# default_flags = []

[safety]
# Destructive Command Guard configuration
dcg_bin = "dcg"
dcg_packs = []
dcg_explain_format = "json"
require_verbatim_approval = true
# When `true`, block commands if DCG is unavailable. When `false` (default),
# fail open with a warning so optional DCG installs do not break ms edit/simulate.
require_dcg = false
"#
    };

    fs::write(config_path, default_config)?;
    Ok(())
}

fn config_path_for(target: &Path, global: bool) -> Result<PathBuf> {
    if global {
        return dirs::config_dir()
            .ok_or_else(|| MsError::MissingConfig("config directory not found".to_string()))
            .map(|dir| dir.join("ms/config.toml"));
    }
    Ok(target.join("config.toml"))
}

fn local_target_initialized(target: &Path, config_path: &Path) -> bool {
    config_path.exists()
        || target.join("ms.db").exists()
        || target.join("archive").is_dir()
        || target.join("index").is_dir()
}

fn global_ms_root() -> Result<PathBuf> {
    let data_dir = dirs::data_dir()
        .ok_or_else(|| MsError::MissingConfig("data directory not found".to_string()))?;
    Ok(data_dir.join("ms"))
}

fn local_ms_root() -> Result<PathBuf> {
    if let Ok(root) = std::env::var("MS_ROOT") {
        return Ok(PathBuf::from(root));
    }
    let cwd = std::env::current_dir()?;
    Ok(cwd.join(".ms"))
}
