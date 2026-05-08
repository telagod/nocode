use crate::provider::types::{ErrorKind, ProviderError};
use futures::StreamExt;
use reqwest::{Client as AsyncClient, blocking::Client as BlockingClient};
use std::io;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{
    Arc,
    mpsc::{self, Receiver, RecvTimeoutError},
};
use std::time::{Duration, Instant};

pub const STREAM_READ_POLL_INTERVAL: Duration = Duration::from_millis(250);
pub const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
pub const USER_CANCELLED_MESSAGE: &str = "Cancelled by user";
const HTTP_TIMEOUT: Duration = Duration::from_secs(120);

enum StreamBridgeMessage {
    Chunk(Vec<u8>),
    Error(ProviderError),
}

/// HTTP transport layer for provider API calls.
pub struct HttpTransport {
    client: BlockingClient,
    async_client: AsyncClient,
    base_url: String,
    api_key: String,
    extra_headers: Vec<(String, String)>,
}

impl HttpTransport {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        let client = BlockingClient::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .expect("HTTP client should build");
        let async_client = AsyncClient::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .expect("HTTP client should build");
        Self {
            client,
            async_client,
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
    ) -> Result<StreamingResponseReader, ProviderError> {
        self.post_json_stream_cancellable(path, body, None)
    }

    /// POST JSON and return a streaming response reader that can be
    /// cancelled while blocked on network reads.
    pub fn post_json_stream_cancellable(
        &self,
        path: &str,
        body: &str,
        cancel_token: Option<Arc<AtomicBool>>,
    ) -> Result<StreamingResponseReader, ProviderError> {
        if let Some(cancel_token) = cancel_token {
            return self.post_json_stream_abortable(path, body, cancel_token);
        }

        self.post_json_stream_blocking(path, body, None)
    }

    fn post_json_stream_blocking(
        &self,
        path: &str,
        body: &str,
        cancel_token: Option<Arc<AtomicBool>>,
    ) -> Result<StreamingResponseReader, ProviderError> {
        let url = format!("{}{path}", self.base_url);
        let mut req = self
            .client
            .post(&url)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {}", self.api_key));

        for (k, v) in &self.extra_headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req.body(body.to_string()).send().map_err(map_http_error)?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().unwrap_or_default();
            return Err(provider_error_from_status(status.as_u16(), &text));
        }

        Ok(StreamingResponseReader::Blocking(CancelableReader::new(
            resp,
            cancel_token,
        )))
    }

    fn post_json_stream_abortable(
        &self,
        path: &str,
        body: &str,
        cancel_token: Arc<AtomicBool>,
    ) -> Result<StreamingResponseReader, ProviderError> {
        let url = format!("{}{path}", self.base_url);
        let body = body.to_string();
        let api_key = self.api_key.clone();
        let extra_headers = self.extra_headers.clone();
        let client = self.async_client.clone();
        let cancel_for_reader = Arc::clone(&cancel_token);
        let cancel_for_worker = Arc::clone(&cancel_token);
        let (status_tx, status_rx) = mpsc::sync_channel(1);
        let (chunk_tx, chunk_rx) = mpsc::channel();

        std::thread::Builder::new()
            .name("nocode-http-stream".to_string())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();

                let Ok(runtime) = runtime else {
                    let _ = status_tx.send(Err(ProviderError::network_error(
                        "Failed to start async stream runtime",
                    )));
                    return;
                };

                runtime.block_on(async move {
                    let mut request = client
                        .post(&url)
                        .header("content-type", "application/json")
                        .header("authorization", format!("Bearer {api_key}"));

                    for (key, value) in &extra_headers {
                        request = request.header(key.as_str(), value.as_str());
                    }

                    let request = match request.body(body).build() {
                        Ok(request) => request,
                        Err(error) => {
                            let _ = status_tx.send(Err(ProviderError::network_error(format!(
                                "Failed to build request: {error}"
                            ))));
                            return;
                        }
                    };

                    let mut send_future = std::boxed::Box::pin(client.execute(request));
                    let response = loop {
                        if cancel_for_worker.load(Ordering::Relaxed) {
                            let _ = status_tx.send(Err(ProviderError::non_retryable(
                                USER_CANCELLED_MESSAGE,
                            )));
                            return;
                        }

                        tokio::select! {
                            biased;
                            _ = tokio::time::sleep(STREAM_READ_POLL_INTERVAL) => {
                                if cancel_for_worker.load(Ordering::Relaxed) {
                                    let _ = status_tx.send(Err(ProviderError::non_retryable(
                                        USER_CANCELLED_MESSAGE,
                                    )));
                                    return;
                                }
                            }
                            response = &mut send_future => {
                                match response {
                                    Ok(response) => break response,
                                    Err(error) => {
                                        let _ = status_tx.send(Err(map_http_error(error)));
                                        return;
                                    }
                                }
                            }
                        }
                    };

                    let status = response.status();
                    if !status.is_success() {
                        let text = response.text().await.unwrap_or_default();
                        let _ = status_tx.send(Err(provider_error_from_status(status.as_u16(), &text)));
                        return;
                    }

                    let _ = status_tx.send(Ok(()));

                    let mut stream = response.bytes_stream();
                    loop {
                        if cancel_for_worker.load(Ordering::Relaxed) {
                            return;
                        }

                        tokio::select! {
                            biased;
                            _ = tokio::time::sleep(STREAM_READ_POLL_INTERVAL) => {
                                if cancel_for_worker.load(Ordering::Relaxed) {
                                    return;
                                }
                            }
                            maybe_chunk = stream.next() => {
                                match maybe_chunk {
                                    Some(Ok(bytes)) => {
                                        if chunk_tx.send(StreamBridgeMessage::Chunk(bytes.to_vec())).is_err() {
                                            return;
                                        }
                                    }
                                    Some(Err(error)) => {
                                        if !cancel_for_worker.load(Ordering::Relaxed) {
                                            let _ = chunk_tx.send(StreamBridgeMessage::Error(map_http_error(error)));
                                        }
                                        return;
                                    }
                                    None => return,
                                }
                            }
                        }
                    }
                });
            })
            .map_err(|error| ProviderError::network_error(format!("Failed to spawn stream bridge: {error}")))?;

        match status_rx.recv() {
            Ok(Ok(())) => Ok(StreamingResponseReader::Async(ChannelStreamReader::new(
                chunk_rx,
                Some(cancel_for_reader),
            ))),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(ProviderError::network_error(
                "Stream bridge stopped before response start",
            )),
        }
    }

    /// GET a URL and return the response body as string.
    pub fn get(&self, path: &str) -> Result<String, ProviderError> {
        let url = format!("{}{path}", self.base_url);
        let mut req = self
            .client
            .get(&url)
            .header("authorization", format!("Bearer {}", self.api_key));

        for (k, v) in &self.extra_headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req.send().map_err(|e| {
            if e.is_timeout() {
                ProviderError::timeout(format!("HTTP timeout: {e}"))
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
}

fn map_http_error(error: reqwest::Error) -> ProviderError {
    if error.is_timeout() {
        ProviderError::timeout(format!("HTTP timeout: {error}"))
    } else if error.is_connect() {
        ProviderError::network_error(format!("Connection error: {error}"))
    } else {
        ProviderError::network_error(format!("HTTP error: {error}"))
    }
}

fn provider_error_from_status(status: u16, text: &str) -> ProviderError {
    ProviderError::with_status(format!("API error ({status}): {text}"), false, status)
}

pub fn is_cancelled_message(message: &str) -> bool {
    message.contains(USER_CANCELLED_MESSAGE)
}

pub fn map_stream_read_error(error: io::Error) -> ProviderError {
    match error.kind() {
        io::ErrorKind::Interrupted => ProviderError::non_retryable(USER_CANCELLED_MESSAGE),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => {
            ProviderError::timeout(format!("Stream timed out: {error}"))
        }
        _ => ProviderError::retryable(format!("Stream read error: {error}")),
    }
}

/// A reader wrapper that polls for cooperative cancellation while waiting for
/// bytes from a streaming HTTP response.
pub struct CancelableReader<R> {
    inner: R,
    cancel_token: Option<Arc<AtomicBool>>,
    idle_timeout: Duration,
    last_progress: Instant,
}

impl<R> CancelableReader<R> {
    pub fn new(inner: R, cancel_token: Option<Arc<AtomicBool>>) -> Self {
        Self {
            inner,
            cancel_token,
            idle_timeout: STREAM_IDLE_TIMEOUT,
            last_progress: Instant::now(),
        }
    }

    fn cancellation_requested(&self) -> bool {
        self.cancel_token
            .as_ref()
            .is_some_and(|token| token.load(Ordering::Relaxed))
    }
}

impl<R: Read> Read for CancelableReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.cancellation_requested() {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    USER_CANCELLED_MESSAGE,
                ));
            }

            match self.inner.read(buf) {
                Ok(0) => return Ok(0),
                Ok(n) => {
                    self.last_progress = Instant::now();
                    return Ok(n);
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                    ) =>
                {
                    if self.cancellation_requested() {
                        return Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            USER_CANCELLED_MESSAGE,
                        ));
                    }

                    if self.last_progress.elapsed() >= self.idle_timeout {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            format!(
                                "SSE stream stalled — no data for {}s",
                                self.idle_timeout.as_secs()
                            ),
                        ));
                    }
                }
                Err(error) => return Err(error),
            }
        }
    }
}

pub enum StreamingResponseReader {
    Blocking(CancelableReader<reqwest::blocking::Response>),
    Async(ChannelStreamReader),
}

impl Read for StreamingResponseReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Blocking(reader) => reader.read(buf),
            Self::Async(reader) => reader.read(buf),
        }
    }
}

pub struct ChannelStreamReader {
    rx: Receiver<StreamBridgeMessage>,
    pending: Vec<u8>,
    pending_offset: usize,
    cancel_token: Option<Arc<AtomicBool>>,
    idle_timeout: Duration,
    last_progress: Instant,
}

impl ChannelStreamReader {
    fn new(rx: Receiver<StreamBridgeMessage>, cancel_token: Option<Arc<AtomicBool>>) -> Self {
        Self {
            rx,
            pending: Vec::new(),
            pending_offset: 0,
            cancel_token,
            idle_timeout: STREAM_IDLE_TIMEOUT,
            last_progress: Instant::now(),
        }
    }

    fn cancellation_requested(&self) -> bool {
        self.cancel_token
            .as_ref()
            .is_some_and(|token| token.load(Ordering::Relaxed))
    }
}

impl Read for ChannelStreamReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.cancellation_requested() {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    USER_CANCELLED_MESSAGE,
                ));
            }

            if self.pending_offset < self.pending.len() {
                let available = &self.pending[self.pending_offset..];
                let to_copy = available.len().min(buf.len());
                buf[..to_copy].copy_from_slice(&available[..to_copy]);
                self.pending_offset += to_copy;
                if self.pending_offset >= self.pending.len() {
                    self.pending.clear();
                    self.pending_offset = 0;
                }
                self.last_progress = Instant::now();
                return Ok(to_copy);
            }

            match self.rx.recv_timeout(STREAM_READ_POLL_INTERVAL) {
                Ok(StreamBridgeMessage::Chunk(chunk)) => {
                    self.pending = chunk;
                    self.pending_offset = 0;
                }
                Ok(StreamBridgeMessage::Error(error)) => {
                    return Err(provider_error_to_io(error));
                }
                Err(RecvTimeoutError::Timeout) => {
                    if self.cancellation_requested() {
                        return Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            USER_CANCELLED_MESSAGE,
                        ));
                    }

                    if self.last_progress.elapsed() >= self.idle_timeout {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            format!(
                                "SSE stream stalled — no data for {}s",
                                self.idle_timeout.as_secs()
                            ),
                        ));
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    if self.cancellation_requested() {
                        return Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            USER_CANCELLED_MESSAGE,
                        ));
                    }
                    return Ok(0);
                }
            }
        }
    }
}

fn provider_error_to_io(error: ProviderError) -> io::Error {
    if is_cancelled_message(&error.message) {
        return io::Error::new(io::ErrorKind::Interrupted, error.message);
    }

    let kind = match error.kind {
        ErrorKind::Timeout => io::ErrorKind::TimedOut,
        ErrorKind::ParseError => io::ErrorKind::InvalidData,
        ErrorKind::NetworkError => io::ErrorKind::ConnectionAborted,
        _ => io::ErrorKind::Other,
    };

    io::Error::new(kind, error.message)
}

/// Retry a fallible operation immediately, without introducing backoff waits.
pub fn with_retry<F, T>(max_attempts: u32, mut f: F) -> Result<T, ProviderError>
where
    F: FnMut() -> Result<T, ProviderError>,
{
    let mut last_err = ProviderError::non_retryable("no attempts made");
    for attempt in 0..max_attempts {
        match f() {
            Ok(val) => return Ok(val),
            Err(e) if e.retryable && attempt + 1 < max_attempts => {
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
                Some(Err(e)) => return Err(map_stream_read_error(e)),
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
    use std::io::{Cursor, Write as _};
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };

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

    #[test]
    fn map_stream_read_error_treats_interrupt_as_cancel() {
        let err = std::io::Error::new(std::io::ErrorKind::Interrupted, "cancel");
        let mapped = map_stream_read_error(err);
        assert_eq!(mapped.message, USER_CANCELLED_MESSAGE);
        assert!(!mapped.retryable);
    }

    #[test]
    fn cancelable_reader_stops_before_read_when_cancelled() {
        let token = Arc::new(AtomicBool::new(true));
        let mut reader =
            CancelableReader::new(Cursor::new(Vec::<u8>::new()), Some(Arc::clone(&token)));
        let mut buf = [0_u8; 8];

        let err = reader.read(&mut buf).expect_err("reader should stop");
        assert_eq!(err.kind(), std::io::ErrorKind::Interrupted);
    }

    #[test]
    fn async_stream_bridge_disconnects_server_on_cancel() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let addr = listener.local_addr().expect("server addr");
        let (disconnect_tx, disconnect_rx) = mpsc::sync_channel(1);

        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept client");
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("set read timeout");
            socket
                .set_write_timeout(Some(Duration::from_secs(2)))
                .expect("set write timeout");

            consume_http_request(&mut socket);
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n",
                )
                .expect("write headers");
            write_chunk(&mut socket, b"data: hello\n\n").expect("write first chunk");

            std::thread::sleep(Duration::from_millis(750));

            let mut disconnected = false;
            for _ in 0..10 {
                match write_chunk(&mut socket, b"data: still-here\n\n") {
                    Ok(()) => std::thread::sleep(Duration::from_millis(100)),
                    Err(_) => {
                        disconnected = true;
                        break;
                    }
                }
            }

            let _ = socket.shutdown(Shutdown::Both);
            disconnect_tx.send(disconnected).expect("report disconnect");
        });

        let transport = HttpTransport::new(format!("http://{addr}"), "dummy");
        let cancel_token = Arc::new(AtomicBool::new(false));
        let mut reader = transport
            .post_json_stream_cancellable(
                "/v1/messages",
                "{\"stream\":true}",
                Some(Arc::clone(&cancel_token)),
            )
            .expect("stream bridge should start");

        let mut first = [0_u8; 64];
        let read = reader.read(&mut first).expect("read first chunk");
        assert!(read > 0, "stream should deliver first bytes");

        cancel_token.store(true, Ordering::Relaxed);
        let err = reader
            .read(&mut first)
            .expect_err("cancel should interrupt the reader");
        assert_eq!(err.kind(), std::io::ErrorKind::Interrupted);

        assert!(
            disconnect_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("server disconnect report"),
            "server should observe client disconnect after cancel"
        );

        server.join().expect("server thread should finish");
    }

    fn consume_http_request(socket: &mut TcpStream) {
        let mut received = Vec::new();
        let mut buf = [0_u8; 1024];

        loop {
            let read = socket.read(&mut buf).expect("read request bytes");
            if read == 0 {
                break;
            }
            received.extend_from_slice(&buf[..read]);

            if let Some(headers_end) = find_bytes(&received, b"\r\n\r\n") {
                let content_length = parse_content_length(&received[..headers_end + 4]);
                let expected = headers_end + 4 + content_length;
                while received.len() < expected {
                    let read = socket.read(&mut buf).expect("read request body");
                    if read == 0 {
                        break;
                    }
                    received.extend_from_slice(&buf[..read]);
                }
                return;
            }
        }
    }

    fn parse_content_length(headers: &[u8]) -> usize {
        let headers = String::from_utf8_lossy(headers);
        headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                if name.eq_ignore_ascii_case("content-length") {
                    value.trim().parse::<usize>().ok()
                } else {
                    None
                }
            })
            .unwrap_or(0)
    }

    fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }

    fn write_chunk(socket: &mut TcpStream, payload: &[u8]) -> std::io::Result<()> {
        write!(socket, "{:X}\r\n", payload.len())?;
        socket.write_all(payload)?;
        socket.write_all(b"\r\n")?;
        socket.flush()
    }
}
