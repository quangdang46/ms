//! ms route - Route a task to the best matching skill
//!
//! Compact decision API: given a task description, return the top N skill
//! candidates with scores and ready-to-use load commands.
//!
//! # Output Schema (JSON)
//!
//! ```json
//! {
//!   "route_schema_version": 1,
//!   "task": "fix rust build error in tokio cli",
//!   "threshold": 0.65,
//!   "decision": "match|no_match",
//!   "candidates": [...],
//!   "fallback": { "search_command": "..." }
//! }
//! ```

use clap::Args;
use tracing::debug;

use crate::app::AppContext;
use crate::cli::output::{OutputFormat, emit_formatted};
use crate::core::disclosure::sanitize_slug;
use crate::error::Result;
use crate::search::NegativeRouteKey;
use crate::storage::sqlite::SkillRecord;

/// Default number of candidates to return
const DEFAULT_TOP_N: usize = 3;

/// Default route threshold (below this → no_match)
const DEFAULT_THRESHOLD: f64 = 0.65;

/// Current route schema version
const ROUTE_SCHEMA_VERSION: u32 = 1;

// =============================================================================
// CLI ARGS
// =============================================================================

#[derive(Args, Debug)]
pub struct RouteArgs {
    /// Task description to route
    pub task: String,

    /// Maximum number of candidates (default: 3)
    #[arg(long, default_value = "3")]
    pub top_n: usize,

    /// Minimum score threshold (0.0–1.0, default: 0.65)
    #[arg(long, default_value = "0.65")]
    pub threshold: f64,

    /// Show debug scores and breakdown
    #[arg(long)]
    pub debug: bool,

    /// Working directory for context detection
    #[arg(long)]
    pub cwd: Option<String>,

    /// Override output format
    #[arg(long)]
    pub output: Option<OutputFormat>,
}

// =============================================================================
// ROUTE CANDIDATE
// =============================================================================

/// A single routing candidate
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RouteCandidate {
    /// Canonical skill ID (e.g., "claude/rust-error-handling")
    pub skill_id: String,
    /// Display ID (short form when unambiguous)
    pub display_id: String,
    /// Match score (0.0–1.0)
    pub score: f64,
    /// Reasons for the match (short, enumerable)
    pub why: Vec<String>,
    /// Guidance on when to use this skill
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when_to_use: Option<String>,
    /// Default load level for this skill
    pub default_load: String,
    /// Entry sections for loading
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entry_sections: Vec<String>,
    /// Ready-to-use load command
    pub load_command: String,
    /// Execution mode
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<String>,
}

/// Debug information for a candidate (only in --debug mode)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RouteDebug {
    /// Raw keyword match scores
    pub keyword_scores: Vec<KeywordScore>,
    /// Trigger phrase matches
    pub trigger_matches: Vec<String>,
    /// Tag matches
    pub tag_matches: Vec<String>,
    /// Context/description match score
    pub description_score: f64,
}

/// Individual keyword score breakdown
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KeywordScore {
    pub keyword: String,
    pub score: f64,
}

/// Fallback information when no candidates meet the threshold
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RouteFallback {
    pub search_command: String,
    /// Optional suggest command; absent when not supported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggest_command: Option<String>,
}

/// Complete route response
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RouteResponse {
    pub route_schema_version: u32,
    pub task: String,
    pub threshold: f64,
    pub decision: String,
    pub candidates: Vec<RouteCandidate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub debug_info: Vec<RouteDebug>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<RouteFallback>,
}

// =============================================================================
// ROUTE IMPLEMENTATION
// =============================================================================

pub fn run(ctx: &AppContext, args: &RouteArgs) -> Result<()> {
    debug!(target: "route", task = %args.task, "routing task");

    let top_n = if args.top_n == 0 {
        DEFAULT_TOP_N
    } else {
        args.top_n
    };
    let threshold = if args.threshold <= 0.0 {
        DEFAULT_THRESHOLD
    } else {
        args.threshold
    };
    let cwd_fingerprint = args.cwd.clone().unwrap_or_else(|| {
        std::env::current_dir()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| "/default".to_string())
    });

    // Check negative route cache before scoring
    let neg_key = NegativeRouteKey {
        task_text: normalize_route_text(&args.task),
        cwd_fingerprint: cwd_fingerprint.clone(),
    };
    if ctx.cache.get_negative_route(&neg_key) {
        debug!(target: "route", "negative route cache hit for task");
        let response = RouteResponse {
            route_schema_version: ROUTE_SCHEMA_VERSION,
            task: args.task.clone(),
            threshold,
            decision: "no_match".to_string(),
            candidates: vec![],
            debug_info: vec![],
            fallback: Some(RouteFallback {
                search_command: format!("ms search \"{}\" -O json", args.task),
                suggest_command: None,
            }),
        };
        return output_response(ctx, args, &response);
    }

    let all_skills = get_all_skills(ctx)?;
    let response = route_task(all_skills, &args.task, top_n, threshold, args.debug);

    // Cache no_match decisions
    if response.decision == "no_match" {
        ctx.cache.put_negative_route(&neg_key);
    }

    output_response(ctx, args, &response)
}

/// Core routing logic: score skills against a task and return top candidates.
/// Used by both CLI command and MCP tool handler.
pub(crate) fn route_task(
    all_skills: Vec<SkillRecord>,
    task: &str,
    top_n: usize,
    threshold: f64,
    debug: bool,
) -> RouteResponse {
    if all_skills.is_empty() {
        return RouteResponse {
            route_schema_version: ROUTE_SCHEMA_VERSION,
            task: task.to_string(),
            threshold,
            decision: "no_match".to_string(),
            candidates: vec![],
            debug_info: vec![],
            fallback: Some(RouteFallback {
                search_command: format!("ms search \"{task}\" -O json"),
                suggest_command: None,
            }),
        };
    }

    let mut scored: Vec<(SkillRecord, RouteCandidate, Option<RouteDebug>)> = all_skills
        .iter()
        .filter_map(|skill| score_skill(skill, task, debug))
        .collect();

    scored.sort_by(|a, b| {
        b.1.score
            .partial_cmp(&a.1.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Deduplicate by skill name across providers: keep highest-scoring only
    let mut seen_names = std::collections::HashSet::new();
    let above_threshold: Vec<_> = scored
        .iter()
        .filter(|(_, candidate, _)| {
            if candidate.score < threshold {
                return false;
            }
            // Use skill_id (which includes provider) for identity,
            // but deduplicate by display_id (skill name) to avoid
            // same skill from 4 providers filling all slots
            seen_names.insert(candidate.display_id.clone())
        })
        .take(top_n)
        .collect();

    let decision = if above_threshold.is_empty() {
        "no_match"
    } else {
        "match"
    };

    let candidates: Vec<RouteCandidate> =
        above_threshold.iter().map(|(_, c, _)| c.clone()).collect();
    let debug_info: Vec<RouteDebug> = if debug {
        above_threshold
            .iter()
            .filter_map(|(_, _, d)| d.clone())
            .collect()
    } else {
        vec![]
    };

    let fallback = if decision == "no_match" {
        Some(RouteFallback {
            search_command: format!("ms search \"{task}\" -O json"),
            suggest_command: None,
        })
    } else {
        None
    };

    RouteResponse {
        route_schema_version: ROUTE_SCHEMA_VERSION,
        task: task.to_string(),
        threshold,
        decision: decision.to_string(),
        candidates,
        debug_info,
        fallback,
    }
}

fn output_response(ctx: &AppContext, args: &RouteArgs, response: &RouteResponse) -> Result<()> {
    if ctx.quiet {
        return Ok(());
    }

    let format = args.output.unwrap_or(ctx.output_format);
    emit_formatted(
        response,
        format,
        format_route_human,
        format_route_plain,
        format_route_tsv,
    )
}

fn format_route_human(response: &RouteResponse) -> String {
    let mut lines = vec![
        format!("Route: {}", response.task),
        format!("Decision: {}", response.decision),
        format!("Threshold: {:.2}", response.threshold),
        String::new(),
    ];

    if response.candidates.is_empty() {
        lines.push("No matching skills found.".to_string());
        if let Some(fallback) = &response.fallback {
            lines.push(format!("Fallback: {}", fallback.search_command));
        }
    } else {
        for (i, candidate) in response.candidates.iter().enumerate() {
            lines.push(format!(
                "{}. {} (score: {:.2})",
                i + 1,
                candidate.display_id,
                candidate.score
            ));
            for reason in &candidate.why {
                lines.push(format!("   - {reason}"));
            }
            lines.push(format!("   load: {}", candidate.load_command));
            lines.push(String::new());
        }
    }

    if !response.debug_info.is_empty() {
        lines.push("--- Debug Info ---".to_string());
        for (i, debug) in response.debug_info.iter().enumerate() {
            lines.push(format!("Candidate {}:", i + 1));
            lines.push(format!(
                "  Description score: {:.3}",
                debug.description_score
            ));
            if !debug.trigger_matches.is_empty() {
                lines.push(format!(
                    "  Trigger matches: {}",
                    debug.trigger_matches.join(", ")
                ));
            }
            if !debug.tag_matches.is_empty() {
                lines.push(format!("  Tag matches: {}", debug.tag_matches.join(", ")));
            }
            if !debug.keyword_scores.is_empty() {
                lines.push("  Keyword scores:".to_string());
                for ks in &debug.keyword_scores {
                    lines.push(format!("    - {} ({:.3})", ks.keyword, ks.score));
                }
            }
        }
    }

    lines.join("\n")
}

fn format_route_plain(response: &RouteResponse) -> String {
    let mut lines = vec![
        format!("task={}", response.task),
        format!("decision={}", response.decision),
        format!("threshold={:.2}", response.threshold),
        format!("candidate_count={}", response.candidates.len()),
    ];

    for (i, candidate) in response.candidates.iter().enumerate() {
        let rank = i + 1;
        lines.push(format!("candidate.{rank}.skill_id={}", candidate.skill_id));
        lines.push(format!(
            "candidate.{rank}.display_id={}",
            candidate.display_id
        ));
        lines.push(format!("candidate.{rank}.score={:.6}", candidate.score));
        lines.push(format!(
            "candidate.{rank}.default_load={}",
            candidate.default_load
        ));
        lines.push(format!(
            "candidate.{rank}.entry_sections={}",
            candidate.entry_sections.join(",")
        ));
        lines.push(format!(
            "candidate.{rank}.load_command={}",
            candidate.load_command
        ));
        lines.push(format!("candidate.{rank}.why={}", candidate.why.join(",")));
        lines.push(format!(
            "candidate.{rank}.when_to_use={}",
            candidate.when_to_use.as_deref().unwrap_or("")
        ));
        lines.push(format!(
            "candidate.{rank}.execution_mode={}",
            candidate.execution_mode.as_deref().unwrap_or("")
        ));
    }

    if let Some(fallback) = &response.fallback {
        lines.push(format!(
            "fallback.search_command={}",
            fallback.search_command
        ));
        if let Some(suggest) = &fallback.suggest_command {
            lines.push(format!("fallback.suggest_command={suggest}"));
        }
    }

    if !response.debug_info.is_empty() {
        for (i, debug) in response.debug_info.iter().enumerate() {
            let rank = i + 1;
            lines.push(format!(
                "debug.{rank}.description_score={:.6}",
                debug.description_score
            ));
            lines.push(format!(
                "debug.{rank}.trigger_matches={}",
                debug.trigger_matches.join(",")
            ));
            lines.push(format!(
                "debug.{rank}.tag_matches={}",
                debug.tag_matches.join(",")
            ));
            let keyword_scores = debug
                .keyword_scores
                .iter()
                .map(|ks| format!("{}:{:.3}", ks.keyword, ks.score))
                .collect::<Vec<_>>()
                .join(",");
            lines.push(format!("debug.{rank}.keyword_scores={keyword_scores}"));
        }
    }

    lines.join("\n")
}

fn format_route_tsv(response: &RouteResponse) -> String {
    let mut rows = vec![
        [
            "decision",
            "threshold",
            "rank",
            "skill_id",
            "display_id",
            "score",
            "default_load",
            "entry_sections",
            "load_command",
            "why",
            "when_to_use",
            "execution_mode",
            "fallback_search_command",
        ]
        .join("\t"),
    ];

    if response.candidates.is_empty() {
        rows.push(
            [
                response.decision.clone(),
                format!("{:.2}", response.threshold),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                response
                    .fallback
                    .as_ref()
                    .map(|fallback| fallback.search_command.clone())
                    .unwrap_or_default(),
            ]
            .join("\t"),
        );
        return rows.join("\n");
    }

    for (i, candidate) in response.candidates.iter().enumerate() {
        rows.push(
            [
                response.decision.clone(),
                format!("{:.2}", response.threshold),
                (i + 1).to_string(),
                tsv_sanitize(&candidate.skill_id),
                tsv_sanitize(&candidate.display_id),
                format!("{:.6}", candidate.score),
                tsv_sanitize(&candidate.default_load),
                tsv_sanitize(&candidate.entry_sections.join(",")),
                tsv_sanitize(&candidate.load_command),
                tsv_sanitize(&candidate.why.join(",")),
                tsv_sanitize(candidate.when_to_use.as_deref().unwrap_or("")),
                tsv_sanitize(candidate.execution_mode.as_deref().unwrap_or("")),
                String::new(),
            ]
            .join("\t"),
        );
    }

    rows.join("\n")
}

fn tsv_sanitize(value: &str) -> String {
    value.replace(['\t', '\n'], " ")
}

// =============================================================================
// SCORING
// =============================================================================

/// Check if two words share a common prefix of at least 4 characters.
/// This handles basic stemming: "profiling" matches "profile", "errors" matches "error", etc.
fn prefix_fuzzy_match(a: &str, b: &str) -> bool {
    let a_char_count = a.chars().count();
    let b_char_count = b.chars().count();
    if a_char_count < 4 || b_char_count < 4 {
        return false;
    }
    // Check if one starts with the other (for at least 4 chars)
    if a.starts_with(b) || b.starts_with(a) {
        return true;
    }
    // Find common prefix length
    let common = a.chars().zip(b.chars()).take_while(|(a, b)| a == b).count();
    common >= 4
}

/// Check if a word matches any word in text, with prefix fuzzy matching.
fn word_fuzzy_contains(text: &str, word: &str) -> bool {
    let text_lower = text.to_lowercase();
    let word_lower = word.to_lowercase();
    // Exact substring first
    if text_lower.contains(&word_lower) {
        return true;
    }
    // Prefix fuzzy: check if word matches any token in text
    for token in text_lower.split(|c: char| c.is_whitespace() || c.is_ascii_punctuation()) {
        if token.len() >= 4 && prefix_fuzzy_match(&word_lower, token) {
            return true;
        }
    }
    false
}

/// Score a skill against a task description.
fn score_skill(
    skill: &SkillRecord,
    task: &str,
    debug: bool,
) -> Option<(SkillRecord, RouteCandidate, Option<RouteDebug>)> {
    let task_normalized = normalize_route_text(task);
    let task_lower = task.to_lowercase();
    let task_words: Vec<&str> = task_lower
        .split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
        .filter(|w| !w.is_empty() && w.len() > 2)
        .collect();

    // Parse metadata for routing fields
    let meta: serde_json::Value = serde_json::from_str(&skill.metadata_json).unwrap_or_default();
    let trigger_phrases: Vec<String> = meta
        .get("trigger_phrases")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let when_to_use: Option<String> = meta
        .get("when_to_use")
        .and_then(|v| v.as_str())
        .map(String::from);
    let route_keywords: Vec<String> = meta
        .get("keywords")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let entry_sections: Vec<String> = meta
        .get("entry_sections")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .filter(|sections: &Vec<String>| !sections.is_empty())
        .unwrap_or_else(|| derive_entry_sections_from_body(&skill.body));
    let execution_mode: String = meta
        .get("execution_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("inline")
        .to_string();

    // 1. Trigger phrase matching (highest weight)
    let trigger_scores: Vec<(&String, f64)> = trigger_phrases
        .iter()
        .map(|phrase| (phrase, score_trigger_phrase(&task_normalized, phrase)))
        .collect();
    let trigger_matches: Vec<String> = trigger_scores
        .iter()
        .filter(|(_, score)| *score > 0.0)
        .map(|(phrase, _)| (*phrase).clone())
        .collect();
    let trigger_score = if trigger_matches.is_empty() {
        0.0
    } else {
        trigger_scores
            .iter()
            .map(|(_, score)| *score)
            .fold(0.0, f64::max)
    };

    // 2. Keyword matching against skill name, description, and tags
    let keyword_scores: Vec<KeywordScore> = task_words
        .iter()
        .filter_map(|word| {
            let word = word.trim_matches(|c: char| c.is_ascii_punctuation());
            if word.is_empty() || word.len() <= 2 {
                return None;
            }
            let mut score = 0.0f64;

            // Check name match (exact substring)
            if skill.name.to_lowercase().contains(word) {
                score += 0.5;
            }
            // Check description match (fuzzy prefix for stemming)
            if word_fuzzy_contains(&skill.description, word) {
                score += 0.3;
            }
            // Check tags
            let tags: Vec<&str> = meta
                .get("tags")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            if tags.iter().any(|t: &&str| word_fuzzy_contains(t, word)) {
                score += 0.4;
            }
            if route_keywords.iter().any(|keyword| {
                keyword
                    .to_ascii_lowercase()
                    .split_whitespace()
                    .any(|part| part == word)
            }) {
                score += 0.7;
            }
            // Check context project types
            let project_types: Vec<&str> = meta
                .get("context")
                .and_then(|c| c.get("project_types"))
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            if project_types.iter().any(|pt: &&str| pt.contains(word)) {
                score += 0.6;
            }

            if score > 0.0 {
                Some(KeywordScore {
                    keyword: word.to_string(),
                    score: score.min(1.0),
                })
            } else {
                None
            }
        })
        .collect();

    let keyword_score = if keyword_scores.is_empty() {
        0.0
    } else {
        let sum: f64 = keyword_scores.iter().map(|ks| ks.score).sum();
        let count = keyword_scores.len() as f64;
        // Use coverage-weighted average: reward queries where many words match
        // A single perfect name match (0.5) = 0.5
        // Three name matches (0.5+0.5+0.5) = 0.5 * (1 + 0.4*2) = 0.9
        let coverage_bonus = 1.0 + 0.4 * (count - 1.0).max(0.0);
        let avg = sum / count;
        (avg * coverage_bonus).min(1.0)
    };
    let route_keyword_score = route_keywords
        .iter()
        .map(|keyword| score_route_keyword_hint(&task_normalized, &task_words, keyword))
        .fold(0.0, f64::max);
    let when_to_use_score = when_to_use
        .as_deref()
        .map(|text| score_route_keyword_hint(&task_normalized, &task_words, text))
        .unwrap_or(0.0);
    let route_metadata_score = route_keyword_score.max(when_to_use_score);

    // 3. Tag matching
    let tags: Vec<String> = meta
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let tag_matches: Vec<String> = tags
        .iter()
        .filter(|tag| task_lower.contains(&tag.to_lowercase()))
        .cloned()
        .collect();
    let tag_score = if tags.is_empty() {
        0.0
    } else {
        (tag_matches.len() as f64 / tags.len() as f64).min(1.0)
    };

    // 4. Description similarity (word overlap)
    let binding = skill.description.to_lowercase();
    let desc_words: Vec<&str> = binding
        .split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
        .filter(|w| !w.is_empty() && w.len() > 2)
        .collect();
    let desc_overlap = task_words
        .iter()
        .filter(|w| desc_words.contains(w) || desc_words.iter().any(|dw| prefix_fuzzy_match(w, dw)))
        .count();
    let description_score = if desc_words.is_empty() {
        0.0
    } else {
        let overlap_ratio = desc_overlap as f64 / desc_words.len().max(1) as f64;
        (overlap_ratio * 0.8).min(1.0)
    };

    // Composite score: weighted average
    let composite = trigger_score * 0.35 // trigger phrases are strongest signal
        + route_metadata_score * 0.25    // compact route metadata from import/index
        + keyword_score * 0.20           // keyword matches in name/description/tags/metadata
        + tag_score * 0.10               // tag relevance
        + description_score * 0.10; // description overlap
    // Boost composite based on signal strength.
    // The linear weighted average alone never reaches the threshold for realistic
    // queries, so we use signal-based boosts to surface relevant skills.
    let composite = if trigger_score >= 1.0 {
        composite.max(0.85)
    } else if trigger_score >= 0.9 {
        composite.max(0.75)
    } else if route_metadata_score >= 0.95 && keyword_score >= 0.6 {
        // Imported compact route metadata should be authoritative enough to
        // clear the default threshold when it fully covers the task.
        composite.max(0.72)
    } else if keyword_score >= 1.0 {
        // Multiple keyword matches covering name+desc+tags — very strong signal
        composite.max(0.82)
    } else if keyword_score >= 0.8 {
        // Strong keyword match (e.g. name + description)
        composite.max(0.75)
    } else if keyword_score >= 0.5 && description_score >= 0.15 {
        // Name or tag match plus some description overlap
        composite.max(0.68)
    } else if keyword_score >= 0.5 {
        // Keyword match to name or tags — meets default threshold
        composite.max(0.65)
    } else {
        composite
    };

    // Add a small bonus for the number of distinct keyword matches
    // so skills matching more of the task words rank higher than those
    // matching just one word at the same boost level.
    let match_count_bonus = keyword_scores.len() as f64 * 0.02;
    let composite = composite + match_count_bonus;

    // Don't return candidates with zero score
    if composite <= 0.0 && !debug {
        return None;
    }

    // Build why list
    let mut why = Vec::new();
    if !trigger_matches.is_empty() {
        for t in trigger_matches.iter().take(2) {
            why.push(format!("trigger:{t}"));
        }
    }
    for ks in &keyword_scores {
        if ks.score >= 0.5 {
            why.push(format!("keyword:{}", ks.keyword));
        }
    }
    if !tag_matches.is_empty() {
        for t in tag_matches.iter().take(2) {
            why.push(format!("tag:{t}"));
        }
    }
    if !why.is_empty() {
        // Keep why short
        why.truncate(4);
    }

    // Determine display_id and skill_id.
    // skill_id MUST be a slug usable by `ms load <skill_id>` — never the
    // human-readable name. display_id is the pretty name for humans.
    let provider = meta
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("local");
    let display_id = skill.name.clone();
    let canonical = meta
        .get("canonical_id")
        .and_then(|v| v.as_str())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            if skill.id.contains('/') {
                skill.id.clone()
            } else {
                format!("{provider}/{}", skill.id)
            }
        });
    let skill_id = if provider == "local" {
        // For local skills, the slug is the DB id. Avoid emitting the pretty
        // display name as `skill_id` — agents read this field and pass it to
        // `ms load`, so it must be the slug.
        skill.id.clone()
    } else {
        canonical
    };

    // Build load command
    let entry_section_str = if !entry_sections.is_empty() {
        format!(" --section {}", entry_sections[0])
    } else {
        String::new()
    };
    // Use skill.id (DB primary key) for load_command because resolve_skill
    // looks up by DB id first. Canonical IDs with provider prefix may not exist in DB.
    let load_ref = skill.id.clone();
    let load_command = format!("ms load {}{} -O json", load_ref, entry_section_str);

    let default_load = if !entry_sections.is_empty() {
        format!("section:{}", entry_sections[0])
    } else {
        "standard".to_string()
    };

    let candidate = RouteCandidate {
        skill_id,
        display_id,
        score: composite,
        why,
        when_to_use,
        default_load,
        entry_sections,
        load_command,
        execution_mode: Some(execution_mode),
    };

    let debug_info = if debug {
        Some(RouteDebug {
            keyword_scores,
            trigger_matches,
            tag_matches,
            description_score,
        })
    } else {
        None
    };

    Some((skill.clone(), candidate, debug_info))
}

fn normalize_route_text(text: &str) -> String {
    text.split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
}

fn score_trigger_phrase(task_normalized: &str, phrase: &str) -> f64 {
    let phrase_normalized = normalize_route_text(phrase);
    if phrase_normalized.is_empty() {
        return 0.0;
    }

    if task_normalized == phrase_normalized {
        return 1.0;
    }

    if task_normalized.contains(&phrase_normalized) {
        return 0.9;
    }

    0.0
}

fn score_route_keyword_hint(task_normalized: &str, task_words: &[&str], hint: &str) -> f64 {
    let hint_normalized = normalize_route_text(hint);
    if hint_normalized.is_empty() {
        return 0.0;
    }

    if task_normalized == hint_normalized {
        return 1.0;
    }

    if task_normalized.contains(&hint_normalized) || hint_normalized.contains(task_normalized) {
        return 0.95;
    }

    let hint_words: Vec<&str> = hint_normalized
        .split_whitespace()
        .filter(|word| !word.is_empty() && word.len() > 2)
        .collect();
    if hint_words.is_empty() || task_words.is_empty() {
        return 0.0;
    }

    let overlap = task_words
        .iter()
        .filter(|word| hint_words.contains(word))
        .count();
    if overlap == 0 {
        return 0.0;
    }

    let task_coverage = overlap as f64 / task_words.len() as f64;
    let hint_density = overlap as f64 / hint_words.len() as f64;
    (task_coverage * 0.8 + hint_density * 0.2).min(1.0)
}

fn derive_entry_sections_from_body(body: &str) -> Vec<String> {
    let mut sections = Vec::new();

    for line in body.lines() {
        let Some(title) = line.trim().strip_prefix("## ") else {
            continue;
        };

        let slug = sanitize_slug(title.trim());
        if !slug.is_empty() && !sections.contains(&slug) {
            sections.push(slug);
        }
    }

    sections
}

// =============================================================================
// HELPERS
// =============================================================================

/// Get all indexed skills from the database
pub(crate) fn get_all_skills(ctx: &AppContext) -> Result<Vec<SkillRecord>> {
    let mut all = Vec::new();
    let mut offset = 0usize;
    let limit = 200usize;

    loop {
        let batch = ctx.db.list_skills(limit, offset)?;
        if batch.is_empty() {
            break;
        }
        offset += batch.len();
        all.extend(batch);
    }

    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::disclosure::sanitize_slug;

    fn make_skill_metadata(_id: &str, _name: &str, _desc: &str, tags: Vec<&str>) -> String {
        serde_json::json!({
            "tags": tags,
            "trigger_phrases": [],
            "execution_mode": "inline"
        })
        .to_string()
    }

    fn make_skill_record(id: &str, name: &str, desc: &str, tags: Vec<&str>) -> SkillRecord {
        SkillRecord {
            id: id.to_string(),
            name: name.to_string(),
            version: Some("1.0".to_string()),
            description: desc.to_string(),
            author: None,
            source_layer: "user".to_string(),
            source_path: format!("/skills/{id}.md"),
            body: format!("# {name}\n\n{desc}"),
            metadata_json: make_skill_metadata(id, name, desc, tags),
            content_hash: "abc123".to_string(),
            assets_json: "{}".to_string(),
            token_count: 100,
            quality_score: 0.8,
            indexed_at: "2025-01-01T00:00:00Z".to_string(),
            modified_at: "2025-01-01T00:00:00Z".to_string(),
            is_deprecated: false,
            deprecation_reason: None,
            provider: None,
            git_remote: None,
            git_commit: None,
            archive_format_version: None,
            provenance_json: "{}".to_string(),
        }
    }

    #[test]
    fn test_route_response_struct() {
        let response = RouteResponse {
            route_schema_version: 1,
            task: "fix rust error".to_string(),
            threshold: 0.65,
            decision: "match".to_string(),
            candidates: vec![RouteCandidate {
                skill_id: "claude/rust-error-handling".to_string(),
                display_id: "rust-error-handling".to_string(),
                score: 0.93,
                why: vec!["keyword:rust".to_string(), "keyword:error".to_string()],
                when_to_use: Some("Compiler errors, runtime failures".to_string()),
                default_load: "section:checklist".to_string(),
                entry_sections: vec!["checklist".to_string(), "pitfalls".to_string()],
                load_command: "ms load claude/rust-error-handling --section checklist -O json"
                    .to_string(),
                execution_mode: Some("inline".to_string()),
            }],
            debug_info: vec![],
            fallback: None,
        };

        let json = serde_json::to_string_pretty(&response).unwrap();
        assert!(json.contains("route_schema_version"));
        assert!(json.contains("rust-error-handling"));
        assert!(json.contains("0.93"));
    }

    #[test]
    fn test_route_no_match_fallback() {
        let response = RouteResponse {
            route_schema_version: 1,
            task: "unknown thing".to_string(),
            threshold: 0.65,
            decision: "no_match".to_string(),
            candidates: vec![],
            debug_info: vec![],
            fallback: Some(RouteFallback {
                search_command: "ms search \"unknown thing\" -O json".to_string(),
                suggest_command: None,
            }),
        };

        assert_eq!(response.decision, "no_match");
        assert!(response.fallback.is_some());
    }

    #[test]
    fn test_route_plain_output_format() {
        let response = RouteResponse {
            route_schema_version: 1,
            task: "fix rust error".to_string(),
            threshold: 0.65,
            decision: "match".to_string(),
            candidates: vec![RouteCandidate {
                skill_id: "claude/rust-error-handling".to_string(),
                display_id: "rust-error-handling".to_string(),
                score: 0.93,
                why: vec!["keyword:rust".to_string(), "keyword:error".to_string()],
                when_to_use: Some("Compiler errors".to_string()),
                default_load: "section:checklist".to_string(),
                entry_sections: vec!["checklist".to_string()],
                load_command: "ms load claude/rust-error-handling --section checklist -O json"
                    .to_string(),
                execution_mode: Some("inline".to_string()),
            }],
            debug_info: vec![],
            fallback: None,
        };

        let plain = format_route_plain(&response);
        assert!(plain.contains("task=fix rust error"));
        assert!(plain.contains("candidate.1.skill_id=claude/rust-error-handling"));
        assert!(plain.contains("candidate.1.load_command=ms load claude/rust-error-handling"));
        assert!(!plain.contains("{\n"));
    }

    #[test]
    fn test_route_tsv_output_format() {
        let response = RouteResponse {
            route_schema_version: 1,
            task: "unknown thing".to_string(),
            threshold: 0.65,
            decision: "no_match".to_string(),
            candidates: vec![],
            debug_info: vec![],
            fallback: Some(RouteFallback {
                search_command: "ms search \"unknown thing\" -O json".to_string(),
                suggest_command: None,
            }),
        };

        let tsv = format_route_tsv(&response);
        let lines: Vec<&str> = tsv.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("decision\tthreshold\trank\tskill_id"));
        assert!(lines[1].contains("no_match\t0.65"));
        assert!(lines[1].contains("ms search \"unknown thing\" -O json"));
    }

    #[test]
    fn test_route_debug_info() {
        let debug = RouteDebug {
            keyword_scores: vec![
                KeywordScore {
                    keyword: "rust".to_string(),
                    score: 0.8,
                },
                KeywordScore {
                    keyword: "error".to_string(),
                    score: 0.6,
                },
            ],
            trigger_matches: vec!["compiler error".to_string()],
            tag_matches: vec!["rust".to_string()],
            description_score: 0.5,
        };

        let response = RouteResponse {
            route_schema_version: 1,
            task: "test".to_string(),
            threshold: 0.5,
            decision: "match".to_string(),
            candidates: vec![RouteCandidate {
                skill_id: "test/skill".to_string(),
                display_id: "skill".to_string(),
                score: 0.7,
                why: vec!["keyword:test".to_string()],
                when_to_use: None,
                default_load: "standard".to_string(),
                entry_sections: vec![],
                load_command: "ms load test/skill -O json".to_string(),
                execution_mode: Some("inline".to_string()),
            }],
            debug_info: vec![debug],
            fallback: None,
        };

        assert_eq!(response.debug_info.len(), 1);
        assert_eq!(response.debug_info[0].trigger_matches[0], "compiler error");
    }

    #[test]
    fn test_score_skill_with_tags() {
        let skill = make_skill_record(
            "rust-error-handling",
            "Rust Error Handling",
            "How to handle Rust compiler errors and runtime failures",
            vec!["rust", "error-handling", "compiler"],
        );

        // We can't easily call score_skill directly without a DB context,
        // but we can verify the metadata JSON structure
        let json: serde_json::Value = serde_json::from_str(&skill.metadata_json).unwrap();
        assert_eq!(json["tags"][0], "rust");
    }

    #[test]
    fn test_route_task_uses_canonical_skill_id_in_load_command() {
        let skill = SkillRecord {
            metadata_json: serde_json::json!({
                "provider": "agents",
                "canonical_id": "agents/common-name",
                "display_id": "common-name",
                "tags": ["common"],
                "entry_sections": ["checklist"],
                "execution_mode": "inline"
            })
            .to_string(),
            ..make_skill_record(
                "common-name",
                "Common Name",
                "Common routing test skill",
                vec!["common"],
            )
        };

        let response = route_task(vec![skill], "common", 3, 0.0, false);
        assert_eq!(response.decision, "match");
        assert_eq!(response.candidates.len(), 1);

        let candidate = &response.candidates[0];
        assert_eq!(candidate.skill_id, "agents/common-name");
        // load_command uses skill.id (DB primary key) so resolve_skill can find it
        assert_eq!(
            candidate.load_command,
            "ms load common-name --section checklist -O json"
        );
        assert!(candidate.load_command.contains("common-name"));
    }

    #[test]
    fn test_route_derives_entry_sections_from_body_when_metadata_missing() {
        let skill = SkillRecord {
            metadata_json: serde_json::json!({
                "provider": "claude",
                "canonical_id": "claude/provider-route",
                "display_id": "provider-route",
                "trigger_phrases": ["provider route verification"],
                "execution_mode": "inline"
            })
            .to_string(),
            body: "# Provider Route\n\n## Overview\n\nOverview content.\n\n## Checklist\n\n- Step one\n".to_string(),
            ..make_skill_record(
                "claude/provider-route",
                "Provider Route",
                "Route-first provider verification skill",
                vec!["provider", "route", "archive"],
            )
        };

        let response = route_task(vec![skill], "provider route verification", 3, 0.65, false);
        assert_eq!(response.decision, "match");
        assert_eq!(
            response.candidates[0].entry_sections,
            vec!["overview".to_string(), "checklist".to_string()]
        );
        assert_eq!(
            response.candidates[0].load_command,
            "ms load claude/provider-route --section overview -O json"
        );
        assert_eq!(response.candidates[0].default_load, "section:overview");
    }

    #[test]
    fn test_exact_trigger_phrase_meets_default_threshold() {
        let skill = SkillRecord {
            metadata_json: serde_json::json!({
                "provider": "claude",
                "canonical_id": "claude/provider-route",
                "display_id": "provider-route",
                "tags": ["provider", "route", "archive"],
                "trigger_phrases": ["provider route verification"],
                "execution_mode": "inline"
            })
            .to_string(),
            ..make_skill_record(
                "claude/provider-route",
                "Provider Route",
                "Route-first provider verification skill",
                vec!["provider", "route", "archive"],
            )
        };

        let response = route_task(vec![skill], "provider route verification", 3, 0.65, false);
        assert_eq!(response.decision, "match");
        assert_eq!(response.candidates[0].skill_id, "claude/provider-route");
        assert!(response.candidates[0].score >= 0.65);
    }

    #[test]
    fn test_exact_trigger_phrase_is_authoritative_with_sparse_metadata() {
        let skill = SkillRecord {
            metadata_json: serde_json::json!({
                "provider": "claude",
                "canonical_id": "claude/common-name",
                "display_id": "common-name",
                "tags": ["test", "provider"],
                "trigger_phrases": ["common collision route"],
                "execution_mode": "inline"
            })
            .to_string(),
            ..make_skill_record(
                "claude/common-name",
                "Common Name",
                "Claude provider collision skill",
                vec!["test", "provider"],
            )
        };

        let response = route_task(vec![skill], "common collision route", 3, 0.65, false);
        assert_eq!(response.decision, "match");
        assert_eq!(response.candidates[0].skill_id, "claude/common-name");
        assert!(response.candidates[0].score >= 0.65);
    }

    #[test]
    fn test_sanitize_slug_used_for_section() {
        let slug = sanitize_slug("Rust Error Handling");
        assert_eq!(slug, "rust-error-handling");
    }

    #[test]
    fn test_score_trigger_phrase_prioritizes_exact_and_contained_matches() {
        assert_eq!(
            score_trigger_phrase("provider route verification", "provider route verification"),
            1.0
        );
        assert_eq!(
            score_trigger_phrase(
                "please run provider route verification with archive fallback",
                "provider route verification",
            ),
            0.9
        );
        assert_eq!(
            score_trigger_phrase("archive fallback only", "provider route verification"),
            0.0
        );
    }

    #[test]
    fn test_compact_route_keywords_can_meet_default_threshold() {
        let skill = SkillRecord {
            metadata_json: serde_json::json!({
                "provider": "codex",
                "canonical_id": "codex/rust-async-errors",
                "display_id": "rust-async-errors",
                "keywords": [
                    "fix rust async borrow checker send sync and lifetime errors with minimal changes",
                    "rust async errors"
                ],
                "tags": ["rust", "async", "debugging"],
                "execution_mode": "inline"
            })
            .to_string(),
            ..make_skill_record(
                "codex/rust-async-errors",
                "Rust Async Error Triage",
                "Fix Rust async borrow checker, Send/Sync, and lifetime errors with minimal changes.",
                vec!["rust", "async", "debugging"],
            )
        };

        let response = route_task(vec![skill], "rust async errors", 3, 0.65, false);
        assert_eq!(response.decision, "match");
        assert_eq!(response.candidates[0].skill_id, "codex/rust-async-errors");
        assert!(response.candidates[0].score >= 0.65);
    }
    // ===================== bd-64zk: ms route CLI acceptance tests =====================

    #[test]
    fn test_threshold_zero_uses_default() {
        let skill = make_skill_record("test-skill", "Test", "A test skill", vec!["test"]);
        let response = route_task(vec![skill], "test", 3, 0.0, false);
        assert_eq!(response.decision, "match");
        assert!(!response.candidates.is_empty());
    }

    #[test]
    fn test_threshold_very_high_produces_no_match() {
        let skill = make_skill_record("test-skill", "Test", "A test skill", vec!["test"]);
        let response = route_task(vec![skill], "test", 3, 1.0, false);
        assert_eq!(response.decision, "no_match");
        assert!(response.candidates.is_empty());
        assert!(response.fallback.is_some());
        assert!(
            response
                .fallback
                .as_ref()
                .unwrap()
                .search_command
                .contains("ms search")
        );
        assert!(
            response
                .fallback
                .as_ref()
                .unwrap()
                .suggest_command
                .is_none()
        );
    }

    #[test]
    fn test_no_match_response_contract() {
        let response = RouteResponse {
            route_schema_version: ROUTE_SCHEMA_VERSION,
            task: "impossible query xyz 123".to_string(),
            threshold: 0.65,
            decision: "no_match".to_string(),
            candidates: vec![],
            debug_info: vec![],
            fallback: Some(RouteFallback {
                search_command: "ms search \"impossible query xyz 123\" -O json".to_string(),
                suggest_command: None,
            }),
        };
        assert_eq!(response.decision, "no_match");
        assert!(response.candidates.is_empty());
        assert!(response.fallback.is_some());
        let fb = response.fallback.as_ref().unwrap();
        assert!(fb.search_command.contains("ms search"));
        assert!(fb.suggest_command.is_none());
    }

    #[test]
    fn test_canonical_ids_with_collision() {
        // Same-named skills from different providers get deduplicated:
        // only the highest-scoring one appears.
        let skill_a = SkillRecord {
            metadata_json: serde_json::json!({
                "provider": "claude",
                "canonical_id": "claude/shared-id",
                "display_id": "shared-id",
                "tags": ["test"],
                "trigger_phrases": ["shared id task"],
                "execution_mode": "inline"
            })
            .to_string(),
            ..make_skill_record("claude/shared-id", "Shared", "A shared skill", vec!["test"])
        };
        let skill_b = SkillRecord {
            metadata_json: serde_json::json!({
                "provider": "codex",
                "canonical_id": "codex/shared-id",
                "display_id": "shared-id",
                "tags": ["test"],
                "trigger_phrases": ["shared id task"],
                "execution_mode": "inline"
            })
            .to_string(),
            ..make_skill_record(
                "codex/shared-id",
                "Shared",
                "Another shared skill",
                vec!["test"],
            )
        };
        let response = route_task(vec![skill_a, skill_b], "shared id task", 3, 0.65, false);
        assert_eq!(response.decision, "match");
        // Dedup by name keeps only 1 (same name "Shared" from two providers)
        assert_eq!(response.candidates.len(), 1);
        // The canonical skill_id is used in load_command
        let cand = &response.candidates[0];
        assert!(
            cand.load_command.contains(&cand.skill_id),
            "load_command should use canonical skill_id: {}",
            cand.load_command
        );
    }

    #[test]
    fn test_suggest_command_always_none_in_current_implementation() {
        let skill = make_skill_record("test-skill", "Test", "A test skill", vec!["test"]);
        let response = route_task(vec![skill], "test", 3, 0.65, false);
        if let Some(ref fb) = response.fallback {
            assert!(
                fb.suggest_command.is_none(),
                "suggest_command must not be fabricated"
            );
        }
    }

    #[test]
    fn test_prefix_fuzzy_match_with_multibyte_utf8() {
        // U+2192 RIGHT ARROW is 3 bytes in UTF-8; must not panic on byte-boundary mismatch
        assert!(!prefix_fuzzy_match("html→screenshot", "git"));
        assert!(!prefix_fuzzy_match("git", "html→screenshot"));
        // Two multi-byte strings sharing a prefix
        assert!(prefix_fuzzy_match("html→screenshot", "html→other"));
        // Short multi-byte string (< 4 chars) should return false, not panic
        assert!(!prefix_fuzzy_match("→→→", "→→→→"));
    }

    #[test]
    fn test_word_fuzzy_contains_with_multibyte_utf8() {
        // Skill description containing non-ASCII arrow must not panic
        assert!(!word_fuzzy_contains(
            "HTML→screenshot multi-platform",
            "git commit"
        ));
        assert!(word_fuzzy_contains(
            "HTML→screenshot multi-platform",
            "multi"
        ));
    }

    #[test]
    fn test_score_skill_with_multibyte_description_does_not_panic() {
        let skill = make_skill_record(
            "ckm-design",
            "CKM Design",
            "social photos (HTML→screenshot, multi-platform)",
            vec!["design"],
        );
        // Must not panic when scoring skills with multi-byte UTF-8 in description
        let result = score_skill(&skill, "git commit", false);
        // Result may or may not match; the important thing is no panic
        drop(result);
    }
}
