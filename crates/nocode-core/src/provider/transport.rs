use crate::provider::types::ProviderError;
use reqwest::blocking::Client;
use std::time::Duration;

/// HTTP transport layer for provider API calls.
pub struct HttpTransport {
    client: Client,
    base_url: String,
    api_key: String,
    extra_headers: Vec<(String, String)>,
}

impl HttpTransport {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("HTTP client should build");
        Self {
            client,
            base_url: base_url.into(),
            api_key: api_key.into(),
            extra_headers: Vec::new(),
        }
    }

    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_headers.push((key.into(), value.into()));
        self
    }

    /// POST JSON and return the response body as string.
    pub fn post_json(&self, path: &str, body: &str) -> Result<String, ProviderError> {
        let url = format!("{}{path}", self.base_url);
        let mut req = self
            .client
            .post(&url)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {}", self.api_key));

        for (k, v) in &self.extra_headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req.body(body.to_string()).send().map_err(|e| {
            if e.is_timeout() {
                ProviderError::timeout(format!("HTTP timeout: {e}"))
            } else if e.is_connect() {
                ProviderError::network_error(format!("Connection error: {e}"))
            } else {
                ProviderError::network_error(format!("HTTP error: {e}"))
            }
        })?;

        let status = resp.status();
        let text = resp.text().map_err(|e| {
            ProviderError::parse_error(format!("Failed to read response body: {e}"))
        })?;

        if status.is_success() {
            Ok(text)
        } else {
            Err(ProviderError::with_status(
                format!("API error ({status}): {text}"),
                false,
                status.as_u16(),
            ))
        }
    }

    /// POST JSON and return a streaming response reader for SSE.
    pub fn post_json_stream(
        &self,
        path: &str,
        body: &str,
    ) -> Result<impl std::io::Read, ProviderError> {
        let url = format!("{}{path}", self.base_url);
        let mut req = self
            .client
            .post(&url)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {}", self.api_key));

        for (k, v) in &self.extra_headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req.body(body.to_string()).send().map_err(|e| {
            if e.is_timeout() {
                ProviderError::timeout(format!("HTTP timeout: {e}"))
            } else if e.is_connect() {
                ProviderError::network_error(format!("Connection error: {e}"))
            } else {
                ProviderError::network_error(format!("HTTP error: {e}"))
            }
        })?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().unwrap_or_default();
            return Err(ProviderError::with_status(
                format!("API error ({status}): {text}"),
                false,
                status.as_u16(),
            ));
        }

        Ok(resp)
    }
}

/// Retry a fallible operation with exponential backoff.
pub fn with_retry<F, T>(max_attempts: u32, mut f: F) -> Result<T, ProviderError>
where
    F: FnMut() -> Result<T, ProviderError>,
{
    let mut last_err = ProviderError::non_retryable("no attempts made");
    for attempt in 0..max_attempts {
        match f() {
            Ok(val) => return Ok(val),
            Err(e) if e.retryable && attempt + 1 < max_attempts => {
                let delay = Duration::from_millis(500 * 2u64.pow(attempt));
                std::thread::sleep(delay);
                last_err = e;
            }
            Err(e) => return Err(e),
        }
    }
    Err(last_err)
}

// ---------------------------------------------------------------------------
// SSE line parser with stall detection
// ---------------------------------------------------------------------------

/// A parsed SSE frame (event type + data payload).
#[derive(Debug, Clone)]
pub struct SseFrame {
    pub event_type: String,
    pub data: String,
}

/// SSE stream state for line-by-line parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SseStreamState {
    /// Waiting for next event.
    Idle,
    /// Received event type, waiting for data line.
    AwaitingData,
    /// Stream completed (received `data: [DONE]` or EOF).
    Done,
    /// Stream stalled (no data received within timeout).
    Stalled,
}

/// Iterator over SSE frames from a reader, with stall detection.
///
/// Parses `event: <type>` + `data: <json>` pairs from an SSE stream.
/// Detects stalls when no data arrives within `stall_timeout`.
pub struct SseReader<R: std::io::Read> {
    lines: std::io::Lines<std::io::BufReader<R>>,
    state: SseStreamState,
    current_event_type: String,
    stall_timeout: Duration,
    last_data_time: std::time::Instant,
}

impl<R: std::io::Read> SseReader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            lines: std::io::BufReader::new(reader).lines(),
            state: SseStreamState::Idle,
            current_event_type: String::new(),
            stall_timeout: Duration::from_secs(30),
            last_data_time: std::time::Instant::now(),
        }
    }

    pub fn with_stall_timeout(mut self, timeout: Duration) -> Self {
        self.stall_timeout = timeout;
        self
    }

    pub fn state(&self) -> SseStreamState {
        self.state
    }

    /// Check if the stream has stalled.
    pub fn is_stalled(&self) -> bool {
        self.last_data_time.elapsed() > self.stall_timeout
    }

    /// Read the next SSE frame. Returns `None` on EOF or `[DONE]`.
    pub fn next_frame(&mut self) -> Result<Option<SseFrame>, ProviderError> {
        if self.state == SseStreamState::Done {
            return Ok(None);
        }

        loop {
            // Check stall
            if self.is_stalled() {
                self.state = SseStreamState::Stalled;
                return Err(ProviderError::timeout(format!(
                    "SSE stream stalled — no data for {}s",
                    self.stall_timeout.as_secs()
                )));
            }

            let line = match self.lines.next() {
                Some(Ok(l)) => l,
                Some(Err(e)) => {
                    return Err(ProviderError::network_error(format!(
                        "Stream read error: {e}"
                    )));
                }
                None => {
                    self.state = SseStreamState::Done;
                    return Ok(None);
                }
            };

            self.last_data_time = std::time::Instant::now();

            if line.starts_with("event: ") {
                self.current_event_type = line.strip_prefix("event: ").unwrap_or("").to_string();
                self.state = SseStreamState::AwaitingData;
                continue;
            }

            if let Some(data) = line.strip_prefix("data: ") {
                // OpenAI-style done signal
                if data == "[DONE]" {
                    self.state = SseStreamState::Done;
                    return Ok(None);
                }

                let frame = SseFrame {
                    event_type: std::mem::take(&mut self.current_event_type),
                    data: data.to_string(),
                };
                self.state = SseStreamState::Idle;
                return Ok(Some(frame));
            }

            // Empty lines or comments — skip
        }
    }
}

use std::io::BufRead;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn sse_reader_parses_frames() {
        let input = "event: message_start\ndata: {\"id\":1}\n\nevent: ping\ndata: {}\n\n";
        let mut reader = SseReader::new(Cursor::new(input.as_bytes()));

        let f1 = reader.next_frame().unwrap().unwrap();
        assert_eq!(f1.event_type, "message_start");
        assert_eq!(f1.data, "{\"id\":1}");

        let f2 = reader.next_frame().unwrap().unwrap();
        assert_eq!(f2.event_type, "ping");
        assert_eq!(f2.data, "{}");

        assert!(reader.next_frame().unwrap().is_none());
        assert_eq!(reader.state(), SseStreamState::Done);
    }

    #[test]
    fn sse_reader_handles_done_signal() {
        let input = "data: {\"text\":\"hi\"}\n\ndata: [DONE]\n\n";
        let mut reader = SseReader::new(Cursor::new(input.as_bytes()));

        let f1 = reader.next_frame().unwrap().unwrap();
        assert_eq!(f1.data, "{\"text\":\"hi\"}");

        assert!(reader.next_frame().unwrap().is_none());
        assert_eq!(reader.state(), SseStreamState::Done);
    }

    #[test]
    fn sse_reader_skips_empty_lines() {
        let input = "\n\nevent: test\n\ndata: {\"ok\":true}\n\n";
        let mut reader = SseReader::new(Cursor::new(input.as_bytes()));

        let f1 = reader.next_frame().unwrap().unwrap();
        assert_eq!(f1.event_type, "test");
        assert_eq!(f1.data, "{\"ok\":true}");
    }

    #[test]
    fn sse_reader_data_without_event_type() {
        let input = "data: {\"no_event\":true}\n\n";
        let mut reader = SseReader::new(Cursor::new(input.as_bytes()));

        let f1 = reader.next_frame().unwrap().unwrap();
        assert_eq!(f1.event_type, "");
        assert_eq!(f1.data, "{\"no_event\":true}");
    }

    #[test]
    fn retry_succeeds_on_second_attempt() {
        let mut attempt = 0;
        let result = with_retry(3, || {
            attempt += 1;
            if attempt < 2 {
                Err(ProviderError::retryable("transient"))
            } else {
                Ok(42)
            }
        });
        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempt, 2);
    }

    #[test]
    fn retry_fails_non_retryable() {
        let result: Result<i32, _> =
            with_retry(3, || Err(ProviderError::non_retryable("permanent")));
        assert!(result.is_err());
        assert!(!result.unwrap_err().retryable);
    }
}
