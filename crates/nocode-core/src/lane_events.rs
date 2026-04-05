use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaneEventName {
    Started,
    Ready,
    PromptMisdelivery,
    Blocked,
    Red,
    Green,
    CommitCreated,
    PrOpened,
    MergeReady,
    Finished,
    Failed,
    Reconciled,
    Merged,
    Superseded,
    Closed,
    BranchStaleAgainstMain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaneEventStatus {
    Running,
    Ready,
    Blocked,
    Red,
    Green,
    Completed,
    Failed,
    Reconciled,
    Merged,
    Superseded,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaneFailureClass {
    PromptDelivery,
    TrustGate,
    BranchDivergence,
    Compile,
    Test,
    PluginStartup,
    McpStartup,
    McpHandshake,
    ToolRuntime,
    Infra,
}

#[derive(Debug, Clone)]
pub struct LaneEvent {
    pub name: LaneEventName,
    pub status: LaneEventStatus,
    pub emitted_at_ms: u64,
    pub failure_class: Option<LaneFailureClass>,
    pub detail: Option<String>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

impl LaneEvent {
    pub fn new(name: LaneEventName, status: LaneEventStatus) -> Self {
        Self {
            name,
            status,
            emitted_at_ms: now_ms(),
            failure_class: None,
            detail: None,
        }
    }

    pub fn with_failure(
        name: LaneEventName,
        status: LaneEventStatus,
        class: LaneFailureClass,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            name,
            status,
            emitted_at_ms: now_ms(),
            failure_class: Some(class),
            detail: Some(detail.into()),
        }
    }

    pub fn blocked(class: LaneFailureClass, detail: impl Into<String>) -> Self {
        Self::with_failure(LaneEventName::Blocked, LaneEventStatus::Blocked, class, detail)
    }

    pub fn failed(class: LaneFailureClass, detail: impl Into<String>) -> Self {
        Self::with_failure(LaneEventName::Failed, LaneEventStatus::Failed, class, detail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_event_has_no_failure() {
        let e = LaneEvent::new(LaneEventName::Started, LaneEventStatus::Running);
        assert_eq!(e.name, LaneEventName::Started);
        assert_eq!(e.status, LaneEventStatus::Running);
        assert!(e.failure_class.is_none());
        assert!(e.detail.is_none());
        assert!(e.emitted_at_ms > 0);
    }

    #[test]
    fn with_failure_captures_class_and_detail() {
        let e = LaneEvent::with_failure(
            LaneEventName::Red,
            LaneEventStatus::Red,
            LaneFailureClass::Compile,
            "rustc failed",
        );
        assert_eq!(e.failure_class, Some(LaneFailureClass::Compile));
        assert_eq!(e.detail.as_deref(), Some("rustc failed"));
    }

    #[test]
    fn blocked_convenience() {
        let e = LaneEvent::blocked(LaneFailureClass::McpStartup, "server unreachable");
        assert_eq!(e.name, LaneEventName::Blocked);
        assert_eq!(e.status, LaneEventStatus::Blocked);
        assert_eq!(e.failure_class, Some(LaneFailureClass::McpStartup));
    }

    #[test]
    fn failed_convenience() {
        let e = LaneEvent::failed(LaneFailureClass::Test, "3 tests failed");
        assert_eq!(e.name, LaneEventName::Failed);
        assert_eq!(e.status, LaneEventStatus::Failed);
        assert_eq!(e.detail.as_deref(), Some("3 tests failed"));
    }
}
