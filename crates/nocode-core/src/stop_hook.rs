use crate::message::QueryMessage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopHookInfo {
    pub command: String,
    pub prompt_text: String,
    pub duration_ms: Option<u64>,
}

impl StopHookInfo {
    pub fn new(command: impl Into<String>, prompt_text: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            prompt_text: prompt_text.into(),
            duration_ms: None,
        }
    }

    pub fn with_duration_ms(mut self, duration_ms: u64) -> Self {
        self.duration_ms = Some(duration_ms);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StopHookResult {
    pub blocking_errors: Vec<QueryMessage>,
    pub prevent_continuation: bool,
    pub stop_reason: Option<String>,
    pub hook_count: u32,
    pub has_output: bool,
    pub hook_errors: Vec<String>,
    pub hook_infos: Vec<StopHookInfo>,
}

impl StopHookResult {
    pub fn summary(&self) -> String {
        format!(
            "stop-hooks:blocking_errors={} prevent_continuation={} hook_count={} has_output={}",
            self.blocking_errors.len(),
            self.prevent_continuation,
            self.hook_count,
            self.has_output
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{StopHookInfo, StopHookResult};
    use crate::message::QueryMessage;

    #[test]
    fn stop_hook_summary_is_stable() {
        let result = StopHookResult {
            blocking_errors: vec![QueryMessage::system("blocked")],
            prevent_continuation: true,
            stop_reason: Some(String::from("hook blocked")),
            hook_count: 2,
            has_output: true,
            hook_errors: vec![String::from("stderr")],
            hook_infos: vec![StopHookInfo::new("cmd", "prompt").with_duration_ms(42)],
        };

        assert_eq!(
            result.summary(),
            "stop-hooks:blocking_errors=1 prevent_continuation=true hook_count=2 has_output=true"
        );
    }

    #[test]
    fn default_stop_hook_result_is_permissive() {
        let result = StopHookResult::default();
        assert!(result.blocking_errors.is_empty());
        assert!(!result.prevent_continuation);
        assert!(result.stop_reason.is_none());
        assert_eq!(result.hook_count, 0);
        assert!(!result.has_output);
        assert!(result.hook_errors.is_empty());
        assert!(result.hook_infos.is_empty());
        assert_eq!(
            result.summary(),
            "stop-hooks:blocking_errors=0 prevent_continuation=false hook_count=0 has_output=false"
        );
    }

    #[test]
    fn stop_hook_info_builder_chain() {
        let info = StopHookInfo::new("cargo test", "run tests");
        assert_eq!(info.command, "cargo test");
        assert_eq!(info.prompt_text, "run tests");
        assert!(info.duration_ms.is_none());

        let info_with_dur = info.with_duration_ms(1500);
        assert_eq!(info_with_dur.duration_ms, Some(1500));
        assert_eq!(info_with_dur.command, "cargo test");
    }

    #[test]
    fn summary_reflects_multiple_blocking_errors() {
        let result = StopHookResult {
            blocking_errors: vec![
                QueryMessage::system("err1"),
                QueryMessage::system("err2"),
                QueryMessage::system("err3"),
            ],
            prevent_continuation: false,
            stop_reason: None,
            hook_count: 3,
            has_output: false,
            hook_errors: Vec::new(),
            hook_infos: Vec::new(),
        };
        assert!(result.summary().contains("blocking_errors=3"));
        assert!(result.summary().contains("hook_count=3"));
    }
}
