//! ms compress - Compress a loaded skill into a compact summary
//!
//! Compresses skill content into a token-efficient summary while preserving
//! rehydrate hints that point to exact `ms load` commands for full context.

use std::collections::HashSet;
use std::path::PathBuf;

use clap::Args;
use serde::Serialize;

use crate::app::AppContext;
use crate::cli::commands::resolve_skill_markdown;
use crate::cli::output::OutputFormat;
use crate::cli::output::{HumanLayout, emit_human, emit_json};
use crate::core::SkillSpec;
use crate::core::spec_lens::{compile_markdown, parse_markdown};
use crate::error::{MsError, Result};
use crate::storage::{SkillRecord, merge_skill_metadata};

/// Estimate word count from text.
fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

/// Generate a rehydrate hint — the exact `ms load` command to restore full context.
fn rehydrate_hint(skill_ref: &str, section: Option<&str>) -> String {
    match section {
        Some(sec) => format!("ms load {skill_ref} --section {sec}"),
        None => format!("ms load {skill_ref}"),
    }
}

#[derive(Debug, Clone)]
struct ResolvedCompressSkill {
    spec: SkillSpec,
    raw_markdown: String,
    load_id: String,
}

/// A compressed skill summary with rehydrate hints.
#[derive(Debug, Clone, Serialize)]
pub struct CompressedSkill {
    /// Skill ID
    pub id: String,
    /// Provider name (if any)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Compressed summary text (~500 words max)
    pub summary: String,
    /// Word count of the summary
    pub summary_words: usize,
    /// Original word count before compression
    pub original_words: usize,
    /// Compression ratio (summary / original)
    pub compression_ratio: f64,
    /// Rehydrate command to restore full context
    pub rehydrate_cmd: String,
    /// Individual section rehydrate commands
    pub section_rehydrate_cmds: Vec<SectionRehydrate>,
    /// Trigger phrases for route matching
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub trigger_phrases: Vec<String>,
}

/// Per-section rehydrate information.
#[derive(Debug, Clone, Serialize)]
pub struct SectionRehydrate {
    /// Section slug
    pub section: String,
    /// Section title
    pub title: String,
    /// Rehydrate command
    pub rehydrate_cmd: String,
    /// Word count in this section
    pub words: usize,
}

#[derive(Args, Debug)]
pub struct CompressArgs {
    /// Skill path, ID, or directory to compress
    #[arg(value_name = "SKILL")]
    pub skill: Option<String>,

    /// Print the rehydrate command to stderr after compression
    #[arg(long, short = 'r')]
    pub rehydrate: bool,

    /// Target word budget for summary
    #[arg(long, default_value_t = 500)]
    pub budget: usize,

    /// Include section-level rehydrate hints
    #[arg(long)]
    pub sections: bool,
}

pub fn run(ctx: &AppContext, args: &CompressArgs) -> Result<()> {
    let resolved = resolve_compress_skill(ctx, args.skill.as_deref())?;
    let compressed_skill = build_compressed_skill(
        &resolved.spec,
        &resolved.raw_markdown,
        &resolved.load_id,
        args,
    )?;

    // Handle --rehydrate flag: execute the rehydrate command
    if args.rehydrate {
        let cmd = &compressed_skill.rehydrate_cmd;
        eprintln!("Rehydrating: {cmd}");
        // Print the compressed summary first, then the user can re-run the load cmd
    }

    // Output
    emit_compress_output(ctx, &compressed_skill)
}

fn resolve_compress_skill(
    ctx: &AppContext,
    skill_ref: Option<&str>,
) -> Result<ResolvedCompressSkill> {
    match skill_ref {
        Some(skill_ref) => {
            if let Ok(path) = resolve_skill_markdown(ctx, skill_ref) {
                return resolve_markdown_skill(path);
            }
            let skill = resolve_stored_skill(ctx, skill_ref)?;
            resolve_db_skill(&skill)
        }
        None => {
            let current = std::env::current_dir()
                .map_err(|e| MsError::Config(format!("cannot get current dir: {e}")))?;
            let skill_md = current.join("SKILL.md");
            if skill_md.exists() {
                resolve_markdown_skill(skill_md)
            } else {
                Err(MsError::Config(
                    "No SKILL.md found. Specify a skill path or ID.".into(),
                ))
            }
        }
    }
}

fn resolve_markdown_skill(path: PathBuf) -> Result<ResolvedCompressSkill> {
    let raw_markdown = std::fs::read_to_string(&path)
        .map_err(|e| MsError::Config(format!("read {}: {e}", path.display())))?;
    let mut spec = parse_markdown(&raw_markdown)?;
    spec.metadata.normalize_ids();
    let load_id = spec.metadata.storage_id();

    Ok(ResolvedCompressSkill {
        spec,
        raw_markdown,
        load_id,
    })
}

fn resolve_db_skill(skill: &SkillRecord) -> Result<ResolvedCompressSkill> {
    let parsed = parse_markdown(&skill.body)
        .map_err(|e| MsError::ValidationFailed(format!("failed to parse skill body: {e}")))?;
    let mut spec = parsed;
    spec.metadata = merge_skill_metadata(skill, &spec.metadata);
    spec.metadata.normalize_ids();
    let raw_markdown = compile_markdown(&spec);
    let load_id = spec.metadata.storage_id();
    Ok(ResolvedCompressSkill {
        spec,
        raw_markdown,
        load_id,
    })
}

fn resolve_stored_skill(ctx: &AppContext, skill_ref: &str) -> Result<SkillRecord> {
    let direct = ctx.db.get_skill(skill_ref)?;

    if skill_ref.contains('/') {
        if let Some(skill) = direct {
            return Ok(skill);
        }
    }

    if let Some(alias_result) = ctx.db.resolve_alias(skill_ref)? {
        if let Some(skill) = ctx.db.get_skill(&alias_result.canonical_id)? {
            return Ok(skill);
        }
    }

    let mut matches = ctx.db.find_skills_by_metadata_ref(skill_ref)?;
    if let Some(skill) = &direct {
        if !matches.iter().any(|candidate| candidate.id == skill.id) {
            matches.push(skill.clone());
        }
    }

    match matches.as_slice() {
        [skill] => return Ok(skill.clone()),
        [] => {}
        matches => {
            let ids = matches
                .iter()
                .map(|skill| skill.id.clone())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(MsError::ValidationFailed(format!(
                "skill reference '{}' is ambiguous; use one of: {}",
                skill_ref, ids
            )));
        }
    }

    if let Some(skill) = direct {
        return Ok(skill);
    }

    if let Some(archive_path) = ctx.git.skill_path(skill_ref) {
        if archive_path.exists() {
            return Err(MsError::SkillNotFound(format!(
                "skill not indexed: {skill_ref} (found in archive - run 'ms index' to add)"
            )));
        }
    }

    Err(MsError::SkillNotFound(format!(
        "skill not found: {skill_ref}"
    )))
}

fn build_compressed_skill(
    spec: &SkillSpec,
    raw_markdown: &str,
    load_id: &str,
    args: &CompressArgs,
) -> Result<CompressedSkill> {
    let compressed = compress_skill(spec, args.budget)?;
    let original_words = word_count(raw_markdown);
    let summary_words = word_count(&compressed);
    let compression_ratio = if original_words > 0 {
        summary_words as f64 / original_words as f64
    } else {
        0.0
    };

    let section_rehydrate_cmds = if args.sections {
        spec.sections
            .iter()
            .map(|section| SectionRehydrate {
                section: section.id.clone(),
                title: section.title.clone(),
                rehydrate_cmd: rehydrate_hint(load_id, Some(section.id.as_str())),
                words: section
                    .blocks
                    .iter()
                    .map(|block| word_count(&block.content))
                    .sum(),
            })
            .collect()
    } else {
        Vec::new()
    };

    Ok(CompressedSkill {
        id: spec.metadata.id.clone(),
        provider: Some(spec.metadata.provider.clone()).filter(|value| !value.is_empty()),
        summary: compressed,
        summary_words,
        original_words,
        compression_ratio,
        rehydrate_cmd: rehydrate_hint(load_id, None),
        section_rehydrate_cmds,
        trigger_phrases: collect_trigger_hints(spec),
    })
}

fn collect_trigger_hints(spec: &SkillSpec) -> Vec<String> {
    let mut hints = Vec::new();
    let mut seen = HashSet::new();

    if !spec.metadata.description.is_empty() {
        let desc_words: Vec<&str> = spec
            .metadata
            .description
            .split_whitespace()
            .filter(|w| w.len() > 3)
            .take(10)
            .collect();
        if !desc_words.is_empty() {
            push_hint(
                &mut hints,
                &mut seen,
                format!("description: {}", desc_words.join(" ")),
            );
        }
    }

    for tag in &spec.metadata.tags {
        push_hint(&mut hints, &mut seen, format!("tag:{tag}"));
    }

    for phrase in &spec.metadata.trigger_phrases {
        push_hint(&mut hints, &mut seen, format!("trigger:{phrase}"));
    }

    hints
}

fn push_hint(hints: &mut Vec<String>, seen: &mut HashSet<String>, hint: String) {
    if seen.insert(hint.clone()) {
        hints.push(hint);
    }
}

/// Compress skill content into a token-efficient summary.
fn compress_skill(spec: &SkillSpec, budget: usize) -> Result<String> {
    let mut summary = String::new();

    // Header with metadata
    summary.push_str(&format!(
        "# {} ({})\n\n",
        spec.metadata.name, spec.metadata.id
    ));

    // Description
    if !spec.metadata.description.is_empty() {
        summary.push_str(&format!("> {}\n\n", spec.metadata.description));
    }

    // Tags
    if !spec.metadata.tags.is_empty() {
        summary.push_str(&format!("**Tags:** {}\n\n", spec.metadata.tags.join(", ")));
    }

    // Compress each section
    let mut total_words = word_count(&summary);
    let word_budget_per_section = if !spec.sections.is_empty() {
        (budget.saturating_sub(total_words)).max(50) / spec.sections.len()
    } else {
        budget
    };

    for section in &spec.sections {
        if total_words >= budget {
            summary.push_str("*(truncated to budget)*\n");
            break;
        }

        summary.push_str(&format!("## {}\n\n", section.title));

        let mut section_words = 0;
        for block in &section.blocks {
            if section_words >= word_budget_per_section {
                break;
            }

            let content = block.content.trim();
            if content.is_empty() {
                continue;
            }

            let block_words = word_count(content);
            let remaining = word_budget_per_section.saturating_sub(section_words);

            if block_words > remaining && remaining > 3 {
                // Truncate text content to fit budget
                let words: Vec<&str> = content.split_whitespace().take(remaining).collect();
                summary.push_str(&format!("{}...\n\n", words.join(" ")));
                section_words += remaining;
            } else if block_words <= remaining || block_words < 5 {
                summary.push_str(content);
                summary.push_str("\n\n");
                section_words += block_words;
            }
        }
        total_words = word_count(&summary);
        _ = total_words;
    }

    if summary.trim().is_empty() {
        // Fallback for very empty skills: use whatever metadata we have
        summary.push_str("*(skill has no content beyond metadata)*");
    }

    Ok(summary)
}

fn emit_compress_output(ctx: &AppContext, compressed: &CompressedSkill) -> Result<()> {
    match ctx.output_format {
        OutputFormat::Human | OutputFormat::Plain => {
            let mut layout = HumanLayout::new();
            layout.title(&format!("Compressed: {}", compressed.id));
            layout.kv("Original Words", &compressed.original_words.to_string());
            layout.kv("Summary Words", &compressed.summary_words.to_string());
            layout.kv(
                "Compression",
                &format!("{:.1}%", (1.0 - compressed.compression_ratio) * 100.0),
            );
            layout.blank();
            layout.section("Summary");
            layout.push_line(&compressed.summary);
            layout.blank();
            layout.section("Rehydrate");
            layout.push_line(format!("`{}`", compressed.rehydrate_cmd));
            if !compressed.section_rehydrate_cmds.is_empty() {
                layout.blank();
                layout.section("Section Rehydrate Commands");
                for sec in &compressed.section_rehydrate_cmds {
                    layout.push_line(format!(
                        "  `{}` — {} ({} words)",
                        sec.rehydrate_cmd, sec.title, sec.words
                    ));
                }
            }
            if !compressed.trigger_phrases.is_empty() {
                layout.blank();
                layout.section("Trigger Phrases");
                for phrase in &compressed.trigger_phrases {
                    layout.bullet(phrase);
                }
            }
            emit_human(layout);
        }
        OutputFormat::Json | OutputFormat::Jsonl => {
            emit_json(compressed)?;
        }
        OutputFormat::Tsv => {
            println!(
                "{}\t{}\t{}\t{}\t{}",
                compressed.id,
                compressed.original_words,
                compressed.summary_words,
                compressed.compression_ratio,
                compressed.rehydrate_cmd
            );
        }
        OutputFormat::Toon => {
            // Toon format output for compress - emit as plain text summary
            println!("{}", compressed.summary);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::skill::{SkillBlock, SkillSection};
    use crate::core::spec_lens::parse_markdown;

    fn make_test_spec() -> SkillSpec {
        let mut spec = SkillSpec::new("test-skill", "Test Skill");
        spec.metadata.description =
            "A skill for testing compress functionality with various sections.".to_string();
        spec.metadata.tags = vec!["test".to_string(), "compress".to_string()];
        spec.metadata.provider = "test-provider".to_string();
        spec.metadata.canonical_id = "test-provider/test-skill".to_string();
        spec.metadata.display_id = "test-skill".to_string();
        spec.sections.push(SkillSection {
            id: "overview".to_string(),
            title: "Overview".to_string(),
            blocks: vec![SkillBlock {
                id: "intro".to_string(),
                block_type: crate::core::skill::BlockType::Text,
                content: "This is the overview section with important information about the skill."
                    .to_string(),
            }],
        });
        spec.sections.push(SkillSection {
            id: "rules".to_string(),
            title: "Rules".to_string(),
            blocks: vec![SkillBlock {
                id: "rule-1".to_string(),
                block_type: crate::core::skill::BlockType::Rule,
                content: "Always test compression output.".to_string(),
            }],
        });
        spec
    }

    #[test]
    fn test_compress_skill_produces_summary() {
        let spec = make_test_spec();
        let result = compress_skill(&spec, 500).unwrap();
        assert!(!result.is_empty());
        assert!(result.contains("Test Skill"));
        assert!(result.contains("test-skill"));
    }

    #[test]
    fn test_compress_respects_budget() {
        let spec = make_test_spec();
        let result = compress_skill(&spec, 50).unwrap();
        assert!(word_count(&result) <= 60); // slight tolerance
    }

    #[test]
    fn test_rehydrate_hint_default() {
        let hint = rehydrate_hint("my-skill", None);
        assert_eq!(hint, "ms load my-skill");
    }

    #[test]
    fn test_rehydrate_hint_with_provider() {
        let hint = rehydrate_hint("community/my-skill", None);
        assert_eq!(hint, "ms load community/my-skill");
    }

    #[test]
    fn test_rehydrate_hint_with_section() {
        let hint = rehydrate_hint("my-skill", Some("rules"));
        assert_eq!(hint, "ms load my-skill --section rules");
    }

    #[test]
    fn test_rehydrate_hint_full() {
        let hint = rehydrate_hint("official/my-skill", Some("overview"));
        assert_eq!(hint, "ms load official/my-skill --section overview");
    }

    #[test]
    fn test_word_count_empty() {
        assert_eq!(word_count(""), 0);
    }

    #[test]
    fn test_word_count_normal() {
        assert_eq!(word_count("hello world"), 2);
    }

    #[test]
    fn test_trigger_phrases_extracted() {
        let mut spec = make_test_spec();
        spec.metadata.trigger_phrases = vec!["compress provider".to_string()];
        let phrases = collect_trigger_hints(&spec);
        assert!(phrases.iter().any(|p| p.contains("testing")));
        assert!(phrases.iter().any(|p| p == "tag:test"));
        assert!(phrases.iter().any(|p| p == "tag:compress"));
        assert!(phrases.iter().any(|p| p == "trigger:compress provider"));
    }

    #[test]
    fn test_compress_from_markdown_roundtrip() {
        let md = r#"---
id: roundtrip-skill
name: Roundtrip Test
description: A skill for roundtrip testing.
tags: [test, roundtrip]
---
# Overview
Some content here.
"#;
        let spec = parse_markdown(md).unwrap();
        let result = compress_skill(&spec, 200).unwrap();
        assert!(result.contains("Roundtrip Test"));
        assert!(result.contains("roundtrip-skill"));
    }

    #[test]
    fn test_compress_empty_sections() {
        let mut spec = SkillSpec::new("empty", "Empty Skill");
        spec.metadata.description = "An empty skill.".to_string();
        let result = compress_skill(&spec, 100).unwrap();
        assert!(!result.is_empty());
        assert!(result.contains("Empty Skill"));
    }

    #[test]
    fn test_build_compressed_skill_uses_canonical_rehydrate_for_provider_skills() {
        let spec = make_test_spec();
        let raw_markdown = compile_markdown(&spec);
        let args = CompressArgs {
            skill: Some("test-provider/test-skill".to_string()),
            rehydrate: false,
            budget: 500,
            sections: true,
        };

        let compressed =
            build_compressed_skill(&spec, &raw_markdown, &spec.metadata.storage_id(), &args)
                .unwrap();

        assert_eq!(compressed.rehydrate_cmd, "ms load test-provider/test-skill");
        assert_eq!(
            compressed.section_rehydrate_cmds[0].rehydrate_cmd,
            "ms load test-provider/test-skill --section overview"
        );
    }
}
