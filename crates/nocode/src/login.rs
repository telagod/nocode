use crossterm::{
    cursor,
    event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode},
    execute,
    terminal::{self, ClearType},
};
use std::io::{self, Write};

use crate::model_fetch;
use crate::provider_presets::ALL_PRESETS;

// ─────────────────────────────────────────────────────────────────────────────
// ANSI helpers
// ─────────────────────────────────────────────────────────────────────────────

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const ITALIC: &str = "\x1b[3m";
const FG_CYAN: &str = "\x1b[36m";
const FG_GREEN: &str = "\x1b[32m";
const FG_YELLOW: &str = "\x1b[33m";
const FG_RED: &str = "\x1b[31m";
const FG_BLUE: &str = "\x1b[34m";
const FG_DIM: &str = "\x1b[90m";
const FG_WHITE: &str = "\x1b[97m";

// ─────────────────────────────────────────────────────────────────────────────
// Data types
// ─────────────────────────────────────────────────────────────────────────────

struct LoginProvider {
    name: &'static str,
    credential_slot: &'static str,
    env_key_name: &'static str,
    auth_hint: &'static str,
    base_url: &'static str,
    api_format: &'static str,
    default_model: &'static str,
    provider_type: &'static str,
}

struct EndpointOverride {
    base_url: String,
    api_format: String,
}

enum InputEvent {
    Key(crossterm::event::KeyEvent),
    Paste(String),
}

// ─────────────────────────────────────────────────────────────────────────────
// Drawing primitives
// ─────────────────────────────────────────────────────────────────────────────

fn clear(stdout: &mut io::Stdout) {
    let _ = execute!(
        stdout,
        terminal::Clear(ClearType::All),
        cursor::MoveTo(0, 0)
    );
}

fn draw_logo(stdout: &mut io::Stdout) {
    let _ = writeln!(stdout, "\r");
    let _ = writeln!(
        stdout,
        "  {FG_CYAN}{BOLD}\u{256D}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{256E}{RESET}\r"
    );
    let _ = writeln!(
        stdout,
        "  {FG_CYAN}{BOLD}\u{2502}{RESET}   {FG_WHITE}{BOLD}n o c o d e{RESET}   {DIM}setup{RESET}            {FG_CYAN}{BOLD}\u{2502}{RESET}\r"
    );
    let _ = writeln!(
        stdout,
        "  {FG_CYAN}{BOLD}\u{2570}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{256F}{RESET}\r"
    );
    let _ = writeln!(stdout, "\r");
}

fn draw_progress(stdout: &mut io::Stdout, current: usize) {
    let steps = [
        ("\u{2460}", "Provider"),
        ("\u{2461}", "Key"),
        ("\u{2462}", "Endpoint"),
        ("\u{2463}", "Model"),
        ("\u{2464}", "Done"),
    ];
    let _ = write!(stdout, "  ");
    for (i, (num, label)) in steps.iter().enumerate() {
        if i == current {
            let _ = write!(stdout, "{FG_CYAN}{BOLD}{num} {label}{RESET}");
        } else if i < current {
            let _ = write!(stdout, "{FG_GREEN}{num} {label}{RESET}");
        } else {
            let _ = write!(stdout, "{FG_DIM}{num} {label}{RESET}");
        }
        if i + 1 < steps.len() {
            let _ = write!(stdout, " {FG_DIM}\u{2500}{RESET} ");
        }
    }
    let _ = writeln!(stdout, "\r");
    let total = 40usize;
    let filled = current * total / steps.len();
    let empty = total - filled;
    let _ = write!(stdout, "  {FG_CYAN}");
    for _ in 0..filled {
        let _ = write!(stdout, "\u{2501}");
    }
    let _ = write!(stdout, "{FG_DIM}");
    for _ in 0..empty {
        let _ = write!(stdout, "\u{2500}");
    }
    let _ = writeln!(stdout, "{RESET}\r");
    let _ = writeln!(stdout, "\r");
}

fn draw_section_title(stdout: &mut io::Stdout, title: &str) {
    let _ = writeln!(stdout, "  {FG_DIM}\u{2500}\u{2500}\u{2500}{RESET} {BOLD}{title}{RESET} {FG_DIM}\u{2500}\u{2500}\u{2500}{RESET}\r");
    let _ = writeln!(stdout, "\r");
}

fn draw_hint(stdout: &mut io::Stdout, hint: &str) {
    let _ = writeln!(stdout, "\r");
    let _ = writeln!(stdout, "  {FG_DIM}{hint}{RESET}\r");
}

fn draw_field(stdout: &mut io::Stdout, label: &str, value: &str) {
    let _ = writeln!(stdout, "  {FG_DIM}{label}{RESET}  {value}\r");
}

fn mask_key(key: &str) -> String {
    if key.len() <= 8 {
        return "\u{2022}".repeat(key.len());
    }
    let prefix: String = key.chars().take(4).collect();
    let suffix: String = key.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    format!("{prefix}{}\u{2026}{suffix}", "\u{2022}".repeat(4))
}

// ─────────────────────────────────────────────────────────────────────────────
// Input helpers
// ─────────────────────────────────────────────────────────────────────────────

fn read_event() -> Option<InputEvent> {
    loop {
        match event::read() {
            Ok(Event::Key(key)) => return Some(InputEvent::Key(key)),
            Ok(Event::Paste(text)) => return Some(InputEvent::Paste(text)),
            Ok(_) => continue,
            Err(_) => return None,
        }
    }
}

fn read_key() -> Option<crossterm::event::KeyEvent> {
    loop {
        match event::read() {
            Ok(Event::Key(key)) => return Some(key),
            Ok(_) => continue,
            Err(_) => return None,
        }
    }
}

fn build_provider_list() -> Vec<LoginProvider> {
    ALL_PRESETS
        .iter()
        .map(|p| LoginProvider {
            name: p.name,
            credential_slot: p.credential_slot,
            env_key_name: p.env_key_name,
            auth_hint: p.auth_hint,
            base_url: p.base_url,
            api_format: p.api_format,
            default_model: p.default_model,
            provider_type: p.provider_type,
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Step 1: Provider selection
// ─────────────────────────────────────────────────────────────────────────────

fn step_provider(stdout: &mut io::Stdout, providers: &[LoginProvider]) -> Option<usize> {
    let mut selected = 0usize;
    let mut filter = String::new();

    // Group indices: native (anthropic/openai/gemini), cloud proxies, local
    let native: Vec<usize> = providers
        .iter()
        .enumerate()
        .filter(|(_, p)| matches!(p.provider_type, "anthropic" | "openai" | "gemini"))
        .map(|(i, _)| i)
        .collect();
    let cloud: Vec<usize> = providers
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            p.provider_type == "custom"
                && !p.base_url.contains("localhost")
                && !p.base_url.contains("127.0.0.1")
                && p.name != "Ollama"
                && p.name != "vLLM"
                && p.name != "LiteLLM"
                && p.name != "LocalAI"
                && p.name != "LM Studio"
        })
        .map(|(i, _)| i)
        .collect();
    let local: Vec<usize> = providers
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            p.provider_type == "custom"
                && (p.base_url.contains("localhost")
                    || p.base_url.contains("127.0.0.1")
                    || matches!(p.name, "Ollama" | "vLLM" | "LiteLLM" | "LocalAI" | "LM Studio"))
        })
        .map(|(i, _)| i)
        .collect();

    // Flat ordered list for navigation
    fn build_flat(
        native: &[usize],
        cloud: &[usize],
        local: &[usize],
        providers: &[LoginProvider],
        filter: &str,
    ) -> Vec<usize> {
        let filter_lower = filter.to_lowercase();
        let matches = |i: &usize| -> bool {
            filter.is_empty() || providers[*i].name.to_lowercase().contains(&filter_lower)
        };
        let mut flat = Vec::new();
        flat.extend(native.iter().copied().filter(|i| matches(i)));
        flat.extend(cloud.iter().copied().filter(|i| matches(i)));
        flat.extend(local.iter().copied().filter(|i| matches(i)));
        flat
    }

    let mut flat = build_flat(&native, &cloud, &local, providers, &filter);

    loop {
        clear(stdout);
        draw_logo(stdout);
        draw_progress(stdout, 0);
        draw_section_title(stdout, "Select Provider");

        if !filter.is_empty() {
            let _ = writeln!(
                stdout,
                "  {FG_YELLOW}filter:{RESET} {filter}  {FG_DIM}(Esc clear){RESET}\r"
            );
            let _ = writeln!(stdout, "\r");
        }

        // Render grouped list
        let mut flat_pos = 0usize;
        let groups: [(&str, &[usize]); 3] = [
            ("Direct API", &native),
            ("Cloud Proxy", &cloud),
            ("Local / Self-hosted", &local),
        ];
        let filter_lower = filter.to_lowercase();

        for (group_name, indices) in &groups {
            let visible: Vec<usize> = indices
                .iter()
                .copied()
                .filter(|i| filter.is_empty() || providers[*i].name.to_lowercase().contains(&filter_lower))
                .collect();
            if visible.is_empty() {
                continue;
            }
            let _ = writeln!(stdout, "  {FG_DIM}{group_name}{RESET}\r");
            for &idx in &visible {
                let is_sel = flat_pos == selected;
                let p = &providers[idx];
                let marker = if is_sel {
                    format!("{FG_CYAN}{BOLD} \u{25B8} {RESET}")
                } else {
                    "   ".to_string()
                };
                let model_hint = format!("{FG_DIM}{}{RESET}", p.default_model);
                let padded_name = format!("{:<18}", p.name);
                let name_display = if is_sel {
                    format!("{FG_WHITE}{BOLD}{padded_name}{RESET}")
                } else {
                    padded_name
                };
                let _ = writeln!(stdout, "{marker}{name_display} {model_hint}\r");
                flat_pos += 1;
            }
            let _ = writeln!(stdout, "\r");
        }

        draw_hint(
            stdout,
            "\u{2191}\u{2193} navigate  Enter select  / filter  Esc quit",
        );
        let _ = stdout.flush();

        let key = read_key()?;
        match key.code {
            KeyCode::Up if selected > 0 => selected -= 1,
            KeyCode::Down if selected + 1 < flat.len() => selected += 1,
            KeyCode::Enter if !flat.is_empty() => return Some(flat[selected]),
            KeyCode::Char('/') => {
                filter.clear();
                // Enter filter mode inline
                loop {
                    flat = build_flat(&native, &cloud, &local, providers, &filter);
                    selected = selected.min(flat.len().saturating_sub(1));

                    clear(stdout);
                    draw_logo(stdout);
                    draw_progress(stdout, 0);
                    draw_section_title(stdout, "Select Provider");
                    let _ = writeln!(
                        stdout,
                        "  {FG_YELLOW}\u{1F50D} {filter}\u{2588}{RESET}\r"
                    );
                    let _ = writeln!(stdout, "\r");
                    for (i, &idx) in flat.iter().enumerate() {
                        let is_sel = i == selected;
                        let p = &providers[idx];
                        let marker = if is_sel {
                            format!("{FG_CYAN}{BOLD} \u{25B8} {RESET}")
                        } else {
                            "   ".to_string()
                        };
                        let padded = format!("{:<18}", p.name);
                        let name_d = if is_sel {
                            format!("{FG_WHITE}{BOLD}{padded}{RESET}")
                        } else {
                            padded
                        };
                        let _ = writeln!(
                            stdout,
                            "{marker}{name_d} {FG_DIM}{}{RESET}\r",
                            p.default_model
                        );
                    }
                    draw_hint(stdout, "type to filter  Enter select  Esc clear");
                    let _ = stdout.flush();

                    let key = read_key()?;
                    match key.code {
                        KeyCode::Char(c) => filter.push(c),
                        KeyCode::Backspace => {
                            filter.pop();
                        }
                        KeyCode::Enter if !flat.is_empty() => return Some(flat[selected]),
                        KeyCode::Up if selected > 0 => selected -= 1,
                        KeyCode::Down if selected + 1 < flat.len() => selected += 1,
                        KeyCode::Esc => {
                            filter.clear();
                            flat = build_flat(&native, &cloud, &local, providers, &filter);
                            selected = selected.min(flat.len().saturating_sub(1));
                            break;
                        }
                        _ => {}
                    }
                }
            }
            KeyCode::Esc | KeyCode::Char('q') => return None,
            _ => {}
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Step 2: API Key
// ─────────────────────────────────────────────────────────────────────────────

fn step_api_key(stdout: &mut io::Stdout, provider: &LoginProvider) -> Option<String> {
    if provider.env_key_name.is_empty() {
        return Some(String::new());
    }

    // Check if key already exists in env
    let existing = std::env::var(provider.env_key_name).ok();
    if let Some(ref key) = existing
        && !key.is_empty()
    {
        clear(stdout);
        draw_logo(stdout);
        draw_progress(stdout, 1);
        draw_section_title(stdout, "API Key");
        let _ = writeln!(
            stdout,
            "  {FG_GREEN}\u{2713}{RESET} Found {BOLD}{}{RESET} in environment\r",
            provider.env_key_name
        );
        let _ = writeln!(stdout, "    {FG_DIM}{}{RESET}\r", mask_key(key));
        let _ = writeln!(stdout, "\r");
        let _ = writeln!(
            stdout,
            "  {FG_DIM}Enter{RESET} use this key  {FG_DIM}n{RESET} enter new key  {FG_DIM}Esc{RESET} back\r"
        );
        let _ = stdout.flush();

        loop {
            let k = read_key()?;
            match k.code {
                KeyCode::Enter => return Some(key.clone()),
                KeyCode::Char('n') | KeyCode::Char('N') => break, // fall through to input
                KeyCode::Esc => return None,
                _ => {}
            }
        }
    }

    let mut key_buf = String::new();
    loop {
        clear(stdout);
        draw_logo(stdout);
        draw_progress(stdout, 1);
        draw_section_title(stdout, "API Key");

        draw_field(stdout, "Provider", &format!("{BOLD}{}{RESET}", provider.name));
        let _ = writeln!(stdout, "\r");
        let _ = writeln!(
            stdout,
            "  {FG_BLUE}{ITALIC}{}{RESET}\r",
            provider.auth_hint
        );
        let _ = writeln!(stdout, "\r");

        let display = if key_buf.is_empty() {
            format!("{FG_DIM}\u{2588}{RESET}")
        } else {
            format!("{}{FG_DIM}\u{2588}{RESET}", mask_key(&key_buf))
        };
        let _ = writeln!(stdout, "  {FG_DIM}\u{1F511}{RESET} {display}\r");

        draw_hint(stdout, "paste or type key  Enter confirm  Esc back");
        let _ = stdout.flush();

        match read_event()? {
            InputEvent::Paste(text) => {
                key_buf.push_str(text.trim());
            }
            InputEvent::Key(key) => match key.code {
                KeyCode::Enter if !key_buf.trim().is_empty() => {
                    return Some(key_buf.trim().to_string());
                }
                KeyCode::Esc => return None,
                KeyCode::Backspace => {
                    key_buf.pop();
                }
                KeyCode::Char(c) => key_buf.push(c),
                _ => {}
            },
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Step 3: Endpoint (Base URL + API Format)
// ─────────────────────────────────────────────────────────────────────────────

fn step_endpoint(stdout: &mut io::Stdout, provider: &LoginProvider) -> Option<EndpointOverride> {
    let formats = ["openai-responses", "openai-chat", "anthropic", "google"];
    let mut base_url = provider.base_url.to_string();
    let mut api_format = provider.api_format.to_string();
    let mut fmt_idx = formats.iter().position(|&f| f == api_format).unwrap_or(0);
    let mut selected: usize = 0; // 0 = Base URL, 1 = Format
    let mut editing = false;
    let mut edit_buf = String::new();

    loop {
        clear(stdout);
        draw_logo(stdout);
        draw_progress(stdout, 2);
        draw_section_title(stdout, "Endpoint");

        draw_field(stdout, "Provider", &format!("{BOLD}{}{RESET}", provider.name));
        let _ = writeln!(stdout, "\r");

        // Row 0: Base URL
        if editing && selected == 0 {
            let display = if edit_buf.is_empty() {
                format!("{FG_DIM}{base_url}{RESET}")
            } else {
                format!("{FG_WHITE}{BOLD}{edit_buf}{RESET}")
            };
            let _ = writeln!(
                stdout,
                "  {FG_CYAN}{BOLD}\u{25B8} Base URL{RESET}  {display}{FG_CYAN}\u{2588}{RESET}\r"
            );
        } else if selected == 0 {
            let _ = writeln!(
                stdout,
                "  {FG_CYAN}\u{25B8}{RESET} {FG_WHITE}{BOLD}Base URL{RESET}  {FG_CYAN}{base_url}{RESET}\r"
            );
        } else {
            let _ = writeln!(
                stdout,
                "  {FG_DIM}  Base URL  {base_url}{RESET}\r"
            );
        }

        // Row 1: Format
        if editing && selected == 1 {
            let _ = writeln!(
                stdout,
                "  {FG_CYAN}{BOLD}\u{25B8} Format{RESET}    {FG_DIM}\u{25C2}{RESET} {FG_WHITE}{BOLD}{}{RESET} {FG_DIM}\u{25B8}{RESET}\r",
                formats[fmt_idx]
            );
        } else if selected == 1 {
            let _ = writeln!(
                stdout,
                "  {FG_CYAN}\u{25B8}{RESET} {FG_WHITE}{BOLD}Format{RESET}    {FG_CYAN}{api_format}{RESET}\r"
            );
        } else {
            let _ = writeln!(
                stdout,
                "  {FG_DIM}  Format    {api_format}{RESET}\r"
            );
        }

        let _ = writeln!(stdout, "\r");

        // Hint line
        if editing && selected == 0 {
            let _ = writeln!(
                stdout,
                "  {FG_DIM}type/paste URL  Enter confirm  Esc cancel{RESET}\r"
            );
        } else if editing && selected == 1 {
            let _ = writeln!(
                stdout,
                "  {FG_DIM}\u{2190}/\u{2192} cycle  Enter confirm  Esc cancel{RESET}\r"
            );
        } else {
            let _ = writeln!(
                stdout,
                "  {FG_DIM}\u{2191}\u{2193} select  Enter edit  Tab next step  Esc back{RESET}\r"
            );
        }
        let _ = stdout.flush();

        match read_event()? {
            InputEvent::Paste(text) => {
                if editing && selected == 0 {
                    edit_buf.push_str(text.trim());
                }
            }
            InputEvent::Key(key) => {
                if editing {
                    match selected {
                        0 => match key.code {
                            KeyCode::Enter => {
                                let trimmed = edit_buf.trim().trim_end_matches('/').to_string();
                                if !trimmed.is_empty() {
                                    base_url = trimmed;
                                }
                                edit_buf.clear();
                                editing = false;
                            }
                            KeyCode::Esc => {
                                edit_buf.clear();
                                editing = false;
                            }
                            KeyCode::Backspace => { edit_buf.pop(); }
                            KeyCode::Char(c) => edit_buf.push(c),
                            _ => {}
                        },
                        1 => match key.code {
                            KeyCode::Left => {
                                fmt_idx = if fmt_idx == 0 { formats.len() - 1 } else { fmt_idx - 1 };
                            }
                            KeyCode::Right => {
                                fmt_idx = (fmt_idx + 1) % formats.len();
                            }
                            KeyCode::Enter => {
                                api_format = formats[fmt_idx].to_string();
                                editing = false;
                            }
                            KeyCode::Esc => {
                                fmt_idx = formats.iter().position(|&f| f == api_format).unwrap_or(0);
                                editing = false;
                            }
                            _ => {}
                        },
                        _ => {}
                    }
                } else {
                    match key.code {
                        KeyCode::Up if selected > 0 => selected -= 1,
                        KeyCode::Down if selected < 1 => selected += 1,
                        KeyCode::Enter => {
                            editing = true;
                            if selected == 0 {
                                edit_buf.clear();
                            } else {
                                fmt_idx = formats.iter().position(|&f| f == api_format).unwrap_or(0);
                            }
                        }
                        KeyCode::Tab | KeyCode::Char('\t') => {
                            return Some(EndpointOverride { base_url, api_format });
                        }
                        KeyCode::Esc => return None,
                        _ => {}
                    }
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Step 4: Model selection
// ─────────────────────────────────────────────────────────────────────────────

fn step_model(
    stdout: &mut io::Stdout,
    provider: &LoginProvider,
    api_key: &str,
    endpoint: &EndpointOverride,
) -> Option<String> {
    // Show loading state
    clear(stdout);
    draw_logo(stdout);
    draw_progress(stdout, 3);
    draw_section_title(stdout, "Select Model");
    let _ = writeln!(
        stdout,
        "  {FG_DIM}\u{25CC} Fetching models from {}...{RESET}\r",
        provider.name
    );
    let _ = stdout.flush();

    let is_native = matches!(provider.provider_type, "anthropic" | "openai" | "gemini")
        && endpoint.base_url == provider.base_url;
    let (prov_str, base, fmt) = if is_native {
        (provider.provider_type, "", "")
    } else {
        (
            "custom",
            endpoint.base_url.as_str(),
            endpoint.api_format.as_str(),
        )
    };

    if !api_key.is_empty() && !provider.env_key_name.is_empty() {
        unsafe { std::env::set_var(provider.env_key_name, api_key) };
    }

    let models = if !is_native && !api_key.is_empty() {
        model_fetch::fetch_model_suggestions_with_key(base, fmt, api_key)
    } else {
        model_fetch::fetch_model_suggestions(prov_str, base, fmt)
    };

    let models = match models {
        Ok(m) if !m.is_empty() => m,
        Ok(_) => {
            clear(stdout);
            draw_logo(stdout);
            draw_progress(stdout, 3);
            draw_section_title(stdout, "Select Model");
            let _ = writeln!(stdout, "  {FG_YELLOW}\u{26A0} No models returned by API{RESET}\r");
            let _ = writeln!(stdout, "\r");
            let _ = writeln!(
                stdout,
                "  {FG_DIM}t{RESET} type model name  {FG_DIM}r{RESET} retry  {FG_DIM}Esc{RESET} back\r"
            );
            let _ = stdout.flush();
            return handle_model_fetch_failure(stdout, provider, api_key, endpoint);
        }
        Err(e) => {
            clear(stdout);
            draw_logo(stdout);
            draw_progress(stdout, 3);
            draw_section_title(stdout, "Select Model");
            let _ = writeln!(stdout, "  {FG_RED}\u{2716} Fetch failed:{RESET} {e}\r");
            let _ = writeln!(stdout, "\r");
            let _ = writeln!(
                stdout,
                "  {FG_DIM}t{RESET} type model name  {FG_DIM}r{RESET} retry  {FG_DIM}Esc{RESET} back\r"
            );
            let _ = stdout.flush();
            return handle_model_fetch_failure(stdout, provider, api_key, endpoint);
        }
    };

    let mut selected = 0usize;
    let mut scroll = 0usize;
    let mut filter = String::new();
    let mut filtered = models.clone();
    let max_visible = 14;

    // Pre-select the default model if it exists in the list
    if let Some(pos) = filtered
        .iter()
        .position(|m| m == provider.default_model)
    {
        selected = pos;
        if selected >= max_visible {
            scroll = selected.saturating_sub(max_visible / 2);
        }
    }

    loop {
        clear(stdout);
        draw_logo(stdout);
        draw_progress(stdout, 3);
        draw_section_title(stdout, "Select Model");

        if !filter.is_empty() {
            let _ = writeln!(
                stdout,
                "  {FG_YELLOW}\u{1F50D} {filter}{RESET}  {FG_DIM}({} matches){RESET}\r",
                filtered.len()
            );
            let _ = writeln!(stdout, "\r");
        } else {
            let _ = writeln!(
                stdout,
                "  {FG_DIM}{} models available{RESET}\r",
                filtered.len()
            );
            let _ = writeln!(stdout, "\r");
        }

        let end = (scroll + max_visible).min(filtered.len());
        if scroll > 0 {
            let _ = writeln!(stdout, "  {FG_DIM}\u{25B4} more above{RESET}\r");
        }
        for (i, model_name) in filtered.iter().enumerate().skip(scroll).take(end - scroll) {
            let is_sel = i == selected;
            let is_default = model_name == provider.default_model;
            let marker = if is_sel {
                format!("{FG_CYAN} \u{25B8} {RESET}")
            } else {
                "   ".to_string()
            };
            let name_style = if is_sel {
                format!("{FG_WHITE}{BOLD}{model_name}{RESET}")
            } else {
                model_name.to_string()
            };
            let badge = if is_default {
                format!(" {FG_GREEN}{DIM}(default){RESET}")
            } else {
                String::new()
            };
            let _ = writeln!(stdout, "{marker}{name_style}{badge}\r");
        }
        if end < filtered.len() {
            let _ = writeln!(stdout, "  {FG_DIM}\u{25BE} more below{RESET}\r");
        }

        draw_hint(
            stdout,
            "\u{2191}\u{2193} navigate  Enter select  / filter  t type manually  Esc back",
        );
        let _ = stdout.flush();

        let key = read_key()?;
        match key.code {
            KeyCode::Up if selected > 0 => {
                selected -= 1;
                if selected < scroll {
                    scroll = selected;
                }
            }
            KeyCode::Down if selected + 1 < filtered.len() => {
                selected += 1;
                if selected >= scroll + max_visible {
                    scroll = selected - max_visible + 1;
                }
            }
            KeyCode::Enter if !filtered.is_empty() => return Some(filtered[selected].clone()),
            KeyCode::Char('/') => {
                filter.clear();
                loop {
                    clear(stdout);
                    draw_logo(stdout);
                    draw_progress(stdout, 3);
                    draw_section_title(stdout, "Select Model");
                    let _ = writeln!(
                        stdout,
                        "  {FG_YELLOW}\u{1F50D} {filter}\u{2588}{RESET}\r"
                    );
                    let _ = writeln!(stdout, "\r");
                    let shown: Vec<_> = filtered.iter().take(max_visible).collect();
                    for (i, m) in shown.iter().enumerate() {
                        let is_sel = i == selected;
                        let marker = if is_sel { format!("{FG_CYAN} \u{25B8} {RESET}") } else { "   ".to_string() };
                        let s = if is_sel { format!("{FG_WHITE}{BOLD}{m}{RESET}") } else { m.to_string() };
                        let _ = writeln!(stdout, "{marker}{s}\r");
                    }
                    draw_hint(stdout, "type to filter  Enter select  Esc clear");
                    let _ = stdout.flush();

                    let key = read_key()?;
                    match key.code {
                        KeyCode::Char(c) => {
                            filter.push(c);
                            filtered = model_fetch::apply_model_filter(&models, &filter);
                            selected = 0;
                            scroll = 0;
                        }
                        KeyCode::Backspace => {
                            filter.pop();
                            filtered = if filter.is_empty() {
                                models.clone()
                            } else {
                                model_fetch::apply_model_filter(&models, &filter)
                            };
                            selected = selected.min(filtered.len().saturating_sub(1));
                        }
                        KeyCode::Up if selected > 0 => selected -= 1,
                        KeyCode::Down if selected + 1 < filtered.len() => selected += 1,
                        KeyCode::Enter if !filtered.is_empty() => {
                            return Some(filtered[selected].clone());
                        }
                        KeyCode::Esc => {
                            if filter.is_empty() {
                                filtered = models.clone();
                            }
                            break;
                        }
                        _ => {}
                    }
                }
            }
            KeyCode::Char('t') | KeyCode::Char('T') => {
                return prompt_model_input(stdout);
            }
            KeyCode::Esc => return None,
            _ => {}
        }
    }
}

fn handle_model_fetch_failure(
    stdout: &mut io::Stdout,
    provider: &LoginProvider,
    api_key: &str,
    endpoint: &EndpointOverride,
) -> Option<String> {
    loop {
        let key = read_key()?;
        match key.code {
            KeyCode::Char('t') | KeyCode::Char('T') => {
                return prompt_model_input(stdout);
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                return step_model(stdout, provider, api_key, endpoint);
            }
            KeyCode::Esc => return None,
            _ => {}
        }
    }
}

fn prompt_model_input(stdout: &mut io::Stdout) -> Option<String> {
    let mut buf = String::new();
    loop {
        clear(stdout);
        draw_logo(stdout);
        draw_progress(stdout, 3);
        draw_section_title(stdout, "Enter Model Name");

        let _ = writeln!(stdout, "  > {buf}{FG_DIM}\u{2588}{RESET}\r");
        draw_hint(stdout, "Enter confirm  Esc back");
        let _ = stdout.flush();

        match read_event()? {
            InputEvent::Paste(text) => buf.push_str(text.trim()),
            InputEvent::Key(key) => match key.code {
                KeyCode::Enter if !buf.trim().is_empty() => return Some(buf.trim().to_string()),
                KeyCode::Esc => return None,
                KeyCode::Backspace => {
                    buf.pop();
                }
                KeyCode::Char(c) => buf.push(c),
                _ => {}
            },
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Step 4: Confirm & save
// ─────────────────────────────────────────────────────────────────────────────

fn step_confirm(
    stdout: &mut io::Stdout,
    provider: &LoginProvider,
    api_key: &str,
    model: &str,
    endpoint: &EndpointOverride,
    cwd: &str,
) -> bool {
    let url_changed = endpoint.base_url != provider.base_url;
    let fmt_changed = endpoint.api_format != provider.api_format;

    clear(stdout);
    draw_logo(stdout);
    draw_progress(stdout, 4);
    draw_section_title(stdout, "Confirm Setup");

    let _ = writeln!(stdout, "\r");
    draw_field(stdout, "Provider", &format!("{BOLD}{}{RESET}", provider.name));
    draw_field(stdout, "Model   ", &format!("{FG_CYAN}{model}{RESET}"));
    if !api_key.is_empty() {
        draw_field(stdout, "Key     ", &mask_key(api_key));
    }
    if url_changed {
        draw_field(stdout, "Base URL", &endpoint.base_url);
    }
    if fmt_changed {
        draw_field(stdout, "Format  ", &endpoint.api_format);
    }
    let _ = writeln!(stdout, "\r");
    let _ = writeln!(
        stdout,
        "  {FG_DIM}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}{RESET}\r"
    );
    let _ = writeln!(stdout, "\r");
    let _ = writeln!(
        stdout,
        "  {FG_GREEN}{BOLD}Enter{RESET} save & start  {FG_DIM}Esc{RESET} go back\r"
    );
    let _ = stdout.flush();

    loop {
        let Some(key) = read_key() else {
            return false;
        };
        match key.code {
            KeyCode::Enter => {
                // Save API key
                if !api_key.is_empty() {
                    let cred_path =
                        nocode_core::storage::credentials::CredentialStore::default_path();
                    let mut store =
                        nocode_core::storage::credentials::CredentialStore::load(&cred_path)
                            .unwrap_or_default();
                    store.set_key(provider.credential_slot, api_key);
                    let _ = store.save(&cred_path);
                    if !provider.env_key_name.is_empty() {
                        unsafe { std::env::set_var(provider.env_key_name, api_key) };
                    }
                }

                // Save settings
                use nocode_core::config::settings::{Settings, SettingsTier};
                let _ = Settings::persist_key_value("model", Some(model), SettingsTier::User, cwd);

                let uses_custom = provider.provider_type == "custom" || url_changed || fmt_changed;
                if uses_custom {
                    let _ = Settings::persist_key_value(
                        "model_provider",
                        Some("custom"),
                        SettingsTier::User,
                        cwd,
                    );
                    let _ = Settings::persist_key_value(
                        "custom_base_url",
                        Some(&endpoint.base_url),
                        SettingsTier::User,
                        cwd,
                    );
                    let _ = Settings::persist_key_value(
                        "custom_api_format",
                        Some(&endpoint.api_format),
                        SettingsTier::User,
                        cwd,
                    );
                    let _ = Settings::persist_key_value(
                        "custom_preset",
                        Some(provider.name),
                        SettingsTier::User,
                        cwd,
                    );
                } else {
                    let _ = Settings::persist_key_value(
                        "model_provider",
                        Some(provider.provider_type),
                        SettingsTier::User,
                        cwd,
                    );
                    let _ = Settings::persist_key_value(
                        "custom_base_url",
                        None,
                        SettingsTier::User,
                        cwd,
                    );
                    let _ = Settings::persist_key_value(
                        "custom_preset",
                        None,
                        SettingsTier::User,
                        cwd,
                    );
                }

                // Success screen
                clear(stdout);
                draw_logo(stdout);
                draw_progress(stdout, 4);
                let _ = writeln!(stdout, "\r");
                let _ = writeln!(
                    stdout,
                    "  {FG_GREEN}{BOLD}\u{2713} Setup complete!{RESET}\r"
                );
                let _ = writeln!(stdout, "\r");
                draw_field(stdout, "Provider", provider.name);
                draw_field(stdout, "Model   ", model);
                let _ = writeln!(stdout, "\r");
                let _ = writeln!(
                    stdout,
                    "  {FG_DIM}Saved to ~/.nocode/settings.json{RESET}\r"
                );
                let _ = writeln!(
                    stdout,
                    "  {FG_DIM}Run {RESET}{BOLD}nocode{RESET}{FG_DIM} to start.{RESET}\r"
                );
                let _ = writeln!(stdout, "\r");
                let _ = stdout.flush();
                return true;
            }
            KeyCode::Esc => return false,
            _ => {}
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Public entry point
// ─────────────────────────────────────────────────────────────────────────────

pub fn run_login(cwd: &str) {
    let providers = build_provider_list();
    let mut stdout = io::stdout();

    terminal::enable_raw_mode().expect("Failed to enable raw mode");
    let _ = execute!(stdout, EnableBracketedPaste);
    let result = run_login_flow(&mut stdout, &providers, cwd);
    let _ = execute!(stdout, DisableBracketedPaste);
    terminal::disable_raw_mode().expect("Failed to disable raw mode");
    let _ = execute!(stdout, cursor::Show);

    if !result {
        println!("\r\n  Login cancelled.\r");
    }
}

fn run_login_flow(stdout: &mut io::Stdout, providers: &[LoginProvider], cwd: &str) -> bool {
    // Step 1: Select provider
    let Some(idx) = step_provider(stdout, providers) else {
        return false;
    };
    let provider = &providers[idx];

    // Step 2: API key
    let Some(api_key) = step_api_key(stdout, provider) else {
        return false;
    };

    // Step 3: Endpoint (base URL + format)
    let Some(endpoint) = step_endpoint(stdout, provider) else {
        return false;
    };

    // Step 4: Select model (fetched live, no fallback)
    let Some(model) = step_model(stdout, provider, &api_key, &endpoint) else {
        return false;
    };

    // Step 5: Confirm & save
    step_confirm(stdout, provider, &api_key, &model, &endpoint, cwd)
}
