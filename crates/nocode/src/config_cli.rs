//! `nocode config` — list / get / set keys in the user-tier config.
//!
//! Mirrors the `git config` mental model. No interactive prompts, no TUI,
//! just plain CLI subcommands operating on `~/.nocode/config.toml`.
//!
//! Subcommands:
//! - `list`             — print the full file (or `--json`)
//! - `get <key>`        — print one scalar
//! - `set <key> <val>`  — write one scalar (creates file if missing)
//! - `unset <key>`      — remove one scalar
//!
//! Supported dotted keys (subset that covers 95% of usage):
//! - `default_provider`, `model`, `permission_mode`, `max_turns`, `max_tokens`
//! - `reasoning_effort`, `system_prompt`, `telemetry_enabled`
//!
//! Provider/profile tables are intentionally NOT mutable from the CLI — those
//! belong in the toml file and benefit from being edited as a block.

use std::fs;
use std::path::PathBuf;

use nocode_core::config::settings::Settings;

pub fn run(args: &[String]) {
    let sub = args.iter().find(|a| !a.starts_with('-')).cloned();
    match sub.as_deref() {
        Some("list") | None => cmd_list(args.iter().any(|a| a == "--json")),
        Some("get") => cmd_get(args),
        Some("set") => cmd_set(args),
        Some("unset") => cmd_unset(args),
        Some(other) => {
            eprintln!("Unknown config subcommand: {other}");
            print_help();
        }
    }
}

fn print_help() {
    println!(
        "Usage: nocode config <list|get|set|unset> [args]\n\n\
         list             Print the full user config (or --json)\n  \
         get <key>        Print a single scalar\n  \
         set <key> <val>  Write a single scalar\n  \
         unset <key>      Remove a single scalar\n"
    );
}

fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_owned());
    PathBuf::from(home).join(".nocode/config.toml")
}

fn cmd_list(json: bool) {
    let path = config_path();
    if !path.exists() {
        eprintln!(
            "No config at {}. Run `nocode init` to scaffold one.",
            path.display()
        );
        return;
    }
    let raw = fs::read_to_string(&path).unwrap_or_default();
    if json {
        // Best-effort: parse and re-emit as JSON.
        match toml::from_str::<toml::Value>(&raw) {
            Ok(v) => println!(
                "{}",
                serde_json::to_string_pretty(&v).unwrap_or_else(|_| "{}".to_owned())
            ),
            Err(e) => eprintln!("Parse error: {e}"),
        }
        return;
    }
    print!("{raw}");
}

fn cmd_get(args: &[String]) {
    let key = match args.iter().skip_while(|a| a.as_str() != "get").nth(1) {
        Some(k) => k,
        None => {
            eprintln!("Usage: nocode config get <key>");
            return;
        }
    };
    let s = Settings::load_from(&config_path());
    match scalar_get(&s, key) {
        Some(v) => println!("{v}"),
        None => {
            eprintln!("Key '{key}' is unset");
            std::process::exit(1);
        }
    }
}

fn cmd_set(args: &[String]) {
    let mut iter = args.iter().skip_while(|a| a.as_str() != "set");
    iter.next(); // skip "set"
    let key = match iter.next() {
        Some(k) => k.clone(),
        None => {
            eprintln!("Usage: nocode config set <key> <value>");
            return;
        }
    };
    let value = match iter.next() {
        Some(v) => v.clone(),
        None => {
            eprintln!("Usage: nocode config set <key> <value>");
            return;
        }
    };
    let mut s = Settings::load_from(&config_path());
    if let Err(e) = scalar_set(&mut s, &key, &value) {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
    if let Err(e) = s.save_to(&config_path()) {
        eprintln!("Failed to save: {e}");
        std::process::exit(1);
    }
    println!("Set {key} = {value}");
}

fn cmd_unset(args: &[String]) {
    let key = match args.iter().skip_while(|a| a.as_str() != "unset").nth(1) {
        Some(k) => k.clone(),
        None => {
            eprintln!("Usage: nocode config unset <key>");
            return;
        }
    };
    let mut s = Settings::load_from(&config_path());
    if let Err(e) = scalar_unset(&mut s, &key) {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
    if let Err(e) = s.save_to(&config_path()) {
        eprintln!("Failed to save: {e}");
        std::process::exit(1);
    }
    println!("Unset {key}");
}

/// Whitelisted scalar keys. Refusing to expose tables (providers/profiles)
/// is deliberate — they belong in the TOML file.
fn scalar_get(s: &Settings, key: &str) -> Option<String> {
    match key {
        "default_provider" => s.default_provider.clone(),
        "model" => s.model.clone(),
        "permission_mode" => s.permission_mode.clone(),
        "max_turns" => s.max_turns.map(|v| v.to_string()),
        "max_tokens" => s.max_tokens.map(|v| v.to_string()),
        "reasoning_effort" => s.reasoning_effort.clone(),
        "system_prompt" => s.system_prompt.clone(),
        "telemetry_enabled" => s.telemetry_enabled.map(|v| v.to_string()),
        _ => None,
    }
}

fn scalar_set(s: &mut Settings, key: &str, value: &str) -> Result<(), String> {
    match key {
        "default_provider" => s.default_provider = Some(value.to_owned()),
        "model" => s.model = Some(value.to_owned()),
        "permission_mode" => s.permission_mode = Some(value.to_owned()),
        "max_turns" => s.max_turns = Some(value.parse().map_err(|e| format!("max_turns: {e}"))?),
        "max_tokens" => s.max_tokens = Some(value.parse().map_err(|e| format!("max_tokens: {e}"))?),
        "reasoning_effort" => s.reasoning_effort = Some(value.to_owned()),
        "system_prompt" => s.system_prompt = Some(value.to_owned()),
        "telemetry_enabled" => {
            s.telemetry_enabled = Some(
                value
                    .parse()
                    .map_err(|e| format!("telemetry_enabled: {e}"))?,
            );
        }
        _ => return Err(unknown_key_msg(key)),
    }
    Ok(())
}

fn scalar_unset(s: &mut Settings, key: &str) -> Result<(), String> {
    match key {
        "default_provider" => s.default_provider = None,
        "model" => s.model = None,
        "permission_mode" => s.permission_mode = None,
        "max_turns" => s.max_turns = None,
        "max_tokens" => s.max_tokens = None,
        "reasoning_effort" => s.reasoning_effort = None,
        "system_prompt" => s.system_prompt = None,
        "telemetry_enabled" => s.telemetry_enabled = None,
        _ => return Err(unknown_key_msg(key)),
    }
    Ok(())
}

fn unknown_key_msg(key: &str) -> String {
    format!(
        "Unknown key '{key}'. Supported: default_provider, model, permission_mode, \
         max_turns, max_tokens, reasoning_effort, system_prompt, telemetry_enabled. \
         Edit ~/.nocode/config.toml directly for [providers.*] / [profiles.*] / \
         [mcp_servers.*] / [hooks] / [sandbox] tables."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_get_set_round_trip() {
        let mut s = Settings::default();
        scalar_set(&mut s, "model", "gpt-5.5").unwrap();
        assert_eq!(scalar_get(&s, "model").as_deref(), Some("gpt-5.5"));
    }

    #[test]
    fn scalar_set_int_fields_validates() {
        let mut s = Settings::default();
        assert!(scalar_set(&mut s, "max_turns", "not-a-number").is_err());
        assert!(scalar_set(&mut s, "max_turns", "42").is_ok());
        assert_eq!(s.max_turns, Some(42));
    }

    #[test]
    fn unknown_key_errors() {
        let mut s = Settings::default();
        assert!(scalar_set(&mut s, "providers.foo.base_url", "x").is_err());
    }

    #[test]
    fn unset_clears_value() {
        let mut s = Settings {
            model: Some("x".to_owned()),
            ..Default::default()
        };
        scalar_unset(&mut s, "model").unwrap();
        assert!(s.model.is_none());
    }
}
