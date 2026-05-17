//! ms - Meta Skill CLI
//!
//! Mine CASS sessions to generate production-quality Claude Code skills.

use std::process::ExitCode;

use clap::Parser;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use ms::Result;
use ms::app::AppContext;
use ms::cli::{Cli, Commands};

fn main() -> ExitCode {
    let cli = Cli::parse();
    apply_process_output_overrides(&cli);
    init_tracing(&cli);

    let stdout_guard = cli.quiet.then(gag::Gag::stdout).transpose();
    let result = match stdout_guard {
        Ok(guard) => {
            let result = run(&cli);
            drop(guard);
            result
        }
        Err(err) => {
            eprintln!("Error: failed to apply --quiet stdout suppression: {err}");
            return ExitCode::FAILURE;
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            let json_output = cli.robot || cli.output_format() == ms::cli::OutputFormat::Json;
            if json_output {
                // Machine-readable JSON error output to stdout
                let (code, message) = match &e {
                    ms::MsError::ApprovalRequired(msg) => ("approval_required", msg.clone()),
                    ms::MsError::DestructiveBlocked(msg) => ("destructive_blocked", msg.clone()),
                    _ => ("error", e.to_string()),
                };
                let error_json = serde_json::json!({
                    "error": true,
                    "code": code,
                    "message": message,
                });
                println!("{}", serde_json::to_string(&error_json).unwrap_or_default());
            } else {
                eprintln!("Error: {e}");
            }
            ExitCode::FAILURE
        }
    }
}

fn apply_process_output_overrides(cli: &Cli) {
    match cli.color {
        Some(ms::cli::ColorMode::Always) => {
            console::set_colors_enabled(true);
            console::set_colors_enabled_stderr(true);
            colored::control::set_override(true);
        }
        Some(ms::cli::ColorMode::Never) => {
            console::set_colors_enabled(false);
            console::set_colors_enabled_stderr(false);
            colored::control::set_override(false);
        }
        Some(ms::cli::ColorMode::Auto) | None => {}
    }
}

fn run(cli: &Cli) -> Result<()> {
    if let Commands::Init(args) = &cli.command {
        return ms::cli::commands::init::run_without_context(
            cli.output_format() != ms::cli::OutputFormat::Human,
            args,
        );
    }
    let ctx = AppContext::from_cli(cli)?;
    ms::cli::commands::run(&ctx, &cli.command)
}

fn init_tracing(cli: &Cli) {
    if cli.quiet {
        return;
    }

    // Default to WARN across the board so interactive `ms` commands don't
    // spam stderr with INFO lines from `ms::storage::tx`, tantivy's file
    // watcher, etc. Verbosity flags lift specific modules:
    //   (no flag)  -> warn everywhere
    //   -v         -> ms at info, others at warn
    //   -vv        -> ms at debug, others at info
    //   -vvv       -> trace everywhere
    // RUST_LOG still wins if set, so power users can override per-crate.
    let filter = match cli.verbose {
        0 => "warn",
        1 => "warn,ms=info",
        2 => "info,ms=debug",
        _ => "trace",
    };

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(filter));

    if cli.robot {
        // JSON logging for robot mode
        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt::layer().json().with_writer(std::io::stderr))
            .init();
    } else {
        // Human-readable logging
        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt::layer().with_writer(std::io::stderr))
            .init();
    }
}
