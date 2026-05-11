use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::path::Path;

use crate::error::{MsError, Result};

/// The default provider name for skills loaded from the built-in "local" layer.
pub const DEFAULT_PROVIDER: &str = "local";

/// A validated canonical skill ID in the form `provider/name`.
///
/// The provider and name segments are constrained to a safe character set:
/// alphanumeric plus `-`, `_`, and `.`. No leading `/` or trailing `/`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CanonicalId {
    pub provider: String,
    pub name: String,
    raw: String,
}

impl CanonicalId {
    /// Create a canonical ID from provider and name.
    ///
    /// Returns an error if either segment contains invalid characters or is empty.
    pub fn new(provider: &str, name: &str) -> Result<Self> {
        let p = sanitize_segment(provider);
        let n = sanitize_segment(name);
        if p.is_empty() || n.is_empty() {
            return Err(MsError::InvalidSkill("provider and name must be non-empty".to_string()));
        }
        let raw = format!("{}/{}", p, n);
        Ok(Self {
            provider: p,
            name: n,
            raw,
        })
    }

    /// Parse a canonical ID from a raw string.
    ///
    /// Accepts `provider/name`, `provider-name`, or plain `name`.
    /// For plain `name`, the default provider (`local`) is used.
    pub fn parse(raw: &str) -> Result<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(MsError::InvalidSkill("canonical id cannot be empty".to_string()));
        }
        if let Some((p, n)) = trimmed.split_once('/') {
            Self::new(p, n)
        } else {
            Self::new(DEFAULT_PROVIDER, trimmed)
        }
    }

    /// The raw canonical string (`provider/name`).
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// A display string used in CLI output (full canonical form).
    pub fn display(&self) -> String {
        self.raw.clone()
    }

    /// Return just the skill name (short form) when unambiguous.
    pub fn skill_id(&self) -> String {
        self.name.clone()
    }

    /// Alias for `as_str` — used by provider import code.
    pub fn to_canonical_string(&self) -> String {
        self.raw.clone()
    }

    /// Compute a stable 64-bit hash for this ID.
    pub fn hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.raw.hash(&mut hasher);
        hasher.finish()
    }
}

impl fmt::Display for CanonicalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.raw)
    }
}

/// Sanitize a provider/name segment.
///
/// Keeps: alphanumeric, `-`, `_`, `.`.
/// Converts spaces and other characters to `-`.
/// Collapses multiple `-` into a single `-`.
fn sanitize_segment(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut last_was_dash = false;
    for c in s.chars() {
        let safe = match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '.' => {
                last_was_dash = false;
                c.to_ascii_lowercase()
            }
            '-' | ' ' | '/' | '\\' => {
                if last_was_dash {
                    continue;
                }
                last_was_dash = true;
                '-'
            }
            _ => {
                last_was_dash = false;
                c.to_ascii_lowercase()
            }
        };
        result.push(safe);
    }
    result.trim_matches('-').to_string()
}

/// A single ID collision between two or more skills.
#[derive(Debug, Clone)]
pub struct SkillIdCollision {
    pub skill_id: String,
    pub providers: Vec<String>,
    pub canonical_ids: Vec<String>,
}

/// Report of potential ID collisions within a skill collection.
#[derive(Debug, Clone, Default)]
pub struct CollisionReport {
    pub has_collisions: bool,
    pub collisions: Vec<SkillIdCollision>,
}

impl CollisionReport {
    pub fn is_empty(&self) -> bool {
        self.collisions.is_empty()
    }

    pub fn len(&self) -> usize {
        self.collisions.len()
    }

    pub fn push(&mut self, collision: SkillIdCollision) {
        self.collisions.push(collision);
        self.has_collisions = true;
    }

    /// Check if the given skill ID has a collision in this report.
    pub fn has(&self, skill_id: &str) -> bool {
        self.collisions.iter().any(|c| c.skill_id == skill_id)
    }

    /// Get the collision record for a specific skill ID.
    pub fn for_skill_id(&self, skill_id: &str) -> Option<&SkillIdCollision> {
        self.collisions.iter().find(|c| c.skill_id == skill_id)
    }
}

/// Detect collisions across a list of `(provider, skill_id)` pairs.
///
/// Returns a `CollisionReport` with one entry per skill ID that appears
/// under more than one provider.
pub fn detect_collisions(skills: Vec<(&str, &str)>) -> CollisionReport {
    let mut by_skill_id: HashMap<String, Vec<String>> = HashMap::new();

    for (provider, skill_id) in skills {
        by_skill_id
            .entry(skill_id.to_string())
            .or_default()
            .push(provider.to_string());
    }

    let mut report = CollisionReport::default();
    for (skill_id, providers) in by_skill_id {
        if providers.len() > 1 {
            let canonical_ids: Vec<String> = providers
                .iter()
                .map(|p| format!("{}/{}", p, skill_id))
                .collect();

            let unique_providers: Vec<String> = {
                let mut set = HashSet::new();
                let mut ordered = Vec::new();
                for p in &providers {
                    if set.insert(p.clone()) {
                        ordered.push(p.clone());
                    }
                }
                ordered
            };

            report.push(SkillIdCollision {
                skill_id: skill_id.clone(),
                providers: unique_providers,
                canonical_ids,
            });
        }
    }
    report
}

/// Check whether a skill ID is unambiguous (no collisions) in the report.
///
/// This is the form used by the provider importer.
pub fn is_unambiguous(skill_id: &str, report: &CollisionReport) -> bool {
    !report.has(skill_id)
}

/// Check whether a proposed name is unambiguous (doesn't collide with existing skills).
///
/// Takes an existing map of canonical ID → file path and returns the first
/// collision found, or None if clear.
pub fn detect_collisions_with_map(
    new_name: &str,
    new_provider: &str,
    existing: &HashMap<String, String>,
) -> Option<SkillIdCollision> {
    let canonical = match CanonicalId::new(new_provider, new_name) {
        Ok(id) => id.as_str().to_string(),
        Err(_) => return None,
    };

    if let Some(existing_path) = existing.get(&canonical) {
        return Some(SkillIdCollision {
            skill_id: new_name.to_string(),
            providers: vec![new_provider.to_string()],
            canonical_ids: vec![canonical, existing_path.clone()],
        });
    }
    None
}

/// Provenance information for a skill ID.
///
/// Tracks where a canonical ID came from (provider, file path, or archive).
#[derive(Debug, Clone, Default)]
pub struct Provenance {
    pub origin_file: Option<String>,
    pub provider: String,
    pub archive_commit: Option<String>,
}

impl Provenance {
    pub fn from_path<P: AsRef<Path>>(path: P, provider: &str) -> Self {
        Self {
            origin_file: Some(path.as_ref().to_string_lossy().to_string()),
            provider: provider.to_string(),
            archive_commit: None,
        }
    }

    pub fn from_archive(path: &str, provider: &str, commit: &str) -> Self {
        Self {
            origin_file: Some(path.to_string()),
            provider: provider.to_string(),
            archive_commit: Some(commit.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_id_basic() {
        let id = CanonicalId::new("acme", "test-skill").unwrap();
        assert_eq!(id.provider, "acme");
        assert_eq!(id.name, "test-skill");
        assert_eq!(id.as_str(), "acme/test-skill");
    }

    #[test]
    fn canonical_id_parse() {
        let id = CanonicalId::parse("acme/test-skill").unwrap();
        assert_eq!(id.as_str(), "acme/test-skill");

        let id2 = CanonicalId::parse("plain-name").unwrap();
        assert_eq!(id2.provider, DEFAULT_PROVIDER);
        assert_eq!(id2.name, "plain-name");
    }

    #[test]
    fn canonical_id_empty_err() {
        assert!(CanonicalId::new("", "foo").is_err());
        assert!(CanonicalId::new("foo", "").is_err());
    }

    #[test]
    fn canonical_id_display() {
        let id = CanonicalId::new("acme", "test").unwrap();
        assert_eq!(id.display(), "acme/test");
        assert_eq!(id.skill_id(), "test");
        assert_eq!(id.to_canonical_string(), "acme/test");
    }

    #[test]
    fn detect_collisions_found() {
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

    #[test]
    fn detect_collisions_empty_on_unique_ids() {
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
    fn is_unambiguous_basic() {
        let skills: Vec<(&str, &str)> = vec![("claude", "foo"), ("codex", "foo")];
        let report = detect_collisions(skills);
        assert!(!is_unambiguous("foo", &report));
        assert!(is_unambiguous("bar", &report));
    }

    #[test]
    fn sanitize_collapses_dashes() {
        let s = sanitize_segment("a--b   c");
        assert_eq!(s, "a-b-c");
    }

    #[test]
    fn sanitize_lowercases() {
        let s = sanitize_segment("UpperCase");
        assert_eq!(s, "uppercase");
    }

    #[test]
    fn default_provider_constant() {
        assert_eq!(DEFAULT_PROVIDER, "local");
    }
}
