//! ms validate - Validate skill specs (optionally with UBS)

use clap::Args;
use serde::Serialize;

use crate::app::AppContext;
use crate::cli::commands::resolve_skill_markdown;
use crate::cli::output::OutputFormat;
use crate::cli::output::{HumanLayout, emit_human, emit_json};
use crate::core::spec_lens::parse_markdown;
use crate::core::validation::{ValidationWarning, validate, validate_with_ubs};
use crate::error::{MsError, Result};
use crate::quality::ubs::{UbsClient, UbsFinding, UbsResult, UbsSeverity};
use crate::security::SafetyGate;

#[derive(Args, Debug)]
pub struct ValidateArgs {
    /// Skill ID or path to SKILL.md (omit when using --all)
    #[arg(required_unless_present = "all")]
    pub skill: Option<String>,

    /// Validate all indexed skills
    #[arg(long, conflicts_with = "skill")]
    pub all: bool,

    /// Run UBS on code blocks
    #[arg(long)]
    pub ubs: bool,
}

pub fn run(ctx: &AppContext, args: &ValidateArgs) -> Result<()> {
    let targets: Vec<(String, std::path::PathBuf)> = if args.all {
        crate::cli::commands::discover_skill_markdowns(ctx)?
            .into_iter()
            .map(|p| (p.display().to_string(), p))
            .collect()
    } else {
        let skill_ref = args.skill.as_deref().ok_or_else(|| {
            crate::error::MsError::Config("missing skill (or use --all)".to_string())
        })?;
        let path = resolve_skill_markdown(ctx, skill_ref)?;
        vec![(skill_ref.to_string(), path)]
    };

    if targets.is_empty() {
        return Err(crate::error::MsError::Config(
            "no skills found to validate".to_string(),
        ));
    }

    let mut all_reports: Vec<ValidateReport> = Vec::new();
    let mut any_ubs_failure = false;

    for (skill_label, skill_md) in &targets {
        let raw = std::fs::read_to_string(skill_md).map_err(|err| {
            crate::error::MsError::Config(format!("read {}: {err}", skill_md.display()))
        })?;
        let spec = parse_markdown(&raw)?;

        let warnings = validate(&spec)?;
        let ubs_result = if args.ubs {
            let gate = SafetyGate::from_context(ctx);
            let client = UbsClient::new(None).with_safety(gate);
            Some(validate_with_ubs(&spec, &client)?)
        } else {
            None
        };

        if let Some(result) = &ubs_result {
            if !result.is_clean() {
                any_ubs_failure = true;
            }
        }

        all_reports.push(build_report(
            skill_label,
            skill_md,
            &warnings,
            ubs_result.as_ref(),
        ));

        if ctx.output_format == OutputFormat::Human {
            render_human(skill_label, skill_md, &warnings, ubs_result.as_ref());
        }
    }

    if ctx.output_format != OutputFormat::Human {
        if all_reports.len() == 1 {
            emit_json(&all_reports[0])?;
        } else {
            emit_json(&serde_json::json!({
                "status": "ok",
                "count": all_reports.len(),
                "reports": all_reports,
            }))?;
        }
    }

    if any_ubs_failure {
        return Err(MsError::ValidationFailed(
            "UBS findings detected".to_string(),
        ));
    }

    Ok(())
}

fn render_human(
    skill_label: &str,
    skill_md: &std::path::Path,
    warnings: &[ValidationWarning],
    ubs_result: Option<&UbsResult>,
) {
    let mut layout = HumanLayout::new();
    layout.title("Validation");
    layout.kv("Skill", skill_label);
    layout.kv("Path", &skill_md.display().to_string());

    if !warnings.is_empty() {
        layout.section("Warnings");
        for warning in warnings {
            layout.bullet(&format!("{}: {}", warning.field, warning.message));
        }
    }

    if let Some(result) = &ubs_result {
        layout.section("UBS");
        layout.kv("Findings", &result.findings.len().to_string());
        layout.kv("Exit code", &result.exit_code.to_string());
        for finding in &result.findings {
            layout.bullet(&format!(
                "{}:{}:{} {} ({})",
                finding.file.display(),
                finding.line,
                finding.column,
                finding.message,
                severity_label(finding.severity)
            ));
            if let Some(fix) = &finding.suggested_fix {
                layout.bullet(&format!("fix: {fix}"));
            }
        }
    }

    if warnings.is_empty()
        && ubs_result
            .as_ref()
            .is_none_or(|r| crate::quality::ubs::UbsResult::is_clean(r))
    {
        layout.section("Status");
        layout.bullet("OK");
    }

    emit_human(layout);
}

#[derive(Serialize)]
struct ValidateReport {
    skill: String,
    path: String,
    warnings: Vec<WarningOutput>,
    ubs: Option<UbsReport>,
    clean: bool,
}

#[derive(Serialize)]
struct WarningOutput {
    field: String,
    message: String,
}

#[derive(Serialize)]
struct UbsReport {
    exit_code: i32,
    findings: usize,
    clean: bool,
    stdout: String,
    stderr: String,
    items: Vec<UbsFindingOutput>,
}

#[derive(Serialize)]
struct UbsFindingOutput {
    category: String,
    severity: String,
    file: String,
    line: u32,
    column: u32,
    message: String,
    suggested_fix: Option<String>,
}

fn build_report(
    skill: &str,
    skill_md: &std::path::Path,
    warnings: &[ValidationWarning],
    ubs_result: Option<&UbsResult>,
) -> ValidateReport {
    let warning_items = warnings
        .iter()
        .map(|warning| WarningOutput {
            field: warning.field.clone(),
            message: warning.message.clone(),
        })
        .collect::<Vec<_>>();
    let ubs = ubs_result.map(|result| UbsReport {
        exit_code: result.exit_code,
        findings: result.findings.len(),
        clean: result.is_clean(),
        stdout: result.stdout.clone(),
        stderr: result.stderr.clone(),
        items: result.findings.iter().map(map_finding).collect::<Vec<_>>(),
    });
    let clean =
        warning_items.is_empty() && ubs_result.is_none_or(crate::quality::ubs::UbsResult::is_clean);

    ValidateReport {
        skill: skill.to_string(),
        path: skill_md.display().to_string(),
        warnings: warning_items,
        ubs,
        clean,
    }
}

fn map_finding(finding: &UbsFinding) -> UbsFindingOutput {
    UbsFindingOutput {
        category: finding.category.clone(),
        severity: severity_label(finding.severity).to_string(),
        file: finding.file.display().to_string(),
        line: finding.line,
        column: finding.column,
        message: finding.message.clone(),
        suggested_fix: finding.suggested_fix.clone(),
    }
}

const fn severity_label(severity: UbsSeverity) -> &'static str {
    match severity {
        UbsSeverity::Critical => "critical",
        UbsSeverity::Important => "important",
        UbsSeverity::Contextual => "contextual",
    }
}
