//! Provider discovery and skill import.
//!
//! Discovers skills from known provider directories (Claude Code, Codex, Gemini),
//! parses SKILL.md files into SkillSpec, and persists them into the .ms archive
//! and SQLite database. This is the bridge between provider-native skill formats
//! and the meta_skill (ms) unified registry.
//!
//! # Provider Roots
//!
//! Provider skills are discovered from local-project and home-directory roots:
//! - `./.claude/skills`, `~/.claude/skills`
//! - `./.agents/skills`, `~/.agents/skills`
//! - `./.codex/skills`, `~/.codex/skills`
//! - `./.gemini/skills`, `~/.gemini/skills`

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::core::ids::{CollisionReport, detect_collisions, is_unambiguous};
use crate::core::skill::{
    BlockType, ReferenceFile, ScriptFile, SkillAssets, SkillProvenance, SkillSpec,
};
use crate::core::spec_lens::parse_markdown;
use crate::error::{MsError, Result};
use crate::lint::rules::OversizedSkillMdRule;
use crate::lint::{Diagnostic, ValidationConfig, ValidationContext, ValidationRule};
use crate::search::SearchIndex;
use crate::storage::{Database, GitArchive, SkillRecord};
use chrono::Utc;

/// Provider root with its display name and home-directory relative path.
const PROVIDER_ROOTS: &[(&str, &str)] = &[
    (".claude/skills", "claude"),
    (".agents/skills", "agents"),
    (".codex/skills", "codex"),
    (".gemini/skills", "gemini"),
];

/// A discovered skill from a provider directory.
#[derive(Debug, Clone)]
pub struct DiscoveredSkill {
    /// Provider name (e.g., "claude", "codex", "gemini")
    pub provider: String,
    /// Absolute path to the SKILL.md file
    pub provider_path: PathBuf,
    /// Parsed skill specification
    pub spec: SkillSpec,
    /// Script files found alongside the skill
    pub scripts: Vec<PathBuf>,
    /// Reference files found alongside the skill
    pub references: Vec<PathBuf>,
}

/// Result of importing discovered skills into the archive.
#[derive(Debug, Clone)]
pub struct ImportResult {
    /// Skills that were successfully imported
    pub imported: Vec<ImportEntry>,
    /// Skills that failed to import
    pub errors: Vec<ImportError>,
    /// Non-blocking import lint diagnostics surfaced during provider import.
    pub warnings: Vec<ImportWarning>,
    /// Total skills discovered
    pub discovered_count: usize,
    /// Collision report for duplicate skill IDs across providers
    pub collision_report: CollisionReport,
}

/// A successfully imported skill entry.
#[derive(Debug, Clone)]
pub struct ImportEntry {
    /// Skill ID in the archive
    pub skill_id: String,
    /// Provider name
    pub provider: String,
    /// Original provider path
    pub provider_path: PathBuf,
    /// Timestamp of import
    pub imported_at: String,
}

/// A skill that failed to import.
#[derive(Debug, Clone)]
pub struct ImportError {
    /// Provider name
    pub provider: String,
    /// Path that was attempted
    pub path: PathBuf,
    /// Error message
    pub message: String,
}

/// Non-blocking diagnostics discovered while importing a provider skill.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImportWarning {
    /// Provider name
    pub provider: String,
    /// Skill ID within the provider
    pub skill_id: String,
    /// Path to the imported provider SKILL.md
    pub path: PathBuf,
    /// Diagnostics emitted for this imported skill
    pub diagnostics: Vec<Diagnostic>,
}

/// Provider discovery engine.
pub struct ProviderDiscovery {
    roots: Vec<(PathBuf, String)>,
}

impl ProviderDiscovery {
    /// Return the known provider root locations for the current cwd and home dir,
    /// whether or not they currently exist on disk.
    #[must_use]
    pub fn known_roots() -> Vec<(PathBuf, String)> {
        let mut roots: Vec<(PathBuf, String)> = Vec::new();

        if let Ok(cwd) = std::env::current_dir() {
            for (rel_path, provider_name) in PROVIDER_ROOTS {
                roots.push((cwd.join(rel_path), (*provider_name).to_string()));
            }
        }

        if let Some(home) = dirs::home_dir() {
            for (rel_path, provider_name) in PROVIDER_ROOTS {
                let global = home.join(rel_path);
                if !roots.iter().any(|(path, _)| path == &global) {
                    roots.push((global, (*provider_name).to_string()));
                }
            }
        }

        roots
    }

    /// Create a new provider discovery instance.
    ///
    /// Scans both the current working directory and the user's home directory
    /// for known provider skill roots. Root paths are checked for existence
    /// at construction time.
    #[must_use]
    pub fn new() -> Self {
        let roots = Self::known_roots()
            .into_iter()
            .filter(|(path, _)| path.is_dir())
            .collect();

        Self { roots }
    }

    /// Create a discovery with specific roots (for testing).
    #[must_use]
    pub fn with_roots(roots: Vec<(PathBuf, String)>) -> Self {
        Self { roots }
    }

    #[must_use]
    pub fn roots(&self) -> &[(PathBuf, String)] {
        &self.roots
    }

    /// Discover all skills from all provider roots.
    ///
    /// Returns a list of discovered skills with their parsed specs and
    /// associated assets (scripts/, references/).
    pub fn discover(&self) -> Result<(Vec<DiscoveredSkill>, CollisionReport)> {
        let mut skills: Vec<DiscoveredSkill> = Vec::new();

        for (root, provider) in &self.roots {
            let found = Self::discover_from_root(root, provider)?;
            skills.extend(found);
        }

        let collision_pairs: Vec<(&str, &str)> = skills
            .iter()
            .map(|s| (s.provider.as_str(), s.spec.metadata.id.as_str()))
            .collect();
        let collision_report = detect_collisions(collision_pairs);

        Ok((skills, collision_report))
    }

    /// Discover skills from a single provider root directory.
    fn discover_from_root(root: &Path, provider: &str) -> Result<Vec<DiscoveredSkill>> {
        let mut skills = Vec::new();
        let skill_files = find_skill_md_files(root)?;

        #[cfg(debug_assertions)]
        eprintln!(
            "debug: found {} SKILL.md files in {} provider root '{}'",
            skill_files.len(),
            provider,
            root.display()
        );

        for skill_path in &skill_files {
            match Self::parse_single_skill(skill_path, provider) {
                Ok(skill) => skills.push(skill),
                Err(e) => {
                    #[cfg(debug_assertions)]
                    eprintln!(
                        "debug: failed to parse skill at {}: {}",
                        skill_path.display(),
                        e
                    );
                }
            }
        }

        Ok(skills)
    }

    /// Parse a single SKILL.md file into a DiscoveredSkill.
    fn parse_single_skill(skill_path: &Path, provider: &str) -> Result<DiscoveredSkill> {
        let content = fs::read_to_string(skill_path)?;
        let spec = parse_markdown(&content)?;

        // Extract scripts and references directories
        let skill_dir = skill_path
            .parent()
            .ok_or_else(|| MsError::ValidationFailed("skill path has no parent".to_string()))?;

        // If the spec has no ID, derive one from the filename
        let mut spec = spec;
        if spec.metadata.id.is_empty() || spec.metadata.id == "imported-skill" {
            if let Some(stem) = skill_path.file_stem().and_then(|s| s.to_str()) {
                spec.metadata.id = slugify_id(stem);
            }
        }

        let scripts = find_asset_files(skill_dir, "scripts");
        let references = find_asset_files(skill_dir, "references");

        Ok(DiscoveredSkill {
            provider: provider.to_string(),
            provider_path: skill_path.to_path_buf(),
            spec,
            scripts,
            references,
        })
    }
}

impl Default for ProviderDiscovery {
    fn default() -> Self {
        Self::new()
    }
}

/// Import discovered skills into the .ms archive, database, and search index.
///
/// For each discovered skill:
/// 1. Write the SkillSpec to the Git archive (skills/by-id/<id>/)
/// 2. Upsert a SkillRecord into the SQLite database
/// 3. Index the skill body in the search index
pub fn import_discovered_skills(
    discovered: Vec<DiscoveredSkill>,
    collision_report: CollisionReport,
    archive: &GitArchive,
    db: &Database,
    search: &SearchIndex,
    ms_root: &Path,
) -> Result<ImportResult> {
    let mut result = ImportResult {
        imported: Vec::new(),
        errors: Vec::new(),
        warnings: Vec::new(),
        discovered_count: discovered.len(),
        collision_report,
    };
    let mut provider_skill_ids: HashMap<String, HashSet<String>> = HashMap::new();

    for skill in &discovered {
        provider_skill_ids
            .entry(skill.provider.clone())
            .or_default()
            .insert(skill.spec.metadata.id.clone());
    }

    for skill in discovered {
        let same_provider_ids = provider_skill_ids
            .get(&skill.provider)
            .cloned()
            .unwrap_or_default();
        let warnings = collect_import_warnings(&skill);

        if !warnings.is_empty() {
            result.warnings.push(ImportWarning {
                provider: skill.provider.clone(),
                skill_id: skill.spec.metadata.id.clone(),
                path: skill.provider_path.clone(),
                diagnostics: warnings,
            });
        }

        match import_single_skill(
            &skill,
            &same_provider_ids,
            &result.collision_report,
            archive,
            db,
            search,
            ms_root,
        ) {
            Ok(entry) => result.imported.push(entry),
            Err(e) => {
                result.errors.push(ImportError {
                    provider: skill.provider.clone(),
                    path: skill.provider_path.clone(),
                    message: e.to_string(),
                });
            }
        }
    }

    Ok(result)
}

/// Import a single discovered skill into the archive.
fn import_single_skill(
    skill: &DiscoveredSkill,
    same_provider_ids: &HashSet<String>,
    collision_report: &CollisionReport,
    archive: &GitArchive,
    db: &Database,
    search: &SearchIndex,
    ms_root: &Path,
) -> Result<ImportEntry> {
    let now = Utc::now().to_rfc3339();
    let skill_id = skill.spec.metadata.id.clone();

    let mut archived_spec = skill.spec.clone();
    ensure_compact_route_metadata(&mut archived_spec);
    archived_spec.metadata.provider = skill.provider.clone();
    let canonical = crate::core::ids::CanonicalId::new(&skill.provider, &skill_id)?;
    archived_spec.metadata.canonical_id = canonical.to_canonical_string();
    archived_spec.metadata.display_id =
        canonical.display();
    canonicalize_provider_references(&mut archived_spec, same_provider_ids);
    archived_spec.archive_format_version = Some(SkillSpec::ARCHIVE_FORMAT_VERSION.to_string());
    archived_spec.provenance = Some(SkillProvenance {
        provider: skill.provider.clone(),
        source_path: skill.provider_path.clone(),
        imported_at: chrono::Utc::now(),
    });
    let storage_id = archived_spec.storage_id();

    // 1. Write spec to Git archive
    let commit = archive.write_skill(&archived_spec)?;

    // 2. Snapshot assets into the archive and commit them so they survive source deletion.
    let skill_dir = skill.provider_path.parent().ok_or_else(|| {
        MsError::ValidationFailed("provider skill path has no parent directory".to_string())
    })?;
    let archived_assets = build_archived_assets(&skill.scripts, &skill.references);
    archive.write_skill_assets(&storage_id, skill_dir, &archived_assets)?;

    // 3. Compute content hash for change tracking using the full provider folder.
    let body = render_skill_body(&archived_spec);
    let content_hash = compute_dir_hash(skill_dir)?;

    // 4. Build metadata JSON from the spec
    let metadata_json = serde_json::to_string(&archived_spec.metadata)
        .map_err(|err| MsError::Serialization(format!("serialize provider metadata: {err}")))?;

    // 5. Build assets JSON from the archived snapshot so `--complete` survives source deletion.
    let assets_json = serde_json::to_string(&archive.read_skill_assets(&storage_id)?)
        .map_err(|err| MsError::Serialization(format!("serialize provider assets: {err}")))?;

    // 6. Token estimate (rough: body chars / 4)
    let token_count = (body.len() / 4).max(1) as i64;

    // 7. Upsert to database
    let record = SkillRecord {
        id: storage_id,
        name: archived_spec.metadata.name.clone(),
        description: archived_spec.metadata.description.clone(),
        version: if archived_spec.metadata.version.is_empty() {
            None
        } else {
            Some(archived_spec.metadata.version.clone())
        },
        author: archived_spec.metadata.author.clone(),
        source_path: skill.provider_path.to_string_lossy().to_string(),
        source_layer: "project".to_string(),
        provider: Some(skill.provider.clone()),
        git_remote: Some(ms_root.join("archive").to_string_lossy().to_string()),
        git_commit: Some(commit.oid.clone()),
        content_hash,
        body,
        metadata_json,
        assets_json,
        token_count,
        quality_score: 0.5, // Default quality for provider-imported skills
        indexed_at: now.clone(),
        modified_at: now.clone(),
        is_deprecated: false,
        deprecation_reason: None,
        archive_format_version: Some(SkillSpec::ARCHIVE_FORMAT_VERSION.to_string()),
        provenance_json: serde_json::json!({
            "provider": skill.provider,
            "source_path": skill.provider_path,
            "imported_at": now,
        })
        .to_string(),
    };
    db.upsert_skill(&record)?;

    // 8. Index in search engine
    let _ = search.index_skill(&record);

    Ok(ImportEntry {
        skill_id: archived_spec.metadata.canonical_id.clone(),
        provider: skill.provider.clone(),
        provider_path: skill.provider_path.clone(),
        imported_at: now,
    })
}

fn collect_import_warnings(skill: &DiscoveredSkill) -> Vec<Diagnostic> {
    let mut spec = skill.spec.clone();
    ensure_compact_route_metadata(&mut spec);

    let config = ValidationConfig::new();
    let rule = OversizedSkillMdRule::default();
    let ctx = ValidationContext::new(&spec, &config).with_file_path(&skill.provider_path);
    rule.validate(&ctx)
}

fn ensure_compact_route_metadata(spec: &mut SkillSpec) {
    if !spec.metadata.keywords.is_empty() {
        return;
    }

    let rule = OversizedSkillMdRule::default();
    let mut seen = HashSet::new();
    let mut keywords = Vec::new();

    for hint in rule.derive_route_metadata(spec) {
        let value = hint
            .strip_prefix("desc:")
            .or_else(|| hint.strip_prefix("tag:"))
            .or_else(|| hint.strip_prefix("trigger:"))
            .unwrap_or(hint.as_str())
            .trim();
        push_route_keyword(&mut keywords, &mut seen, value);
    }

    spec.metadata.keywords = keywords;
}

fn push_route_keyword(keywords: &mut Vec<String>, seen: &mut HashSet<String>, value: &str) {
    if let Some(normalized) = normalize_route_keyword(value) {
        if seen.insert(normalized.clone()) {
            keywords.push(normalized);
        }
    }
}

fn normalize_route_keyword(value: &str) -> Option<String> {
    let normalized = value
        .split_whitespace()
        .map(|word| word.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_'))
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();

    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

/// Find all SKILL.md files recursively under a root directory.
fn find_skill_md_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if !root.is_dir() {
        return Ok(files);
    }

    for entry in walkdir::WalkDir::new(root)
        .max_depth(3)
        .into_iter()
        .filter_entry(|e| {
            // Skip hidden directories (except the root)
            if e.depth() == 0 {
                return true;
            }
            if let Some(name) = e.file_name().to_str() {
                if name.starts_with('.') {
                    return false;
                }
            }
            true
        })
    {
        let entry = entry.map_err(|err| MsError::Config(format!("walk provider skills: {err}")))?;

        if entry.file_type().is_file() && entry.file_name().to_string_lossy() == "SKILL.md" {
            files.push(entry.into_path());
        }
    }

    files.sort();
    Ok(files)
}

/// Find asset files in a subdirectory of the skill directory.
fn find_asset_files(skill_dir: &Path, subdir: &str) -> Vec<PathBuf> {
    let asset_dir = skill_dir.join(subdir);
    if !asset_dir.is_dir() {
        return Vec::new();
    }

    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(&asset_dir) {
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|t| t.is_file()) {
                files.push(entry.path());
            }
        }
    }
    files.sort();
    files
}

/// Copy asset files to a target directory.
/// Render a SkillSpec as a body string for the database.
fn render_skill_body(spec: &SkillSpec) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", spec.metadata.name));
    if !spec.metadata.description.is_empty() {
        out.push_str(&spec.metadata.description);
        out.push('\n');
    }
    for section in &spec.sections {
        out.push_str(&format!("## {}\n\n", section.title));
        for block in &section.blocks {
            match block.block_type {
                BlockType::Code => {
                    out.push_str("```\n");
                    out.push_str(&block.content);
                    out.push_str("\n```\n\n");
                }
                _ => {
                    out.push_str(&block.content);
                    out.push('\n');
                }
            }
        }
    }
    out
}

/// Compute a Blake3 content hash over the full skill directory.
fn compute_dir_hash(dir: &Path) -> Result<String> {
    let mut entries = Vec::new();
    collect_files_sorted(dir, &mut entries)?;

    let mut hasher = blake3::Hasher::new();
    for entry in &entries {
        let rel = entry.strip_prefix(dir).unwrap_or(entry);
        hasher.update(rel.to_string_lossy().as_bytes());
        hasher.update(&fs::read(entry)?);
    }

    Ok(hasher.finalize().to_hex().to_string())
}

fn collect_files_sorted(dir: &Path, entries: &mut Vec<PathBuf>) -> Result<()> {
    let mut listing: Vec<_> = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .collect();
    listing.sort();

    for path in listing {
        if path.is_dir() {
            collect_files_sorted(&path, entries)?;
        } else if path.is_file() {
            entries.push(path);
        }
    }

    Ok(())
}

fn build_archived_assets(scripts: &[PathBuf], references: &[PathBuf]) -> SkillAssets {
    SkillAssets {
        scripts: scripts
            .iter()
            .filter_map(|path| {
                path.file_name().map(|name| ScriptFile {
                    path: PathBuf::from("scripts").join(name),
                    language: "unknown".to_string(),
                    description: None,
                })
            })
            .collect(),
        references: references
            .iter()
            .filter_map(|path| {
                path.file_name().map(|name| ReferenceFile {
                    path: PathBuf::from("references").join(name),
                    file_type: path
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .unwrap_or("file")
                        .to_string(),
                })
            })
            .collect(),
        tests: Vec::new(),
    }
}

#[cfg(test)]
fn copy_assets(sources: &[PathBuf], target: &Path) -> Result<()> {
    if sources.is_empty() {
        return Ok(());
    }
    fs::create_dir_all(target)?;
    for src in sources {
        if let Some(name) = src.file_name() {
            let dst = target.join(name);
            if dst.exists() {
                continue;
            }
            fs::copy(src, &dst)?;
        }
    }
    Ok(())
}

fn canonicalize_provider_references(spec: &mut SkillSpec, same_provider_ids: &HashSet<String>) {
    let provider = spec.metadata.provider.clone();

    if let Some(parent) = spec.extends.as_mut() {
        if !parent.contains('/') && same_provider_ids.contains(parent) {
            *parent = format!("{provider}/{}", parent);
        }
    }

    for include in &mut spec.includes {
        if !include.skill.contains('/') && same_provider_ids.contains(&include.skill) {
            include.skill = format!("{provider}/{}", include.skill);
        }
    }
}

/// Slugify a filename into a valid skill ID.
fn slugify_id(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else if c.is_whitespace() {
                '-'
            } else {
                '_'
            }
        })
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect::<String>()
        .trim_matches('-')
        .trim_matches('_')
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{SkillBlock, SkillSection};
    use tempfile::tempdir;

    fn create_skill_md(dir: &Path, name: &str, content: &str) -> PathBuf {
        let skill_dir = dir.join(name);
        fs::create_dir_all(&skill_dir).unwrap();
        let path = skill_dir.join("SKILL.md");
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn test_find_skill_md_files() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("skills");
        fs::create_dir_all(&root).unwrap();

        create_skill_md(&root, "skill-a", "# Skill A\n\nSome content");
        create_skill_md(&root, "skill-b", "# Skill B\n\nOther content");

        let files = find_skill_md_files(&root).unwrap();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_find_skill_md_files_empty() {
        let dir = tempdir().unwrap();
        let files = find_skill_md_files(dir.path()).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn test_find_skill_md_files_nonexistent() {
        let dir = tempdir().unwrap();
        let files = find_skill_md_files(&dir.path().join("nonexistent")).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn test_slugify_id() {
        assert_eq!(slugify_id("My Skill"), "my-skill");
        assert_eq!(slugify_id("hello world!"), "hello-world");
        assert_eq!(slugify_id("  spaces  "), "spaces");
        assert_eq!(slugify_id("already-kebab"), "already-kebab");
    }

    #[test]
    fn test_compute_dir_hash() {
        let dir_a = tempdir().unwrap();
        let dir_b = tempdir().unwrap();
        let dir_c = tempdir().unwrap();

        fs::write(dir_a.path().join("SKILL.md"), "hello").unwrap();
        fs::write(dir_b.path().join("SKILL.md"), "hello").unwrap();
        fs::write(dir_c.path().join("SKILL.md"), "world").unwrap();

        let h1 = compute_dir_hash(dir_a.path()).unwrap();
        let h2 = compute_dir_hash(dir_b.path()).unwrap();
        let h3 = compute_dir_hash(dir_c.path()).unwrap();
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
        assert_eq!(h1.len(), 64); // Blake3 hex
    }

    #[test]
    fn test_provider_roots_keep_agents_and_codex_distinct() {
        assert!(PROVIDER_ROOTS.contains(&(".agents/skills", "agents")));
        assert!(PROVIDER_ROOTS.contains(&(".codex/skills", "codex")));
    }

    #[test]
    fn test_discover_from_root_with_valid_skill() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("skills");
        fs::create_dir_all(&root).unwrap();

        create_skill_md(
            &root,
            "test-skill",
            "# Test Skill\n\n## Rules\n\n- Always test\n",
        );

        let skills = ProviderDiscovery::discover_from_root(&root, "claude").unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].provider, "claude");
    }

    #[test]
    fn test_discover_skips_non_skill_files() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("skills");
        fs::create_dir_all(&root).unwrap();

        // Create a regular file (not SKILL.md)
        let other = root.join("other.md");
        fs::write(&other, "# Not a skill").unwrap();

        let skills = ProviderDiscovery::discover_from_root(&root, "claude").unwrap();
        assert!(skills.is_empty());
    }

    #[test]
    fn test_render_skill_body_with_sections() {
        let mut spec = SkillSpec::new("test", "Test Skill");
        spec.sections.push(SkillSection {
            id: "rules".to_string(),
            title: "Rules".to_string(),
            blocks: vec![SkillBlock {
                id: "rule-1".to_string(),
                block_type: BlockType::Rule,
                content: "Always test".to_string(),
            }],
        });
        spec.sections.push(SkillSection {
            id: "examples".to_string(),
            title: "Examples".to_string(),
            blocks: vec![SkillBlock {
                id: "ex-1".to_string(),
                block_type: BlockType::Code,
                content: "fn main() {}".to_string(),
            }],
        });

        let body = render_skill_body(&spec);
        assert!(body.contains("Test Skill"));
        assert!(body.contains("Always test"));
        assert!(body.contains("fn main()"));
    }

    #[test]
    fn test_copy_assets() {
        let src_dir = tempdir().unwrap();
        let dst_dir = tempdir().unwrap();

        let asset_file = src_dir.path().join("script.sh");
        fs::write(&asset_file, "#!/bin/sh").unwrap();

        let sources = vec![asset_file];
        copy_assets(&sources, dst_dir.path()).unwrap();

        assert!(dst_dir.path().join("script.sh").exists());
    }

    #[test]
    fn test_copy_assets_skips_existing() {
        let src_dir = tempdir().unwrap();
        let dst_dir = tempdir().unwrap();

        let existing = dst_dir.path().join("script.sh");
        fs::write(&existing, "original").unwrap();

        let new_src = src_dir.path().join("script.sh");
        fs::write(&new_src, "overwrite").unwrap();

        copy_assets(&[new_src], dst_dir.path()).unwrap();
        assert_eq!(fs::read_to_string(&existing).unwrap(), "original");
    }

    #[test]
    fn test_find_asset_files() {
        let dir = tempdir().unwrap();
        let scripts = dir.path().join("scripts");
        fs::create_dir_all(&scripts).unwrap();
        fs::write(scripts.join("a.sh"), "").unwrap();
        fs::write(scripts.join("b.sh"), "").unwrap();

        let found = find_asset_files(dir.path(), "scripts");
        assert_eq!(found.len(), 2);

        let missing = find_asset_files(dir.path(), "nonexistent");
        assert!(missing.is_empty());
    }

    #[test]
    fn test_discover_returns_collision_report() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("skills");
        fs::create_dir_all(&root).unwrap();

        create_skill_md(
            &root,
            "rust-errors",
            "# Rust Errors\n\n## Rules\n\n- Handle errors\n",
        );

        let discovery = ProviderDiscovery::with_roots(vec![(root, "claude".into())]);
        let (skills, report) = discovery.discover().unwrap();

        assert_eq!(skills.len(), 1);
        assert!(!report.has_collisions);
        assert!(report.is_empty());
    }

    #[test]
    fn test_collision_report_empty_on_unique_ids() {
        let skills: Vec<(&str, &str)> = vec![
            ("claude", "skill-a"),
            ("claude", "skill-b"),
            ("codex", "skill-c"),
        ];
        let report = detect_collisions(skills);
        assert!(!report.has_collisions);
        assert!(report.is_empty());
    }

    #[test]
    fn test_collision_report_detects_duplicates() {
        let skills: Vec<(&str, &str)> = vec![("claude", "shared-id"), ("codex", "shared-id")];
        let report = detect_collisions(skills);
        assert!(report.has_collisions);
        assert_eq!(report.len(), 1);
        assert!(report.has("shared-id"));

        let collision = report.for_skill_id("shared-id").unwrap();
        assert_eq!(collision.providers.len(), 2);
        assert!(collision.providers.contains(&"claude".to_string()));
        assert!(collision.providers.contains(&"codex".to_string()));
        assert!(
            collision
                .canonical_ids
                .contains(&"claude/shared-id".to_string())
        );
        assert!(
            collision
                .canonical_ids
                .contains(&"codex/shared-id".to_string())
        );
    }

    // ===================== Archive Integrity (bd-28jh) =====================

    #[test]
    fn test_compute_dir_hash_deterministic() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("SKILL.md"), "hello world").unwrap();
        fs::write(dir.path().join("notes.txt"), "some notes").unwrap();

        let hash1 = compute_dir_hash(dir.path()).unwrap();
        let hash2 = compute_dir_hash(dir.path()).unwrap();
        assert_eq!(hash1, hash2, "hash should be deterministic");
        assert_eq!(hash1.len(), 64); // Blake3 hex
    }

    #[test]
    fn test_compute_dir_hash_detects_content_change() {
        let dir_a = tempdir().unwrap();
        let dir_b = tempdir().unwrap();
        fs::write(dir_a.path().join("SKILL.md"), "original").unwrap();
        fs::write(dir_b.path().join("SKILL.md"), "modified").unwrap();

        let hash_a = compute_dir_hash(dir_a.path()).unwrap();
        let hash_b = compute_dir_hash(dir_b.path()).unwrap();
        assert_ne!(hash_a, hash_b, "hash should differ on content change");
    }

    #[test]
    fn test_build_archived_assets_preserves_scripts_and_refs() {
        let scripts: Vec<PathBuf> = vec![
            PathBuf::from("scripts/run.sh"),
            PathBuf::from("scripts/setup.sh"),
        ];
        let refs: Vec<PathBuf> = vec![
            PathBuf::from("references/readme.md"),
            PathBuf::from("references/api.yaml"),
        ];
        let assets = build_archived_assets(&scripts, &refs);

        assert_eq!(assets.scripts.len(), 2);
        assert_eq!(assets.references.len(), 2);
        assert_eq!(assets.tests.len(), 0);

        assert_eq!(assets.scripts[0].path, PathBuf::from("scripts/run.sh"));
        assert_eq!(assets.scripts[1].path, PathBuf::from("scripts/setup.sh"));
        assert_eq!(
            assets.references[0].path,
            PathBuf::from("references/readme.md")
        );
        assert_eq!(
            assets.references[1].path,
            PathBuf::from("references/api.yaml")
        );
    }

    #[test]
    fn test_build_archived_assets_handles_empty() {
        let assets = build_archived_assets(&[], &[]);
        assert!(assets.scripts.is_empty());
        assert!(assets.references.is_empty());
        assert!(assets.tests.is_empty());
    }

    #[test]
    fn test_collect_import_warnings_surfaces_references_guidance() {
        let mut spec = SkillSpec::new("oversized", "Oversized Skill");
        spec.metadata.description = "A provider skill with a very large main body.".to_string();
        spec.metadata.trigger_phrases = vec!["provider import warning".to_string()];
        spec.sections.push(crate::core::skill::SkillSection {
            id: "overview".to_string(),
            title: "Overview".to_string(),
            blocks: vec![crate::core::skill::SkillBlock {
                id: "overview-1".to_string(),
                block_type: BlockType::Text,
                content: "word ".repeat(5_100).trim().to_string(),
            }],
        });

        let skill = DiscoveredSkill {
            provider: "claude".to_string(),
            provider_path: PathBuf::from("/tmp/provider/oversized/SKILL.md"),
            spec,
            scripts: Vec::new(),
            references: Vec::new(),
        };

        let warnings = collect_import_warnings(&skill);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].rule_id, "oversized-skill-md");
        assert!(
            warnings[0]
                .suggestion
                .as_deref()
                .unwrap_or_default()
                .contains("references/")
        );
        assert!(
            warnings[0]
                .suggestion
                .as_deref()
                .unwrap_or_default()
                .contains("trigger:provider import warning")
        );
    }

    #[test]
    fn test_ensure_compact_route_metadata_derives_keywords() {
        let mut spec = SkillSpec::new("route-meta", "Route Meta");
        spec.metadata.description =
            "Advanced Rust error handling patterns for async services.".to_string();
        spec.metadata.tags = vec!["rust".to_string(), "async".to_string()];
        spec.metadata.trigger_phrases = vec!["error handling".to_string()];

        ensure_compact_route_metadata(&mut spec);

        assert!(
            spec.metadata
                .keywords
                .iter()
                .any(|keyword| keyword == "advanced rust error handling patterns async services")
        );
        assert!(
            spec.metadata
                .keywords
                .iter()
                .any(|keyword| keyword == "rust")
        );
        assert!(
            spec.metadata
                .keywords
                .iter()
                .any(|keyword| keyword == "error handling")
        );
    }
}
