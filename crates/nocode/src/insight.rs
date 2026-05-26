//! `nocode insight` — observability subcommands.
//!
//! Inspired by [antcode](https://github.com/telagod/antcode)'s observer
//! discipline: each subcommand answers one specific question. No monolithic
//! dashboard, no auto-refresh, no JSON-rendered text. Plain columnar output;
//! `--json` flag flips to structured output for scripting.
//!
//! Subcommands:
//! - `where`     — show the active state directory and current sqlite volume
//! - `sessions`  — list recent sessions (id / created / model / msgs)
//! - `tools`     — tool-call frequency and deny-trail breakdown
//! - `gates`     — which Policy gate denied the most this week?
//! - `cost`      — token usage by provider/model
//! - (default)   — terse 7-day summary
//!
//! The data backend is `nocode-core`'s `SqlStore` (date-partitioned rusqlite
//! volumes under `~/.nocode/data/`).

use std::collections::BTreeMap;
use std::env;

use nocode_core::storage::sql::SqlStore;

/// Default base directory. Mirrors `SqlStore::new(&format!("{home}/.nocode/data"))`.
fn default_data_dir() -> String {
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_owned());
    format!("{home}/.nocode/data")
}

/// Entry point — dispatched from `main.rs` when `--insight` is on argv.
pub fn run(args: &[String]) {
    let sub = args.iter().find(|a| !a.starts_with('-')).cloned();
    let json = args.iter().any(|a| a == "--json");
    match sub.as_deref() {
        None | Some("summary") => cmd_summary(json),
        Some("where") => cmd_where(json),
        Some("sessions") => cmd_sessions(json),
        Some("tools") => cmd_tools(json),
        Some("gates") => cmd_gates(json),
        Some("cost") => cmd_cost(json),
        Some(other) => {
            eprintln!("Unknown insight subcommand: {other}");
            print_help();
        }
    }
}

fn print_help() {
    println!(
        "Usage: nocode insight [<subcommand>] [--json]\n\n\
         Subcommands:\n  \
         where     Show active state directory and sqlite volume\n  \
         sessions  Recent sessions (default 20)\n  \
         tools     Tool-call frequency and error counts\n  \
         gates     Which Policy gate denied the most\n  \
         cost      Token usage by provider/model\n  \
         (none)    Default 7-day summary\n"
    );
}

// ---------------------------------------------------------------------------
// where
// ---------------------------------------------------------------------------

fn cmd_where(json: bool) {
    let dir = default_data_dir();
    let store = match SqlStore::new(&dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error opening data dir {dir}: {e}");
            return;
        }
    };
    let volumes = store.list_volumes().unwrap_or_default();
    let active = volumes.last().cloned();
    if json {
        println!(
            "{{\"data_dir\":\"{dir}\",\"active_volume\":{active},\"all_volumes\":{all}}}",
            active = match active {
                Some(ref v) => format!("\"nocode_{v}.db\""),
                None => "null".to_owned(),
            },
            all = serde_json_array(&volumes)
        );
        return;
    }
    println!("Data directory: {dir}");
    if let Some(v) = active {
        println!("Active volume:  {dir}/nocode_{v}.db");
    } else {
        println!("Active volume:  (none — no sessions persisted yet)");
    }
    println!("Volumes total:  {}", volumes.len());
}

fn serde_json_array(items: &[String]) -> String {
    let mut s = String::from("[");
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push('"');
        s.push_str(item);
        s.push('"');
    }
    s.push(']');
    s
}

// ---------------------------------------------------------------------------
// sessions
// ---------------------------------------------------------------------------

fn cmd_sessions(json: bool) {
    let store = match SqlStore::new(&default_data_dir()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return;
        }
    };
    let rows = store.list_sessions(20).unwrap_or_default();
    if json {
        println!("[");
        for (i, r) in rows.iter().enumerate() {
            print!(
                "  {{\"id\":\"{}\",\"created_at\":\"{}\",\"model\":\"{}\",\"messages\":{},\"status\":\"{}\"}}",
                r.id,
                r.created_at,
                r.model.as_deref().unwrap_or(""),
                r.message_count,
                r.status,
            );
            if i + 1 < rows.len() {
                print!(",");
            }
            println!();
        }
        println!("]");
        return;
    }
    if rows.is_empty() {
        println!("(no sessions yet)");
        return;
    }
    println!("{:<32} {:<20} {:<32} {:>5} STATUS", "ID", "CREATED", "MODEL", "MSGS");
    for r in rows {
        println!(
            "{:<32} {:<20} {:<32} {:>5} {}",
            truncate(&r.id, 32),
            truncate(&r.created_at, 20),
            truncate(r.model.as_deref().unwrap_or("-"), 32),
            r.message_count,
            r.status
        );
    }
}

// ---------------------------------------------------------------------------
// tools — tool-call frequency from the telemetry stream
// ---------------------------------------------------------------------------

fn cmd_tools(json: bool) {
    let store = match SqlStore::new(&default_data_dir()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return;
        }
    };
    let rows = store.list_telemetry(2_000).unwrap_or_default();
    let mut counts: BTreeMap<String, (u64, u64)> = BTreeMap::new(); // tool -> (calls, errors)
    for row in &rows {
        if row.event_type != "tool_call" {
            continue;
        }
        // data is best-effort JSON; extract "tool" + "is_error" via cheap text scan.
        let data = row.data.as_deref().unwrap_or("");
        let tool = json_str_field(data, "tool").unwrap_or_else(|| "unknown".to_owned());
        let entry = counts.entry(tool).or_insert((0, 0));
        entry.0 += 1;
        if json_bool_field(data, "is_error").unwrap_or(false) {
            entry.1 += 1;
        }
    }

    let mut entries: Vec<_> = counts.into_iter().collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.1.0));

    if json {
        println!("[");
        for (i, (tool, (calls, errs))) in entries.iter().enumerate() {
            print!(
                "  {{\"tool\":\"{tool}\",\"calls\":{calls},\"errors\":{errs}}}"
            );
            if i + 1 < entries.len() {
                print!(",");
            }
            println!();
        }
        println!("]");
        return;
    }
    if entries.is_empty() {
        println!("(no tool_call telemetry recorded yet)");
        return;
    }
    println!("{:<24} {:>8} {:>8}", "TOOL", "CALLS", "ERRORS");
    for (tool, (calls, errs)) in entries {
        println!("{tool:<24} {calls:>8} {errs:>8}");
    }
}

// ---------------------------------------------------------------------------
// gates — which Policy gate denied the most
// ---------------------------------------------------------------------------

fn cmd_gates(json: bool) {
    let store = match SqlStore::new(&default_data_dir()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return;
        }
    };
    let rows = store.list_telemetry(2_000).unwrap_or_default();
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    for row in &rows {
        if row.event_type != "tool_call" {
            continue;
        }
        let data = row.data.as_deref().unwrap_or("");
        if !json_bool_field(data, "is_error").unwrap_or(false) {
            continue;
        }
        let trail = json_str_field(data, "result").unwrap_or_default();
        if let Some(gate) = parse_gate(&trail) {
            *counts.entry(gate).or_insert(0) += 1;
        }
    }
    let mut entries: Vec<_> = counts.into_iter().collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.1));

    if json {
        println!("[");
        for (i, (gate, n)) in entries.iter().enumerate() {
            print!("  {{\"gate\":\"{gate}\",\"denials\":{n}}}");
            if i + 1 < entries.len() {
                print!(",");
            }
            println!();
        }
        println!("]");
        return;
    }
    if entries.is_empty() {
        println!("(no Policy denials recorded — your harness has been silent)");
        return;
    }
    println!("{:<16} {:>10}", "GATE", "DENIALS");
    for (gate, n) in entries {
        println!("{gate:<16} {n:>10}");
    }
}

/// Extract the gate name from a `Denied [<gate>: <reason>]` trail.
fn parse_gate(trail: &str) -> Option<String> {
    let rest = trail.strip_prefix("Denied [")?;
    let close = rest.find(']')?;
    let inside = &rest[..close];
    let (gate, _) = inside.split_once(':')?;
    Some(gate.trim().to_owned())
}

// ---------------------------------------------------------------------------
// cost — token usage by provider/model
// ---------------------------------------------------------------------------

fn cmd_cost(json: bool) {
    let store = match SqlStore::new(&default_data_dir()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return;
        }
    };
    let rows = store.list_telemetry(2_000).unwrap_or_default();
    let mut by_model: BTreeMap<String, (u64, u64)> = BTreeMap::new(); // model -> (input, output)
    for row in &rows {
        if row.event_type != "token_usage" {
            continue;
        }
        let data = row.data.as_deref().unwrap_or("");
        let model = json_str_field(data, "model").unwrap_or_else(|| "unknown".to_owned());
        let input = json_int_field(data, "input_tokens").unwrap_or(0);
        let output = json_int_field(data, "output_tokens").unwrap_or(0);
        let entry = by_model.entry(model).or_insert((0, 0));
        entry.0 += input;
        entry.1 += output;
    }
    let mut entries: Vec<_> = by_model.into_iter().collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.1.0 + e.1.1));

    if json {
        println!("[");
        for (i, (model, (input, output))) in entries.iter().enumerate() {
            print!(
                "  {{\"model\":\"{model}\",\"input_tokens\":{input},\"output_tokens\":{output}}}"
            );
            if i + 1 < entries.len() {
                print!(",");
            }
            println!();
        }
        println!("]");
        return;
    }
    if entries.is_empty() {
        println!("(no token_usage telemetry — provider may not have reported it yet)");
        return;
    }
    println!("{:<32} {:>12} {:>12} {:>14}", "MODEL", "INPUT", "OUTPUT", "TOTAL");
    for (model, (input, output)) in &entries {
        println!(
            "{model:<32} {input:>12} {output:>12} {total:>14}",
            total = input + output
        );
    }
}

// ---------------------------------------------------------------------------
// summary — terse default
// ---------------------------------------------------------------------------

fn cmd_summary(json: bool) {
    let store = match SqlStore::new(&default_data_dir()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return;
        }
    };
    let sessions = store.list_sessions(50).unwrap_or_default();
    let telemetry = store.list_telemetry(2_000).unwrap_or_default();
    let volumes = store.list_volumes().unwrap_or_default();

    let tool_calls = telemetry.iter().filter(|r| r.event_type == "tool_call").count();
    let denies = telemetry
        .iter()
        .filter(|r| r.event_type == "tool_call")
        .filter(|r| {
            json_bool_field(r.data.as_deref().unwrap_or(""), "is_error").unwrap_or(false)
        })
        .count();

    if json {
        println!(
            "{{\"sessions\":{},\"volumes\":{},\"tool_calls\":{tool_calls},\"denies\":{denies}}}",
            sessions.len(),
            volumes.len()
        );
        return;
    }
    println!("nocode insight — last 7 days");
    println!();
    println!("Sessions:  {}", sessions.len());
    println!("Volumes:   {}", volumes.len());
    println!("Tool calls: {tool_calls}");
    println!("Denies:    {denies}");
    println!();
    println!("More: nocode insight where | sessions | tools | gates | cost");
}

// ---------------------------------------------------------------------------
// helpers — minimal JSON probes (no full parser needed)
// ---------------------------------------------------------------------------

fn json_str_field(data: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\":");
    let start = data.find(&needle)? + needle.len();
    let rest = data[start..].trim_start();
    if !rest.starts_with('"') {
        return None;
    }
    let body = &rest[1..];
    let end = body.find('"')?;
    Some(body[..end].to_owned())
}

fn json_bool_field(data: &str, key: &str) -> Option<bool> {
    let needle = format!("\"{key}\":");
    let start = data.find(&needle)? + needle.len();
    let rest = data[start..].trim_start();
    if rest.starts_with("true") {
        Some(true)
    } else if rest.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

fn json_int_field(data: &str, key: &str) -> Option<u64> {
    let needle = format!("\"{key}\":");
    let start = data.find(&needle)? + needle.len();
    let rest = data[start..].trim_start();
    let end = rest.find(|c: char| !c.is_ascii_digit())?;
    rest[..end].parse().ok()
}

fn truncate(s: &str, n: usize) -> &str {
    if s.len() <= n { s } else { &s[..n] }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_gate_extracts_canonical_name() {
        assert_eq!(parse_gate("Denied [permission: x]").as_deref(), Some("permission"));
        assert_eq!(parse_gate("Denied [sandbox: net off]").as_deref(), Some("sandbox"));
    }

    #[test]
    fn parse_gate_returns_none_for_non_trail() {
        assert!(parse_gate("ok").is_none());
        assert!(parse_gate("Permission denied").is_none());
    }

    #[test]
    fn json_str_field_extracts_value() {
        let data = r#"{"tool":"FileRead","is_error":false}"#;
        assert_eq!(json_str_field(data, "tool").as_deref(), Some("FileRead"));
    }

    #[test]
    fn json_bool_field_extracts_value() {
        let data = r#"{"tool":"x","is_error":true}"#;
        assert_eq!(json_bool_field(data, "is_error"), Some(true));
    }

    #[test]
    fn json_int_field_extracts_value() {
        let data = r#"{"input_tokens":1234,"output_tokens":56}"#;
        assert_eq!(json_int_field(data, "input_tokens"), Some(1234));
        assert_eq!(json_int_field(data, "output_tokens"), Some(56));
    }

    #[test]
    fn truncate_short_returns_unchanged() {
        assert_eq!(truncate("hi", 32), "hi");
    }
}
