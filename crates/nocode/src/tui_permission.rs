//! TUI-aware PermissionPrompter that bridges tool permission decisions
//! to the TUI overlay via mpsc channels.

use nocode_core::tool_execution::{PermissionPrompter, ToolPermissionDecision};
use std::sync::mpsc;
use std::time::Duration;

/// A permission request sent to the TUI overlay for interactive approval.
#[derive(Debug)]
pub struct TuiPermissionBridgeRequest {
    pub tool_name: String,
    pub arguments_summary: String,
    pub response_tx: mpsc::Sender<bool>,
}

/// PermissionPrompter implementation that sends requests to the TUI overlay
/// and blocks until the user approves or denies via F3 overlay (a/d keys).
pub struct TuiPermissionPrompter {
    tx: mpsc::Sender<TuiPermissionBridgeRequest>,
    timeout: Duration,
}

impl TuiPermissionPrompter {
    pub fn new(tx: mpsc::Sender<TuiPermissionBridgeRequest>, timeout: Duration) -> Self {
        Self { tx, timeout }
    }

    /// Create with default 60s timeout.
    pub fn with_default_timeout(tx: mpsc::Sender<TuiPermissionBridgeRequest>) -> Self {
        Self::new(tx, Duration::from_secs(60))
    }
}

impl std::fmt::Debug for TuiPermissionPrompter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TuiPermissionPrompter")
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl PermissionPrompter for TuiPermissionPrompter {
    fn check(&self, tool_name: &str, arguments_summary: &str) -> ToolPermissionDecision {
        let (response_tx, response_rx) = mpsc::channel();
        let request = TuiPermissionBridgeRequest {
            tool_name: tool_name.to_string(),
            arguments_summary: arguments_summary.to_string(),
            response_tx,
        };

        // Send request to TUI overlay.
        if self.tx.send(request).is_err() {
            // Channel closed — fall back to deny.
            return ToolPermissionDecision::deny(format!(
                "TUI permission channel closed for {tool_name}"
            ));
        }

        // Block until user responds or timeout.
        match response_rx.recv_timeout(self.timeout) {
            Ok(true) => ToolPermissionDecision::allow(true),
            Ok(false) => ToolPermissionDecision::deny(format!("user denied {tool_name} via TUI")),
            Err(mpsc::RecvTimeoutError::Timeout) => ToolPermissionDecision::deny(format!(
                "permission timeout for {tool_name} ({}s)",
                self.timeout.as_secs()
            )),
            Err(mpsc::RecvTimeoutError::Disconnected) => ToolPermissionDecision::deny(format!(
                "TUI permission channel disconnected for {tool_name}"
            )),
        }
    }

    fn name(&self) -> &str {
        "tui-interactive"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn tui_prompter_approved() {
        let (tx, rx) = mpsc::channel();
        let prompter = TuiPermissionPrompter::new(tx, Duration::from_secs(5));

        // Simulate TUI overlay approving in background.
        thread::spawn(move || {
            let req = rx.recv().unwrap();
            assert_eq!(req.tool_name, "Bash");
            req.response_tx.send(true).unwrap();
        });

        let decision = prompter.check("Bash", "command=ls");
        assert!(matches!(
            decision,
            ToolPermissionDecision::Allow {
                user_modified: true
            }
        ));
    }

    #[test]
    fn tui_prompter_denied() {
        let (tx, rx) = mpsc::channel();
        let prompter = TuiPermissionPrompter::new(tx, Duration::from_secs(5));

        thread::spawn(move || {
            let req = rx.recv().unwrap();
            req.response_tx.send(false).unwrap();
        });

        let decision = prompter.check("Write", "file_path=foo.rs");
        assert!(matches!(decision, ToolPermissionDecision::Deny { .. }));
    }

    #[test]
    fn tui_prompter_timeout() {
        let (tx, _rx) = mpsc::channel();
        let prompter = TuiPermissionPrompter::new(tx, Duration::from_millis(50));

        // No one reads from rx — will timeout.
        let decision = prompter.check("Agent", "prompt=test");
        assert!(matches!(decision, ToolPermissionDecision::Deny { .. }));
        if let ToolPermissionDecision::Deny { reason } = decision {
            assert!(reason.contains("timeout"));
        }
    }

    #[test]
    fn tui_prompter_channel_closed() {
        let (tx, rx) = mpsc::channel();
        drop(rx); // Close receiving end.
        let prompter = TuiPermissionPrompter::new(tx, Duration::from_secs(5));

        let decision = prompter.check("Edit", "old=a new=b");
        assert!(matches!(decision, ToolPermissionDecision::Deny { .. }));
        if let ToolPermissionDecision::Deny { reason } = decision {
            assert!(reason.contains("closed"));
        }
    }

    #[test]
    fn tui_prompter_name() {
        let (tx, _rx) = mpsc::channel();
        let prompter = TuiPermissionPrompter::with_default_timeout(tx);
        assert_eq!(prompter.name(), "tui-interactive");
    }

    #[test]
    fn tui_prompter_debug() {
        let (tx, _rx) = mpsc::channel();
        let prompter = TuiPermissionPrompter::new(tx, Duration::from_secs(30));
        let debug = format!("{prompter:?}");
        assert!(debug.contains("TuiPermissionPrompter"));
        assert!(debug.contains("30"));
    }
}
