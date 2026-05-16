//! ms fmt - Format skill files

use clap::Args;
use itertools::Itertools;

use crate::app::AppContext;
use crate::cli::commands::{discover_skill_markdowns, resolve_skill_markdown};
use crate::cli::output::OutputFormat;
use crate::cli::output::emit_json;
use crate::core::spec_lens::{compile_markdown, parse_markdown};
use crate::error::Result;

#[derive(Args, Debug)]
pub struct FmtArgs {
    /// Skills to format (default: all when --all is set or no skills given)
    pub skills: Vec<String>,

    /// Format every indexed skill (explicit)
    #[arg(long)]
    pub all: bool,

    /// Check formatting without modifying
    #[arg(long)]
    pub check: bool,

    /// Show diff instead of modifying
    #[arg(long)]
    pub diff: bool,
}

pub fn run(ctx: &AppContext, args: &FmtArgs) -> Result<()> {
    let targets = if args.skills.is_empty() || args.all {
        discover_skill_markdowns(ctx)?
    } else {
        args.skills
            .iter()
            .map(|skill| resolve_skill_markdown(ctx, skill))
            .collect::<Result<Vec<_>>>()?
    };

    let total = targets.len();
    let mut dirty = Vec::new();
    let mut formatted = Vec::new();

    for path in targets {
        let raw = std::fs::read_to_string(&path).map_err(|err| {
            crate::error::MsError::Config(format!("read {}: {err}", path.display()))
        })?;
        let spec = parse_markdown(&raw)?;
        let compiled = compile_markdown(&spec);

        if raw != compiled {
            dirty.push(path.clone());
        }

        if args.diff {
            let diff = simple_diff(&raw, &compiled);
            println!("--- {}", path.display());
            println!("+++ formatted");
            print!("{diff}");
            continue;
        }

        if args.check {
            continue;
        }

        if raw != compiled {
            std::fs::write(&path, compiled).map_err(|err| {
                crate::error::MsError::Config(format!("write {}: {err}", path.display()))
            })?;
            formatted.push(path);
        }
    }

    if args.check && !dirty.is_empty() {
        // Surface the list of files that would change so users (and CI) can
        // see exactly what is out of sync without a separate diff run.
        if ctx.output_format != OutputFormat::Human {
            emit_json(&serde_json::json!({
                "status": "error",
                "code": "fmt_check_failed",
                "files_needing_format": dirty
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>(),
                "count": dirty.len(),
            }))?;
        } else {
            eprintln!("Files needing formatting ({}):", dirty.len());
            for path in &dirty {
                eprintln!("  {}", path.display());
            }
        }
        return Err(crate::error::MsError::ValidationFailed(format!(
            "{} files need formatting",
            dirty.len()
        )));
    }

    // Surface a summary so users can tell whether `fmt` did anything. The
    // bug report flagged silent fmt as indistinguishable from a no-op crash.
    if !args.diff {
        if ctx.output_format != OutputFormat::Human {
            emit_json(&serde_json::json!({
                "status": "ok",
                "scanned": total,
                "formatted": formatted
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>(),
                "formatted_count": formatted.len(),
            }))?;
        } else if formatted.is_empty() {
            println!("fmt: scanned {total} file(s), no changes needed");
        } else {
            println!(
                "fmt: scanned {total} file(s), formatted {}:",
                formatted.len()
            );
            for path in &formatted {
                println!("  {}", path.display());
            }
        }
    }

    Ok(())
}

fn simple_diff(old: &str, new: &str) -> String {
    let mut out = String::new();
    for pair in old.lines().zip_longest(new.lines()) {
        match pair {
            itertools::EitherOrBoth::Both(left, right) if left == right => {}
            itertools::EitherOrBoth::Both(left, right) => {
                out.push('-');
                out.push_str(left);
                out.push('\n');
                out.push('+');
                out.push_str(right);
                out.push('\n');
            }
            itertools::EitherOrBoth::Left(left) => {
                out.push('-');
                out.push_str(left);
                out.push('\n');
            }
            itertools::EitherOrBoth::Right(right) => {
                out.push('+');
                out.push_str(right);
                out.push('\n');
            }
        }
    }
    out
}
