//! `nocode init` — scaffold `~/.nocode/config.toml` with a commented template.
//!
//! Replaces the deleted interactive `--login` wizard. The philosophy is
//! codex-style: edit a TOML file by hand, the tool does not interrogate the
//! user. `init` is a one-shot scaffold + open hint, never overwriting an
//! existing file.

use std::fs;
use std::path::PathBuf;

const TEMPLATE: &str = r#"# nocode configuration. Edit this file directly. See docs/10_provider_config.md.

# Default provider name — must match a [providers.<name>] table below
# (or one of the builtin aliases: claude / openai / gemini).
default_provider = "openai"

# Default model when no per-provider or per-call override is set.
# model = "gpt-5.5"

# Permission mode: auto | ask | deny | read-only
# permission_mode = "ask"

# ----- Providers -----
# Define one [providers.<name>] table per endpoint. Switch at runtime with
# `nocode --provider <name>` or `nocode --profile <name>` or NOCODE_PROVIDER.

# [providers.subfox]
# base_url     = "https://sub.foxnio.com/v1"
# wire_api     = "openai-responses"      # anthropic | openai-responses | openai-chat | google
# api_key_env  = "OPENAI_API_KEY"        # name of the env var holding the key
# default_model = "gpt-5.5"

# [providers.local-vllm]
# base_url     = "http://localhost:8000/v1"
# wire_api     = "openai-chat"
# api_key_env  = "VLLM_API_KEY"          # leave unset for keyless local servers
# default_model = "Qwen2.5-Coder-32B-Instruct"

# [providers.together]
# base_url     = "https://api.together.xyz/v1"
# wire_api     = "openai-chat"
# api_key_env  = "TOGETHER_API_KEY"

# ----- Profiles -----
# Group (provider, model, mode, reasoning_effort) under one name.
# Switch with `nocode --profile <name>`.

# [profiles.work]
# provider = "subfox"
# model    = "gpt-5.5"
# permission_mode = "ask"

# [profiles.home]
# provider = "local-vllm"
# permission_mode = "auto"
"#;

/// Entry point — invoked from `main.rs` when argv has `init`.
pub fn run(args: &[String]) {
    let force = args.iter().any(|a| a == "--force");
    let path = config_path();

    if path.exists() && !force {
        eprintln!("Config already exists: {}", path.display());
        eprintln!("Edit it directly, or pass --force to overwrite.");
        std::process::exit(1);
    }

    if let Some(parent) = path.parent()
        && let Err(e) = fs::create_dir_all(parent)
    {
        eprintln!("Failed to create config dir: {e}");
        std::process::exit(1);
    }
    if let Err(e) = fs::write(&path, TEMPLATE) {
        eprintln!("Failed to write {}: {e}", path.display());
        std::process::exit(1);
    }
    println!("Wrote {}", path.display());
    println!();
    println!("Edit it to point at your provider, then run `nocode`.");
    println!("Set the API key in your shell:");
    println!("  export OPENAI_API_KEY=sk-...   # or whichever api_key_env you chose");
}

fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_owned());
    PathBuf::from(home).join(".nocode/config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_contains_known_keys() {
        // Cheap sanity check — the keys we document publicly must stay in the
        // scaffold so users can copy-paste from the template.
        assert!(TEMPLATE.contains("default_provider"));
        assert!(TEMPLATE.contains("[providers."));
        assert!(TEMPLATE.contains("wire_api"));
        assert!(TEMPLATE.contains("api_key_env"));
        assert!(TEMPLATE.contains("[profiles."));
    }

    #[test]
    fn template_documents_all_wire_apis() {
        // Comment line must list the four valid wire_api values.
        assert!(TEMPLATE.contains("anthropic"));
        assert!(TEMPLATE.contains("openai-responses"));
        assert!(TEMPLATE.contains("openai-chat"));
        assert!(TEMPLATE.contains("google"));
    }
}
