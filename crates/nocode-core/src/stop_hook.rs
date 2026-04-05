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
}
