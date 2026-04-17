//! Ratatui-based TUI application core — v2 rewrite.
//!
//! Bridges the agentic loop (running on a background thread) to the TUI
//! via mpsc channels. The TUI thread owns the terminal and polls both
//! crossterm events and channel events at 50ms intervals.

use crate::command_registry::CommandRegistry;
use crate::markdown_render::render_markdown_to_lines;
use crate::resolve_provider;
use crate::spinner::Spinner;
use crate::status_hud::StatusHud;
use crate::tui_commands::{SlashResult, handle_slash_command};
use crate::tui_events::{ChannelObserver, TuiEvent};
use crate::tui_widgets::{
    ChatMessage, ChatMessageKind, HintsBar, InputBox, StatusBar, WelcomeBanner, WelcomeBannerInfo,
};

use base64::Engine as _;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use nocode_core::config::settings::{Settings, SettingsTier};
use nocode_core::message::{ContentBlock, Message, SystemBlock};
use nocode_core::provider::Provider;
use nocode_core::query::r#loop::{self, LoopConfig};
use nocode_core::tool::ToolRegistry;
use nocode_core::tool::executor::ToolExecutor;
use nocode_core::tool::global_registry::tool_definitions_for_model;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Block, Borders};
use std::io;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;
use unicode_width::UnicodeWidthChar;

const LOG_LIMIT: usize = 256;

// ---------------------------------------------------------------------------
// Custom provider presets
// ---------------------------------------------------------------------------

pub(crate) struct ProviderPreset {
    pub name: &'static str,
    pub base_url: &'static str,
    pub api_format: &'static str,
    pub auth_hint: &'static str,
    pub env_key_name: &'static str,
    pub credential_slot: &'static str,
    pub default_model: &'static str,
}

pub(crate) static CUSTOM_PRESETS: &[ProviderPreset] = &[
    // --- Cloud API proxies ---
    ProviderPreset {
        name: "OpenRouter",
        base_url: "https://openrouter.ai/api/v1",
        api_format: "openai-chat",
        auth_hint: "Get key at openrouter.ai/keys",
        env_key_name: "OPENROUTER_API_KEY",
        credential_slot: "openrouter",
        default_model: "anthropic/claude-sonnet-4",
    },
    ProviderPreset {
        name: "Together",
        base_url: "https://api.together.xyz/v1",
        api_format: "openai-chat",
        auth_hint: "Get key at api.together.xyz/settings/api-keys",
        env_key_name: "TOGETHER_API_KEY",
        credential_slot: "together",
        default_model: "meta-llama/Llama-3-70b-chat-hf",
    },
    ProviderPreset {
        name: "Groq",
        base_url: "https://api.groq.com/openai/v1",
        api_format: "openai-chat",
        auth_hint: "Get key at console.groq.com/keys",
        env_key_name: "GROQ_API_KEY",
        credential_slot: "groq",
        default_model: "llama-3.3-70b-versatile",
    },
    ProviderPreset {
        name: "Fireworks",
        base_url: "https://api.fireworks.ai/inference/v1",
        api_format: "openai-chat",
        auth_hint: "Get key at fireworks.ai/account/api-keys",
        env_key_name: "FIREWORKS_API_KEY",
        credential_slot: "fireworks",
        default_model: "accounts/fireworks/models/llama-v3p1-70b-instruct",
    },
    ProviderPreset {
        name: "DeepSeek",
        base_url: "https://api.deepseek.com/v1",
        api_format: "openai-chat",
        auth_hint: "Get key at platform.deepseek.com/api_keys",
        env_key_name: "DEEPSEEK_API_KEY",
        credential_slot: "deepseek",
        default_model: "deepseek-chat",
    },
    ProviderPreset {
        name: "Mistral",
        base_url: "https://api.mistral.ai/v1",
        api_format: "openai-chat",
        auth_hint: "Get key at console.mistral.ai/api-keys",
        env_key_name: "MISTRAL_API_KEY",
        credential_slot: "mistral",
        default_model: "mistral-large-latest",
    },
    // --- Local inference ---
    ProviderPreset {
        name: "Ollama",
        base_url: "http://localhost:11434/v1",
        api_format: "openai-chat",
        auth_hint: "No API key needed for local Ollama",
        env_key_name: "",
        credential_slot: "ollama",
        default_model: "",
    },
    ProviderPreset {
        name: "vLLM",
        base_url: "http://localhost:8000/v1",
        api_format: "openai-chat",
        auth_hint: "Use --api-key flag if set on server",
        env_key_name: "VLLM_API_KEY",
        credential_slot: "vllm",
        default_model: "",
    },
    ProviderPreset {
        name: "LiteLLM",
        base_url: "http://localhost:4000/v1",
        api_format: "openai-chat",
        auth_hint: "Set LITELLM_API_KEY or use proxy key",
        env_key_name: "LITELLM_API_KEY",
        credential_slot: "litellm",
        default_model: "",
    },
    ProviderPreset {
        name: "LocalAI",
        base_url: "http://localhost:8080/v1",
        api_format: "openai-chat",
        auth_hint: "Optional, depends on config",
        env_key_name: "",
        credential_slot: "localai",
        default_model: "",
    },
    ProviderPreset {
        name: "LM Studio",
        base_url: "http://localhost:1234/v1",
        api_format: "openai-chat",
        auth_hint: "No key required for local LM Studio",
        env_key_name: "",
        credential_slot: "lmstudio",
        default_model: "",
    },
];

/// Detect which preset matches the current custom URL, if any.
pub(crate) fn detect_preset(base_url: &str) -> Option<usize> {
    let normalized = base_url.trim().trim_end_matches('/');
    CUSTOM_PRESETS.iter().position(|p| {
        p.base_url
            .trim_end_matches('/')
            .eq_ignore_ascii_case(normalized)
    })
}

/// Get the preset name for display, or "Manual" if no preset matches.
pub(crate) fn preset_label(index: Option<usize>) -> &'static str {
    match index {
        Some(i) if i < CUSTOM_PRESETS.len() => CUSTOM_PRESETS[i].name,
        _ => "Manual",
    }
}

// ---------------------------------------------------------------------------
// Vim input mode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum InputMode {
    #[default]
    Insert,
    Normal,
}

impl InputMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Insert => "INSERT",
            Self::Normal => "NORMAL",
        }
    }
}

/// All mutable state for the config overlay, extracted to keep `Overlay` small.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ConfigState {
    pub selected: usize,
    pub tier: usize,
    pub suggestion_index: usize,
    pub suggestion_scroll: usize,
    pub filtering_models: bool,
    pub editing: bool,
    pub input: String,
    pub status: Option<String>,
    pub provider: String,
    pub provider_source: String,
    pub api_key: String,
    pub api_key_source: String,
    pub model: String,
    pub model_source: String,
    pub custom_base_url: String,
    pub custom_base_url_source: String,
    pub custom_api_format: String,
    pub custom_api_format_source: String,
    pub model_filter: String,
    pub all_model_suggestions: Vec<String>,
    pub model_suggestions: Vec<String>,
    /// Active preset index for custom provider, None = manual.
    pub preset_index: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum Overlay {
    #[default]
    None,
    Help,
    Status,
    Sessions,
    Mcp,
    Agents,
    Config(Box<ConfigState>),
    Memory,
    Cost,
    Permission {
        tool_name: String,
        tool_id: String,
    },
    Question {
        questions: serde_json::Value,
        selected: Vec<usize>,
    },
    Errors(Vec<String>),
}

impl Overlay {
    fn is_open(&self) -> bool {
        !matches!(self, Self::None)
    }
}

impl TuiApp {
    pub(crate) fn open_config_overlay(&mut self) {
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let settings = Settings::load_merged(&cwd);
        let user_settings = Settings::load_tier(SettingsTier::User, &cwd);
        let project_settings = Settings::load_tier(SettingsTier::Project, &cwd);
        let local_settings = Settings::load_tier(SettingsTier::Local, &cwd);
        let custom_base_url = settings.custom_base_url.clone().unwrap_or_default();
        let custom_api_format = settings
            .custom_api_format
            .clone()
            .unwrap_or_else(|| "openai-responses".to_string());
        let provider = settings
            .model_provider
            .clone()
            .unwrap_or_else(|| resolve_provider(&settings).as_str().to_string());
        let detected_preset = settings
            .custom_preset
            .as_deref()
            .and_then(|name| {
                CUSTOM_PRESETS
                    .iter()
                    .position(|p| p.name.eq_ignore_ascii_case(name))
            })
            .or_else(|| detect_preset(&custom_base_url));
        let preset_name_str = detected_preset
            .and_then(|i| CUSTOM_PRESETS.get(i))
            .map(|p| p.name);
        let credential_store = load_credential_store();
        let (api_key_slot, api_key_env_var) =
            provider_key_slot(&provider, &custom_api_format, preset_name_str);
        let api_key = if !api_key_env_var.is_empty() {
            std::env::var(api_key_env_var).ok()
        } else {
            None
        }
        .or_else(|| credential_store.get_key(api_key_slot))
        .unwrap_or_default();
        // Spawn background model fetch instead of blocking the UI
        self.spawn_model_fetch(
            provider.as_str(),
            custom_base_url.trim(),
            custom_api_format.trim(),
        );
        self.overlay = Overlay::Config(Box::new(ConfigState {
            selected: 0,
            tier: 0,
            suggestion_index: 0,
            suggestion_scroll: 0,
            filtering_models: false,
            editing: false,
            input: String::new(),
            status: Some("Loading models...".to_string()),
            provider,
            provider_source: setting_source_label(
                "model_provider",
                "NOCODE_MODEL_PROVIDER",
                &user_settings,
                &project_settings,
                &local_settings,
                (!custom_base_url.is_empty() || !custom_api_format.is_empty()).then_some("derived"),
            ),
            api_key,
            api_key_source: if std::env::var(api_key_env_var).is_ok() {
                "env".to_string()
            } else if credential_store.get_key(api_key_slot).is_some() {
                "credentials".to_string()
            } else {
                "unset".to_string()
            },
            model: settings.model.unwrap_or_default(),
            model_source: setting_source_label(
                "model",
                "NOCODE_MODEL",
                &user_settings,
                &project_settings,
                &local_settings,
                None,
            ),
            custom_base_url,
            custom_base_url_source: setting_source_label(
                "custom_base_url",
                "NOCODE_CUSTOM_BASE_URL",
                &user_settings,
                &project_settings,
                &local_settings,
                None,
            ),
            custom_api_format,
            custom_api_format_source: setting_source_label(
                "custom_api_format",
                "NOCODE_CUSTOM_API_FORMAT",
                &user_settings,
                &project_settings,
                &local_settings,
                Some("default"),
            ),
            model_filter: String::new(),
            all_model_suggestions: Vec::new(),
            model_suggestions: Vec::new(),
            preset_index: detected_preset,
        }));
        self.dirty = true;
    }
}

// ---------------------------------------------------------------------------
// TuiApp — main application state
// ---------------------------------------------------------------------------

pub(crate) struct TuiApp {
    pub(crate) chat_messages: Vec<ChatMessage>,
    pub(crate) input: String,
    pub(crate) cursor_pos: usize,
    pub(crate) chat_scroll: u16,
    pub(crate) overlay: Overlay,
    pub(crate) thinking_spinner: Option<Spinner>,
    pub(crate) dirty: bool,
    height_cache: Vec<u16>,
    height_cache_width: u16,
    pub(crate) sticky_scroll: bool,
    pub(crate) unseen_count: usize,
    pub(crate) show_banner: bool,
    banner_info: WelcomeBannerInfo,
    pub(crate) streaming_text: String,
    pub(crate) streaming_thinking: String,
    pub(crate) input_history: Vec<String>,
    pub(crate) history_index: Option<usize>,
    pub(crate) saved_input: String,
    pub(crate) input_mode: InputMode,
    pub(crate) vim_pending: Option<char>,
    pub(crate) hud: StatusHud,
    pub(crate) error_log: Vec<String>,
    /// Channel to send permission decisions back to the executor thread.
    pub(crate) permission_tx:
        Option<std::sync::mpsc::Sender<nocode_core::tool::permission::PermissionDecision>>,
    /// Channel to send question answers back to the executor thread.
    pub(crate) question_tx:
        Option<std::sync::mpsc::Sender<Result<nocode_core::tool::permission::UserAnswer, String>>>,
    /// Search state
    pub(crate) search_query: String,
    pub(crate) search_active: bool,
    pub(crate) search_matches: Vec<usize>,
    pub(crate) search_index: usize,
    /// Background model fetch receiver
    pub(crate) model_fetch_rx: Option<std::sync::mpsc::Receiver<Result<Vec<String>, String>>>,
    /// Background worker event receiver
    pub(crate) worker_event_rx:
        Option<std::sync::mpsc::Receiver<nocode_core::agent::worker::WorkerEvent>>,
    /// Command completion state
    pub(crate) completion_selected: Option<usize>,
    /// Pending images from clipboard paste, awaiting submit.
    pub(crate) pending_images: Vec<PendingImage>,
    /// Overlay scroll offset for scrollable overlays.
    pub(crate) overlay_scroll: u16,
    /// Horizontal scroll offset for input box (single-line long input).
    pub(crate) input_view_offset: usize,
    /// Vertical scroll offset for input box (multi-line input).
    pub(crate) input_scroll_y: u16,
}

/// An image pasted from clipboard, waiting to be sent with the next message.
pub(crate) struct PendingImage {
    pub media_type: String,
    pub base64_data: String,
    pub size_bytes: usize,
}

impl TuiApp {
    pub fn new(model: &str) -> Self {
        Self {
            chat_messages: Vec::new(),
            input: String::new(),
            cursor_pos: 0,
            chat_scroll: 0,
            overlay: Overlay::None,
            thinking_spinner: None,
            dirty: true,
            height_cache: Vec::new(),
            height_cache_width: 0,
            sticky_scroll: true,
            unseen_count: 0,
            show_banner: true,
            banner_info: WelcomeBannerInfo::default(),
            streaming_text: String::new(),
            streaming_thinking: String::new(),
            input_history: Vec::new(),
            history_index: None,
            saved_input: String::new(),
            input_mode: InputMode::Insert,
            vim_pending: None,
            hud: StatusHud::new(model, ""),
            error_log: Vec::new(),
            permission_tx: None,
            question_tx: None,
            search_query: String::new(),
            search_active: false,
            search_matches: Vec::new(),
            search_index: 0,
            model_fetch_rx: None,
            worker_event_rx: None,
            completion_selected: None,
            pending_images: Vec::new(),
            overlay_scroll: 0,
            input_view_offset: 0,
            input_scroll_y: 0,
        }
    }

    /// Spawn a background thread to fetch model suggestions without blocking the UI.
    fn spawn_model_fetch(
        &mut self,
        provider: &str,
        custom_base_url: &str,
        custom_api_format: &str,
    ) {
        let provider = provider.to_string();
        let base_url = custom_base_url.to_string();
        let api_format = custom_api_format.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        self.model_fetch_rx = Some(rx);
        std::thread::spawn(move || {
            let result = fetch_model_suggestions(&provider, &base_url, &api_format);
            let _ = tx.send(result);
        });
    }

    /// Poll for background model fetch results. Returns Some if results are ready.
    fn poll_model_fetch(&mut self) -> Option<Result<Vec<String>, String>> {
        let rx = self.model_fetch_rx.as_ref()?;
        match rx.try_recv() {
            Ok(result) => {
                self.model_fetch_rx = None;
                Some(result)
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.model_fetch_rx = None;
                Some(Err("Model fetch thread disconnected".to_string()))
            }
        }
    }

    // -- content push methods --

    fn on_message_added(&mut self) {
        self.trim_log();
        self.invalidate_height_cache();
        if self.sticky_scroll {
            self.chat_scroll = 0;
        } else {
            self.unseen_count += 1;
        }
        self.dirty = true;
    }

    fn trim_log(&mut self) {
        if self.chat_messages.len() > LOG_LIMIT {
            let drain = self.chat_messages.len() - LOG_LIMIT;
            self.chat_messages.drain(..drain);
            self.invalidate_height_cache();
        }
    }

    pub fn push_system(&mut self, text: &str) {
        self.chat_messages
            .push(ChatMessage::plain(ChatMessageKind::System, text));
        self.on_message_added();
    }

    pub fn push_user_message(&mut self, text: &str) {
        self.chat_messages
            .push(ChatMessage::plain(ChatMessageKind::User, text));
        self.on_message_added();
    }

    pub fn push_error(&mut self, text: &str) {
        self.error_log.push(text.to_string());
        self.chat_messages
            .push(ChatMessage::plain(ChatMessageKind::Error, text));
        self.on_message_added();
    }

    pub fn push_tool_start(&mut self, name: &str) {
        let info = crate::tui_widgets::ToolCallInfo::new(name, "");
        let msg = ChatMessage::tool_call(info, Vec::new());
        self.chat_messages.push(msg);
        self.on_message_added();
    }

    pub fn push_tool_done(&mut self, name: &str, content: &str, is_error: bool) {
        // Try to update the last Tool message with structured result
        let content_lines = crate::markdown_render::render_markdown_to_lines(content);
        if let Some(last) = self.chat_messages.last_mut()
            && last.kind == ChatMessageKind::Tool
        {
            if let Some(info) = &mut last.tool_info {
                info.result_preview = Some(content.to_string());
                // Auto-expand errors, keep normal results collapsed
                info.collapsed = !is_error;
                if is_error {
                    last.kind = ChatMessageKind::Error;
                }
            }
            last.lines = content_lines;
            self.invalidate_height_cache();
            self.dirty = true;
            return;
        }
        // Fallback: plain message
        let prefix = if is_error { "✖" } else { "⎿" };
        let display = if content.len() > 200 {
            crate::tool_render::truncate_str(content, 200)
        } else {
            content.to_string()
        };
        let kind = if is_error {
            ChatMessageKind::Error
        } else {
            ChatMessageKind::Tool
        };
        self.chat_messages.push(ChatMessage::plain(
            kind,
            &format!("{prefix} {name} {display}"),
        ));
        self.on_message_added();
    }

    /// Update streaming assistant text incrementally.
    pub fn update_streaming_assistant(&mut self, accumulated: &str) {
        let lines = render_markdown_to_lines(accumulated);
        if let Some(last) = self.chat_messages.last_mut()
            && last.kind == ChatMessageKind::Assistant
        {
            last.lines = lines;
            self.invalidate_last_height();
            self.dirty = true;
            return;
        }
        self.chat_messages
            .push(ChatMessage::new(ChatMessageKind::Assistant, lines));
        self.on_message_added();
    }

    /// Update streaming thinking text incrementally.
    pub fn update_streaming_thinking(&mut self, accumulated: &str) {
        let lines: Vec<crate::markdown_render::RenderedLine> = accumulated
            .lines()
            .enumerate()
            .map(|(i, l)| {
                let mut rl = crate::markdown_render::RenderedLine::new();
                let prefix = if i == 0 { "∴ " } else { "  " };
                rl.push(crate::markdown_render::LineSegment::new(
                    format!("{prefix}{l}"),
                    crossterm::style::Color::DarkGrey,
                ));
                rl
            })
            .collect();
        if let Some(last) = self.chat_messages.last_mut()
            && last.kind == ChatMessageKind::Thinking
        {
            last.lines = lines;
            self.invalidate_last_height();
            self.dirty = true;
            return;
        }
        self.chat_messages
            .push(ChatMessage::new(ChatMessageKind::Thinking, lines));
        self.on_message_added();
    }

    // -- height cache --

    pub(crate) fn invalidate_height_cache(&mut self) {
        self.height_cache.clear();
        self.height_cache_width = 0;
    }

    /// Invalidate only the last message's cached height (for streaming updates).
    fn invalidate_last_height(&mut self) {
        if !self.height_cache.is_empty() {
            self.height_cache.pop();
        }
    }

    fn message_height(msg: &ChatMessage, width: u16) -> u16 {
        // Fast path: collapsed tool messages are 1 line
        if (msg.kind == ChatMessageKind::Tool || msg.kind == ChatMessageKind::Error)
            && let Some(info) = &msg.tool_info
            && info.collapsed
        {
            return 1;
        }
        // Use pre-computed lines count instead of re-rendering
        let mut total: u16 = 0;
        for line in &msg.lines {
            let line_width: usize = line.segments.iter().map(|s| s.text.len()).sum();
            let wrapped = if width > 0 {
                (line_width as u16).div_ceil(width)
            } else {
                1
            };
            total += wrapped.max(1);
        }
        total.max(1)
    }

    fn ensure_height_cache(&mut self, width: u16) {
        if self.height_cache_width != width {
            self.height_cache.clear();
            self.height_cache_width = width;
        }
        while self.height_cache.len() < self.chat_messages.len() {
            let idx = self.height_cache.len();
            let h = Self::message_height(&self.chat_messages[idx], width);
            self.height_cache.push(h);
        }
    }

    fn total_content_height(&self) -> u16 {
        self.height_cache.iter().copied().sum()
    }

    // -- drawing --

    pub fn draw(&mut self, frame: &mut Frame) {
        let total_height = frame.area().height;
        // Minimum terminal size protection
        if total_height < 4 || frame.area().width < 20 {
            let msg = ratatui::widgets::Paragraph::new("Terminal too small");
            frame.render_widget(msg, frame.area());
            self.dirty = false;
            return;
        }

        let is_busy = self.thinking_spinner.is_some();
        let hints_height: u16 = if is_busy || total_height < 8 { 0 } else { 1 };
        let input_lines = (self.input.chars().filter(|&c| c == '\n').count() as u16 + 1).min(10);

        // Layout: Chat → Status (separator) → Input → Hints
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(4),
                Constraint::Length(1),
                Constraint::Length(input_lines),
                Constraint::Length(hints_height),
            ])
            .split(frame.area());

        // 1. Banner or chat
        if self.show_banner && self.chat_messages.is_empty() {
            let banner = WelcomeBanner::new(&self.banner_info);
            frame.render_widget(banner, chunks[0]);
        } else {
            self.draw_chat_area(frame, chunks[0]);
        }

        // 2. Status line (separator between chat and input)
        let pending_img_hint = if self.pending_images.is_empty() {
            None
        } else {
            let total_kb: usize = self
                .pending_images
                .iter()
                .map(|i| i.size_bytes / 1024)
                .sum();
            Some(format!(
                "[{} image{} attached, {}KB]",
                self.pending_images.len(),
                if self.pending_images.len() > 1 {
                    "s"
                } else {
                    ""
                },
                total_kb
            ))
        };
        let mut status_base = pending_img_hint
            .or_else(|| self.search_status())
            .or_else(|| self.slash_command_hint())
            .unwrap_or_else(|| self.hud.render_line());
        // Append undo/redo stack depth
        {
            let history = nocode_core::storage::file_history::global_file_history();
            if let Ok(h) = history.lock() {
                let uc = h.undo_count();
                let rc = h.redo_count();
                if uc > 0 || rc > 0 {
                    status_base.push_str(&format!(" | undo:{uc} redo:{rc}"));
                }
            }
        }
        // Append unseen message indicator when scrolled up
        if self.unseen_count > 0 && self.chat_scroll > 0 {
            status_base.push_str(&format!(" | {} new", self.unseen_count));
        }
        let status = StatusBar::new(&status_base);
        frame.render_widget(status, chunks[1]);

        // 3. Input
        let mode_label = if self.input_mode == InputMode::Normal {
            self.input_mode.label()
        } else {
            ""
        };
        let input_widget = InputBox::new(&self.input, self.cursor_pos)
            .with_mode(mode_label)
            .with_view_offset(self.input_view_offset)
            .with_scroll_y(self.input_scroll_y);
        frame.render_widget(input_widget, chunks[2]);

        // 4. Hints
        if !is_busy {
            let hints = HintsBar {
                vim_normal: self.input_mode == InputMode::Normal,
                has_completion: self.completion_selected.is_some(),
                has_images: !self.pending_images.is_empty(),
            };
            frame.render_widget(hints, chunks[3]);
        }

        // Cursor position (relative to input chunk)
        let text_before_cursor = &self.input[..self.cursor_pos];
        let cursor_line = text_before_cursor.chars().filter(|&c| c == '\n').count() as u16;
        let last_newline = text_before_cursor.rfind('\n').map_or(0, |p| p + 1);
        let line_text = &self.input[last_newline..self.cursor_pos];
        let mode_prefix_width: u16 = if cursor_line == 0 && !mode_label.is_empty() {
            (mode_label.len() + 3) as u16
        } else {
            0
        };
        let char_col = line_text.chars().count();
        // Update horizontal scroll so cursor stays visible
        let usable_width = chunks[2].width.saturating_sub(2 + mode_prefix_width) as usize;
        if usable_width > 0 {
            if char_col < self.input_view_offset {
                self.input_view_offset = char_col;
            } else if char_col >= self.input_view_offset + usable_width {
                self.input_view_offset = char_col.saturating_sub(usable_width) + 1;
            }
        }
        let visible_col = char_col.saturating_sub(self.input_view_offset) as u16;
        let cursor_col = visible_col + 2 + mode_prefix_width;
        let cursor_x = chunks[2].x + cursor_col;
        // Update vertical scroll so cursor line stays visible
        if input_lines > 0 {
            if cursor_line < self.input_scroll_y {
                self.input_scroll_y = cursor_line;
            } else if cursor_line >= self.input_scroll_y + input_lines {
                self.input_scroll_y = cursor_line.saturating_sub(input_lines) + 1;
            }
        }
        let visible_cursor_line = cursor_line.saturating_sub(self.input_scroll_y);
        let cursor_y = chunks[2].y + visible_cursor_line;
        frame.set_cursor_position((cursor_x, cursor_y));

        // 5. Command completion popup (above input box)
        if self.completion_selected.is_some() && !self.overlay.is_open() {
            self.draw_completion_popup(frame, chunks[2]);
        }

        // 6. Overlay
        if self.overlay.is_open() {
            self.draw_overlay(frame, frame.area());
        }

        self.dirty = false;
    }

    fn draw_chat_area(&mut self, frame: &mut Frame, area: Rect) {
        let block = Block::default().borders(Borders::NONE);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        self.ensure_height_cache(inner.width);

        let total_h = self.total_content_height();
        let visible = inner.height;
        let max_scroll = total_h.saturating_sub(visible);
        let scroll = self.chat_scroll.min(max_scroll);
        let scroll_from_top = max_scroll.saturating_sub(scroll);

        let theme = crate::tui_theme::default_theme();

        let mut accumulated: u16 = 0;
        let mut first_visible_skip: u16 = 0;
        let mut y_offset: u16 = 0;

        for (i, msg) in self.chat_messages.iter().enumerate() {
            let h = self.height_cache.get(i).copied().unwrap_or(1);
            let msg_end = accumulated + h;

            if msg_end <= scroll_from_top {
                accumulated = msg_end;
                continue;
            }

            if y_offset == 0 && accumulated < scroll_from_top {
                first_visible_skip = scroll_from_top.saturating_sub(accumulated);
            }

            // Pick background color by message kind
            let bg = match msg.kind {
                ChatMessageKind::User => theme.user_msg_bg,
                ChatMessageKind::Assistant => theme.assistant_msg_bg,
                ChatMessageKind::Tool => theme.tool_msg_bg,
                ChatMessageKind::Error => theme.error_msg_bg,
                _ => ratatui::style::Color::Reset,
            };

            let rlines = msg.to_ratatui_lines();
            let search_q = if self.search_active && !self.search_query.is_empty() {
                Some(self.search_query.clone())
            } else {
                None
            };
            for line in rlines {
                let line = if let Some(ref q) = search_q {
                    highlight_search_in_line(line, q)
                } else {
                    line
                };
                if first_visible_skip > 0 {
                    first_visible_skip -= 1;
                    continue;
                }

                // Render background fill for this line
                if bg != ratatui::style::Color::Reset {
                    let line_rect = Rect {
                        x: inner.x,
                        y: inner.y + y_offset,
                        width: inner.width,
                        height: 1,
                    };
                    let bg_block = Block::default().style(ratatui::style::Style::default().bg(bg));
                    frame.render_widget(bg_block, line_rect);
                }

                // Render the text line
                let line_rect = Rect {
                    x: inner.x,
                    y: inner.y + y_offset,
                    width: inner.width,
                    height: 1,
                };
                let para = ratatui::widgets::Paragraph::new(vec![line]);
                frame.render_widget(para, line_rect);

                y_offset += 1;
                if y_offset >= visible {
                    break;
                }
            }

            accumulated = msg_end;
            if y_offset >= visible {
                break;
            }
        }
    }

    fn draw_overlay(&self, frame: &mut Frame, area: Rect) {
        crate::tui_overlays::draw_overlay(
            &self.overlay,
            &self.hud,
            self.overlay_scroll,
            frame,
            area,
        );
    }

    /// Draw command completion popup above the input box.
    fn draw_completion_popup(&self, frame: &mut Frame, input_area: Rect) {
        let suggestions = self.completion_suggestions();
        if suggestions.is_empty() {
            return;
        }
        let selected = self.completion_selected.unwrap_or(0);
        let count = suggestions.len().min(10) as u16;
        let popup_height = count + 2; // +2 for border

        // Position: above input box if space, otherwise below
        let (popup_y, actual_height) = if input_area.y >= popup_height {
            (input_area.y - popup_height, popup_height)
        } else if input_area.y > 2 {
            // Partial fit above
            (0, input_area.y)
        } else {
            // No space above — render below input
            let below_y = input_area.y + input_area.height;
            let available = frame.area().height.saturating_sub(below_y);
            if available < 3 {
                return; // No space at all
            }
            (below_y, popup_height.min(available))
        };

        let popup_width = input_area.width.min(60);
        let popup_area = Rect {
            x: input_area.x,
            y: popup_y,
            width: popup_width,
            height: actual_height,
        };

        // Clear background
        frame.render_widget(ratatui::widgets::Clear, popup_area);

        let theme = crate::tui_theme::default_theme();
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(ratatui::style::Style::default().fg(theme.border))
            .title(" Commands ")
            .title_style(ratatui::style::Style::default().fg(theme.claude));

        let inner = block.inner(popup_area);
        frame.render_widget(block, popup_area);

        // Render each suggestion line
        for (i, (label, summary)) in suggestions.iter().take(count as usize).enumerate() {
            let is_selected = i == selected;
            let y = inner.y + i as u16;
            if y >= inner.y + inner.height {
                break;
            }

            let line_area = Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: 1,
            };

            // Truncate label safely (UTF-8 aware)
            let max_label = 20.min(inner.width as usize / 2);
            let display_label = safe_truncate(label, max_label);
            let remaining = inner.width as usize - display_label.len() - 1;
            let display_summary = safe_truncate_ellipsis(summary, remaining);

            let style = if is_selected {
                ratatui::style::Style::default()
                    .fg(theme.background)
                    .bg(theme.claude)
            } else {
                ratatui::style::Style::default().fg(theme.text)
            };

            // Fill background for selected line
            if is_selected {
                let bg_block =
                    Block::default().style(ratatui::style::Style::default().bg(theme.claude));
                frame.render_widget(bg_block, line_area);
            }

            let text = format!("{display_label} {display_summary}");
            let para = ratatui::widgets::Paragraph::new(text).style(style);
            frame.render_widget(para, line_area);
        }
    }

    // -- key handling --

    /// Returns false if the app should quit.
    pub fn handle_key(&mut self, key: KeyEvent) -> HandleKeyResult {
        // Ctrl-C = quit
        if matches!(
            (key.code, key.modifiers),
            (KeyCode::Char('c'), KeyModifiers::CONTROL)
        ) {
            return HandleKeyResult::Quit;
        }

        // Overlay open — handle keys
        if self.overlay.is_open() {
            if matches!(self.overlay, Overlay::Config(_)) {
                let mut system_notice: Option<String> = None;
                if let Overlay::Config(ref mut cs) = self.overlay {
                    let ConfigState {
                        ref mut selected,
                        ref mut tier,
                        ref mut suggestion_index,
                        ref mut suggestion_scroll,
                        ref mut filtering_models,
                        ref mut editing,
                        ref mut input,
                        ref mut status,
                        ref mut provider,
                        ref mut provider_source,
                        ref mut api_key,
                        ref mut api_key_source,
                        ref mut model,
                        ref mut model_source,
                        ref mut custom_base_url,
                        ref mut custom_base_url_source,
                        ref mut custom_api_format,
                        ref mut custom_api_format_source,
                        ref mut model_filter,
                        ref mut all_model_suggestions,
                        ref mut model_suggestions,
                        ref mut preset_index,
                    } = **cs;
                    let field_count = 5; // All fields always navigable
                    match key.code {
                        KeyCode::Esc => {
                            if *editing {
                                *editing = false;
                                input.clear();
                                *filtering_models = false;
                                *status = Some("Edit cancelled".to_string());
                            } else {
                                self.overlay = Overlay::None;
                            }
                            self.dirty = true;
                        }
                        KeyCode::Up if !*editing => {
                            if *selected == 2 && !model_suggestions.is_empty() {
                                // Navigate model list
                                if *suggestion_index > 0 {
                                    *suggestion_index -= 1;
                                    if *suggestion_index < *suggestion_scroll {
                                        *suggestion_scroll = *suggestion_index;
                                    }
                                } else {
                                    // At top of list, move to previous field
                                    *selected -= 1;
                                }
                            } else if *selected > 0 {
                                *selected -= 1;
                            }
                            self.dirty = true;
                        }
                        KeyCode::Down if !*editing => {
                            if *selected == 2 && !model_suggestions.is_empty() {
                                // Navigate model list
                                if *suggestion_index + 1 < model_suggestions.len() {
                                    *suggestion_index += 1;
                                    if *suggestion_index >= *suggestion_scroll + 5 {
                                        *suggestion_scroll = suggestion_index.saturating_sub(4);
                                    }
                                } else {
                                    // At bottom of list, move to next field
                                    if *selected + 1 < field_count {
                                        *selected += 1;
                                    }
                                }
                            } else if *selected + 1 < field_count {
                                *selected += 1;
                            }
                            self.dirty = true;
                        }
                        KeyCode::Left | KeyCode::Right if !*editing && *selected == 0 => {
                            *provider =
                                cycle_provider(provider, matches!(key.code, KeyCode::Right));
                            if provider != "custom" && *selected > 2 {
                                *selected = 2;
                            }
                            // Update preset detection on provider change
                            *preset_index = if provider == "custom" {
                                detect_preset(custom_base_url)
                            } else {
                                None
                            };
                            let pn = preset_index
                                .and_then(|i| CUSTOM_PRESETS.get(i))
                                .map(|p| p.name);
                            let (slot, env_var) = provider_key_slot(
                                provider.as_str(),
                                custom_api_format.as_str(),
                                pn,
                            );
                            let store = load_credential_store();
                            *api_key = std::env::var(env_var)
                                .ok()
                                .or_else(|| store.get_key(slot))
                                .unwrap_or_default();
                            *api_key_source = if std::env::var(env_var).is_ok() {
                                "env".to_string()
                            } else if store.get_key(slot).is_some() {
                                "credentials".to_string()
                            } else {
                                "unset".to_string()
                            };
                            if provider == "auto" {
                                all_model_suggestions.clear();
                                model_suggestions.clear();
                                *suggestion_index = 0;
                                *suggestion_scroll = 0;
                                *status = Some("Switched provider to auto".to_string());
                            } else {
                                apply_api_key_to_env(
                                    provider,
                                    custom_api_format,
                                    api_key,
                                    preset_index
                                        .and_then(|i| CUSTOM_PRESETS.get(i))
                                        .map(|p| p.name),
                                );
                                self.model_fetch_rx = Some(spawn_model_fetch_bg(
                                    provider,
                                    custom_base_url.trim(),
                                    custom_api_format.trim(),
                                ));
                                *suggestion_index = 0;
                                *suggestion_scroll = 0;
                                *status = Some(
                                    "Switched provider to custom; loading models...".to_string(),
                                );
                            }
                            self.dirty = true;
                        }
                        KeyCode::Left
                            if !*editing && *selected == 2 && !model_suggestions.is_empty() =>
                        {
                            if *suggestion_index > 0 {
                                *suggestion_index -= 1;
                            } else {
                                *suggestion_index = model_suggestions.len().saturating_sub(1);
                            }
                            if *suggestion_index < *suggestion_scroll {
                                *suggestion_scroll = *suggestion_index;
                            }
                            *status = Some("Moved model suggestion selection".to_string());
                            self.dirty = true;
                        }
                        KeyCode::Left | KeyCode::Right if !*editing && *selected == 4 => {
                            let formats = nocode_core::config::settings::API_FORMATS;
                            let normalized = nocode_core::config::settings::normalize_api_format(
                                custom_api_format,
                            );
                            let current_idx =
                                formats.iter().position(|&f| f == normalized).unwrap_or(0);
                            let next_idx = if matches!(key.code, KeyCode::Right) {
                                (current_idx + 1) % formats.len()
                            } else {
                                (current_idx + formats.len() - 1) % formats.len()
                            };
                            *custom_api_format = formats[next_idx].to_string();
                            // Re-detect preset after format toggle
                            *preset_index = detect_preset(custom_base_url);
                            apply_api_key_to_env(
                                provider,
                                custom_api_format,
                                api_key,
                                preset_index
                                    .and_then(|i| CUSTOM_PRESETS.get(i))
                                    .map(|p| p.name),
                            );
                            self.model_fetch_rx = Some(spawn_model_fetch_bg(
                                provider,
                                custom_base_url.trim(),
                                custom_api_format.trim(),
                            ));
                            *suggestion_index = 0;
                            *suggestion_scroll = 0;
                            *status = Some(format!(
                                "Switched API format to {}; loading models...",
                                custom_api_format
                            ));
                            self.dirty = true;
                        }
                        KeyCode::Right
                            if !*editing && *selected == 2 && !model_suggestions.is_empty() =>
                        {
                            *suggestion_index = (*suggestion_index + 1) % model_suggestions.len();
                            if *suggestion_index >= *suggestion_scroll + 5 {
                                *suggestion_scroll = suggestion_index.saturating_sub(4);
                            }
                            *status = Some("Moved model suggestion selection".to_string());
                            self.dirty = true;
                        }
                        KeyCode::PageUp
                            if !*editing && *selected == 2 && !model_suggestions.is_empty() =>
                        {
                            *suggestion_index = (*suggestion_index).saturating_sub(5);
                            *suggestion_scroll = (*suggestion_scroll).saturating_sub(5);
                            *status = Some("Scrolled model suggestions up".to_string());
                            self.dirty = true;
                        }
                        KeyCode::PageDown
                            if !*editing && *selected == 2 && !model_suggestions.is_empty() =>
                        {
                            let max_index = model_suggestions.len().saturating_sub(1);
                            *suggestion_index = (*suggestion_index + 5).min(max_index);
                            *suggestion_scroll =
                                (*suggestion_scroll + 5).min(max_index.saturating_sub(4));
                            *status = Some("Scrolled model suggestions down".to_string());
                            self.dirty = true;
                        }
                        KeyCode::Home
                            if !*editing && *selected == 2 && !model_suggestions.is_empty() =>
                        {
                            *suggestion_index = 0;
                            *suggestion_scroll = 0;
                            *status = Some("Jumped to first model suggestion".to_string());
                            self.dirty = true;
                        }
                        KeyCode::End
                            if !*editing && *selected == 2 && !model_suggestions.is_empty() =>
                        {
                            let max_index = model_suggestions.len().saturating_sub(1);
                            *suggestion_index = max_index;
                            *suggestion_scroll = max_index.saturating_sub(4);
                            *status = Some("Jumped to last model suggestion".to_string());
                            self.dirty = true;
                        }
                        KeyCode::Tab if !*editing => {
                            *tier = (*tier + 1) % 3;
                            self.dirty = true;
                        }
                        KeyCode::Enter => {
                            if *editing {
                                if *filtering_models {
                                    *model_filter = input.clone();
                                    *model_suggestions =
                                        apply_model_filter(all_model_suggestions, model_filter);
                                    *suggestion_index = 0;
                                    *suggestion_scroll = 0;
                                    *status = Some(format!(
                                        "Filtered to {} model suggestion(s)",
                                        model_suggestions.len()
                                    ));
                                    *filtering_models = false;
                                } else {
                                    match *selected {
                                        1 => *api_key = input.clone(),
                                        2 => *model = input.clone(),
                                        3 => *custom_base_url = input.clone(),
                                        _ => *custom_api_format = input.clone(),
                                    }
                                    if *selected == 3 || *selected == 4 {
                                        // Re-detect preset after manual edit
                                        *preset_index = detect_preset(custom_base_url);
                                        apply_api_key_to_env(
                                            provider,
                                            custom_api_format,
                                            api_key,
                                            preset_index
                                                .and_then(|i| CUSTOM_PRESETS.get(i))
                                                .map(|p| p.name),
                                        );
                                        self.model_fetch_rx = Some(spawn_model_fetch_bg(
                                            provider,
                                            custom_base_url.trim(),
                                            custom_api_format.trim(),
                                        ));
                                        *suggestion_index = 0;
                                        *suggestion_scroll = 0;
                                        *status =
                                            Some("Field updated; loading models...".to_string());
                                    } else {
                                        *status = Some("Field updated locally".to_string());
                                    }
                                }
                                *editing = false;
                                input.clear();
                            } else if *selected == 2 && !model_suggestions.is_empty() {
                                let suggestion = &model_suggestions[*suggestion_index];
                                if model != suggestion {
                                    *model = suggestion.clone();
                                    *status =
                                        Some(format!("Applied model suggestion: {suggestion}"));
                                } else {
                                    *editing = true;
                                    input.clone_from(model);
                                    *status = Some("Editing model field".to_string());
                                }
                            } else if *selected == 0 {
                                *provider = cycle_provider(provider, true);
                                if provider == "auto" {
                                    all_model_suggestions.clear();
                                    model_suggestions.clear();
                                    *suggestion_index = 0;
                                    *suggestion_scroll = 0;
                                    *status = Some("Switched provider to auto".to_string());
                                } else {
                                    apply_api_key_to_env(
                                        provider,
                                        custom_api_format,
                                        api_key,
                                        preset_index
                                            .and_then(|i| CUSTOM_PRESETS.get(i))
                                            .map(|p| p.name),
                                    );
                                    self.model_fetch_rx = Some(spawn_model_fetch_bg(
                                        provider,
                                        custom_base_url.trim(),
                                        custom_api_format.trim(),
                                    ));
                                    *suggestion_index = 0;
                                    *suggestion_scroll = 0;
                                    *status = Some(format!(
                                        "Switched provider to {}; loading models...",
                                        provider
                                    ));
                                }
                            } else {
                                // Auto-switch to custom provider when editing endpoint fields
                                if (*selected == 3 || *selected == 4) && provider != "custom" {
                                    *provider = "custom".to_string();
                                    *status = Some("Auto-switched to custom provider".to_string());
                                }
                                *editing = true;
                                match *selected {
                                    1 => input.clone_from(api_key),
                                    2 => input.clone_from(model),
                                    3 => input.clone_from(custom_base_url),
                                    _ => input.clone_from(custom_api_format),
                                }
                                if status.as_deref() != Some("Auto-switched to custom provider") {
                                    *status = Some("Editing field".to_string());
                                }
                            }
                            self.dirty = true;
                        }
                        KeyCode::Char('e') | KeyCode::Char('E') if !*editing => {
                            if *selected == 0 {
                                *status = Some("Provider uses ←/→ or Enter to switch".to_string());
                            } else {
                                // Auto-switch to custom provider when editing endpoint fields
                                if (*selected == 3 || *selected == 4) && provider != "custom" {
                                    *provider = "custom".to_string();
                                    *status = Some("Auto-switched to custom provider".to_string());
                                }
                                *editing = true;
                                match *selected {
                                    1 => input.clone_from(api_key),
                                    2 => input.clone_from(model),
                                    3 => input.clone_from(custom_base_url),
                                    _ => input.clone_from(custom_api_format),
                                }
                                if status.as_deref() != Some("Auto-switched to custom provider") {
                                    *status = Some("Editing field".to_string());
                                }
                            }
                            self.dirty = true;
                        }
                        KeyCode::Char('/') | KeyCode::Char('f') | KeyCode::Char('F')
                            if !*editing && *selected == 2 =>
                        {
                            *editing = true;
                            *filtering_models = true;
                            input.clone_from(model_filter);
                            *status = Some("Filtering model suggestions".to_string());
                            self.dirty = true;
                        }
                        KeyCode::Char('x') | KeyCode::Char('X') if !*editing => {
                            let cwd = std::env::current_dir()
                                .map(|p| p.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            let selected_tier = match *tier {
                                0 => SettingsTier::User,
                                1 => SettingsTier::Project,
                                _ => SettingsTier::Local,
                            };
                            match *selected {
                                0 => {
                                    let (value, source) = restore_setting_from_source(
                                        "model_provider",
                                        selected_tier,
                                        &cwd,
                                    );
                                    *provider = value;
                                    *provider_source = source;
                                    let (value, source) = restore_api_key_from_source(
                                        provider,
                                        custom_api_format,
                                        preset_index
                                            .and_then(|i| CUSTOM_PRESETS.get(i))
                                            .map(|p| p.name),
                                    );
                                    *api_key = value;
                                    *api_key_source = source;
                                }
                                1 => {
                                    let (value, source) = restore_api_key_from_source(
                                        provider,
                                        custom_api_format,
                                        preset_index
                                            .and_then(|i| CUSTOM_PRESETS.get(i))
                                            .map(|p| p.name),
                                    );
                                    *api_key = value;
                                    *api_key_source = source;
                                }
                                2 => {
                                    let (value, source) =
                                        restore_setting_from_source("model", selected_tier, &cwd);
                                    *model = value;
                                    *model_source = source;
                                    *model_filter = String::new();
                                    *model_suggestions =
                                        apply_model_filter(all_model_suggestions, model_filter);
                                    *suggestion_index = 0;
                                    *suggestion_scroll = 0;
                                }
                                3 => {
                                    let (value, source) = restore_setting_from_source(
                                        "custom_base_url",
                                        selected_tier,
                                        &cwd,
                                    );
                                    *custom_base_url = value;
                                    *custom_base_url_source = source;
                                }
                                _ => {
                                    let (value, source) = restore_setting_from_source(
                                        "custom_api_format",
                                        selected_tier,
                                        &cwd,
                                    );
                                    *custom_api_format = value;
                                    *custom_api_format_source = source;
                                }
                            }
                            if *selected == 0 || *selected >= 3 {
                                // Re-detect preset after field reset
                                *preset_index = if provider == "custom" {
                                    detect_preset(custom_base_url)
                                } else {
                                    None
                                };
                                if provider == "auto" {
                                    all_model_suggestions.clear();
                                    model_suggestions.clear();
                                    *suggestion_index = 0;
                                    *suggestion_scroll = 0;
                                } else {
                                    apply_api_key_to_env(
                                        provider,
                                        custom_api_format,
                                        api_key,
                                        preset_index
                                            .and_then(|i| CUSTOM_PRESETS.get(i))
                                            .map(|p| p.name),
                                    );
                                    self.model_fetch_rx = Some(spawn_model_fetch_bg(
                                        provider,
                                        custom_base_url.trim(),
                                        custom_api_format.trim(),
                                    ));
                                    *suggestion_index = 0;
                                    *suggestion_scroll = 0;
                                }
                            }
                            *status =
                                Some("Reverted current field to inherited source".to_string());
                            self.dirty = true;
                        }
                        KeyCode::Char('s') | KeyCode::Char('S') if !*editing => {
                            let cwd = std::env::current_dir()
                                .map(|p| p.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            let tier_value = match *tier {
                                0 => SettingsTier::User,
                                1 => SettingsTier::Project,
                                _ => SettingsTier::Local,
                            };
                            let persist_error = [
                                Settings::persist_key_value(
                                    "model_provider",
                                    Some(provider.trim()),
                                    tier_value,
                                    &cwd,
                                ),
                                Settings::persist_key_value(
                                    "model",
                                    if model.trim().is_empty() {
                                        None
                                    } else {
                                        Some(model.trim())
                                    },
                                    tier_value,
                                    &cwd,
                                ),
                                Settings::persist_key_value(
                                    "custom_base_url",
                                    if custom_base_url.trim().is_empty() {
                                        None
                                    } else {
                                        Some(custom_base_url.trim())
                                    },
                                    tier_value,
                                    &cwd,
                                ),
                                Settings::persist_key_value(
                                    "custom_api_format",
                                    if custom_api_format.trim().is_empty() {
                                        None
                                    } else {
                                        Some(custom_api_format.trim())
                                    },
                                    tier_value,
                                    &cwd,
                                ),
                                Settings::persist_key_value(
                                    "custom_preset",
                                    preset_index
                                        .and_then(|i| CUSTOM_PRESETS.get(i))
                                        .map(|p| p.name),
                                    tier_value,
                                    &cwd,
                                ),
                            ]
                            .into_iter()
                            .find_map(Result::err);
                            match persist_error {
                                None => {
                                    let (slot, env_var) = provider_key_slot(
                                        provider.as_str(),
                                        custom_api_format.as_str(),
                                        preset_index
                                            .and_then(|i| CUSTOM_PRESETS.get(i))
                                            .map(|p| p.name),
                                    );
                                    if api_key.trim().is_empty() {
                                        let mut store = load_credential_store();
                                        let cred_path = nocode_core::storage::credentials::CredentialStore::default_path();
                                        store.remove_key(slot);
                                        let _ = store.save(&cred_path);
                                        unsafe {
                                            std::env::remove_var(env_var);
                                        }
                                    } else {
                                        let mut store = load_credential_store();
                                        let cred_path = nocode_core::storage::credentials::CredentialStore::default_path();
                                        store.set_key(slot, api_key.trim());
                                        store
                                            .save(&cred_path)
                                            .map_err(|e| format!("Failed to save credentials: {e}"))
                                            .ok();
                                        unsafe {
                                            std::env::set_var(env_var, api_key.trim());
                                        }
                                    }
                                    let user_settings =
                                        Settings::load_tier(SettingsTier::User, &cwd);
                                    let project_settings =
                                        Settings::load_tier(SettingsTier::Project, &cwd);
                                    let local_settings =
                                        Settings::load_tier(SettingsTier::Local, &cwd);
                                    let provider_name = provider_display_name(provider);
                                    if provider == "custom" {
                                        unsafe {
                                            std::env::set_var("NOCODE_MODEL_PROVIDER", "custom");
                                        }
                                    } else if provider == "auto" {
                                        unsafe {
                                            std::env::remove_var("NOCODE_MODEL_PROVIDER");
                                        }
                                    } else {
                                        unsafe {
                                            std::env::set_var("NOCODE_MODEL_PROVIDER", &**provider);
                                        }
                                    }
                                    if model.trim().is_empty() {
                                        unsafe {
                                            std::env::remove_var("NOCODE_MODEL");
                                        }
                                    } else {
                                        unsafe {
                                            std::env::set_var("NOCODE_MODEL", model.trim());
                                        }
                                        self.hud.model_name = model.trim().to_string();
                                    }
                                    if custom_base_url.trim().is_empty() {
                                        unsafe {
                                            std::env::remove_var("NOCODE_CUSTOM_BASE_URL");
                                            std::env::remove_var("NOCODE_CUSTOM_API_FORMAT");
                                        }
                                    } else {
                                        unsafe {
                                            std::env::set_var(
                                                "NOCODE_CUSTOM_BASE_URL",
                                                custom_base_url.trim(),
                                            );
                                            std::env::set_var(
                                                "NOCODE_CUSTOM_API_FORMAT",
                                                custom_api_format.trim(),
                                            );
                                        }
                                    }
                                    *status = Some(format!(
                                        "Saved configuration; current provider {provider_name}"
                                    ));
                                    *provider_source = setting_source_label(
                                        "model_provider",
                                        "NOCODE_MODEL_PROVIDER",
                                        &user_settings,
                                        &project_settings,
                                        &local_settings,
                                        (!custom_base_url.is_empty()
                                            || !custom_api_format.is_empty())
                                        .then_some("derived"),
                                    );
                                    *model_source = setting_source_label(
                                        "model",
                                        "NOCODE_MODEL",
                                        &user_settings,
                                        &project_settings,
                                        &local_settings,
                                        None,
                                    );
                                    *custom_base_url_source = setting_source_label(
                                        "custom_base_url",
                                        "NOCODE_CUSTOM_BASE_URL",
                                        &user_settings,
                                        &project_settings,
                                        &local_settings,
                                        None,
                                    );
                                    *custom_api_format_source = setting_source_label(
                                        "custom_api_format",
                                        "NOCODE_CUSTOM_API_FORMAT",
                                        &user_settings,
                                        &project_settings,
                                        &local_settings,
                                        Some("default"),
                                    );
                                    *api_key_source = if std::env::var(env_var).is_ok() {
                                        "env".to_string()
                                    } else if !api_key.trim().is_empty() {
                                        "credentials".to_string()
                                    } else {
                                        "unset".to_string()
                                    };
                                    system_notice = Some(format!(
                                        "Config applied to current session from {} settings: provider {}, model {}",
                                        tier_value.label(),
                                        provider_name,
                                        if model.trim().is_empty() {
                                            "(inherit)".to_string()
                                        } else {
                                            model.trim().to_string()
                                        }
                                    ));
                                    // Auto-test connection after save
                                    let mut test_settings = Settings::load_merged(&cwd);
                                    test_settings.model_provider = Some(provider.clone());
                                    test_settings.custom_base_url =
                                        if custom_base_url.trim().is_empty() {
                                            None
                                        } else {
                                            Some(custom_base_url.trim().to_string())
                                        };
                                    test_settings.custom_api_format =
                                        if custom_api_format.trim().is_empty() {
                                            None
                                        } else {
                                            Some(custom_api_format.trim().to_string())
                                        };
                                    let test_provider_type =
                                        crate::resolve_provider(&test_settings);
                                    let (test_impl, _) =
                                        crate::build_provider(&test_provider_type, &test_settings);
                                    let test_result = match test_impl.verify_key() {
                                        Ok(msg) => format!("Saved + Connection OK: {msg}"),
                                        Err(err) => format!("Saved, but connection failed: {err}"),
                                    };
                                    *status = Some(test_result);
                                }
                                Some(e) => {
                                    *status = Some(format!("Save failed: {e}"));
                                }
                            }
                            self.dirty = true;
                        }
                        KeyCode::Char('t') | KeyCode::Char('T') if !*editing => {
                            let cwd = std::env::current_dir()
                                .map(|p| p.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            let mut settings = Settings::load_merged(&cwd);
                            settings.model_provider = Some(provider.clone());
                            settings.custom_base_url = if custom_base_url.trim().is_empty() {
                                None
                            } else {
                                Some(custom_base_url.trim().to_string())
                            };
                            settings.custom_api_format = if custom_api_format.trim().is_empty() {
                                None
                            } else {
                                Some(custom_api_format.trim().to_string())
                            };
                            if !api_key.trim().is_empty() {
                                let (_, env_var) = provider_key_slot(
                                    provider.as_str(),
                                    custom_api_format.as_str(),
                                    preset_index
                                        .and_then(|i| CUSTOM_PRESETS.get(i))
                                        .map(|p| p.name),
                                );
                                unsafe {
                                    std::env::set_var(env_var, api_key.trim());
                                }
                            }
                            let provider_type = crate::resolve_provider(&settings);
                            let (provider_impl, _) =
                                crate::build_provider(&provider_type, &settings);
                            *status = Some(match provider_impl.verify_key() {
                                Ok(msg) => format!("Connection OK: {msg}"),
                                Err(err) => format!("Connection failed: {err}"),
                            });
                            self.dirty = true;
                        }
                        KeyCode::Char('r') | KeyCode::Char('R') if !*editing => {
                            apply_api_key_to_env(
                                provider,
                                custom_api_format,
                                api_key,
                                preset_index
                                    .and_then(|i| CUSTOM_PRESETS.get(i))
                                    .map(|p| p.name),
                            );
                            self.model_fetch_rx = Some(spawn_model_fetch_bg(
                                provider,
                                custom_base_url.trim(),
                                custom_api_format.trim(),
                            ));
                            *suggestion_index = 0;
                            *suggestion_scroll = 0;
                            *status = Some("Loading models...".to_string());
                            self.dirty = true;
                        }
                        KeyCode::Char('p') | KeyCode::Char('P')
                            if !*editing && provider == "custom" =>
                        {
                            // Cycle through presets: None → 0 → 1 → ... → N-1 → None
                            let next = match *preset_index {
                                None => Some(0),
                                Some(i) if i + 1 < CUSTOM_PRESETS.len() => Some(i + 1),
                                Some(_) => None,
                            };
                            *preset_index = next;
                            if let Some(idx) = next {
                                let preset = &CUSTOM_PRESETS[idx];
                                *custom_base_url = preset.base_url.to_string();
                                *custom_api_format = preset.api_format.to_string();
                                *custom_base_url_source = "preset".to_string();
                                *custom_api_format_source = "preset".to_string();
                                // Load preset-specific API key
                                if !preset.env_key_name.is_empty() {
                                    let store = load_credential_store();
                                    *api_key = std::env::var(preset.env_key_name)
                                        .ok()
                                        .or_else(|| store.get_key(preset.credential_slot))
                                        .unwrap_or_default();
                                    *api_key_source = if std::env::var(preset.env_key_name).is_ok()
                                    {
                                        "env".to_string()
                                    } else if load_credential_store()
                                        .get_key(preset.credential_slot)
                                        .is_some()
                                    {
                                        "credentials".to_string()
                                    } else {
                                        "unset".to_string()
                                    };
                                } else {
                                    *api_key = String::new();
                                    *api_key_source = "not required".to_string();
                                }
                                // Auto-fill default model if empty
                                if model.is_empty() && !preset.default_model.is_empty() {
                                    *model = preset.default_model.to_string();
                                }
                                *status = Some(format!(
                                    "Applied {} preset: {} ({})",
                                    preset.name, preset.base_url, preset.api_format
                                ));
                            } else {
                                *custom_base_url = String::new();
                                *custom_api_format = "openai".to_string();
                                *custom_base_url_source = "unset".to_string();
                                *custom_api_format_source = "default".to_string();
                                *status =
                                    Some("Switched to manual endpoint configuration".to_string());
                            }
                            // Refresh model suggestions for new endpoint
                            apply_api_key_to_env(
                                provider,
                                custom_api_format,
                                api_key,
                                preset_index
                                    .and_then(|i| CUSTOM_PRESETS.get(i))
                                    .map(|p| p.name),
                            );
                            self.model_fetch_rx = Some(spawn_model_fetch_bg(
                                provider,
                                custom_base_url.trim(),
                                custom_api_format.trim(),
                            ));
                            *suggestion_index = 0;
                            *suggestion_scroll = 0;
                            self.dirty = true;
                        }
                        KeyCode::Char(c)
                            if !*editing
                                && *selected == 2
                                && ('1'..='8').contains(&c)
                                && !model_suggestions.is_empty() =>
                        {
                            let idx = (c as usize) - ('1' as usize);
                            if idx < model_suggestions.len().min(8) {
                                let actual_idx = *suggestion_scroll + idx;
                                if actual_idx < model_suggestions.len() {
                                    *suggestion_index = actual_idx;
                                    *model = model_suggestions[actual_idx].clone();
                                }
                                *status = Some(format!("Applied model suggestion: {}", model));
                            }
                            self.dirty = true;
                        }
                        KeyCode::Backspace if *editing => {
                            input.pop();
                            self.dirty = true;
                        }
                        KeyCode::Char(c) if *editing => {
                            input.push(c);
                            self.dirty = true;
                        }
                        _ => {}
                    }
                }
                if let Some(notice) = system_notice {
                    self.push_system(&notice);
                }
                return HandleKeyResult::Continue;
            }
            // Permission overlay: y/n/a to respond
            if matches!(self.overlay, Overlay::Permission { .. }) {
                use nocode_core::tool::permission::PermissionDecision;
                let decision = match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => Some(PermissionDecision::Allow),
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        Some(PermissionDecision::Deny)
                    }
                    KeyCode::Char('a') | KeyCode::Char('A') => {
                        Some(PermissionDecision::AlwaysAllow)
                    }
                    _ => None,
                };
                if let Some(d) = decision {
                    if let Some(tx) = self.permission_tx.take() {
                        let _ = tx.send(d);
                    }
                    self.overlay = Overlay::None;
                    self.dirty = true;
                }
                return HandleKeyResult::Continue;
            }
            // Question overlay: navigate options and confirm
            if matches!(self.overlay, Overlay::Question { .. }) {
                if let Overlay::Question {
                    ref questions,
                    ref mut selected,
                } = self.overlay
                {
                    match key.code {
                        KeyCode::Up if !selected.is_empty() => {
                            // Move to previous question
                        }
                        KeyCode::Down if !selected.is_empty() => {
                            // Move to next question
                        }
                        KeyCode::Left => {
                            if let Some(cur) = selected.first_mut()
                                && *cur > 0
                            {
                                *cur -= 1;
                            }
                        }
                        KeyCode::Right => {
                            if let (Some(cur), Some(arr)) =
                                (selected.first_mut(), questions.as_array())
                                && let Some(q) = arr.first()
                            {
                                let opt_count =
                                    q["options"].as_array().map(|a| a.len()).unwrap_or(1);
                                if *cur + 1 < opt_count {
                                    *cur += 1;
                                }
                            }
                        }
                        KeyCode::Char('\n') | KeyCode::Enter => {
                            // Build answer from selected options
                            let selections: Vec<String> = if let Some(arr) = questions.as_array() {
                                arr.iter()
                                    .enumerate()
                                    .map(|(i, q)| {
                                        let idx = selected.get(i).copied().unwrap_or(0);
                                        q["options"]
                                            .as_array()
                                            .and_then(|opts| opts.get(idx))
                                            .and_then(|o| o["label"].as_str())
                                            .unwrap_or("N/A")
                                            .to_string()
                                    })
                                    .collect()
                            } else {
                                Vec::new()
                            };
                            let answer = nocode_core::tool::permission::UserAnswer { selections };
                            if let Some(tx) = self.question_tx.take() {
                                let _ = tx.send(Ok(answer));
                            }
                            self.overlay = Overlay::None;
                            self.dirty = true;
                        }
                        KeyCode::Esc => {
                            if let Some(tx) = self.question_tx.take() {
                                let _ = tx.send(Err("Cancelled by user".to_string()));
                            }
                            self.overlay = Overlay::None;
                            self.dirty = true;
                        }
                        // Number keys 1-4 for quick option selection
                        KeyCode::Char(c) if ('1'..='4').contains(&c) => {
                            let idx = (c as usize) - ('1' as usize);
                            if let Some(cur) = selected.first_mut() {
                                *cur = idx;
                            }
                        }
                        _ => {}
                    }
                }
                self.dirty = true;
                return HandleKeyResult::Continue;
            }
            // Other overlays: Esc closes, Up/Down/PageUp/PageDown scrolls
            match key.code {
                KeyCode::Esc => {
                    self.overlay = Overlay::None;
                    self.overlay_scroll = 0;
                    self.dirty = true;
                }
                KeyCode::Up => {
                    self.overlay_scroll = self.overlay_scroll.saturating_add(1);
                    self.dirty = true;
                }
                KeyCode::Down => {
                    self.overlay_scroll = self.overlay_scroll.saturating_sub(1);
                    self.dirty = true;
                }
                KeyCode::PageUp => {
                    self.overlay_scroll = self.overlay_scroll.saturating_add(10);
                    self.dirty = true;
                }
                KeyCode::PageDown => {
                    self.overlay_scroll = self.overlay_scroll.saturating_sub(10);
                    self.dirty = true;
                }
                _ => {}
            }
            return HandleKeyResult::Continue;
        }

        // Search mode — intercept keys for search input
        if self.search_active {
            match key.code {
                KeyCode::Esc => {
                    self.search_active = false;
                    self.search_query.clear();
                    self.search_matches.clear();
                    self.dirty = true;
                }
                KeyCode::Enter
                    // Jump to next match
                    if !self.search_matches.is_empty() => {
                        self.search_index = (self.search_index + 1) % self.search_matches.len();
                        let msg_idx = self.search_matches[self.search_index];
                        self.scroll_to_message(msg_idx);
                        self.dirty = true;
                    }
                KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL)
                    && !self.search_matches.is_empty() => {
                        self.search_index = (self.search_index + 1) % self.search_matches.len();
                        let msg_idx = self.search_matches[self.search_index];
                        self.scroll_to_message(msg_idx);
                        self.dirty = true;
                    }
                KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL)
                    && !self.search_matches.is_empty() => {
                        self.search_index = if self.search_index == 0 {
                            self.search_matches.len() - 1
                        } else {
                            self.search_index - 1
                        };
                        let msg_idx = self.search_matches[self.search_index];
                        self.scroll_to_message(msg_idx);
                        self.dirty = true;
                    }
                KeyCode::Backspace => {
                    self.search_query.pop();
                    self.update_search_matches();
                    self.dirty = true;
                }
                KeyCode::Char(c) => {
                    self.search_query.push(c);
                    self.update_search_matches();
                    self.dirty = true;
                }
                _ => {}
            }
            return HandleKeyResult::Continue;
        }

        // Command completion intercept — when popup is active
        if self.completion_selected.is_some() {
            match (key.code, key.modifiers) {
                (KeyCode::Up, KeyModifiers::NONE) => {
                    if let Some(sel) = self.completion_selected.as_mut() {
                        *sel = sel.saturating_sub(1);
                    }
                    self.dirty = true;
                    return HandleKeyResult::Continue;
                }
                (KeyCode::Down, KeyModifiers::NONE) => {
                    let count = self.completion_suggestions().len();
                    if let Some(sel) = self.completion_selected.as_mut()
                        && *sel + 1 < count
                    {
                        *sel += 1;
                    }
                    self.dirty = true;
                    return HandleKeyResult::Continue;
                }
                (KeyCode::Tab, _) | (KeyCode::Enter, KeyModifiers::NONE) => {
                    // Apply selected completion
                    let suggestions = self.completion_suggestions();
                    if let Some(sel) = self.completion_selected
                        && let Some((label, _)) = suggestions.get(sel)
                    {
                        // Extract command name (e.g. "/help" from "/help [topic]")
                        let cmd = label.split_whitespace().next().unwrap_or(label);
                        self.input = cmd.to_string();
                        self.cursor_pos = self.input.len();
                    }
                    self.completion_selected = None;
                    self.dirty = true;
                    return HandleKeyResult::Continue;
                }
                (KeyCode::Esc, _) => {
                    self.completion_selected = None;
                    self.dirty = true;
                    return HandleKeyResult::Continue;
                }
                _ => {
                    // Fall through to normal handling, but clear completion
                    // if it's not a character key (completion will be re-evaluated)
                }
            }
        }

        match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => {
                if self.input_mode == InputMode::Insert {
                    self.input_mode = InputMode::Normal;
                    self.vim_pending = None;
                    if self.cursor_pos > 0 && self.cursor_pos >= self.input.len() {
                        self.cursor_pos = prev_char_boundary(&self.input, self.input.len());
                    }
                    self.dirty = true;
                } else if !self.input.is_empty() {
                    self.input.clear();
                    self.cursor_pos = 0;
                    self.input_view_offset = 0;
                    self.input_scroll_y = 0;
                    self.input_mode = InputMode::Insert;
                    self.update_completion();
                    self.dirty = true;
                } else if !self.pending_images.is_empty() {
                    let count = self.pending_images.len();
                    self.pending_images.clear();
                    self.push_system(&format!("Cleared {count} pending image(s)."));
                    self.dirty = true;
                }
            }
            // Scroll
            (KeyCode::Up, KeyModifiers::NONE) => {
                self.chat_scroll = self.chat_scroll.saturating_add(1);
                self.sticky_scroll = false;
                self.dirty = true;
            }
            (KeyCode::Down, KeyModifiers::NONE) => {
                if self.chat_scroll > 0 {
                    self.chat_scroll = self.chat_scroll.saturating_sub(1);
                }
                if self.chat_scroll == 0 {
                    self.sticky_scroll = true;
                    self.unseen_count = 0;
                }
                self.dirty = true;
            }
            (KeyCode::PageUp, _) => {
                self.chat_scroll = self.chat_scroll.saturating_add(10);
                self.sticky_scroll = false;
                self.dirty = true;
            }
            (KeyCode::PageDown, _) => {
                self.chat_scroll = self.chat_scroll.saturating_sub(10);
                if self.chat_scroll == 0 {
                    self.sticky_scroll = true;
                    self.unseen_count = 0;
                }
                self.dirty = true;
            }
            // Clear chat
            (KeyCode::Char('l'), KeyModifiers::CONTROL) => {
                self.chat_messages.clear();
                self.invalidate_height_cache();
                self.sticky_scroll = true;
                self.unseen_count = 0;
                self.chat_scroll = 0;
                self.dirty = true;
            }
            // Clear input
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                self.input.clear();
                self.cursor_pos = 0;
                self.input_view_offset = 0;
                self.input_scroll_y = 0;
                self.update_completion();
                self.dirty = true;
            }
            // Jump to start of line
            (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
                // Find start of current line
                let before = &self.input[..self.cursor_pos];
                self.cursor_pos = before.rfind('\n').map_or(0, |p| p + 1);
                self.dirty = true;
            }
            // Jump to end of line
            (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
                let after = &self.input[self.cursor_pos..];
                self.cursor_pos += after.find('\n').unwrap_or(after.len());
                self.dirty = true;
            }
            // Delete word backward
            (KeyCode::Char('w'), KeyModifiers::CONTROL) if self.cursor_pos > 0 => {
                let before = &self.input[..self.cursor_pos];
                // Skip trailing whitespace, then skip word chars
                let trimmed = before.trim_end();
                let word_start = trimmed
                    .rfind(|c: char| c.is_whitespace() || c == '/')
                    .map_or(0, |p| p + 1);
                self.input.drain(word_start..self.cursor_pos);
                self.cursor_pos = word_start;
                self.update_completion();
                self.dirty = true;
            }
            // Delete to end of line
            (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
                let after = &self.input[self.cursor_pos..];
                let end = self.cursor_pos + after.find('\n').unwrap_or(after.len());
                self.input.drain(self.cursor_pos..end);
                self.dirty = true;
            }
            // Theme toggle
            (KeyCode::Char('t'), KeyModifiers::CONTROL) => {
                let variant = crate::tui_theme::toggle_theme();
                self.push_system(&format!("theme: {variant:?}"));
                self.invalidate_height_cache();
            }
            // Thinking toggle
            (KeyCode::Char('o'), KeyModifiers::CONTROL) => {
                self.toggle_thinking_blocks();
            }
            // Copy last assistant message to clipboard
            (KeyCode::Char('y'), KeyModifiers::CONTROL) => {
                self.copy_last_assistant_to_clipboard();
            }
            // Paste image from clipboard (text paste handled by Event::Paste)
            (KeyCode::Char('v'), KeyModifiers::CONTROL) => {
                let _ = self.try_paste_image();
            }
            // Search toggle
            (KeyCode::Char('f'), KeyModifiers::CONTROL) => {
                if self.search_active {
                    self.search_active = false;
                    self.search_query.clear();
                    self.search_matches.clear();
                } else {
                    self.search_active = true;
                    self.search_query.clear();
                    self.search_matches.clear();
                    self.search_index = 0;
                }
                self.dirty = true;
            }
            // Error log overlay
            (KeyCode::F(4), _) => {
                if self.error_log.is_empty() {
                    self.push_system("No errors logged.");
                } else {
                    self.overlay = Overlay::Errors(self.error_log.clone());
                    self.overlay_scroll = 0;
                }
                self.dirty = true;
            }
            // History prev
            (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
                self.history_prev();
            }
            // History next
            (KeyCode::Char('n'), KeyModifiers::CONTROL) => {
                self.history_next();
            }
            // Submit
            (KeyCode::Enter, KeyModifiers::NONE) => {
                if self.input_mode == InputMode::Normal {
                    self.input_mode = InputMode::Insert;
                    self.dirty = true;
                    return HandleKeyResult::Continue;
                }
                let text = self.input.trim().to_string();
                if text.is_empty() {
                    return HandleKeyResult::Continue;
                }
                self.input_history.push(text.clone());
                self.save_history_entry(&text);
                self.history_index = None;
                self.input.clear();
                self.cursor_pos = 0;
                self.input_view_offset = 0;
                self.input_scroll_y = 0;
                self.dirty = true;
                return HandleKeyResult::Submit(text);
            }
            // Newline
            (KeyCode::Enter, KeyModifiers::SHIFT) => {
                self.input.insert(self.cursor_pos, '\n');
                self.cursor_pos += 1;
                self.dirty = true;
            }
            // Backspace
            (KeyCode::Backspace, _) if self.cursor_pos > 0 => {
                let prev = prev_char_boundary(&self.input, self.cursor_pos);
                self.input.drain(prev..self.cursor_pos);
                self.cursor_pos = prev;
                self.update_completion();
                self.dirty = true;
            }
            // Delete
            (KeyCode::Delete, _) if self.cursor_pos < self.input.len() => {
                let next = next_char_boundary(&self.input, self.cursor_pos);
                self.input.drain(self.cursor_pos..next);
                self.update_completion();
                self.dirty = true;
            }
            // Left/Right
            (KeyCode::Left, _) if self.cursor_pos > 0 => {
                self.cursor_pos = prev_char_boundary(&self.input, self.cursor_pos);
                self.dirty = true;
            }
            (KeyCode::Right, _) if self.cursor_pos < self.input.len() => {
                self.cursor_pos = next_char_boundary(&self.input, self.cursor_pos);
                self.dirty = true;
            }
            // Home/End
            (KeyCode::Home, _) => {
                self.cursor_pos = 0;
                self.dirty = true;
            }
            (KeyCode::End, _) => {
                self.cursor_pos = self.input.len();
                self.dirty = true;
            }
            // Normal char input
            (KeyCode::Char(c), _) => {
                if self.input_mode == InputMode::Normal {
                    self.handle_vim_key(c);
                } else {
                    self.input.insert(self.cursor_pos, c);
                    self.cursor_pos += c.len_utf8();
                    self.update_completion();
                    self.dirty = true;
                }
            }
            _ => {}
        }
        HandleKeyResult::Continue
    }

    fn handle_vim_key(&mut self, c: char) {
        match c {
            'i' => {
                self.input_mode = InputMode::Insert;
                self.dirty = true;
            }
            'a' => {
                self.input_mode = InputMode::Insert;
                if self.cursor_pos < self.input.len() {
                    self.cursor_pos = next_char_boundary(&self.input, self.cursor_pos);
                }
                self.dirty = true;
            }
            'A' => {
                self.input_mode = InputMode::Insert;
                self.cursor_pos = self.input.len();
                self.dirty = true;
            }
            'I' => {
                self.input_mode = InputMode::Insert;
                self.cursor_pos = 0;
                self.dirty = true;
            }
            'h' if self.cursor_pos > 0 => {
                self.cursor_pos = prev_char_boundary(&self.input, self.cursor_pos);
                self.dirty = true;
            }
            'l' if self.cursor_pos < self.input.len() => {
                self.cursor_pos = next_char_boundary(&self.input, self.cursor_pos);
                self.dirty = true;
            }
            'x' if self.cursor_pos < self.input.len() => {
                let next = next_char_boundary(&self.input, self.cursor_pos);
                self.input.drain(self.cursor_pos..next);
                self.dirty = true;
            }
            '0' => {
                self.cursor_pos = 0;
                self.dirty = true;
            }
            '$' => {
                self.cursor_pos = self.input.len();
                if self.cursor_pos > 0 {
                    self.cursor_pos = prev_char_boundary(&self.input, self.cursor_pos);
                }
                self.dirty = true;
            }
            'w' => {
                self.cursor_pos = next_word_boundary(&self.input, self.cursor_pos);
                self.dirty = true;
            }
            'b' => {
                self.cursor_pos = prev_word_boundary(&self.input, self.cursor_pos);
                self.dirty = true;
            }
            _ => {}
        }
    }

    fn toggle_thinking_blocks(&mut self) {
        for msg in &mut self.chat_messages {
            if msg.kind == ChatMessageKind::Thinking {
                msg.thinking_collapsed = !msg.thinking_collapsed;
            }
            if (msg.kind == ChatMessageKind::Tool || msg.kind == ChatMessageKind::Error)
                && let Some(info) = &mut msg.tool_info
            {
                info.collapsed = !info.collapsed;
            }
        }
        self.invalidate_height_cache();
        self.dirty = true;
    }

    pub(crate) fn copy_last_assistant_to_clipboard(&mut self) {
        // Find last assistant message
        let text = self
            .chat_messages
            .iter()
            .rev()
            .find(|m| m.kind == ChatMessageKind::Assistant)
            .map(|m| {
                m.lines
                    .iter()
                    .map(|l| {
                        l.segments
                            .iter()
                            .map(|s| s.text.as_str())
                            .collect::<String>()
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            });

        let Some(text) = text else {
            self.push_system("No assistant message to copy.");
            return;
        };

        match copy_to_clipboard(&text) {
            Ok(()) => self.push_system("Copied to clipboard."),
            Err(e) => self.push_error(&format!("Clipboard: {e}")),
        }
    }

    fn update_search_matches(&mut self) {
        self.search_matches.clear();
        if self.search_query.is_empty() {
            return;
        }
        let query = self.search_query.to_lowercase();
        for (i, msg) in self.chat_messages.iter().enumerate() {
            let text: String = msg
                .lines
                .iter()
                .flat_map(|l| l.segments.iter().map(|s| s.text.as_str()))
                .collect();
            if text.to_lowercase().contains(&query) {
                self.search_matches.push(i);
            }
        }
        self.search_index = 0;
        // Auto-scroll to first match
        if let Some(&msg_idx) = self.search_matches.first() {
            self.scroll_to_message(msg_idx);
        }
    }

    /// Scroll chat so that the message at `msg_idx` is visible.
    fn scroll_to_message(&mut self, msg_idx: usize) {
        if msg_idx >= self.height_cache.len() {
            return;
        }
        // Calculate the row offset of this message from the bottom
        let total = self.total_content_height();
        let msg_top: u16 = self.height_cache[..msg_idx].iter().copied().sum();
        let msg_bottom: u16 = msg_top + self.height_cache[msg_idx];
        // scroll is measured from bottom (0 = at bottom)
        let scroll_to_show = total.saturating_sub(msg_bottom);
        self.chat_scroll = scroll_to_show;
        self.sticky_scroll = scroll_to_show == 0;
        self.dirty = true;
    }

    /// Get the search status line for display.
    pub(crate) fn search_status(&self) -> Option<String> {
        if !self.search_active {
            return None;
        }
        let count = self.search_matches.len();
        let idx = if count > 0 { self.search_index + 1 } else { 0 };
        Some(format!("Search: {} ({}/{})", self.search_query, idx, count))
    }

    pub(crate) fn slash_command_hint(&self) -> Option<String> {
        if !self.input.trim_start().starts_with('/') {
            return None;
        }
        let registry = CommandRegistry::with_defaults();
        let suggestions = registry.recommend(&self.input, 10);
        if suggestions.is_empty() {
            return Some("Commands: no matches".to_string());
        }
        // When completion popup is active, show selected command in status bar
        if let Some(sel) = self.completion_selected
            && let Some(cmd) = suggestions.get(sel)
        {
            return Some(match cmd.argument_hint {
                Some(hint) => format!("/{} {} — {}", cmd.name, hint, cmd.summary),
                None => format!("/{} — {}", cmd.name, cmd.summary),
            });
        }
        let parts: Vec<String> = suggestions
            .iter()
            .take(4)
            .map(|cmd| match cmd.argument_hint {
                Some(hint) => format!("/{} {}", cmd.name, hint),
                None => format!("/{}", cmd.name),
            })
            .collect();
        Some(format!("Commands: {}", parts.join("  ·  ")))
    }

    /// Get completion suggestions for the current input.
    pub(crate) fn completion_suggestions(&self) -> Vec<(String, String)> {
        if !self.input.trim_start().starts_with('/') {
            return Vec::new();
        }
        let registry = CommandRegistry::with_defaults();
        registry
            .recommend(&self.input, 10)
            .into_iter()
            .map(|cmd| {
                let label = match cmd.argument_hint {
                    Some(hint) => format!("/{} {}", cmd.name, hint),
                    None => format!("/{}", cmd.name),
                };
                (label, cmd.summary.to_string())
            })
            .collect()
    }

    /// Update completion state after input changes.
    fn update_completion(&mut self) {
        if self.input.trim_start().starts_with('/') && !self.input.contains(' ') {
            // Activate completion, reset selection to 0 if we have suggestions
            let has_suggestions = {
                let registry = CommandRegistry::with_defaults();
                !registry.recommend(&self.input, 1).is_empty()
            };
            self.completion_selected = if has_suggestions { Some(0) } else { None };
        } else {
            self.completion_selected = None;
        }
    }

    fn history_prev(&mut self) {
        if self.input_history.is_empty() {
            return;
        }
        match self.history_index {
            None => {
                self.saved_input = self.input.clone();
                self.history_index = Some(0);
                self.input
                    .clone_from(&self.input_history[self.input_history.len() - 1]);
                self.cursor_pos = self.input.len();
                self.dirty = true;
            }
            Some(idx) if idx + 1 < self.input_history.len() => {
                self.history_index = Some(idx + 1);
                let hist_idx = self.input_history.len() - 1 - (idx + 1);
                self.input.clone_from(&self.input_history[hist_idx]);
                self.cursor_pos = self.input.len();
                self.dirty = true;
            }
            _ => {}
        }
    }

    fn history_next(&mut self) {
        match self.history_index {
            Some(0) => {
                self.history_index = None;
                self.input = std::mem::take(&mut self.saved_input);
                self.cursor_pos = self.input.len();
                self.dirty = true;
            }
            Some(idx) => {
                self.history_index = Some(idx - 1);
                let hist_idx = self.input_history.len() - idx;
                self.input.clone_from(&self.input_history[hist_idx]);
                self.cursor_pos = self.input.len();
                self.dirty = true;
            }
            None => {}
        }
    }

    /// Load input history from persistent file.
    pub(crate) fn load_input_history(&mut self) {
        let Some(path) = input_history_path() else {
            return;
        };
        if let Ok(content) = std::fs::read_to_string(&path) {
            self.input_history = content
                .lines()
                .filter(|l| !l.is_empty())
                .map(|l| l.replace("\\n", "\n"))
                .collect();
            // Cap at 500 entries
            if self.input_history.len() > 500 {
                let start = self.input_history.len() - 500;
                self.input_history = self.input_history.split_off(start);
            }
        }
    }

    /// Append a single entry to the persistent history file.
    pub(crate) fn save_history_entry(&self, entry: &str) {
        let Some(path) = input_history_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        else {
            return;
        };
        use std::io::Write;
        // Escape newlines for multi-line inputs
        let escaped = entry.replace('\n', "\\n");
        let _ = writeln!(file, "{escaped}");
    }

    fn handle_paste(&mut self, text: &str) {
        if let Overlay::Config(ref mut cs) = self.overlay
            && cs.editing
        {
            cs.input.push_str(text);
            cs.status = Some("Pasted into config field".to_string());
            self.dirty = true;
            return;
        }
        self.input.insert_str(self.cursor_pos, text);
        self.cursor_pos += text.len();
        self.update_completion();
        self.dirty = true;
    }

    /// Try to read an image from the system clipboard and add it to pending_images.
    fn try_paste_image(&mut self) -> bool {
        const MAX_IMAGES: usize = 10;
        const MAX_DIMENSION: usize = 4096;
        const MAX_BYTES: usize = 20 * 1024 * 1024; // 20MB

        // Check if current model supports vision
        let caps = nocode_core::provider::model_caps::lookup(&self.hud.model_name);
        if !caps.supports_vision {
            self.push_system(&format!(
                "Model '{}' does not support images.",
                self.hud.model_name
            ));
            self.dirty = true;
            return true;
        }

        let Ok(mut clipboard) = arboard::Clipboard::new() else {
            return false;
        };
        let Ok(img) = clipboard.get_image() else {
            return false;
        };

        // Validate limits
        if self.pending_images.len() >= MAX_IMAGES {
            self.push_system(&format!(
                "Cannot paste: already {MAX_IMAGES} images attached. Submit or clear first."
            ));
            self.dirty = true;
            return true;
        }

        let width = img.width;
        let height = img.height;

        if width > MAX_DIMENSION || height > MAX_DIMENSION {
            self.push_system(&format!(
                "Image too large ({width}x{height}). Max {MAX_DIMENSION}x{MAX_DIMENSION}."
            ));
            self.dirty = true;
            return true;
        }

        let rgba_bytes = img.bytes;

        // Encode as PNG
        let mut png_data: Vec<u8> = Vec::new();
        if let Err(e) = encode_rgba_to_png(&mut png_data, &rgba_bytes, width, height) {
            self.push_system(&format!("Failed to encode image: {e}"));
            self.dirty = true;
            return true;
        }

        let size_bytes = png_data.len();
        if size_bytes > MAX_BYTES {
            let mb = size_bytes / (1024 * 1024);
            self.push_system(&format!(
                "Image too large ({mb}MB). Max 20MB after encoding."
            ));
            self.dirty = true;
            return true;
        }

        let base64_data = base64::engine::general_purpose::STANDARD.encode(&png_data);

        self.pending_images.push(PendingImage {
            media_type: "image/png".to_string(),
            base64_data,
            size_bytes,
        });

        let count = self.pending_images.len();
        let size_kb = size_bytes / 1024;
        self.push_system(&format!(
            "Image {count}/{MAX_IMAGES} pasted ({width}x{height}, {size_kb}KB). Will be sent with your next message."
        ));
        self.dirty = true;
        true
    }
}

pub(crate) enum HandleKeyResult {
    Continue,
    Quit,
    Submit(String),
}

// ---------------------------------------------------------------------------
// Main event loop
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_app_loop(
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<io::Stdout>>,
    provider: Box<dyn Provider>,
    registry: ToolRegistry,
    system: Vec<SystemBlock>,
    model: &str,
    max_tokens: u32,
    max_turns: u32,
    warnings: Vec<String>,
) -> io::Result<()> {
    let mut app = TuiApp::new(model);

    let mut messages: Vec<Message> = Vec::new();
    let tool_defs = tool_definitions_for_model(&registry);

    // Auto-generate session ID for persistence
    let session_id = format!(
        "{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        std::process::id()
    );
    app.hud.session_id = session_id;

    // Load persistent input history
    app.load_input_history();

    // Auto-update check (non-blocking, cached)
    {
        let ff = nocode_core::config::feature_flags::global_feature_flags();
        let ff_guard = ff.lock().unwrap_or_else(|e| e.into_inner());
        if ff_guard.is_enabled(nocode_core::config::feature_flags::FeatureFlag::AutoUpdate) {
            drop(ff_guard);
            let home = std::env::var("HOME").unwrap_or_default();
            let cache_path = format!("{home}/.nocode/update_cache.json");
            let checker = nocode_core::update_checker::UpdateChecker::new(
                env!("CARGO_PKG_VERSION"),
                &cache_path,
                "telagod/nocode",
            );
            if let nocode_core::update_checker::UpdateStatus::UpdateAvailable {
                current,
                latest,
                download_url,
            } = checker.check_cached_only()
            {
                app.push_system(&format!(
                    "Update available: {current} → {latest}\n  {download_url}"
                ));
            }
        }
    }

    // Install worker event channel into global registry
    {
        let (worker_tx, worker_rx) = std::sync::mpsc::channel();
        let reg = nocode_core::agent::worker::global_worker_registry();
        let mut guard = reg.lock().unwrap_or_else(|e| e.into_inner());
        guard.set_event_channel(worker_tx);
        app.worker_event_rx = Some(worker_rx);
    }

    // Display provider warnings after all init (so banner renders first)
    // If there are warnings, auto-open config overlay so user can fix settings
    if !warnings.is_empty() {
        for w in &warnings {
            app.push_system(w);
        }
        app.open_config_overlay();
    }

    let mut event_rx: Option<mpsc::Receiver<TuiEvent>> = None;
    let mut is_busy = false;

    let _provider: Arc<dyn Provider> = Arc::from(provider);
    let mut registry_slot: Option<ToolRegistry> = Some(registry);

    loop {
        // 1. Tick spinner
        if let Some(spinner) = app.thinking_spinner.as_mut() {
            spinner.tick();
        }

        // 2. Drain channel events
        if let Some(rx) = &event_rx {
            loop {
                match rx.try_recv() {
                    Ok(TuiEvent::TextDelta(text)) => {
                        if app.streaming_text.is_empty() {
                            app.thinking_spinner = None;
                        }
                        app.streaming_text.push_str(&text);
                        let snap = app.streaming_text.clone();
                        app.update_streaming_assistant(&snap);
                        app.show_banner = false;
                    }
                    Ok(TuiEvent::ThinkingDelta(text)) => {
                        if app.streaming_thinking.is_empty() {
                            app.thinking_spinner = None;
                        }
                        app.streaming_thinking.push_str(&text);
                        let snap = app.streaming_thinking.clone();
                        app.update_streaming_thinking(&snap);
                    }
                    Ok(TuiEvent::ToolStart { name }) => {
                        app.thinking_spinner = None;
                        app.push_tool_start(&name);
                    }
                    Ok(TuiEvent::InputJsonDelta { name, partial_json }) => {
                        // Update last tool message with streaming args
                        if let Some(last) = app.chat_messages.last_mut()
                            && last.kind == ChatMessageKind::Tool
                        {
                            let preview = if partial_json.len() > 120 {
                                crate::tool_render::truncate_str(&partial_json, 120)
                            } else {
                                partial_json
                            };
                            let label = if name.is_empty() { "tool" } else { &name };
                            last.lines = vec![{
                                let mut rl = crate::markdown_render::RenderedLine::new();
                                rl.push(crate::markdown_render::LineSegment::new(
                                    format!("\u{276F} {label} {preview}"),
                                    crossterm::style::Color::DarkGrey,
                                ));
                                rl
                            }];
                            app.invalidate_height_cache();
                            app.dirty = true;
                        }
                    }
                    Ok(TuiEvent::ToolDone {
                        name,
                        content,
                        is_error,
                    }) => {
                        app.push_tool_done(&name, &content, is_error);
                        // Model will be called again — show spinner
                        app.thinking_spinner = Some(Spinner::new("Thinking..."));
                    }
                    Ok(TuiEvent::PermissionRequest {
                        tool_name,
                        tool_id,
                        response_tx,
                    }) => {
                        app.permission_tx = Some(response_tx);
                        app.overlay = Overlay::Permission { tool_name, tool_id };
                        app.dirty = true;
                    }
                    Ok(TuiEvent::QuestionRequest {
                        questions,
                        response_tx,
                    }) => {
                        app.question_tx = Some(response_tx);
                        // Initialize selection index per question (default: first option)
                        let selected = questions
                            .as_array()
                            .map(|arr| vec![0usize; arr.len()])
                            .unwrap_or_default();
                        app.overlay = Overlay::Question {
                            questions,
                            selected,
                        };
                        app.dirty = true;
                    }
                    Ok(TuiEvent::MessagesUpdated(updated_msgs)) => {
                        // Incremental session persistence
                        use nocode_core::session::persistence::SessionPersistence;
                        let cwd = std::env::current_dir()
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let session_id = app.hud.session_id.clone();
                        if !session_id.is_empty() {
                            let mut persistence = SessionPersistence::new(&cwd, &session_id);
                            persistence.flush_incremental(&updated_msgs);
                        }
                    }
                    Ok(TuiEvent::Complete(result, returned_registry)) => {
                        app.thinking_spinner = None;
                        app.streaming_text.clear();
                        app.streaming_thinking.clear();
                        is_busy = false;
                        event_rx = None;
                        registry_slot = Some(returned_registry);

                        match result {
                            Ok(loop_result) => {
                                app.hud.record_tokens(
                                    loop_result.total_input_tokens,
                                    loop_result.total_output_tokens,
                                );
                                app.hud.record_cache_tokens(
                                    loop_result.total_cache_read_tokens,
                                    loop_result.total_cache_write_tokens,
                                );
                                app.hud.end_turn();
                                messages = loop_result.messages;
                            }
                            Err(e) => {
                                app.push_error(&e);
                                app.hud.end_turn();
                            }
                        }
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        if is_busy {
                            app.push_error("Model response interrupted. Try submitting again or restart with /clear.");
                            is_busy = false;
                            event_rx = None;
                        }
                        break;
                    }
                }
            }
        }

        // 2b. Poll background model fetch results
        if let Some(result) = app.poll_model_fetch()
            && let Overlay::Config(ref mut config) = app.overlay
        {
            match result {
                Ok(models) if !models.is_empty() => {
                    let count = models.len();
                    config.status = Some(format!("Loaded {count} model suggestion(s)"));
                    config.all_model_suggestions = models.clone();
                    config.model_suggestions = models;
                }
                Ok(_) => {
                    config.status = Some("No model suggestions returned".to_string());
                }
                Err(e) => {
                    config.status = Some(format!("Model suggestions unavailable: {e}"));
                }
            }
            app.dirty = true;
        }

        // 2c. Poll worker events
        {
            use nocode_core::agent::worker::WorkerEvent;
            let mut worker_events = Vec::new();
            if let Some(ref worker_rx) = app.worker_event_rx {
                loop {
                    match worker_rx.try_recv() {
                        Ok(event) => worker_events.push(event),
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                    }
                }
            }
            for event in worker_events {
                match event {
                    WorkerEvent::Finished {
                        worker_id,
                        name,
                        result,
                    } => {
                        let preview = crate::tool_render::truncate_str(&result, 500);
                        app.push_system(&format!(
                            "Agent '{name}' ({worker_id}) finished:\n{preview}"
                        ));
                    }
                    WorkerEvent::Failed {
                        worker_id,
                        name,
                        error,
                    } => {
                        app.push_error(&format!("Agent '{name}' ({worker_id}) failed: {error}"));
                    }
                    WorkerEvent::TimedOut { worker_id, name } => {
                        app.push_error(&format!(
                            "Agent '{name}' ({worker_id}) timed out and was cancelled"
                        ));
                    }
                    WorkerEvent::StateChanged { .. } => {
                        if matches!(app.overlay, Overlay::Agents) {
                            app.dirty = true;
                        }
                    }
                }
            }
        }

        // 2d. Check worker timeouts
        {
            let reg = nocode_core::agent::worker::global_worker_registry();
            let mut guard = reg.lock().unwrap_or_else(|e| e.into_inner());
            let _ = guard.check_timeouts();
        }

        // 3. Render
        terminal.draw(|frame| app.draw(frame))?;

        // 4. Poll crossterm events (50ms for responsive spinner)
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => {
                    if is_busy {
                        // Engine busy — only scroll/quit/cancel
                        match (key.code, key.modifiers) {
                            (KeyCode::Char('c'), KeyModifiers::CONTROL) => break,
                            (KeyCode::Esc, _) => {
                                // Can't cancel the sync loop, just note it
                                app.push_system("waiting for model response...");
                            }
                            (KeyCode::Up, _) => {
                                app.chat_scroll = app.chat_scroll.saturating_add(1);
                                app.sticky_scroll = false;
                                app.dirty = true;
                            }
                            (KeyCode::Down, _) => {
                                if app.chat_scroll > 0 {
                                    app.chat_scroll = app.chat_scroll.saturating_sub(1);
                                }
                                if app.chat_scroll == 0 {
                                    app.sticky_scroll = true;
                                    app.unseen_count = 0;
                                }
                                app.dirty = true;
                            }
                            _ => {}
                        }
                    } else {
                        match app.handle_key(key) {
                            HandleKeyResult::Quit => break,
                            HandleKeyResult::Submit(text) => {
                                // Handle slash commands via registry
                                let cmd_reg = CommandRegistry::with_defaults();
                                if let Some((action, args)) = cmd_reg.resolve(&text) {
                                    match handle_slash_command(
                                        action,
                                        args,
                                        &mut app,
                                        &mut messages,
                                        model,
                                    ) {
                                        SlashResult::Quit => break,
                                        SlashResult::Handled => continue,
                                    }
                                }

                                // Submit to agentic loop
                                app.push_user_message(&text);
                                app.show_banner = false;

                                // Build message with text + any pending images
                                let mut blocks: Vec<ContentBlock> = Vec::new();
                                for img in app.pending_images.drain(..) {
                                    blocks
                                        .push(ContentBlock::image(img.media_type, img.base64_data));
                                }
                                blocks.push(ContentBlock::text(&text));
                                messages.push(Message::user(blocks));

                                app.hud.start_turn();
                                app.thinking_spinner = Some(Spinner::new("Thinking..."));
                                is_busy = true;

                                // Launch background thread
                                let (tx, rx) = mpsc::channel();
                                event_rx = Some(rx);

                                let r = registry_slot.take().expect("registry available");
                                let msgs = messages.clone();
                                let current_model = app.hud.model_name.clone();
                                let cfg = LoopConfig {
                                    model: current_model.clone(),
                                    max_tokens,
                                    max_turns,
                                    system: system.clone(),
                                    tools: tool_defs.clone(),
                                    parallel_tool_execution: true,
                                    reasoning_effort: None,
                                };

                                let tx_complete = tx.clone();
                                let tx_perm = tx.clone();
                                std::thread::spawn(move || {
                                    let perm_bridge =
                                        crate::tui_events::TuiEventPermissionBridge::new(tx_perm);
                                    let q_bridge =
                                        crate::tui_events::TuiEventQuestionBridge::new(tx.clone());
                                    // Inject question prompter into AskUserQuestion tool
                                    if let Some(ask_tool) = r.get_as::<nocode_core::tool::interactive_tools::AskUserQuestionTool>("AskUserQuestion") {
                                        ask_tool.set_prompter(Box::new(q_bridge));
                                    }
                                    let cwd = std::env::current_dir()
                                        .map(|p| p.to_string_lossy().into_owned())
                                        .unwrap_or_default();
                                    let settings = Settings::load_merged(&cwd);
                                    let provider_type = crate::resolve_provider(&settings);
                                    let (provider, _) =
                                        crate::build_provider(&provider_type, &settings);
                                    let executor =
                                        ToolExecutor::new(&r).with_prompter(&perm_bridge);
                                    let mut observer = ChannelObserver { tx };
                                    let result = r#loop::run_agentic_loop(
                                        provider.as_ref(),
                                        &executor,
                                        &cfg,
                                        msgs,
                                        &mut observer,
                                    );
                                    let loop_result = result.map_err(|e| format!("{e}"));
                                    let _ = tx_complete.send(TuiEvent::Complete(loop_result, r));
                                });
                            }
                            HandleKeyResult::Continue => {}
                        }
                    }
                }
                Event::Paste(text) if !is_busy => {
                    app.handle_paste(&text);
                }
                Event::Mouse(mouse) => {
                    use crossterm::event::{MouseEvent, MouseEventKind};
                    match mouse {
                        MouseEvent {
                            kind: MouseEventKind::ScrollUp,
                            ..
                        } => {
                            app.chat_scroll = app.chat_scroll.saturating_add(3);
                            app.sticky_scroll = false;
                            app.dirty = true;
                        }
                        MouseEvent {
                            kind: MouseEventKind::ScrollDown,
                            ..
                        } => {
                            app.chat_scroll = app.chat_scroll.saturating_sub(3);
                            if app.chat_scroll == 0 {
                                app.sticky_scroll = true;
                                app.unseen_count = 0;
                            }
                            app.dirty = true;
                        }
                        _ => {}
                    }
                }
                Event::Resize(_, _) => {
                    app.invalidate_height_cache();
                    app.dirty = true;
                }
                _ => {}
            }
        }
    }

    // Show resume hint if session has messages
    if !messages.is_empty() && !app.hud.session_id.is_empty() {
        eprintln!(
            "\n  To resume this session: nocode --resume {}\n",
            app.hud.session_id
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn highlight_search_in_line<'a>(
    line: ratatui::text::Line<'a>,
    query: &str,
) -> ratatui::text::Line<'a> {
    use ratatui::style::{Color, Modifier};
    use ratatui::text::Span;
    if query.is_empty() {
        return line;
    }
    let query_lower = query.to_lowercase();
    let mut new_spans = Vec::new();
    for span in line.spans {
        let text = span.content.to_string();
        let text_lower = text.to_lowercase();
        let base_style = span.style;
        let hl_style = base_style
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD);
        let mut start = 0;
        while let Some(pos) = text_lower[start..].find(&query_lower) {
            let abs = start + pos;
            if abs > start {
                new_spans.push(Span::styled(text[start..abs].to_string(), base_style));
            }
            let end = abs + query.len();
            let end = end.min(text.len());
            new_spans.push(Span::styled(text[abs..end].to_string(), hl_style));
            start = end;
        }
        if start < text.len() {
            new_spans.push(Span::styled(text[start..].to_string(), base_style));
        } else if start == 0 && text.is_empty() {
            new_spans.push(span);
        }
    }
    ratatui::text::Line::from(new_spans)
}

fn prev_char_boundary(s: &str, pos: usize) -> usize {
    let mut p = pos.saturating_sub(1);
    while p > 0 && !s.is_char_boundary(p) {
        p -= 1;
    }
    p
}

fn next_char_boundary(s: &str, pos: usize) -> usize {
    let mut p = pos + 1;
    while p < s.len() && !s.is_char_boundary(p) {
        p += 1;
    }
    p.min(s.len())
}

#[allow(dead_code)]
fn display_width_of(s: &str) -> usize {
    s.chars().map(|c| c.width().unwrap_or(0)).sum()
}

fn next_word_boundary(s: &str, pos: usize) -> usize {
    let mut p = pos;
    // Skip current word chars (non-whitespace)
    for c in s[pos..].chars() {
        if c.is_whitespace() {
            break;
        }
        p += c.len_utf8();
    }
    // Skip whitespace
    for c in s[p..].chars() {
        if !c.is_whitespace() {
            break;
        }
        p += c.len_utf8();
    }
    p.min(s.len())
}

fn provider_options() -> &'static [&'static str] {
    &["auto", "claude", "openai", "gemini", "custom"]
}

fn cycle_provider(current: &str, forward: bool) -> String {
    let options = provider_options();
    let current_idx = options
        .iter()
        .position(|p| p.eq_ignore_ascii_case(current))
        .unwrap_or(0);
    let next_idx = if forward {
        (current_idx + 1) % options.len()
    } else if current_idx == 0 {
        options.len() - 1
    } else {
        current_idx - 1
    };
    options[next_idx].to_string()
}

fn provider_display_name(provider: &str) -> &'static str {
    use nocode_core::provider::types::ModelProvider;
    match ModelProvider::parse(provider) {
        Some(ModelProvider::Claude) => "Anthropic",
        Some(ModelProvider::OpenAi) => "OpenAI",
        Some(ModelProvider::Gemini) => "Gemini",
        Some(ModelProvider::Custom) => "Custom",
        None => "Auto",
    }
}

pub(crate) fn find_preset_by_name(name: &str) -> Option<&'static ProviderPreset> {
    CUSTOM_PRESETS
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(name))
}

fn provider_key_slot(
    provider: &str,
    api_format: &str,
    preset_name: Option<&str>,
) -> (&'static str, &'static str) {
    use nocode_core::provider::types::ModelProvider;
    match ModelProvider::parse(provider) {
        Some(ModelProvider::Claude) => ("anthropic", "ANTHROPIC_API_KEY"),
        Some(ModelProvider::OpenAi) => ("openai", "OPENAI_API_KEY"),
        Some(ModelProvider::Gemini) => ("gemini", "GEMINI_API_KEY"),
        Some(ModelProvider::Custom) => {
            if let Some(preset) = preset_name.and_then(find_preset_by_name) {
                if preset.env_key_name.is_empty() {
                    return (preset.credential_slot, "");
                }
                return (preset.credential_slot, preset.env_key_name);
            }
            let normalized = nocode_core::config::settings::normalize_api_format(api_format);
            match normalized {
                "anthropic" => ("anthropic", "ANTHROPIC_API_KEY"),
                "google" => ("gemini", "GEMINI_API_KEY"),
                _ => ("openai", "OPENAI_API_KEY"),
            }
        }
        None => ("anthropic", "ANTHROPIC_API_KEY"),
    }
}

fn load_credential_store() -> nocode_core::storage::credentials::CredentialStore {
    let cred_path = nocode_core::storage::credentials::CredentialStore::default_path();
    nocode_core::storage::credentials::CredentialStore::load(&cred_path).unwrap_or_default()
}

fn apply_model_filter(all_models: &[String], filter: &str) -> Vec<String> {
    crate::model_fetch::apply_model_filter(all_models, filter)
}

fn apply_api_key_to_env(
    provider: &str,
    api_format: &str,
    api_key: &str,
    preset_name: Option<&str>,
) {
    if api_key.trim().is_empty() {
        return;
    }
    let (_, env_var) = provider_key_slot(provider, api_format, preset_name);
    if env_var.is_empty() {
        return;
    }
    unsafe {
        std::env::set_var(env_var, api_key.trim());
    }
}

fn load_settings_triplet(cwd: &str) -> (Settings, Settings, Settings) {
    (
        Settings::load_tier(SettingsTier::User, cwd),
        Settings::load_tier(SettingsTier::Project, cwd),
        Settings::load_tier(SettingsTier::Local, cwd),
    )
}

fn merged_without_tier(selected_tier: SettingsTier, cwd: &str) -> Settings {
    let (user, project, local) = load_settings_triplet(cwd);
    match selected_tier {
        SettingsTier::User => Settings::default().merge(project).merge(local),
        SettingsTier::Project => user.merge(Settings::default()).merge(local),
        SettingsTier::Local => user.merge(project).merge(Settings::default()),
    }
}

fn restore_setting_from_source(
    key: &str,
    selected_tier: SettingsTier,
    cwd: &str,
) -> (String, String) {
    let (user, project, local) = load_settings_triplet(cwd);
    let merged = merged_without_tier(selected_tier, cwd);
    let source_without_tier = |env_var: &str, fallback: Option<&str>| -> String {
        if std::env::var(env_var).is_ok() {
            "env".to_string()
        } else if selected_tier != SettingsTier::Local && local.get_key(key).is_some() {
            "local".to_string()
        } else if selected_tier != SettingsTier::Project && project.get_key(key).is_some() {
            "project".to_string()
        } else if selected_tier != SettingsTier::User && user.get_key(key).is_some() {
            "user".to_string()
        } else {
            fallback.unwrap_or("default").to_string()
        }
    };
    match key {
        "model_provider" => {
            if let Ok(value) = std::env::var("NOCODE_MODEL_PROVIDER") {
                return (value, "env".to_string());
            }
            if local.get_key(key).is_some() && selected_tier != SettingsTier::Local {
                return (
                    merged.model_provider.unwrap_or_else(|| "auto".to_string()),
                    "local".to_string(),
                );
            }
            if project.get_key(key).is_some() && selected_tier != SettingsTier::Project {
                return (
                    merged.model_provider.unwrap_or_else(|| "auto".to_string()),
                    "project".to_string(),
                );
            }
            if user.get_key(key).is_some() && selected_tier != SettingsTier::User {
                return (
                    merged.model_provider.unwrap_or_else(|| "auto".to_string()),
                    "user".to_string(),
                );
            }
            if merged.custom_base_url.is_some() || merged.custom_api_format.is_some() {
                return ("custom".to_string(), "derived".to_string());
            }
            ("auto".to_string(), "default".to_string())
        }
        "model" => (
            std::env::var("NOCODE_MODEL")
                .ok()
                .or(merged.model)
                .unwrap_or_default(),
            source_without_tier("NOCODE_MODEL", None),
        ),
        "custom_base_url" => (
            std::env::var("NOCODE_CUSTOM_BASE_URL")
                .ok()
                .or(merged.custom_base_url)
                .unwrap_or_default(),
            source_without_tier("NOCODE_CUSTOM_BASE_URL", None),
        ),
        "custom_api_format" => (
            std::env::var("NOCODE_CUSTOM_API_FORMAT")
                .ok()
                .or(merged.custom_api_format)
                .unwrap_or_else(|| "openai-responses".to_string()),
            source_without_tier("NOCODE_CUSTOM_API_FORMAT", Some("default")),
        ),
        _ => (String::new(), "default".to_string()),
    }
}

fn restore_api_key_from_source(
    provider: &str,
    api_format: &str,
    preset_name: Option<&str>,
) -> (String, String) {
    let store = load_credential_store();
    let (slot, env_var) = provider_key_slot(provider, api_format, preset_name);
    if !env_var.is_empty()
        && let Ok(value) = std::env::var(env_var)
    {
        return (value, "env".to_string());
    }
    if let Some(value) = store.get_key(slot) {
        return (value, "credentials".to_string());
    }
    (String::new(), "unset".to_string())
}

fn setting_source_label(
    key: &str,
    env_var: &str,
    user_settings: &Settings,
    project_settings: &Settings,
    local_settings: &Settings,
    fallback: Option<&str>,
) -> String {
    if std::env::var(env_var).is_ok() {
        return "env".to_string();
    }
    if local_settings.get_key(key).is_some() {
        return "local".to_string();
    }
    if project_settings.get_key(key).is_some() {
        return "project".to_string();
    }
    if user_settings.get_key(key).is_some() {
        return "user".to_string();
    }
    fallback.unwrap_or("default").to_string()
}

fn fetch_model_suggestions(
    provider: &str,
    custom_base_url: &str,
    custom_api_format: &str,
) -> Result<Vec<String>, String> {
    crate::model_fetch::fetch_model_suggestions(provider, custom_base_url, custom_api_format)
}

fn spawn_model_fetch_bg(
    provider: &str,
    custom_base_url: &str,
    custom_api_format: &str,
) -> std::sync::mpsc::Receiver<Result<Vec<String>, String>> {
    crate::model_fetch::spawn_model_fetch_bg(provider, custom_base_url, custom_api_format)
}

fn prev_word_boundary(s: &str, pos: usize) -> usize {
    if pos == 0 {
        return 0;
    }
    // Collect char boundaries up to pos
    let mut boundaries: Vec<usize> = s.char_indices().map(|(i, _)| i).collect();
    boundaries.push(s.len());
    // Find current char index
    let char_idx = boundaries
        .iter()
        .position(|&b| b >= pos)
        .unwrap_or(boundaries.len());
    let mut ci = char_idx.saturating_sub(1);
    // Skip whitespace backwards
    while ci > 0 {
        let ch = s[boundaries[ci]..].chars().next().unwrap_or(' ');
        if !ch.is_whitespace() {
            break;
        }
        ci -= 1;
    }
    // Skip word chars backwards
    while ci > 0 {
        let prev_ch = s[boundaries[ci - 1]..].chars().next().unwrap_or(' ');
        if prev_ch.is_whitespace() {
            break;
        }
        ci -= 1;
    }
    boundaries.get(ci).copied().unwrap_or(0)
}

/// Copy text to system clipboard. Cross-platform: xclip/xsel (Linux), pbcopy (macOS), clip.exe (WSL/Windows).
fn copy_to_clipboard(text: &str) -> Result<(), String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let candidates: &[&[&str]] = if cfg!(target_os = "macos") {
        &[&["pbcopy"]]
    } else {
        // Linux: try xclip, xsel, wl-copy (Wayland), clip.exe (WSL)
        &[
            &["xclip", "-selection", "clipboard"],
            &["xsel", "--clipboard", "--input"],
            &["wl-copy"],
            &["clip.exe"],
        ]
    };

    for args in candidates {
        let program = args[0];
        let extra_args = &args[1..];
        if let Ok(mut child) = Command::new(program)
            .args(extra_args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            if let Some(stdin) = child.stdin.as_mut() {
                let _ = stdin.write_all(text.as_bytes());
            }
            if let Ok(status) = child.wait()
                && status.success()
            {
                return Ok(());
            }
        }
    }

    Err("Clipboard not available. Install xclip, xsel, or wl-copy.".to_string())
}

/// Truncate a string to at most `max` bytes on a valid UTF-8 boundary.
fn safe_truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut idx = max;
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    &s[..idx]
}

/// Truncate a string with "..." suffix if it exceeds `max` bytes.
fn safe_truncate_ellipsis(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut idx = max.saturating_sub(3);
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    format!("{}...", &s[..idx])
}

/// Path to persistent input history file.
fn input_history_path() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(
        std::path::PathBuf::from(home)
            .join(".nocode")
            .join("input_history.txt"),
    )
}

/// Encode RGBA pixel data to PNG format.
fn encode_rgba_to_png(
    out: &mut Vec<u8>,
    rgba: &[u8],
    width: usize,
    height: usize,
) -> Result<(), String> {
    let mut encoder = png::Encoder::new(std::io::Cursor::new(out), width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|e| format!("PNG header: {e}"))?;
    writer
        .write_image_data(rgba)
        .map_err(|e| format!("PNG data: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // detect_preset
    // -----------------------------------------------------------------------

    #[test]
    fn detect_preset_exact_match() {
        assert_eq!(detect_preset("https://openrouter.ai/api/v1"), Some(0)); // OpenRouter
        assert_eq!(detect_preset("https://api.together.xyz/v1"), Some(1)); // Together
        assert_eq!(detect_preset("https://api.groq.com/openai/v1"), Some(2)); // Groq
        assert_eq!(
            detect_preset("https://api.fireworks.ai/inference/v1"),
            Some(3)
        ); // Fireworks
        assert_eq!(detect_preset("https://api.deepseek.com/v1"), Some(4)); // DeepSeek
        assert_eq!(detect_preset("https://api.mistral.ai/v1"), Some(5)); // Mistral
        assert_eq!(detect_preset("http://localhost:11434/v1"), Some(6)); // Ollama
        assert_eq!(detect_preset("http://localhost:8000/v1"), Some(7)); // vLLM
        assert_eq!(detect_preset("http://localhost:4000/v1"), Some(8)); // LiteLLM
        assert_eq!(detect_preset("http://localhost:8080/v1"), Some(9)); // LocalAI
        assert_eq!(detect_preset("http://localhost:1234/v1"), Some(10)); // LM Studio
    }

    #[test]
    fn detect_preset_trailing_slash() {
        assert_eq!(detect_preset("https://openrouter.ai/api/v1/"), Some(0));
        assert_eq!(detect_preset("http://localhost:11434/v1/"), Some(6));
    }

    #[test]
    fn detect_preset_whitespace() {
        assert_eq!(detect_preset("  https://openrouter.ai/api/v1  "), Some(0));
        assert_eq!(detect_preset("\thttp://localhost:8000/v1\n"), Some(7));
    }

    #[test]
    fn detect_preset_case_insensitive() {
        assert_eq!(detect_preset("HTTPS://OPENROUTER.AI/API/V1"), Some(0));
        assert_eq!(detect_preset("HTTP://LOCALHOST:11434/V1"), Some(6));
    }

    #[test]
    fn detect_preset_no_match() {
        assert_eq!(detect_preset("http://localhost:9999/v1"), None);
        assert_eq!(detect_preset("https://api.openai.com/v1"), None);
        assert_eq!(detect_preset(""), None);
        assert_eq!(detect_preset("  "), None);
    }

    // -----------------------------------------------------------------------
    // preset_label
    // -----------------------------------------------------------------------

    #[test]
    fn preset_label_valid_indices() {
        assert_eq!(preset_label(Some(0)), "OpenRouter");
        assert_eq!(preset_label(Some(1)), "Together");
        assert_eq!(preset_label(Some(6)), "Ollama");
        assert_eq!(preset_label(Some(7)), "vLLM");
        assert_eq!(preset_label(Some(10)), "LM Studio");
    }

    #[test]
    fn preset_label_manual_cases() {
        assert_eq!(preset_label(None), "Manual");
        assert_eq!(preset_label(Some(999)), "Manual");
        assert_eq!(preset_label(Some(usize::MAX)), "Manual");
    }

    // -----------------------------------------------------------------------
    // CUSTOM_PRESETS invariants
    // -----------------------------------------------------------------------

    #[test]
    fn presets_have_unique_urls() {
        let urls: Vec<&str> = CUSTOM_PRESETS.iter().map(|p| p.base_url).collect();
        for (i, url) in urls.iter().enumerate() {
            for (j, other) in urls.iter().enumerate() {
                if i != j {
                    assert_ne!(
                        url.to_ascii_lowercase(),
                        other.to_ascii_lowercase(),
                        "Duplicate preset URL: {url}"
                    );
                }
            }
        }
    }

    #[test]
    fn presets_have_valid_api_format() {
        let valid = nocode_core::config::settings::API_FORMATS;
        for preset in CUSTOM_PRESETS {
            assert!(
                valid.contains(&preset.api_format),
                "Invalid api_format '{}' in preset '{}'",
                preset.api_format,
                preset.name
            );
        }
    }

    #[test]
    fn presets_not_empty() {
        assert!(!CUSTOM_PRESETS.is_empty(), "Preset list must not be empty");
        for preset in CUSTOM_PRESETS {
            assert!(!preset.name.is_empty(), "Preset name must not be empty");
            assert!(
                !preset.base_url.is_empty(),
                "Preset base_url must not be empty"
            );
            assert!(
                !preset.auth_hint.is_empty(),
                "Preset auth_hint must not be empty"
            );
        }
    }

    #[test]
    fn detect_preset_roundtrip() {
        // Every preset's own URL should be detected back to its index.
        for (i, preset) in CUSTOM_PRESETS.iter().enumerate() {
            assert_eq!(
                detect_preset(preset.base_url),
                Some(i),
                "Preset '{}' at index {} failed roundtrip",
                preset.name,
                i
            );
        }
    }
}
