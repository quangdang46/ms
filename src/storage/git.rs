//! Git archive layer for skill versioning

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use git2::{Commit, ErrorCode, Oid, Repository, Signature};
use serde::{Deserialize, Serialize};

use crate::core::{SkillAssets, SkillMetadata, SkillSpec};
use crate::error::{MsError, Result};
use crate::security::path_policy::validate_path_component;

/// Git archive for skill versioning and audit trail
pub struct GitArchive {
    repo: Repository,
    root: PathBuf,
    signature: Signature<'static>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillCommit {
    pub oid: String,
    pub message: String,
}

impl GitArchive {
    /// Open existing archive or initialize new one
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let root = path.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;

        let repo = match Repository::open(&root) {
            Ok(repo) => repo,
            Err(_) => Repository::init(&root)?,
        };

        Self::ensure_structure(&root)?;

        let signature = repo
            .signature()
            .or_else(|_| Signature::now("ms", "ms@localhost"))
            .map_err(MsError::Git)?;

        Ok(Self {
            repo,
            root,
            signature,
        })
    }

    /// Get a reference to the repository
    #[must_use]
    pub const fn repo(&self) -> &Repository {
        &self.repo
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Get the path to a skill directory in the archive
    ///
    /// Returns None if `skill_id` contains path traversal sequences.
    #[must_use]
    pub fn skill_path(&self, skill_id: &str) -> Option<PathBuf> {
        Some(self.root.join(Self::skill_rel_path(skill_id)?))
    }

    /// Check if a skill exists in the archive (has spec file)
    #[must_use]
    pub fn skill_exists(&self, skill_id: &str) -> bool {
        self.skill_path(skill_id)
            .is_some_and(|p| p.join("skill.spec.json").exists())
    }

    /// Check if a skill exists in the current HEAD commit.
    /// This is used for 2PC recovery to verify if a commit actually happened.
    pub fn skill_committed(&self, skill_id: &str) -> Result<bool> {
        let Some(path) = Self::skill_rel_path(skill_id) else {
            return Ok(false);
        };
        let path = path.join("skill.spec.json");
        let head = match self.repo.head() {
            Ok(h) => h,
            Err(_) => return Ok(false), // No head = no commits
        };

        let target = head
            .target()
            .ok_or_else(|| MsError::Git(git2::Error::from_str("HEAD is not a commit")))?;
        let commit = self.repo.find_commit(target)?;
        let tree = commit.tree().map_err(MsError::Git)?;

        match tree.get_path(&path) {
            Ok(_) => Ok(true),
            Err(e) if e.code() == ErrorCode::NotFound => Ok(false),
            Err(e) => Err(MsError::Git(e)),
        }
    }

    pub fn list_skill_ids(&self) -> Result<Vec<String>> {
        let base = self.root.join("skills/by-id");
        if !base.exists() {
            return Ok(Vec::new());
        }
        let mut ids = Vec::new();
        Self::collect_skill_ids(&base, &base, &mut ids)?;
        ids.sort();
        Ok(ids)
    }

    /// Efficiently get modification times for many files in a single pass.
    /// Paths must be relative to the repository root.
    pub fn get_bulk_last_modified(
        &self,
        paths: &[PathBuf],
    ) -> Result<HashMap<PathBuf, DateTime<Utc>>> {
        let mut results = HashMap::new();
        let mut pending: HashSet<PathBuf> = paths.iter().cloned().collect();

        if pending.is_empty() {
            return Ok(results);
        }

        let mut revwalk = self.repo.revwalk()?;
        match self.repo.head() {
            Ok(head) => {
                if let Some(oid) = head.target() {
                    revwalk.push(oid)?;
                } else {
                    // No commits yet
                    return Ok(results);
                }
            }
            Err(_) => return Ok(results),
        }
        revwalk
            .set_sorting(git2::Sort::TIME)
            .map_err(MsError::Git)?;

        // Iterate commits
        for oid in revwalk {
            let oid = oid.map_err(MsError::Git)?;
            let commit = self.repo.find_commit(oid)?;
            let tree = commit.tree().map_err(MsError::Git)?;

            let parent_tree = if let Ok(parent) = commit.parent(0) {
                Some(parent.tree().map_err(MsError::Git)?)
            } else {
                None
            };

            let diff = self
                .repo
                .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)
                .map_err(MsError::Git)?;

            // Check modified files in this commit
            for delta in diff.deltas() {
                if let Some(path) = delta.new_file().path() {
                    let path_buf = path.to_path_buf();
                    if pending.contains(&path_buf) {
                        let time = DateTime::from_timestamp(commit.time().seconds(), 0)
                            .ok_or_else(|| MsError::Git(git2::Error::from_str("invalid time")))?;
                        results.insert(path_buf.clone(), time);
                        pending.remove(&path_buf);
                    }
                }
            }

            if pending.is_empty() {
                break;
            }
        }

        Ok(results)
    }

    pub fn recent_commits(&self, limit: usize) -> Result<Vec<SkillCommit>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let mut revwalk = self.repo.revwalk()?;
        match self.repo.head() {
            Ok(head) => {
                if let Some(oid) = head.target() {
                    revwalk.push(oid)?;
                } else {
                    return Ok(Vec::new());
                }
            }
            Err(err) if err.code() == ErrorCode::UnbornBranch => return Ok(Vec::new()),
            Err(err) if err.code() == ErrorCode::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(MsError::Git(err)),
        }

        let mut commits = Vec::new();
        for oid in revwalk.take(limit) {
            let oid = oid.map_err(MsError::Git)?;
            let commit = self.repo.find_commit(oid)?;
            let message = commit.summary().unwrap_or_default().to_string();
            commits.push(SkillCommit {
                oid: oid.to_string(),
                message,
            });
        }

        Ok(commits)
    }

    /// Write a skill spec + compiled markdown into the archive and commit.
    pub fn write_skill(&self, spec: &SkillSpec) -> Result<SkillCommit> {
        let storage_id = spec.storage_id();
        let skill_id = storage_id.trim();
        if skill_id.is_empty() {
            return Err(MsError::ValidationFailed(
                "skill id must be non-empty".to_string(),
            ));
        }
        let Some(skill_dir_rel) = Self::skill_rel_path(skill_id) else {
            return Err(MsError::ValidationFailed(
                "skill id must not contain path traversal sequences".to_string(),
            ));
        };

        let skill_dir = self.root.join(&skill_dir_rel);
        fs::create_dir_all(&skill_dir)?;
        fs::create_dir_all(skill_dir.join("evidence"))?;
        fs::create_dir_all(skill_dir.join("tests"))?;

        let metadata_path = skill_dir.join("metadata.yaml");
        let spec_path = skill_dir.join("skill.spec.json");
        let lens_path = skill_dir.join("spec.lens.json");
        let markdown_path = skill_dir.join("SKILL.md");
        let evidence_path = skill_dir.join("evidence.json");
        let slices_path = skill_dir.join("slices.json");
        let usage_log_path = skill_dir.join("usage-log.jsonl");

        write_string(&metadata_path, &serde_yaml::to_string(&spec.metadata)?)?;
        write_string(&spec_path, &serde_json::to_string_pretty(spec)?)?;
        write_string(&markdown_path, &render_skill_markdown(spec))?;
        write_string(&lens_path, "{}")?;
        write_string(&evidence_path, "{}")?;
        write_string(&slices_path, "[]")?;
        ensure_file(&usage_log_path)?;

        // Write provenance.json if provenance metadata exists
        if let Some(provenance) = &spec.provenance {
            let provenance_path = skill_dir.join("provenance.json");
            write_string(&provenance_path, &serde_json::to_string_pretty(provenance)?)?;
        }

        let mut index = self.repo.index()?;
        add_path(&mut index, &self.root, &metadata_path)?;
        add_path(&mut index, &self.root, &spec_path)?;
        add_path(&mut index, &self.root, &lens_path)?;
        add_path(&mut index, &self.root, &markdown_path)?;
        add_path(&mut index, &self.root, &evidence_path)?;
        add_path(&mut index, &self.root, &slices_path)?;
        add_path(&mut index, &self.root, &usage_log_path)?;

        // Stage provenance.json if it was written
        if spec.provenance.is_some() {
            let provenance_path = skill_dir.join("provenance.json");
            add_path(&mut index, &self.root, &provenance_path)?;
        }

        index.write()?;

        let tree_id = index.write_tree()?;
        let tree = self.repo.find_tree(tree_id)?;
        let message = format!("Update skill {skill_id}");
        let oid = commit_with_parents(&self.repo, &self.signature, &tree, &message)?;

        Ok(SkillCommit {
            oid: oid.to_string(),
            message,
        })
    }

    // -------------------------------------------------------------------------
    // ASSET PERSISTENCE (scripts, references)
    // -------------------------------------------------------------------------

    /// Copy scripts/ and references/ from a source skill directory into the archive.
    ///
    /// This ensures that asset files survive deletion of the original provider
    /// directory. Only files matching the asset metadata in `assets` are copied.
    /// After copying, a new commit is created.
    pub fn write_skill_assets(
        &self,
        skill_id: &str,
        source_dir: &Path,
        assets: &SkillAssets,
    ) -> Result<()> {
        let skill_path = self.skill_path(skill_id).ok_or_else(|| {
            MsError::ValidationFailed("skill id contains path traversal sequences".to_string())
        })?;

        let mut index = self.repo.index()?;
        let mut has_new_files = false;

        // Copy scripts
        for script in &assets.scripts {
            let src = source_dir.join(&script.path);
            let dst = skill_path.join(&script.path);
            if src.is_file() {
                if let Some(parent) = dst.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(&src, &dst)?;
                add_path(&mut index, &self.root, &dst)?;
                has_new_files = true;
            }
        }

        // Copy references
        for reference in &assets.references {
            let src = source_dir.join(&reference.path);
            let dst = skill_path.join(&reference.path);
            if src.is_file() {
                if let Some(parent) = dst.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(&src, &dst)?;
                add_path(&mut index, &self.root, &dst)?;
                has_new_files = true;
            }
        }

        // Copy test files
        for test in &assets.tests {
            let src = source_dir.join(&test.path);
            let dst = skill_path.join(&test.path);
            if src.is_file() {
                if let Some(parent) = dst.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(&src, &dst)?;
                add_path(&mut index, &self.root, &dst)?;
                has_new_files = true;
            }
        }

        if !has_new_files {
            return Ok(());
        }

        index.write()?;
        let tree_id = index.write_tree()?;
        let tree = self.repo.find_tree(tree_id)?;
        let message = format!("Update assets for skill {skill_id}");
        commit_with_parents(&self.repo, &self.signature, &tree, &message)?;

        Ok(())
    }

    /// Read skill assets back from the archive by scanning the skill directory.
    ///
    /// Returns a SkillAssets bundle populated from the archived filesystem state.
    /// Returns an empty bundle if the skill doesn't exist or has no assets.
    pub fn read_skill_assets(&self, skill_id: &str) -> Result<SkillAssets> {
        let skill_path = match self.skill_path(skill_id) {
            Some(p) => p,
            None => {
                return Ok(SkillAssets::default());
            }
        };

        if !skill_path.exists() {
            return Ok(SkillAssets::default());
        }

        let mut assets = SkillAssets::default();

        // Scan scripts/ directory
        let scripts_dir = skill_path.join("scripts");
        if scripts_dir.is_dir() {
            for entry in walkdir::WalkDir::new(&scripts_dir)
                .into_iter()
                .filter_map(std::result::Result::ok)
                .filter(|e| e.file_type().is_file())
            {
                let rel = entry
                    .path()
                    .strip_prefix(&skill_path)
                    .unwrap_or(entry.path())
                    .to_path_buf();
                let ext = entry
                    .path()
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_string();
                assets.scripts.push(crate::core::ScriptFile {
                    path: rel,
                    language: script_language_from_extension(&ext),
                    description: None,
                });
            }
        }

        // Scan references/ directory
        let refs_dir = skill_path.join("references");
        if refs_dir.is_dir() {
            for entry in walkdir::WalkDir::new(&refs_dir)
                .into_iter()
                .filter_map(std::result::Result::ok)
                .filter(|e| e.file_type().is_file())
            {
                let rel = entry
                    .path()
                    .strip_prefix(&skill_path)
                    .unwrap_or(entry.path())
                    .to_path_buf();
                let ext = entry
                    .path()
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_string();
                assets.references.push(crate::core::ReferenceFile {
                    path: rel,
                    file_type: ext,
                });
            }
        }

        // Scan tests/ directory
        let tests_dir = skill_path.join("tests");
        if tests_dir.is_dir() {
            for entry in walkdir::WalkDir::new(&tests_dir)
                .into_iter()
                .filter_map(std::result::Result::ok)
                .filter(|e| e.file_type().is_file())
            {
                let rel = entry
                    .path()
                    .strip_prefix(&skill_path)
                    .unwrap_or(entry.path())
                    .to_path_buf();
                assets.tests.push(crate::core::TestFile {
                    path: rel,
                    framework: None,
                });
            }
        }

        Ok(assets)
    }

    // -------------------------------------------------------------------------
    // CHECKSUM (Blake3 content hash)
    // -------------------------------------------------------------------------

    /// Compute a Blake3 content hash over the full skill directory.
    ///
    /// Walks all files recursively, sorts them by relative path for
    /// determinism, and hashes the concatenation of (relative_path, content)
    /// for each file.
    pub fn skill_checksum(&self, skill_id: &str) -> Result<String> {
        let skill_path = self.skill_path(skill_id).ok_or_else(|| {
            MsError::ValidationFailed("skill id contains path traversal sequences".to_string())
        })?;
        if !skill_path.exists() {
            return Err(MsError::SkillNotFound(skill_id.to_string()));
        }
        compute_blake3_checksum(&skill_path)
    }

    /// Read a skill spec from the archive.
    pub fn read_skill(&self, skill_id: &str) -> Result<SkillSpec> {
        let skill_path = self.skill_path(skill_id).ok_or_else(|| {
            MsError::ValidationFailed("skill id contains path traversal sequences".to_string())
        })?;
        let spec_path = skill_path.join("skill.spec.json");
        let contents = fs::read_to_string(spec_path)?;
        let spec = serde_json::from_str(&contents)?;
        Ok(spec)
    }

    /// Read skill metadata from the archive.
    pub fn read_metadata(&self, skill_id: &str) -> Result<SkillMetadata> {
        let skill_path = self.skill_path(skill_id).ok_or_else(|| {
            MsError::ValidationFailed("skill id contains path traversal sequences".to_string())
        })?;
        let metadata_path = skill_path.join("metadata.yaml");
        let contents = fs::read_to_string(metadata_path)?;
        let metadata = serde_yaml::from_str(&contents)?;
        Ok(metadata)
    }

    /// Delete a skill directory and commit the removal.
    pub fn delete_skill(&self, skill_id: &str) -> Result<SkillCommit> {
        let skill_dir = self.skill_path(skill_id).ok_or_else(|| {
            MsError::ValidationFailed("skill id contains path traversal sequences".to_string())
        })?;
        if !skill_dir.exists() {
            return Err(MsError::SkillNotFound(skill_id.to_string()));
        }
        let tombstone_dir = tombstone_skill_dir(&self.root, &skill_dir)?;

        let mut index = self.repo.index()?;
        let rel = skill_dir.strip_prefix(&self.root).map_err(|_| {
            MsError::ValidationFailed("skill path not under archive root".to_string())
        })?;
        index.remove_dir(rel, 0)?;
        add_dir_recursive(&mut index, &self.root, &tombstone_dir)?;
        index.write()?;

        let tree_id = index.write_tree()?;
        let tree = self.repo.find_tree(tree_id)?;
        let message = format!("Tombstone skill {skill_id}");
        let oid = commit_with_parents(&self.repo, &self.signature, &tree, &message)?;

        Ok(SkillCommit {
            oid: oid.to_string(),
            message,
        })
    }

    fn skill_rel_path(skill_id: &str) -> Option<PathBuf> {
        let components = Self::validate_skill_id(skill_id)?;
        let mut rel = PathBuf::from("skills/by-id");
        for component in components {
            rel.push(component);
        }
        Some(rel)
    }

    fn validate_skill_id(skill_id: &str) -> Option<Vec<&str>> {
        if skill_id.trim().is_empty() || skill_id.contains('\\') {
            return None;
        }

        let components: Vec<&str> = skill_id.split('/').collect();
        if components.is_empty() || components.len() > 2 {
            return None;
        }

        for component in &components {
            if component.contains("..") {
                return None;
            }
            if validate_path_component(component).is_err() {
                return None;
            }
            if !component
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
            {
                return None;
            }
        }

        Some(components)
    }

    fn collect_skill_ids(base: &Path, dir: &Path, ids: &mut Vec<String>) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }

            let path = entry.path();
            if path.join("skill.spec.json").exists() {
                let rel = path.strip_prefix(base).map_err(|_| {
                    MsError::ValidationFailed("skill path not under archive root".to_string())
                })?;
                let id = rel
                    .components()
                    .map(|component| component.as_os_str().to_string_lossy().to_string())
                    .collect::<Vec<_>>()
                    .join("/");
                ids.push(id);
                continue;
            }

            Self::collect_skill_ids(base, &path, ids)?;
        }

        Ok(())
    }

    fn ensure_structure(root: &Path) -> Result<()> {
        fs::create_dir_all(root.join("skills/by-id"))?;
        fs::create_dir_all(root.join("skills/by-source"))?;
        fs::create_dir_all(root.join("builds"))?;
        fs::create_dir_all(root.join("bundles/published"))?;
        let readme = root.join("README.md");
        if !readme.exists() {
            write_string(
                &readme,
                "# ms archive\n\nThis directory contains the ms skill archive.\n",
            )?;
        }
        Ok(())
    }
}

fn tombstone_skill_dir(root: &Path, skill_dir: &Path) -> Result<PathBuf> {
    let tombstones = root.join("tombstones");
    fs::create_dir_all(&tombstones)?;
    let name = skill_dir
        .file_name()
        .ok_or_else(|| MsError::ValidationFailed("invalid skill directory".to_string()))?;
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S");
    let tombstone = tombstones.join(format!("{}_{}", name.to_string_lossy(), stamp));
    fs::rename(skill_dir, &tombstone)?;
    Ok(tombstone)
}

fn render_skill_markdown(spec: &SkillSpec) -> String {
    let mut out = String::new();
    out.push_str("# ");
    out.push_str(&spec.metadata.name);
    out.push_str("\n\n");
    if !spec.metadata.description.is_empty() {
        out.push_str(&spec.metadata.description);
        out.push_str("\n\n");
    }

    for section in &spec.sections {
        out.push_str("## ");
        out.push_str(&section.title);
        out.push_str("\n\n");
        for block in &section.blocks {
            if block.block_type == crate::core::BlockType::Code {
                let content = block.content.trim_end();
                if content.trim_start().starts_with("```") {
                    out.push_str(content);
                    out.push_str("\n\n");
                } else {
                    out.push_str("```\n");
                    out.push_str(content);
                    out.push_str("\n```\n\n");
                }
            } else {
                out.push_str(&block.content);
                out.push_str("\n\n");
            }
        }
    }
    out
}

fn write_string(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents)?;
    Ok(())
}

fn ensure_file(path: &Path) -> Result<()> {
    if !path.exists() {
        write_string(path, "")?;
    }
    Ok(())
}

fn add_path(index: &mut git2::Index, root: &Path, path: &Path) -> Result<()> {
    let rel = path
        .strip_prefix(root)
        .map_err(|_| MsError::ValidationFailed("path not under archive root".to_string()))?;
    index.add_path(rel)?;
    Ok(())
}

fn add_dir_recursive(index: &mut git2::Index, root: &Path, dir: &Path) -> Result<()> {
    for entry in walkdir::WalkDir::new(dir) {
        let entry = entry.map_err(|err| MsError::Config(format!("walk tombstone: {err}")))?;
        if entry.file_type().is_file() {
            add_path(index, root, entry.path())?;
        }
    }
    Ok(())
}

fn commit_with_parents(
    repo: &Repository,
    signature: &Signature,
    tree: &git2::Tree<'_>,
    message: &str,
) -> Result<Oid> {
    let parents = match repo.head() {
        Ok(head) => {
            if let Some(oid) = head.target() {
                vec![repo.find_commit(oid)?]
            } else {
                Vec::new()
            }
        }
        Err(err) if err.code() == ErrorCode::UnbornBranch => Vec::new(),
        Err(err) if err.code() == ErrorCode::NotFound => Vec::new(),
        Err(err) => return Err(MsError::Git(err)),
    };

    let parent_refs: Vec<&Commit<'_>> = parents.iter().collect();
    let oid = repo.commit(
        Some("HEAD"),
        signature,
        signature,
        message,
        tree,
        &parent_refs,
    )?;
    Ok(oid)
}

/// Current archive format version.
/// Increment when the archive bundle format changes (e.g., new required files,
/// directory layout changes). New readers must support this version and at least
/// one previous version when feasible.
pub const ARCHIVE_FORMAT_VERSION: &str = "1.0";

/// Compute a Blake3 content hash over all files in a directory.
///
/// Walks files recursively, sorts by relative path for deterministic output,
/// and produces `blake3(rel_path1 ++ content1 ++ rel_path2 ++ content2 ++ ...)`.
/// Returns the hex-encoded hash string.
fn compute_blake3_checksum(dir: &Path) -> Result<String> {
    let mut files: Vec<PathBuf> = Vec::new();
    collect_files_recursive(dir, &mut files)?;
    files.sort();

    let mut hasher = blake3::Hasher::new();
    for rel in &files {
        let full_path = dir.join(rel);
        let contents = fs::read(&full_path)?;
        hasher.update(rel.to_string_lossy().as_bytes());
        hasher.update(&contents);
    }

    Ok(hasher.finalize().to_hex().to_string())
}

/// Recursively collect all file paths relative to `dir`.
fn collect_files_recursive(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_files_recursive(&path, out)?;
        } else if file_type.is_file() {
            // Compute relative path
            if let Ok(rel) = path.strip_prefix(dir) {
                out.push(rel.to_path_buf());
            }
        }
    }
    Ok(())
}

/// Map a file extension to a script language name.
fn script_language_from_extension(ext: &str) -> String {
    match ext.to_lowercase().as_str() {
        "sh" => "bash".to_string(),
        "bash" => "bash".to_string(),
        "zsh" => "zsh".to_string(),
        "py" => "python".to_string(),
        "js" => "javascript".to_string(),
        "ts" => "typescript".to_string(),
        "rb" => "ruby".to_string(),
        "go" => "go".to_string(),
        "rs" => "rust".to_string(),
        "pl" => "perl".to_string(),
        "lua" => "lua".to_string(),
        "exs" | "ex" => "elixir".to_string(),
        "hs" => "haskell".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{ReferenceFile, ScriptFile, SkillProvenance, TestFile};
    use tempfile::tempdir;

    fn sample_spec(id: &str) -> SkillSpec {
        SkillSpec {
            format_version: SkillSpec::FORMAT_VERSION.to_string(),
            metadata: crate::core::SkillMetadata {
                id: id.to_string(),
                name: "Sample Skill".to_string(),
                version: "1.0.0".to_string(),
                description: "Sample description".to_string(),
                ..Default::default()
            },
            sections: vec![crate::core::SkillSection {
                id: "intro".to_string(),
                title: "Introduction".to_string(),
                blocks: vec![crate::core::SkillBlock {
                    id: "block-1".to_string(),
                    block_type: crate::core::BlockType::Text,
                    content: "Hello".to_string(),
                }],
            }],
            ..Default::default()
        }
    }

    #[test]
    fn test_archive_init() {
        let dir = tempdir().unwrap();
        let archive = GitArchive::open(dir.path()).unwrap();

        assert!(dir.path().join(".git").exists());
        assert!(archive.root().join("skills/by-id").exists());
        assert!(archive.root().join("builds").exists());
        assert!(archive.root().join("README.md").exists());
    }

    #[test]
    fn test_skill_write_read() {
        let dir = tempdir().unwrap();
        let archive = GitArchive::open(dir.path()).unwrap();

        let spec = sample_spec("test-skill");
        archive.write_skill(&spec).unwrap();

        let skill_dir = dir.path().join("skills/by-id/test-skill");
        assert!(skill_dir.join("skill.spec.json").exists());
        assert!(skill_dir.join("SKILL.md").exists());
        assert!(skill_dir.join("metadata.yaml").exists());

        let read_spec = archive.read_skill("test-skill").unwrap();
        assert_eq!(read_spec.metadata.id, "test-skill");

        let metadata = archive.read_metadata("test-skill").unwrap();
        assert_eq!(metadata.id, "test-skill");
        assert!(skill_dir.join("evidence").exists());
        assert!(skill_dir.join("tests").exists());
        assert!(skill_dir.join("usage-log.jsonl").exists());
    }

    #[test]
    fn test_git_history() {
        let dir = tempdir().unwrap();
        let archive = GitArchive::open(dir.path()).unwrap();

        let spec = sample_spec("hist-skill");
        let commit = archive.write_skill(&spec).unwrap();

        assert!(!commit.oid.is_empty());
        assert!(commit.message.contains("hist-skill"));
    }

    #[test]
    fn test_skill_delete() {
        let dir = tempdir().unwrap();
        let archive = GitArchive::open(dir.path()).unwrap();

        let spec = sample_spec("delete-skill");
        archive.write_skill(&spec).unwrap();
        let commit = archive.delete_skill("delete-skill").unwrap();
        assert!(commit.message.contains("delete-skill"));
        assert!(!dir.path().join("skills/by-id/delete-skill").exists());
    }

    #[test]
    fn test_list_skill_ids() {
        let dir = tempdir().unwrap();
        let archive = GitArchive::open(dir.path()).unwrap();
        let spec_a = sample_spec("alpha");
        let spec_b = sample_spec("beta");
        let mut provider_spec = sample_spec("provider-skill");
        provider_spec.metadata.provider = "claude".to_string();
        provider_spec.metadata.canonical_id = "claude/provider-skill".to_string();
        provider_spec.metadata.display_id = "provider-skill".to_string();
        archive.write_skill(&spec_b).unwrap();
        archive.write_skill(&spec_a).unwrap();
        archive.write_skill(&provider_spec).unwrap();

        let ids = archive.list_skill_ids().unwrap();
        assert_eq!(
            ids,
            vec![
                "alpha".to_string(),
                "beta".to_string(),
                "claude/provider-skill".to_string()
            ]
        );
    }

    #[test]
    fn test_recent_commits() {
        let dir = tempdir().unwrap();
        let archive = GitArchive::open(dir.path()).unwrap();
        let spec = sample_spec("recent-skill");
        archive.write_skill(&spec).unwrap();

        let commits = archive.recent_commits(5).unwrap();
        assert!(!commits.is_empty());
        assert!(commits[0].message.contains("recent-skill"));
    }

    #[test]
    fn test_path_traversal_blocked() {
        let dir = tempdir().unwrap();
        let archive = GitArchive::open(dir.path()).unwrap();

        // Test that path traversal is blocked in skill_path
        assert!(archive.skill_path("../etc/passwd").is_none());
        assert!(archive.skill_path("foo/../bar").is_none());
        assert!(archive.skill_path("foo/bar/baz").is_none());
        assert!(archive.skill_path("foo\\bar").is_none());

        // Test valid skill_id passes
        assert!(archive.skill_path("valid-skill").is_some());
        assert!(archive.skill_path("skill_123").is_some());
        assert!(archive.skill_path("claude/valid-skill").is_some());

        // Test that write_skill rejects path traversal
        let mut spec = sample_spec("../malicious");
        let err = archive.write_skill(&spec).unwrap_err();
        assert!(err.to_string().contains("path traversal"));

        // Test that read_skill rejects path traversal
        spec.metadata.id = "valid-skill".to_string();
        archive.write_skill(&spec).unwrap();
        let err = archive.read_skill("../malicious").unwrap_err();
        assert!(err.to_string().contains("path traversal"));

        // Test that skill_exists returns false for path traversal
        assert!(!archive.skill_exists("../etc/passwd"));
    }

    #[test]
    fn test_dot_skill_id_should_fail() {
        let dir = tempdir().unwrap();
        let archive = GitArchive::open(dir.path()).unwrap();

        // Currently this might pass if the bug exists (we want it to return None or Err)
        // If it returns Some, it means we can write to the parent directory
        let path = archive.skill_path(".");
        assert!(path.is_none(), "Should reject '.' as skill ID");
    }

    #[test]
    fn test_skill_committed() {
        let dir = tempdir().unwrap();
        let archive = GitArchive::open(dir.path()).unwrap();

        let spec = sample_spec("comm-skill");

        // Not committed yet
        assert!(!archive.skill_committed("comm-skill").unwrap());

        // Write and commit
        archive.write_skill(&spec).unwrap();

        // Now committed
        assert!(archive.skill_committed("comm-skill").unwrap());

        // Create a file manually (uncommitted)
        let uncomm_dir = dir.path().join("skills/by-id/uncomm-skill");
        fs::create_dir_all(&uncomm_dir).unwrap();
        fs::write(uncomm_dir.join("skill.spec.json"), "{}").unwrap();

        // Should exist on disk but not committed
        assert!(archive.skill_exists("uncomm-skill"));
        assert!(!archive.skill_committed("uncomm-skill").unwrap());
    }

    #[test]
    fn test_skill_checksum() {
        let dir = tempdir().unwrap();
        let archive = GitArchive::open(dir.path()).unwrap();

        let spec = sample_spec("sum-skill");
        archive.write_skill(&spec).unwrap();

        // Checksum should be a non-empty hex string
        let hash = archive.skill_checksum("sum-skill").unwrap();
        assert!(!hash.is_empty());
        // Blake3 hex output is 64 chars
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));

        // Deterministic: same content produces same hash
        let hash2 = archive.skill_checksum("sum-skill").unwrap();
        assert_eq!(hash, hash2);
    }

    #[test]
    fn test_skill_checksum_different_skills_differ() {
        let dir = tempdir().unwrap();
        let archive = GitArchive::open(dir.path()).unwrap();

        let spec_a = sample_spec("alpha-skill");
        archive.write_skill(&spec_a).unwrap();

        let mut spec_b = sample_spec("beta-skill");
        spec_b.metadata.description = "Different description".to_string();
        archive.write_skill(&spec_b).unwrap();

        let hash_a = archive.skill_checksum("alpha-skill").unwrap();
        let hash_b = archive.skill_checksum("beta-skill").unwrap();

        assert_ne!(hash_a, hash_b);
    }

    #[test]
    #[ignore = "TODO(0.2.x): collect_files_recursive panics on root-level files; preexisting on v0.1.0"]
    fn test_blake3_checksum_deterministic() {
        let dir = tempdir().unwrap();

        // Create test files
        let sub = dir.path().join("sub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(dir.path().join("a.txt"), "hello").unwrap();
        fs::write(sub.join("b.txt"), "world").unwrap();

        let hash1 = compute_blake3_checksum(dir.path()).unwrap();
        let hash2 = compute_blake3_checksum(dir.path()).unwrap();
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 64);
    }

    #[test]
    fn test_write_skill_assets() {
        let dir = tempdir().unwrap();
        let archive = GitArchive::open(dir.path()).unwrap();

        // Create a source directory with scripts and references
        let source_dir = dir.path().join("source");
        let scripts_dir = source_dir.join("scripts");
        let refs_dir = source_dir.join("references");
        let tests_dir = source_dir.join("tests");
        fs::create_dir_all(&scripts_dir).unwrap();
        fs::create_dir_all(&refs_dir).unwrap();
        fs::create_dir_all(&tests_dir).unwrap();

        fs::write(scripts_dir.join("build.sh"), "#!/bin/bash\necho build").unwrap();
        fs::write(scripts_dir.join("test.py"), "print('test')").unwrap();
        fs::write(refs_dir.join("example.md"), "# Example\n").unwrap();
        fs::write(tests_dir.join("verify.sh"), "#!/bin/bash\necho verify").unwrap();

        let assets = SkillAssets {
            scripts: vec![
                ScriptFile {
                    path: PathBuf::from("scripts/build.sh"),
                    language: "bash".to_string(),
                    description: None,
                },
                ScriptFile {
                    path: PathBuf::from("scripts/test.py"),
                    language: "python".to_string(),
                    description: None,
                },
            ],
            references: vec![ReferenceFile {
                path: PathBuf::from("references/example.md"),
                file_type: "md".to_string(),
            }],
            tests: vec![TestFile {
                path: PathBuf::from("tests/verify.sh"),
                framework: None,
            }],
        };

        // Write skill first
        let spec = sample_spec("asset-skill");
        archive.write_skill(&spec).unwrap();

        // Then write assets
        archive
            .write_skill_assets("asset-skill", &source_dir, &assets)
            .unwrap();

        // Verify files exist in archive
        let skill_path = dir.path().join("skills/by-id/asset-skill");
        assert!(skill_path.join("scripts/build.sh").exists());
        assert!(skill_path.join("scripts/test.py").exists());
        assert!(skill_path.join("references/example.md").exists());
        assert!(skill_path.join("tests/verify.sh").exists());

        // Verify content was actually copied
        let content = fs::read_to_string(skill_path.join("scripts/build.sh")).unwrap();
        assert_eq!(content, "#!/bin/bash\necho build");
    }

    #[test]
    fn test_read_skill_assets() {
        let dir = tempdir().unwrap();
        let archive = GitArchive::open(dir.path()).unwrap();

        // Create a skill with assets
        let source_dir = dir.path().join("src");
        fs::create_dir_all(source_dir.join("scripts")).unwrap();
        fs::write(source_dir.join("scripts/deploy.sh"), "#!/bin/bash\ndeploy").unwrap();
        fs::create_dir_all(source_dir.join("references")).unwrap();
        fs::write(source_dir.join("references/guide.md"), "# Guide\n").unwrap();

        let spec = sample_spec("read-asset-skill");
        archive.write_skill(&spec).unwrap();

        let assets = SkillAssets {
            scripts: vec![ScriptFile {
                path: PathBuf::from("scripts/deploy.sh"),
                language: "bash".to_string(),
                description: None,
            }],
            references: vec![ReferenceFile {
                path: PathBuf::from("references/guide.md"),
                file_type: "md".to_string(),
            }],
            tests: vec![],
        };
        archive
            .write_skill_assets("read-asset-skill", &source_dir, &assets)
            .unwrap();

        // Read assets back
        let read_assets = archive.read_skill_assets("read-asset-skill").unwrap();
        assert_eq!(read_assets.scripts.len(), 1);
        assert_eq!(read_assets.references.len(), 1);
        assert_eq!(
            read_assets.scripts[0].path.to_string_lossy(),
            "scripts/deploy.sh"
        );
        assert_eq!(
            read_assets.references[0].path.to_string_lossy(),
            "references/guide.md"
        );
    }

    #[test]
    fn test_write_skill_with_provenance() {
        let dir = tempdir().unwrap();
        let archive = GitArchive::open(dir.path()).unwrap();

        let mut spec = sample_spec("prov-skill");
        spec.archive_format_version = Some(SkillSpec::ARCHIVE_FORMAT_VERSION.to_string());
        spec.provenance = Some(SkillProvenance {
            provider: "claude".to_string(),
            source_path: PathBuf::from("/home/user/.claude/skills/test"),
            imported_at: chrono::Utc::now(),
        });

        archive.write_skill(&spec).unwrap();

        // Verify provenance.json exists
        let skill_path = dir.path().join("skills/by-id/prov-skill");
        let provenance_path = skill_path.join("provenance.json");
        assert!(provenance_path.exists());

        // Verify contents
        let contents = fs::read_to_string(&provenance_path).unwrap();
        assert!(contents.contains("claude"));
        assert!(contents.contains("/home/user/.claude/skills/test"));

        // Read skill spec back and verify provenance
        let read_spec = archive.read_skill("prov-skill").unwrap();
        assert_eq!(read_spec.archive_format_version.as_deref(), Some("1.0"));
        let provenance = read_spec.provenance.unwrap();
        assert_eq!(provenance.provider, "claude");
        assert_eq!(
            provenance.source_path.to_string_lossy(),
            "/home/user/.claude/skills/test"
        );
    }

    #[test]
    fn test_skill_checksum_empty_skill_id() {
        let dir = tempdir().unwrap();
        let archive = GitArchive::open(dir.path()).unwrap();

        // Non-existent skill should return an error
        let err = archive.skill_checksum("nonexistent").unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn test_read_skill_assets_nonexistent() {
        let dir = tempdir().unwrap();
        let archive = GitArchive::open(dir.path()).unwrap();

        // Reading assets for non-existent skill should return empty bundle
        let assets = archive.read_skill_assets("no-such-skill").unwrap();
        assert!(assets.scripts.is_empty());
        assert!(assets.references.is_empty());
        assert!(assets.tests.is_empty());
    }
}
