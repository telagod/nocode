use ratatui::style::Color;
use std::sync::OnceLock;

/// Centralized theme for the nocode TUI.
///
/// All semantic colors live here so every widget draws from one palette.
/// Currently only `dark()` exists — theme switching can be added later.
pub struct Theme {
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
            claude: Color::Rgb(255, 149, 0),
            user: Color::Green,
            assistant: Color::Cyan,
            error: Color::Red,
            warning: Color::Yellow,
            tool: Color::Yellow,
            system: Color::DarkGray,
            success: Color::Green,

            border: Color::DarkGray,
            text: Color::White,
            text_dim: Color::Gray,
            text_inactive: Color::DarkGray,
            background: Color::Reset,

            user_msg_bg: Color::Rgb(20, 30, 20),
            assistant_msg_bg: Color::Reset,
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

/// Global singleton access to the default theme.
pub fn default_theme() -> &'static Theme {
    static THEME: OnceLock<Theme> = OnceLock::new();
    THEME.get_or_init(Theme::dark)
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
    fn default_theme_returns_consistent_singleton() {
        let a = default_theme();
        let b = default_theme();
        // Same static reference.
        assert!(std::ptr::eq(a, b));
        // Spot-check a value.
        assert_eq!(a.claude, Color::Rgb(255, 149, 0));
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
