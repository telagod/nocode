use crossterm::{
    cursor,
    event::{self, Event, KeyCode},
    execute,
    terminal::{self, ClearType},
};
use std::io::{self, Write};

use crate::model_fetch;
use crate::provider_presets::ALL_PRESETS;

struct LoginProvider {
    name: &'static str,
    credential_slot: &'static str,
    env_key_name: &'static str,
    auth_hint: &'static str,
    base_url: &'static str,
    api_format: &'static str,
    _default_model: &'static str,
    provider_type: &'static str,
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
            _default_model: p.default_model,
            provider_type: p.provider_type,
        })
        .collect()
}

fn mask_key(key: &str) -> String {
    if key.len() <= 8 {
        return "*".repeat(key.len());
    }
    format!("{}...{}", &key[..4], &key[key.len() - 4..])
}

fn clear_screen(stdout: &mut io::Stdout) {
    let _ = execute!(
        stdout,
        terminal::Clear(ClearType::All),
        cursor::MoveTo(0, 0)
    );
}

fn draw_header(stdout: &mut io::Stdout, step: &str) {
    let _ = writeln!(stdout, "  nocode login\r");
    let _ = writeln!(stdout, "  {step}\r");
    let _ = writeln!(stdout, "\r");
}

fn read_key() -> Option<crossterm::event::KeyEvent> {
    loop {
        if let Ok(Event::Key(key)) = event::read() {
            return Some(key);
        }
    }
}

fn step_select_provider(stdout: &mut io::Stdout, providers: &[LoginProvider]) -> Option<usize> {
    let mut selected = 0usize;
    let max_visible = 14;
    let mut scroll = 0usize;
    loop {
        clear_screen(stdout);
        draw_header(stdout, "Step 1: Select Provider");
        let visible_end = (scroll + max_visible).min(providers.len());
        for (i, prov) in providers
            .iter()
            .enumerate()
            .skip(scroll)
            .take(visible_end - scroll)
        {
            let marker = if i == selected { " ▸ " } else { "   " };
            let _ = writeln!(stdout, "{marker}{}\r", prov.name);
        }
        if providers.len() > max_visible {
            let _ = writeln!(stdout, "\r\n  ({}/{})\r", selected + 1, providers.len());
        }
        let _ = writeln!(stdout, "\r\n  ↑/↓ navigate  Enter select  Esc quit\r");
        let _ = stdout.flush();

        let key = read_key()?;
        match key.code {
            KeyCode::Up if selected > 0 => {
                selected -= 1;
                if selected < scroll {
                    scroll = selected;
                }
            }
            KeyCode::Down if selected + 1 < providers.len() => {
                selected += 1;
                if selected >= scroll + max_visible {
                    scroll = selected - max_visible + 1;
                }
            }
            KeyCode::Enter => return Some(selected),
            KeyCode::Esc | KeyCode::Char('q') => return None,
            _ => {}
        }
    }
}

fn step_api_key(stdout: &mut io::Stdout, provider: &LoginProvider) -> Option<String> {
    if provider.env_key_name.is_empty() {
        return Some(String::new());
    }
    let mut key_buf = String::new();
    loop {
        clear_screen(stdout);
        draw_header(stdout, "Step 2: API Key");
        let _ = writeln!(stdout, "  Provider: {}\r", provider.name);
        let _ = writeln!(stdout, "  Hint: {}\r", provider.auth_hint);
        if !provider.env_key_name.is_empty() {
            let _ = writeln!(stdout, "  Env var: {}\r", provider.env_key_name);
        }
        let _ = writeln!(stdout, "\r");
        let display = if key_buf.is_empty() {
            String::new()
        } else {
            mask_key(&key_buf)
        };
        let _ = writeln!(stdout, "  Key: {display}\r");
        let _ = writeln!(stdout, "\r\n  Type/paste key, Enter confirm, Esc back\r");
        let _ = stdout.flush();

        let key = read_key()?;
        match key.code {
            KeyCode::Enter => {
                if key_buf.trim().is_empty() {
                    continue;
                }
                return Some(key_buf.trim().to_string());
            }
            KeyCode::Esc => return None,
            KeyCode::Backspace => {
                key_buf.pop();
            }
            KeyCode::Char(c) => key_buf.push(c),
            _ => {}
        }
    }
}

fn step_select_model(
    stdout: &mut io::Stdout,
    provider: &LoginProvider,
    api_key: &str,
) -> Option<String> {
    clear_screen(stdout);
    draw_header(stdout, "Step 3: Select Model");
    let _ = writeln!(stdout, "  Fetching models from {}...\r", provider.name);
    let _ = stdout.flush();

    let is_native = matches!(provider.provider_type, "anthropic" | "openai" | "gemini");
    let (prov_str, base, fmt) = if is_native {
        (provider.provider_type, "", "")
    } else {
        ("custom", provider.base_url, provider.api_format)
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
        Ok(_) => return prompt_text_input(stdout, "  No models found. Model name: "),
        Err(e) => {
            let _ = writeln!(stdout, "  Fetch failed: {e}\r");
            let _ = stdout.flush();
            return prompt_text_input(stdout, "  Model name: ");
        }
    };

    let mut selected = 0usize;
    let mut scroll = 0usize;
    let mut filter = String::new();
    let mut filtered = models.clone();
    let max_visible = 12;

    loop {
        clear_screen(stdout);
        draw_header(stdout, "Step 3: Select Model");
        let _ = writeln!(
            stdout,
            "  {} models{}\r",
            filtered.len(),
            if filter.is_empty() {
                String::new()
            } else {
                format!(" (filter: {filter})")
            }
        );
        let _ = writeln!(stdout, "\r");
        let end = (scroll + max_visible).min(filtered.len());
        for (i, model_name) in filtered.iter().enumerate().skip(scroll).take(end - scroll) {
            let m = if i == selected { " ▸ " } else { "   " };
            let _ = writeln!(stdout, "{m}{model_name}\r");
        }
        let _ = writeln!(
            stdout,
            "\r\n  ↑/↓ navigate  Enter select  / filter  Esc back\r"
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
                if let Some(f) = prompt_text_input(stdout, "  Filter: ") {
                    filter = f;
                    filtered = model_fetch::apply_model_filter(&models, &filter);
                    selected = 0;
                    scroll = 0;
                }
            }
            KeyCode::Esc => return None,
            _ => {}
        }
    }
}

fn prompt_text_input(stdout: &mut io::Stdout, prompt: &str) -> Option<String> {
    let mut buf = String::new();
    loop {
        clear_screen(stdout);
        draw_header(stdout, "");
        let _ = writeln!(stdout, "{prompt}{buf}\r");
        let _ = writeln!(stdout, "\r\n  Enter confirm  Esc cancel\r");
        let _ = stdout.flush();

        let key = read_key()?;
        match key.code {
            KeyCode::Enter if !buf.trim().is_empty() => return Some(buf.trim().to_string()),
            KeyCode::Esc => return None,
            KeyCode::Backspace => {
                buf.pop();
            }
            KeyCode::Char(c) => buf.push(c),
            _ => {}
        }
    }
}

fn step_confirm_save(
    stdout: &mut io::Stdout,
    provider: &LoginProvider,
    api_key: &str,
    model: &str,
    cwd: &str,
) -> bool {
    clear_screen(stdout);
    draw_header(stdout, "Step 4: Confirm & Save");
    let _ = writeln!(stdout, "  Provider:  {}\r", provider.name);
    let _ = writeln!(stdout, "  Model:     {model}\r");
    if !api_key.is_empty() {
        let _ = writeln!(stdout, "  Key:       {}\r", mask_key(api_key));
    }
    let _ = writeln!(stdout, "\r");
    let _ = writeln!(stdout, "  Enter save  Esc cancel\r");
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
                if provider.provider_type == "custom" {
                    let _ = Settings::persist_key_value(
                        "model_provider",
                        Some("custom"),
                        SettingsTier::User,
                        cwd,
                    );
                    let _ = Settings::persist_key_value(
                        "custom_base_url",
                        Some(provider.base_url),
                        SettingsTier::User,
                        cwd,
                    );
                    let _ = Settings::persist_key_value(
                        "custom_api_format",
                        Some(provider.api_format),
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
                    let _ =
                        Settings::persist_key_value("custom_preset", None, SettingsTier::User, cwd);
                }

                clear_screen(stdout);
                draw_header(stdout, "Done!");
                let _ = writeln!(stdout, "  Provider:  {}\r", provider.name);
                let _ = writeln!(stdout, "  Model:     {model}\r");
                let _ = writeln!(stdout, "  Saved to ~/.nocode/settings.json\r");
                let _ = writeln!(stdout, "\r");
                let _ = writeln!(stdout, "  Run `nocode` to start.\r");
                let _ = writeln!(stdout, "\r");
                let _ = stdout.flush();
                return true;
            }
            KeyCode::Esc => return false,
            _ => {}
        }
    }
}

pub fn run_login(cwd: &str) {
    let providers = build_provider_list();
    let mut stdout = io::stdout();

    terminal::enable_raw_mode().expect("Failed to enable raw mode");
    let result = run_login_inner(&mut stdout, &providers, cwd);
    terminal::disable_raw_mode().expect("Failed to disable raw mode");
    let _ = execute!(stdout, cursor::Show);

    if !result {
        println!("\r\n  Login cancelled.\r");
    }
}

fn run_login_inner(stdout: &mut io::Stdout, providers: &[LoginProvider], cwd: &str) -> bool {
    // Step 1: Select provider
    let idx = match step_select_provider(stdout, providers) {
        Some(i) => i,
        None => return false,
    };
    let provider = &providers[idx];

    // Step 2: API key
    let api_key = match step_api_key(stdout, provider) {
        Some(k) => k,
        None => return false,
    };

    // Step 3: Select model
    let model = match step_select_model(stdout, provider, &api_key) {
        Some(m) => m,
        None => return false,
    };

    // Step 4: Confirm & save
    step_confirm_save(stdout, provider, &api_key, &model, cwd)
}
