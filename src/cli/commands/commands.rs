//! ms commands - Print a tree of every subcommand and its options.
//!
//! Discoverability surface that complements `ms <subcommand> --help`. Without
//! this, agents and operators have no way to enumerate the full CLI surface
//! short of running `--help` against every command in turn.

use clap::{Args, CommandFactory};
use serde::Serialize;

use crate::app::AppContext;
use crate::cli::Cli;
use crate::cli::output::OutputFormat;
use crate::cli::output::{HumanLayout, emit_human, emit_json};
use crate::error::Result;

#[derive(Args, Debug)]
pub struct CommandsArgs {
    /// Include flags/options for each command (full surface). Without this
    /// the output is just the command tree itself.
    #[arg(long)]
    pub options: bool,

    /// Filter by command name prefix (e.g. `bundle`, `graph`).
    #[arg(long)]
    pub filter: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct OptionInfo {
    name: String,
    short: Option<String>,
    description: String,
    takes_value: bool,
    required: bool,
}

#[derive(Debug, Clone, Serialize)]
struct CommandInfo {
    /// Full command path e.g. `ms graph export`.
    path: String,
    description: String,
    aliases: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    options: Vec<OptionInfo>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    positionals: Vec<OptionInfo>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    subcommands: Vec<CommandInfo>,
}

pub fn run(ctx: &AppContext, args: &CommandsArgs) -> Result<()> {
    let mut root_cmd = Cli::command();
    // Walk the clap tree.
    let tree = build_tree(&mut root_cmd, args.options);

    let filtered: Vec<CommandInfo> = match &args.filter {
        Some(needle) => tree
            .subcommands
            .into_iter()
            .filter(|cmd| {
                cmd.path
                    .split_whitespace()
                    .any(|tok| tok.starts_with(needle))
            })
            .collect(),
        None => tree.subcommands,
    };

    if ctx.output_format != OutputFormat::Human {
        return emit_json(&serde_json::json!({
            "status": "ok",
            "binary": tree.path,
            "description": tree.description,
            "count": filtered.len(),
            "commands": filtered,
        }));
    }

    let mut layout = HumanLayout::new();
    layout.title("ms commands");
    layout.kv("Binary", &tree.path);
    if !tree.description.is_empty() {
        layout.kv("About", &tree.description);
    }
    layout.blank();
    for cmd in &filtered {
        render_human(&mut layout, cmd, args.options, 0);
    }
    emit_human(layout);
    Ok(())
}

fn build_tree(cmd: &mut clap::Command, include_options: bool) -> CommandInfo {
    build_node(cmd, &cmd.get_name().to_string(), include_options)
}

fn build_node(cmd: &mut clap::Command, path: &str, include_options: bool) -> CommandInfo {
    let description = cmd.get_about().map(|s| s.to_string()).unwrap_or_default();

    let aliases = cmd
        .get_visible_aliases()
        .map(str::to_string)
        .collect::<Vec<_>>();

    let (options, positionals) = if include_options {
        let mut opts = Vec::new();
        let mut posns = Vec::new();
        for arg in cmd.get_arguments() {
            // Skip globally-propagated flags (--robot, --color, …) on
            // subcommands; they're documented at the top level.
            if arg.is_global_set() && path.contains(' ') {
                continue;
            }
            let info = OptionInfo {
                name: arg.get_id().to_string(),
                short: arg.get_short().map(|c| format!("-{c}")),
                description: arg.get_help().map(|s| s.to_string()).unwrap_or_default(),
                takes_value: arg.get_action().takes_values(),
                required: arg.is_required_set(),
            };
            if arg.is_positional() {
                posns.push(info);
            } else {
                opts.push(info);
            }
        }
        (opts, posns)
    } else {
        (Vec::new(), Vec::new())
    };

    let mut subcommands = Vec::new();
    for sub in cmd.get_subcommands_mut() {
        let name = sub.get_name().to_string();
        // Skip clap's auto-generated `help` subcommand, which is just an
        // alternate spelling of `--help` and adds noise to the tree.
        if name == "help" {
            continue;
        }
        let sub_path = format!("{path} {name}");
        subcommands.push(build_node(sub, &sub_path, include_options));
    }

    CommandInfo {
        path: path.to_string(),
        description,
        aliases,
        options,
        positionals,
        subcommands,
    }
}

fn render_human(layout: &mut HumanLayout, cmd: &CommandInfo, include_options: bool, depth: usize) {
    let indent = "  ".repeat(depth);
    let line = if cmd.description.is_empty() {
        format!("{indent}{}", cmd.path)
    } else {
        format!("{indent}{} — {}", cmd.path, cmd.description)
    };
    layout.push_line(line);

    if include_options {
        for arg in &cmd.positionals {
            let req = if arg.required { " (required)" } else { "" };
            layout.push_line(format!(
                "{indent}    <{}>{req}{}",
                arg.name,
                if arg.description.is_empty() {
                    String::new()
                } else {
                    format!(" — {}", arg.description)
                }
            ));
        }
        for opt in &cmd.options {
            let short = opt
                .short
                .as_deref()
                .map(|s| format!("{s}, "))
                .unwrap_or_default();
            let value = if opt.takes_value { " <VAL>" } else { "" };
            let desc = if opt.description.is_empty() {
                String::new()
            } else {
                format!(" — {}", opt.description)
            };
            layout.push_line(format!("{indent}    {short}--{}{value}{desc}", opt.name));
        }
    }

    for sub in &cmd.subcommands {
        render_human(layout, sub, include_options, depth + 1);
    }
}
