use ratatui::style::Color;
use std::sync::{OnceLock, RwLock};

/// Which theme variant is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeVariant {
    Dark,
    Light,
}

/// Centralized theme for the nocode TUI.
///
/// All semantic colors live here so every widget draws from one palette.
#[derive(Clone)]
pub struct Theme {
    pub variant: ThemeVariant,
    // Role colors
    pub claude: Color,
    pub user: Color,
    pub assistant: Color,
    pub error: Color,
    pub warning: Color,
    pub tool: Color,
    pub system: Color,
    pub success: Color,

    // Chrome
    pub border: Color,
    pub text: Color,
    pub text_dim: Color,
    pub text_inactive: Color,
    pub background: Color,

    // Message backgrounds
    pub user_msg_bg: Color,
    pub assistant_msg_bg: Color,
    pub tool_msg_bg: Color,
    pub error_msg_bg: Color,

    // Status bar
    pub status_bar_bg: Color,
    pub status_bar_fg: Color,

    // Input
    pub input_border: Color,

    // Spinner
    pub spinner: Color,

    // Diff
    pub diff_added: Color,
    pub diff_removed: Color,

    // Badges (bg, fg)
    pub badge_user_bg: Color,
    pub badge_user_fg: Color,
    pub badge_assistant_bg: Color,
    pub badge_assistant_fg: Color,
    pub badge_tool_bg: Color,
    pub badge_tool_fg: Color,
    pub badge_error_bg: Color,
    pub badge_error_fg: Color,
    pub badge_system_bg: Color,
    pub badge_system_fg: Color,
}

impl Theme {
    /// The default dark theme with all semantic colors defined.
    pub fn dark() -> Self {
        Self {
            variant: ThemeVariant::Dark,
            claude: Color::Rgb(255, 149, 0),
            user: Color::Green,
            assistant: Color::Cyan,
            error: Color::Red,
            warning: Color::Yellow,
            tool: Color::Rgb(180, 160, 60),
            system: Color::Rgb(100, 100, 110),
            success: Color::Green,

            border: Color::DarkGray,
            text: Color::White,
            text_dim: Color::Gray,
            text_inactive: Color::DarkGray,
            background: Color::Reset,

            user_msg_bg: Color::Rgb(20, 30, 20),
            assistant_msg_bg: Color::Reset,
            tool_msg_bg: Color::Rgb(25, 25, 15),
            error_msg_bg: Color::Rgb(40, 15, 15),

            status_bar_bg: Color::Rgb(30, 30, 40),
            status_bar_fg: Color::White,

            input_border: Color::Rgb(80, 80, 100),

            spinner: Color::Rgb(255, 149, 0),

            diff_added: Color::Rgb(80, 200, 80),
            diff_removed: Color::Rgb(200, 80, 80),

            badge_user_bg: Color::Green,
            badge_user_fg: Color::Black,
            badge_assistant_bg: Color::Cyan,
            badge_assistant_fg: Color::Black,
            badge_tool_bg: Color::Yellow,
            badge_tool_fg: Color::Black,
            badge_error_bg: Color::Red,
            badge_error_fg: Color::White,
            badge_system_bg: Color::DarkGray,
            badge_system_fg: Color::White,
        }
    }

    /// Light theme — high contrast on white/light backgrounds.
    pub fn light() -> Self {
        Self {
            variant: ThemeVariant::Light,
            claude: Color::Rgb(200, 100, 0),
            user: Color::Rgb(0, 120, 0),
            assistant: Color::Rgb(0, 100, 140),
            error: Color::Rgb(180, 0, 0),
            warning: Color::Rgb(180, 140, 0),
            tool: Color::Rgb(120, 90, 20),
            system: Color::Rgb(120, 120, 130),
            success: Color::Rgb(0, 140, 0),

            border: Color::Gray,
            text: Color::Black,
            text_dim: Color::DarkGray,
            text_inactive: Color::Gray,
            background: Color::Reset,

            user_msg_bg: Color::Rgb(230, 245, 230),
            assistant_msg_bg: Color::Reset,
            tool_msg_bg: Color::Rgb(245, 242, 225),
            error_msg_bg: Color::Rgb(255, 230, 230),

            status_bar_bg: Color::Rgb(230, 230, 240),
            status_bar_fg: Color::Black,

            input_border: Color::Rgb(160, 160, 180),

            spinner: Color::Rgb(200, 100, 0),

            diff_added: Color::Rgb(0, 140, 0),
            diff_removed: Color::Rgb(180, 0, 0),

            badge_user_bg: Color::Rgb(0, 120, 0),
            badge_user_fg: Color::White,
            badge_assistant_bg: Color::Rgb(0, 100, 140),
            badge_assistant_fg: Color::White,
            badge_tool_bg: Color::Rgb(140, 100, 0),
            badge_tool_fg: Color::White,
            badge_error_bg: Color::Rgb(180, 0, 0),
            badge_error_fg: Color::White,
            badge_system_bg: Color::Gray,
            badge_system_fg: Color::White,
        }
    }

    /// Returns `(background, foreground)` for a message-kind badge.
    pub fn badge_style(&self, kind: &str) -> (Color, Color) {
        match kind {
            "user" => (self.badge_user_bg, self.badge_user_fg),
            "assistant" => (self.badge_assistant_bg, self.badge_assistant_fg),
            "tool" => (self.badge_tool_bg, self.badge_tool_fg),
            "error" => (self.badge_error_bg, self.badge_error_fg),
            "system" => (self.badge_system_bg, self.badge_system_fg),
            _ => (self.border, self.text),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

/// Global theme storage — `RwLock` allows runtime switching.
static THEME: OnceLock<RwLock<Theme>> = OnceLock::new();

fn theme_lock() -> &'static RwLock<Theme> {
    THEME.get_or_init(|| RwLock::new(Theme::dark()))
}

/// Read-access to the current theme. Returns a snapshot copy to avoid holding the lock.
pub fn default_theme() -> Theme {
    theme_lock().read().expect("theme lock poisoned").clone()
}

/// Toggle between dark and light themes. Returns the new variant.
pub fn toggle_theme() -> ThemeVariant {
    let mut guard = theme_lock().write().expect("theme lock poisoned");
    let new = match guard.variant {
        ThemeVariant::Dark => Theme::light(),
        ThemeVariant::Light => Theme::dark(),
    };
    let variant = new.variant;
    *guard = new;
    variant
}

/// Set a specific theme variant.
pub fn set_theme(variant: ThemeVariant) {
    let mut guard = theme_lock().write().expect("theme lock poisoned");
    if guard.variant != variant {
        *guard = match variant {
            ThemeVariant::Dark => Theme::dark(),
            ThemeVariant::Light => Theme::light(),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_theme_has_all_colors_defined() {
        let theme = Theme::dark();
        // Verify key semantic colors are non-reset where expected.
        assert_ne!(theme.claude, Color::Reset);
        assert_ne!(theme.user, Color::Reset);
        assert_ne!(theme.assistant, Color::Reset);
        assert_ne!(theme.error, Color::Reset);
        assert_ne!(theme.warning, Color::Reset);
        assert_ne!(theme.text, Color::Reset);
        assert_ne!(theme.spinner, Color::Reset);
        assert_ne!(theme.diff_added, Color::Reset);
        assert_ne!(theme.diff_removed, Color::Reset);
        // Background fields that ARE Reset by design.
        assert_eq!(theme.background, Color::Reset);
        assert_eq!(theme.assistant_msg_bg, Color::Reset);
    }

    #[test]
    fn default_theme_returns_consistent_values() {
        let a = default_theme();
        let b = default_theme();
        // Same variant and colors.
        assert_eq!(a.variant, b.variant);
        assert_eq!(a.claude, Color::Rgb(255, 149, 0));
    }

    #[test]
    fn light_theme_has_all_colors_defined() {
        let theme = Theme::light();
        assert_eq!(theme.variant, ThemeVariant::Light);
        assert_ne!(theme.claude, Color::Reset);
        assert_ne!(theme.user, Color::Reset);
        assert_ne!(theme.text, Color::Reset);
        assert_eq!(theme.text, Color::Black);
        assert_eq!(theme.background, Color::Reset);
        assert_eq!(theme.assistant_msg_bg, Color::Reset);
    }

    #[test]
    fn toggle_theme_switches_variant() {
        // Reset to dark first
        set_theme(ThemeVariant::Dark);
        assert_eq!(default_theme().variant, ThemeVariant::Dark);

        let v = toggle_theme();
        assert_eq!(v, ThemeVariant::Light);
        assert_eq!(default_theme().variant, ThemeVariant::Light);

        let v = toggle_theme();
        assert_eq!(v, ThemeVariant::Dark);
        assert_eq!(default_theme().variant, ThemeVariant::Dark);
    }

    #[test]
    fn badge_style_returns_correct_pairs() {
        let theme = Theme::dark();
        assert_eq!(theme.badge_style("user"), (Color::Green, Color::Black));
        assert_eq!(theme.badge_style("assistant"), (Color::Cyan, Color::Black));
        assert_eq!(theme.badge_style("tool"), (Color::Yellow, Color::Black));
        assert_eq!(theme.badge_style("error"), (Color::Red, Color::White));
        assert_eq!(theme.badge_style("system"), (Color::DarkGray, Color::White));
        // Unknown kind falls back to border/text.
        assert_eq!(
            theme.badge_style("unknown"),
            (Color::DarkGray, Color::White)
        );
    }

    #[test]
    fn default_trait_matches_dark() {
        let default = Theme::default();
        let dark = Theme::dark();
        assert_eq!(default.claude, dark.claude);
        assert_eq!(default.status_bar_bg, dark.status_bar_bg);
        assert_eq!(default.input_border, dark.input_border);
    }
}
