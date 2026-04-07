//! TUI-aware permission bridge that sends tool permission decisions
//! to the TUI overlay via mpsc channels.

use std::sync::mpsc;
use std::time::Duration;

/// A permission request sent to the TUI overlay for interactive approval.
#[derive(Debug)]
pub struct TuiPermissionRequest {
    pub tool_name: String,
    pub arguments_summary: String,
    pub response_tx: mpsc::Sender<bool>,
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

    /// Check permission for a tool call. Blocks until user responds or timeout.
    /// Returns `true` if approved, `false` if denied/timeout/error.
    pub fn check(&self, tool_name: &str, arguments_summary: &str) -> bool {
        let (response_tx, response_rx) = mpsc::channel();
        let request = TuiPermissionRequest {
            tool_name: tool_name.to_string(),
            arguments_summary: arguments_summary.to_string(),
            response_tx,
        };

        if self.tx.send(request).is_err() {
            return false;
        }

        match response_rx.recv_timeout(self.timeout) {
            Ok(approved) => approved,
            Err(_) => false,
        }
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
            req.response_tx.send(true).unwrap();
        });

        assert!(bridge.check("Bash", "command=ls"));
    }

    #[test]
    fn bridge_denied() {
        let (tx, rx) = mpsc::channel();
        let bridge = TuiPermissionBridge::new(tx, Duration::from_secs(5));

        thread::spawn(move || {
            let req = rx.recv().unwrap();
            req.response_tx.send(false).unwrap();
        });

        assert!(!bridge.check("Write", "file_path=foo.rs"));
    }

    #[test]
    fn bridge_timeout() {
        let (tx, _rx) = mpsc::channel();
        let bridge = TuiPermissionBridge::new(tx, Duration::from_millis(50));
        assert!(!bridge.check("Agent", "prompt=test"));
    }

    #[test]
    fn bridge_channel_closed() {
        let (tx, rx) = mpsc::channel();
        drop(rx);
        let bridge = TuiPermissionBridge::new(tx, Duration::from_secs(5));
        assert!(!bridge.check("Edit", "old=a new=b"));
    }
}
