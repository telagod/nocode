//! TUI-aware permission bridge that sends tool permission decisions
//! to the TUI overlay via mpsc channels.

use nocode_core::tool::permission::{PermissionDecision, PermissionPrompter};
use std::sync::mpsc;
use std::time::Duration;

/// A permission request sent to the TUI overlay for interactive approval.
#[derive(Debug)]
pub struct TuiPermissionRequest {
    pub tool_name: String,
    pub arguments_summary: String,
    pub response_tx: mpsc::Sender<PermissionDecision>,
}

/// Permission bridge that sends requests to the TUI overlay
/// and blocks until the user approves or denies.
pub struct TuiPermissionBridge {
    tx: mpsc::Sender<TuiPermissionRequest>,
    timeout: Duration,
}

impl TuiPermissionBridge {
    pub fn new(tx: mpsc::Sender<TuiPermissionRequest>, timeout: Duration) -> Self {
        Self { tx, timeout }
    }

    /// Create with default 60s timeout.
    pub fn with_default_timeout(tx: mpsc::Sender<TuiPermissionRequest>) -> Self {
        Self::new(tx, Duration::from_secs(60))
    }
}

impl PermissionPrompter for TuiPermissionBridge {
    fn prompt(&self, tool_name: &str, arguments_summary: &str) -> PermissionDecision {
        let (response_tx, response_rx) = mpsc::channel();
        let request = TuiPermissionRequest {
            tool_name: tool_name.to_string(),
            arguments_summary: arguments_summary.to_string(),
            response_tx,
        };

        if self.tx.send(request).is_err() {
            return PermissionDecision::Deny;
        }

        response_rx
            .recv_timeout(self.timeout)
            .unwrap_or(PermissionDecision::Deny)
    }
}

impl std::fmt::Debug for TuiPermissionBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TuiPermissionBridge")
            .field("timeout", &self.timeout)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn bridge_approved() {
        let (tx, rx) = mpsc::channel();
        let bridge = TuiPermissionBridge::new(tx, Duration::from_secs(5));

        thread::spawn(move || {
            let req = rx.recv().unwrap();
            assert_eq!(req.tool_name, "Bash");
            req.response_tx.send(PermissionDecision::Allow).unwrap();
        });

        assert_eq!(
            bridge.prompt("Bash", "command=ls"),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn bridge_denied() {
        let (tx, rx) = mpsc::channel();
        let bridge = TuiPermissionBridge::new(tx, Duration::from_secs(5));

        thread::spawn(move || {
            let req = rx.recv().unwrap();
            req.response_tx.send(PermissionDecision::Deny).unwrap();
        });

        assert_eq!(
            bridge.prompt("Write", "file_path=foo.rs"),
            PermissionDecision::Deny
        );
    }

    #[test]
    fn bridge_always_allow() {
        let (tx, rx) = mpsc::channel();
        let bridge = TuiPermissionBridge::new(tx, Duration::from_secs(5));

        thread::spawn(move || {
            let req = rx.recv().unwrap();
            req.response_tx
                .send(PermissionDecision::AlwaysAllow)
                .unwrap();
        });

        assert_eq!(
            bridge.prompt("Edit", "old=a new=b"),
            PermissionDecision::AlwaysAllow
        );
    }

    #[test]
    fn bridge_timeout() {
        let (tx, _rx) = mpsc::channel();
        let bridge = TuiPermissionBridge::new(tx, Duration::from_millis(50));
        assert_eq!(
            bridge.prompt("Agent", "prompt=test"),
            PermissionDecision::Deny
        );
    }

    #[test]
    fn bridge_channel_closed() {
        let (tx, rx) = mpsc::channel();
        drop(rx);
        let bridge = TuiPermissionBridge::new(tx, Duration::from_secs(5));
        assert_eq!(
            bridge.prompt("Edit", "old=a new=b"),
            PermissionDecision::Deny
        );
    }
}
