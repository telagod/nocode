//! Remote daemon — submit agent tasks to a remote nocode instance via HTTP.
//!
//! Connects to a remote nocode bridge endpoint (`/v1/agents`) to submit
//! tasks and poll for results, enabling distributed agent execution.

use serde::{Deserialize, Serialize};

/// Status of a remote agent task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteTaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Timeout,
}

/// A task submitted to a remote daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteTask {
    pub id: String,
    pub prompt: String,
    pub model: Option<String>,
    pub status: RemoteTaskStatus,
    pub result: Option<String>,
    pub error: Option<String>,
    pub submitted_at: i64,
    pub completed_at: Option<i64>,
}

/// Request to submit a remote agent task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteSubmitRequest {
    pub prompt: String,
    pub model: Option<String>,
    pub timeout_secs: Option<u64>,
}

/// Response from submitting a remote task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteSubmitResponse {
    pub task_id: String,
    pub status: RemoteTaskStatus,
}

/// Response from polling a remote task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemotePollResponse {
    // APPEND_REST
    pub task_id: String,
    pub status: RemoteTaskStatus,
    pub result: Option<String>,
    pub error: Option<String>,
}

/// Remote daemon client — connects to a remote nocode bridge.
pub struct RemoteDaemon {
    base_url: String,
    auth_token: Option<String>,
    timeout_secs: u64,
}

impl RemoteDaemon {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            auth_token: None,
            timeout_secs: 30,
        }
    }

    pub fn with_auth(mut self, token: &str) -> Self {
        self.auth_token = Some(token.to_string());
        self
    }

    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    /// Submit a task to the remote daemon.
    pub fn submit(&self, request: &RemoteSubmitRequest) -> Result<RemoteSubmitResponse, String> {
        let url = format!("{}/v1/agents/submit", self.base_url);
        let client = self.build_client()?;
        let mut req = client.post(&url).json(request);
        if let Some(token) = &self.auth_token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }
        let resp = req.send().map_err(|e| format!("submit error: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }
        resp.json().map_err(|e| format!("parse error: {e}"))
    }

    /// Poll a task's status and result.
    pub fn poll(&self, task_id: &str) -> Result<RemotePollResponse, String> {
        let url = format!("{}/v1/agents/{task_id}", self.base_url);
        let client = self.build_client()?;
        let mut req = client.get(&url);
        if let Some(token) = &self.auth_token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }
        let resp = req.send().map_err(|e| format!("poll error: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }
        resp.json().map_err(|e| format!("parse error: {e}"))
    }

    /// Cancel a remote task.
    pub fn cancel(&self, task_id: &str) -> Result<(), String> {
        let url = format!("{}/v1/agents/{task_id}/cancel", self.base_url);
        let client = self.build_client()?;
        let mut req = client.post(&url);
        if let Some(token) = &self.auth_token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }
        let resp = req.send().map_err(|e| format!("cancel error: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }
        Ok(())
    }

    /// List all tasks on the remote daemon.
    pub fn list_tasks(&self) -> Result<Vec<RemoteTask>, String> {
        let url = format!("{}/v1/agents", self.base_url);
        let client = self.build_client()?;
        let mut req = client.get(&url);
        if let Some(token) = &self.auth_token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }
        let resp = req.send().map_err(|e| format!("list error: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }
        resp.json().map_err(|e| format!("parse error: {e}"))
    }

    /// Submit and wait for completion (blocking poll loop).
    pub fn submit_and_wait(
        &self,
        request: &RemoteSubmitRequest,
        poll_interval_ms: u64,
        max_polls: u32,
    ) -> Result<RemotePollResponse, String> {
        let submit_resp = self.submit(request)?;
        let task_id = &submit_resp.task_id;

        for _ in 0..max_polls {
            std::thread::sleep(std::time::Duration::from_millis(poll_interval_ms));
            let poll_resp = self.poll(task_id)?;
            match poll_resp.status {
                RemoteTaskStatus::Completed
                | RemoteTaskStatus::Failed
                | RemoteTaskStatus::Timeout => return Ok(poll_resp),
                _ => continue,
            }
        }
        Err(format!(
            "task '{task_id}' did not complete within {max_polls} polls"
        ))
    }

    fn build_client(&self) -> Result<reqwest::blocking::Client, String> {
        reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(self.timeout_secs))
            .user_agent("nocode-remote-daemon")
            .build()
            .map_err(|e| format!("client error: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_task_status_serde() {
        for status in &[
            RemoteTaskStatus::Pending,
            RemoteTaskStatus::Running,
            RemoteTaskStatus::Completed,
            RemoteTaskStatus::Failed,
            RemoteTaskStatus::Timeout,
        ] {
            let json = serde_json::to_string(status).unwrap();
            let parsed: RemoteTaskStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(&parsed, status);
        }
    }

    #[test]
    fn submit_request_serialization() {
        let req = RemoteSubmitRequest {
            prompt: "find all bugs".to_string(),
            model: Some("claude-sonnet-4-20250514".to_string()),
            timeout_secs: Some(120),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("find all bugs"));
        let parsed: RemoteSubmitRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.prompt, "find all bugs");
        assert_eq!(parsed.timeout_secs, Some(120));
    }

    #[test]
    fn submit_response_deserialization() {
        let json = r#"{"task_id":"task-42","status":"pending"}"#;
        let resp: RemoteSubmitResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.task_id, "task-42");
        assert_eq!(resp.status, RemoteTaskStatus::Pending);
    }

    #[test]
    fn poll_response_deserialization() {
        let json = r#"{"task_id":"task-42","status":"completed","result":"done","error":null}"#;
        let resp: RemotePollResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status, RemoteTaskStatus::Completed);
        assert_eq!(resp.result.as_deref(), Some("done"));
        assert!(resp.error.is_none());
    }

    #[test]
    fn remote_task_full_deserialization() {
        let json = r#"{
            "id": "task-1",
            "prompt": "test prompt",
            "model": null,
            "status": "running",
            "result": null,
            "error": null,
            "submitted_at": 1700000000,
            "completed_at": null
        }"#;
        let task: RemoteTask = serde_json::from_str(json).unwrap();
        assert_eq!(task.id, "task-1");
        assert_eq!(task.status, RemoteTaskStatus::Running);
        assert!(task.completed_at.is_none());
    }

    #[test]
    fn daemon_builder() {
        let daemon = RemoteDaemon::new("https://remote.example.com/")
            .with_auth("token-123")
            .with_timeout(60);
        assert_eq!(daemon.base_url, "https://remote.example.com");
        assert_eq!(daemon.auth_token.as_deref(), Some("token-123"));
        assert_eq!(daemon.timeout_secs, 60);
    }

    #[test]
    fn daemon_submit_fails_no_server() {
        let daemon = RemoteDaemon::new("http://127.0.0.1:1").with_timeout(1);
        let req = RemoteSubmitRequest {
            prompt: "test".to_string(),
            model: None,
            timeout_secs: None,
        };
        assert!(daemon.submit(&req).is_err());
    }
}
