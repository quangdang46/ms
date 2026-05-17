//! ms config - Manage configuration

use clap::Args;

use std::path::PathBuf;

use crate::app::AppContext;
use crate::cli::output;
use crate::cli::output::OutputFormat;
use crate::config::Config;
use crate::error::Result;

#[derive(Args, Debug)]
pub struct ConfigArgs {
    /// Configuration key to get/set, or one of `get`/`set`/`unset` to use
    /// the explicit subcommand-style invocation.
    pub key: Option<String>,

    /// Value to set (or, when `key` is `get`/`set`/`unset`, the actual key)
    pub value: Option<String>,

    /// Optional third positional: when invoked as `ms config set <key> <value>`,
    /// this is the value to assign.
    pub extra: Option<String>,

    /// List all configuration
    #[arg(long)]
    pub list: bool,

    /// Unset a configuration key
    #[arg(long)]
    pub unset: bool,
}

pub fn run(ctx: &AppContext, args: &ConfigArgs) -> Result<()> {
    let ctx = ConfigContext {
        config: ctx.config.clone(),
        config_path: ctx.config_path.clone(),
        robot_mode: ctx.output_format != OutputFormat::Human,
    };

    if args.list || args.key.is_none() {
        return emit_config(&ctx);
    }

    // Friendly handling for users who type `ms config show` thinking it's
    // a subcommand. The clap signature is `ms config [KEY] [VALUE]`, so
    // "show" is parsed as a config key and produces the opaque error
    // `unknown key: show`. Steer them to `--list`.
    if let Some(key) = args.key.as_deref() {
        if matches!(key, "show" | "list" | "ls" | "dump") && args.value.is_none() {
            return emit_config(&ctx);
        }

        // README documents `ms config get <key>` and `ms config set <key> <value>`.
        // The clap signature is `ms config [KEY] [VALUE] [EXTRA]`, so the
        // pseudo-subcommands `get`/`set`/`unset` need explicit translation.
        match key {
            "get" => {
                let target = args.value.as_deref().ok_or_else(|| {
                    crate::error::MsError::Config("ms config get <key>: missing key".to_string())
                })?;
                return get_key(&ctx, target);
            }
            "set" => {
                let target = args.value.as_deref().ok_or_else(|| {
                    crate::error::MsError::Config(
                        "ms config set <key> <value>: missing key".to_string(),
                    )
                })?;
                let raw = args.extra.as_deref().ok_or_else(|| {
                    crate::error::MsError::Config(
                        "ms config set <key> <value>: missing value".to_string(),
                    )
                })?;
                return set_key(&ctx, target, raw);
            }
            "unset" => {
                let target = args.value.as_deref().ok_or_else(|| {
                    crate::error::MsError::Config("ms config unset <key>: missing key".to_string())
                })?;
                return unset_key(&ctx, target);
            }
            _ => {}
        }
    }

    if args.unset && args.value.is_some() {
        return Err(crate::error::MsError::Config(
            "cannot use --unset with a value".to_string(),
        ));
    }

    if args.unset {
        let key = args
            .key
            .as_ref()
            .ok_or_else(|| crate::error::MsError::Config("missing key".to_string()))?;
        return unset_key(&ctx, key);
    }

    // Reject stray trailing positionals like `ms config skill_paths.project add ./skills`.
    // Without this check, the third positional ("./skills") was silently dropped and the
    // intended array key was overwritten with the literal string "add".
    if let (Some(key), Some(value), Some(extra)) = (
        args.key.as_deref(),
        args.value.as_deref(),
        args.extra.as_deref(),
    ) {
        return Err(crate::error::MsError::Config(format!(
            "ms config takes at most two positional arguments (got 3: `{key}`, `{value}`, `{extra}`).\n\
             There is no `add` subcommand. To set an array-valued key, pass the full TOML array:\n  \
             ms config {key} '[\"{extra}\"]'"
        )));
    }

    if let (Some(key), Some(value)) = (args.key.as_ref(), args.value.as_ref()) {
        return set_key(&ctx, key, value);
    }

    let key = args
        .key
        .as_ref()
        .ok_or_else(|| crate::error::MsError::Config("missing key".to_string()))?;
    get_key(&ctx, key)
}

struct ConfigContext {
    config: Config,
    config_path: PathBuf,
    robot_mode: bool,
}

fn emit_config(ctx: &ConfigContext) -> Result<()> {
    if ctx.robot_mode {
        return output::emit_json(&ctx.config);
    }

    let rendered = toml::to_string_pretty(&ctx.config)
        .map_err(|err| crate::error::MsError::Config(format!("render config: {err}")))?;
    println!("{rendered}");
    Ok(())
}

fn get_key(ctx: &ConfigContext, key: &str) -> Result<()> {
    let value = config_value_at(&ctx.config, key)?;
    if ctx.robot_mode {
        return output::emit_json(&value);
    }
    println!("{}", format_value(&value));
    Ok(())
}

fn set_key(ctx: &ConfigContext, key: &str, raw_value: &str) -> Result<()> {
    // Validate the key is a known configuration path. set_path writes into
    // the raw TOML document, so it would silently accept unknown keys.
    let current = config_value_at(&ctx.config, key)?;

    let mut doc = load_config_doc(&ctx.config_path)?;
    let value = parse_value(raw_value)?;

    // Reject type-incompatible writes. Without this check, running
    // `ms config skill_paths.project add ./skills` (where `add` is silently
    // parsed as the value because clap accepts three positionals) would
    // overwrite an array-valued key with a bare string, leaving the on-disk
    // config in a state that fails to parse on the next ms invocation.
    if !value_kinds_compatible(&current, &value) {
        return Err(crate::error::MsError::Config(format!(
            "type mismatch for `{key}`: existing value is {} but `{}` parses as {}.\n  \
             hint: to set an array, pass it as TOML, e.g. \
             ms config {key} '[\"item1\", \"item2\"]'",
            type_label(&current),
            raw_value,
            type_label(&value),
        )));
    }

    set_path(&mut doc, key, value.clone())?;
    write_config_doc(&ctx.config_path, &doc)?;

    if ctx.robot_mode {
        output::emit_json(&serde_json::json!({
            "status": "ok",
            "action": "set",
            "key": key,
            "value": value,
        }))
    } else {
        println!("set {key} = {}", format_value(&value));
        Ok(())
    }
}

fn unset_key(ctx: &ConfigContext, key: &str) -> Result<()> {
    let mut doc = load_config_doc(&ctx.config_path)?;
    unset_path(&mut doc, key)?;
    write_config_doc(&ctx.config_path, &doc)?;

    if ctx.robot_mode {
        output::emit_json(&serde_json::json!({
            "status": "ok",
            "action": "unset",
            "key": key,
        }))
    } else {
        println!("unset {key}");
        Ok(())
    }
}

fn load_config_doc(path: &std::path::Path) -> Result<toml::Value> {
    if path.exists() {
        let raw = std::fs::read_to_string(path)
            .map_err(|err| crate::error::MsError::Config(format!("read config: {err}")))?;
        let doc = toml::from_str(&raw)
            .map_err(|err| crate::error::MsError::Config(format!("parse config: {err}")))?;
        Ok(doc)
    } else {
        Ok(toml::Value::Table(toml::map::Map::new()))
    }
}

fn write_config_doc(path: &std::path::Path, doc: &toml::Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| crate::error::MsError::Config(format!("create config dir: {err}")))?;
    }
    let rendered = toml::to_string_pretty(doc)
        .map_err(|err| crate::error::MsError::Config(format!("render config: {err}")))?;
    std::fs::write(path, rendered)
        .map_err(|err| crate::error::MsError::Config(format!("write config: {err}")))?;
    Ok(())
}

fn parse_value(raw: &str) -> Result<toml::Value> {
    let direct = format!("value = {raw}");
    if let Ok(value) = toml::from_str::<toml::Value>(&direct) {
        if let Some(parsed) = value.get("value") {
            return Ok(parsed.clone());
        }
    }

    let quoted = format!("value = {}", toml::Value::String(raw.to_string()));
    let parsed = toml::from_str::<toml::Value>(&quoted)
        .map_err(|err| crate::error::MsError::Config(format!("parse value: {err}")))?;
    parsed
        .get("value")
        .cloned()
        .ok_or_else(|| crate::error::MsError::Config("parse value: missing".to_string()))
}

fn config_value_at(config: &Config, key: &str) -> Result<toml::Value> {
    let doc = toml::Value::try_from(config)
        .map_err(|err| crate::error::MsError::Config(format!("serialize config: {err}")))?;
    get_path(&doc, key)
}

fn get_path(doc: &toml::Value, key: &str) -> Result<toml::Value> {
    let mut current = doc;
    for part in key.split('.') {
        current = current
            .get(part)
            .ok_or_else(|| crate::error::MsError::Config(format!("unknown key: {key}")))?;
    }
    Ok(current.clone())
}

fn set_path(doc: &mut toml::Value, key: &str, value: toml::Value) -> Result<()> {
    let parts: Vec<&str> = key.split('.').collect();
    if parts.is_empty() {
        return Err(crate::error::MsError::Config("empty key".to_string()));
    }

    ensure_table(doc)?;
    let mut current = doc;
    for part in &parts[..parts.len() - 1] {
        let table = current
            .as_table_mut()
            .ok_or_else(|| crate::error::MsError::Config("invalid config table".to_string()))?;
        current = table
            .entry((*part).to_string())
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
        ensure_table(current)?;
    }

    let table = current
        .as_table_mut()
        .ok_or_else(|| crate::error::MsError::Config("invalid config table".to_string()))?;
    table.insert(parts[parts.len() - 1].to_string(), value);
    Ok(())
}

fn unset_path(doc: &mut toml::Value, key: &str) -> Result<()> {
    let parts: Vec<&str> = key.split('.').collect();
    if parts.is_empty() {
        return Err(crate::error::MsError::Config("empty key".to_string()));
    }

    ensure_table(doc)?;
    let mut current = doc;
    for part in &parts[..parts.len() - 1] {
        let table = current
            .as_table_mut()
            .ok_or_else(|| crate::error::MsError::Config("invalid config table".to_string()))?;
        current = table
            .get_mut(*part)
            .ok_or_else(|| crate::error::MsError::Config(format!("unknown key: {key}")))?;
        ensure_table(current)?;
    }

    let table = current
        .as_table_mut()
        .ok_or_else(|| crate::error::MsError::Config("invalid config table".to_string()))?;
    table.remove(parts[parts.len() - 1]);
    Ok(())
}

fn ensure_table(value: &mut toml::Value) -> Result<()> {
    if value.is_table() {
        Ok(())
    } else {
        Err(crate::error::MsError::Config(
            "config path is not a table".to_string(),
        ))
    }
}

fn format_value(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => s.clone(),
        _ => value.to_string(),
    }
}

/// Stable, human-friendly name for a TOML value kind. Used in error messages
/// when the user tries to overwrite a key with a value of an incompatible type.
fn type_label(value: &toml::Value) -> &'static str {
    match value {
        toml::Value::String(_) => "string",
        toml::Value::Integer(_) => "integer",
        toml::Value::Float(_) => "float",
        toml::Value::Boolean(_) => "boolean",
        toml::Value::Datetime(_) => "datetime",
        toml::Value::Array(_) => "array",
        toml::Value::Table(_) => "table",
    }
}

/// Decide whether a new value can replace an existing config value. Both
/// sides must use the same TOML kind; the one exception is that empty
/// tables are accepted in place of any value because the in-memory defaults
/// may serialize an absent key as an empty table.
fn value_kinds_compatible(current: &toml::Value, candidate: &toml::Value) -> bool {
    std::mem::discriminant(current) == std::mem::discriminant(candidate)
        || matches!(current, toml::Value::Table(t) if t.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> toml::Value {
        parse_value(raw).expect("parse_value should succeed for known-good input")
    }

    #[test]
    fn type_labels_for_common_kinds() {
        assert_eq!(type_label(&parse("\"hello\"")), "string");
        assert_eq!(type_label(&parse("42")), "integer");
        assert_eq!(type_label(&parse("true")), "boolean");
        assert_eq!(type_label(&parse("[]")), "array");
    }

    #[test]
    fn value_kinds_compatible_matches_same_kinds() {
        assert!(value_kinds_compatible(
            &parse("[\"a\"]"),
            &parse("[\"b\", \"c\"]")
        ));
        assert!(value_kinds_compatible(&parse("\"a\""), &parse("\"b\"")));
        assert!(value_kinds_compatible(&parse("1"), &parse("2")));
    }

    #[test]
    fn value_kinds_compatible_rejects_array_overwrite_with_string() {
        // This is the exact failure mode from `ms config skill_paths.project add ./skills`:
        // an array-valued key is silently overwritten with a string, corrupting the
        // on-disk config until the user manually re-runs `ms init`.
        assert!(!value_kinds_compatible(
            &parse("[\"./skills\"]"),
            &parse("\"add\"")
        ));
    }

    #[test]
    fn value_kinds_compatible_allows_empty_table_as_unknown_default() {
        let empty = toml::Value::Table(toml::map::Map::new());
        assert!(value_kinds_compatible(&empty, &parse("\"anything\"")));
        assert!(value_kinds_compatible(&empty, &parse("[\"a\"]")));
    }
}
