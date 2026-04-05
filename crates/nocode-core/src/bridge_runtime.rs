use crate::provider::{
    ModelError, ModelErrorWire, ModelStreamEvent, ModelStreamEventWire, ModelStreamSink,
};
use crate::query_engine::{QueryEngine, QuerySubmissionPlan, SubmitMessageOptions};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeModule;

impl BridgeModule {
    pub const LABEL: &'static str = "bridge-runtime";
    pub const TS_SOURCE: &'static str = "src/bridge/sessionRunner.ts";
    pub const RESPONSIBILITY: &'static str =
        "Owns bridge/session runner orchestration, permission callbacks, and session pointers.";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BridgeMode {
    LocalRepl,
    Remote,
}

impl BridgeMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LocalRepl => "local-repl",
            Self::Remote => "remote",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PermissionCallback {
    AutoApprove,
    AutoDeny { reason: String },
    Remote,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeRequest {
    pub prompt: String,
    pub options: SubmitMessageOptions,
    pub transport_label: String,
    pub permission: PermissionCallback,
}

impl BridgeRequest {
    pub fn approved(prompt: impl Into<String>, transport_label: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            options: SubmitMessageOptions::default(),
            transport_label: transport_label.into(),
            permission: PermissionCallback::AutoApprove,
        }
    }

    pub fn denied(
        prompt: impl Into<String>,
        transport_label: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            prompt: prompt.into(),
            options: SubmitMessageOptions::default(),
            transport_label: transport_label.into(),
            permission: PermissionCallback::AutoDeny {
                reason: reason.into(),
            },
        }
    }

    pub fn remote(prompt: impl Into<String>, transport_label: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            options: SubmitMessageOptions::default(),
            transport_label: transport_label.into(),
            permission: PermissionCallback::Remote,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionPointer {
    pub session_id: String,
    pub cwd: String,
    pub transcript_entries: usize,
    pub history_entries: usize,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeTurnOutcome {
    Submitted(QuerySubmissionPlan),
    PermissionDenied { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeTurn {
    pub mode: BridgeMode,
    pub transport_label: String,
    pub pointer: SessionPointer,
    pub outcome: BridgeTurnOutcome,
}

impl BridgeTurn {
    pub fn summary(&self) -> String {
        match &self.outcome {
            BridgeTurnOutcome::Submitted(plan) => format!(
                "bridge-turn: mode={} transport={} session={} prompt={} response={} transcript={} response-result={}",
                self.mode.as_str(),
                self.transport_label,
                self.pointer.session_id,
                plan.prompt,
                plan.model_response.response_id,
                self.pointer.transcript_entries,
                if plan.response_result.is_some() {
                    "yes"
                } else {
                    "no"
                }
            ),
            BridgeTurnOutcome::PermissionDenied { reason } => format!(
                "bridge-turn: mode={} transport={} denied={}",
                self.mode.as_str(),
                self.transport_label,
                reason
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SubmitMessageOptionsWire {
    pub uuid: Option<String>,
    pub is_meta: bool,
}

impl From<&SubmitMessageOptions> for SubmitMessageOptionsWire {
    fn from(value: &SubmitMessageOptions) -> Self {
        Self {
            uuid: value.uuid.clone(),
            is_meta: value.is_meta,
        }
    }
}

impl From<SubmitMessageOptionsWire> for SubmitMessageOptions {
    fn from(value: SubmitMessageOptionsWire) -> Self {
        Self {
            uuid: value.uuid,
            is_meta: value.is_meta,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeWireRequest {
    pub request_id: String,
    pub mode: BridgeMode,
    pub transport_label: String,
    pub pointer: SessionPointer,
    pub prompt: String,
    pub options: SubmitMessageOptionsWire,
    pub permission: PermissionCallback,
}

impl BridgeWireRequest {
    pub fn from_request(
        request_id: impl Into<String>,
        mode: BridgeMode,
        pointer: SessionPointer,
        request: &BridgeRequest,
    ) -> Self {
        Self {
            request_id: request_id.into(),
            mode,
            transport_label: request.transport_label.clone(),
            pointer,
            prompt: request.prompt.clone(),
            options: SubmitMessageOptionsWire::from(&request.options),
            permission: request.permission.clone(),
        }
    }

    pub fn into_request(self) -> BridgeRequest {
        BridgeRequest {
            prompt: self.prompt,
            options: self.options.into(),
            transport_label: self.transport_label,
            permission: self.permission,
        }
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub fn from_json(value: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRequestWire {
    pub request_id: String,
    pub mode: BridgeMode,
    pub transport_label: String,
    pub pointer: SessionPointer,
    pub prompt: String,
}

impl PermissionRequestWire {
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub fn from_json(value: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionResponseWire {
    pub request_id: String,
    pub approved: bool,
    pub reason: Option<String>,
}

impl PermissionResponseWire {
    pub fn approved(request_id: impl Into<String>) -> Self {
        Self {
            request_id: request_id.into(),
            approved: true,
            reason: None,
        }
    }

    pub fn denied(request_id: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            request_id: request_id.into(),
            approved: false,
            reason: Some(reason.into()),
        }
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub fn from_json(value: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeSubmittedWire {
    pub prompt: String,
    pub prompt_uuid: Option<String>,
    pub is_meta: bool,
    pub response_id: String,
    pub assistant_message: Option<String>,
    #[serde(default)]
    pub stream_events: Vec<ModelStreamEventWire>,
    #[serde(alias = "structured_output")]
    pub response_result: Option<Value>,
    #[serde(default)]
    pub model_error: Option<ModelErrorWire>,
    pub tool_result_count: usize,
    pub output_tokens: u64,
}

impl BridgeSubmittedWire {
    pub fn from_plan(plan: &QuerySubmissionPlan) -> Self {
        Self {
            prompt: plan.prompt.clone(),
            prompt_uuid: plan.prompt_uuid.clone(),
            is_meta: plan.is_meta,
            response_id: plan.model_response.response_id.clone(),
            assistant_message: plan
                .model_response
                .final_assistant_message
                .as_ref()
                .map(|message| message.content.clone()),
            stream_events: plan
                .model_invocation
                .as_ref()
                .map(|invocation| {
                    invocation
                        .stream_events
                        .iter()
                        .map(ModelStreamEventWire::from)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
            response_result: plan.response_result.clone(),
            model_error: plan.model_error.as_ref().map(ModelErrorWire::from),
            tool_result_count: plan.tool_results.len(),
            output_tokens: plan.usage_snapshot.output_tokens,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum BridgeEventPayloadWire {
    Stream { event: ModelStreamEventWire },
    ModelError { error: ModelErrorWire },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeEventWire {
    pub request_id: String,
    pub mode: BridgeMode,
    pub transport_label: String,
    pub pointer: SessionPointer,
    pub sequence: usize,
    pub payload: BridgeEventPayloadWire,
}

impl BridgeEventWire {
    pub fn stream(
        request_id: impl Into<String>,
        mode: BridgeMode,
        transport_label: impl Into<String>,
        pointer: SessionPointer,
        sequence: usize,
        event: &ModelStreamEvent,
    ) -> Self {
        Self {
            request_id: request_id.into(),
            mode,
            transport_label: transport_label.into(),
            pointer,
            sequence,
            payload: BridgeEventPayloadWire::Stream {
                event: ModelStreamEventWire::from(event),
            },
        }
    }

    pub fn model_error(
        request_id: impl Into<String>,
        mode: BridgeMode,
        transport_label: impl Into<String>,
        pointer: SessionPointer,
        sequence: usize,
        error: &ModelError,
    ) -> Self {
        Self {
            request_id: request_id.into(),
            mode,
            transport_label: transport_label.into(),
            pointer,
            sequence,
            payload: BridgeEventPayloadWire::ModelError {
                error: ModelErrorWire::from(error),
            },
        }
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub fn from_json(value: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(value)
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum BridgeTurnOutcomeWire {
    Submitted { submission: BridgeSubmittedWire },
    PermissionDenied { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeTurnWire {
    pub mode: BridgeMode,
    pub transport_label: String,
    pub pointer: SessionPointer,
    pub outcome: BridgeTurnOutcomeWire,
}

impl BridgeTurnWire {
    pub fn from_turn(turn: &BridgeTurn) -> Self {
        let outcome = match &turn.outcome {
            BridgeTurnOutcome::Submitted(plan) => BridgeTurnOutcomeWire::Submitted {
                submission: BridgeSubmittedWire::from_plan(plan),
            },
            BridgeTurnOutcome::PermissionDenied { reason } => {
                BridgeTurnOutcomeWire::PermissionDenied {
                    reason: reason.clone(),
                }
            }
        };
        Self {
            mode: turn.mode,
            transport_label: turn.transport_label.clone(),
            pointer: turn.pointer.clone(),
            outcome,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeWireResponse {
    pub request_id: String,
    pub turn: BridgeTurnWire,
}

impl BridgeWireResponse {
    pub fn from_turn(request_id: impl Into<String>, turn: &BridgeTurn) -> Self {
        Self {
            request_id: request_id.into(),
            turn: BridgeTurnWire::from_turn(turn),
        }
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub fn from_json(value: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeTransportError {
    TransportFailure {
        stage: &'static str,
        message: String,
    },
    PermissionMismatch {
        expected_request_id: String,
        actual_request_id: String,
    },
}

impl BridgeTransportError {
    pub fn transport(stage: &'static str, message: impl Into<String>) -> Self {
        Self::TransportFailure {
            stage,
            message: message.into(),
        }
    }
}

pub trait RemoteBridgeTransport {
    fn publish_request(&mut self, request: &BridgeWireRequest) -> Result<(), BridgeTransportError>;

    fn request_permission(
        &mut self,
        request: &PermissionRequestWire,
    ) -> Result<PermissionResponseWire, BridgeTransportError>;

    fn publish_event(&mut self, event: &BridgeEventWire) -> Result<(), BridgeTransportError>;

    fn publish_response(
        &mut self,
        response: &BridgeWireResponse,
    ) -> Result<(), BridgeTransportError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpRemoteBridgeAuth {
    BearerToken(String),
    Header { name: String, value: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRemoteBridgeTransportConfig {
    pub base_url: String,
    pub request_path: String,
    pub permission_path: String,
    pub event_path: String,
    pub response_path: String,
    pub auth: Option<HttpRemoteBridgeAuth>,
    pub timeout_secs: u64,
}

impl HttpRemoteBridgeTransportConfig {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            request_path: String::from("/v1/bridge/request"),
            permission_path: String::from("/v1/bridge/permission"),
            event_path: String::from("/v1/bridge/event"),
            response_path: String::from("/v1/bridge/response"),
            auth: None,
            timeout_secs: 30,
        }
    }
}

#[derive(Debug)]
pub struct HttpRemoteBridgeTransport {
    config: HttpRemoteBridgeTransportConfig,
    client: Client,
}

impl HttpRemoteBridgeTransport {
    pub fn new(config: HttpRemoteBridgeTransportConfig) -> Result<Self, BridgeTransportError> {
        let timeout_secs = config.timeout_secs.max(1);
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .map_err(|error| BridgeTransportError::transport("http_client", error.to_string()))?;
        Ok(Self { config, client })
    }

    fn post_json(
        &self,
        stage: &'static str,
        path: &str,
        payload: &impl Serialize,
    ) -> Result<reqwest::blocking::Response, BridgeTransportError> {
        let url = self.url(path);
        let mut request = self.client.post(url).json(payload);
        if let Some(auth) = &self.config.auth {
            request = match auth {
                HttpRemoteBridgeAuth::BearerToken(token) => request.bearer_auth(token),
                HttpRemoteBridgeAuth::Header { name, value } => request.header(name, value),
            };
        }
        request
            .send()
            .map_err(|error| BridgeTransportError::transport(stage, error.to_string()))
    }

    fn ensure_success(
        &self,
        stage: &'static str,
        response: reqwest::blocking::Response,
    ) -> Result<reqwest::blocking::Response, BridgeTransportError> {
        if response.status().is_success() {
            return Ok(response);
        }
        let status = response.status();
        let body = response.text().unwrap_or_default();
        let body_preview = if body.len() > 256 {
            format!("{}...", &body[..256])
        } else {
            body
        };
        Err(BridgeTransportError::transport(
            stage,
            format!("unexpected status {status} body={body_preview}"),
        ))
    }

    fn url(&self, path: &str) -> String {
        let base = self.config.base_url.trim_end_matches('/');
        let normalized = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        };
        format!("{base}{normalized}")
    }
}

impl RemoteBridgeTransport for HttpRemoteBridgeTransport {
    fn publish_request(&mut self, request: &BridgeWireRequest) -> Result<(), BridgeTransportError> {
        let response = self.post_json("publish_request", &self.config.request_path, request)?;
        let _ = self.ensure_success("publish_request", response)?;
        Ok(())
    }

    fn request_permission(
        &mut self,
        request: &PermissionRequestWire,
    ) -> Result<PermissionResponseWire, BridgeTransportError> {
        let response =
            self.post_json("request_permission", &self.config.permission_path, request)?;
        let response = self.ensure_success("request_permission", response)?;
        response.json::<PermissionResponseWire>().map_err(|error| {
            BridgeTransportError::transport("request_permission", error.to_string())
        })
    }

    fn publish_event(&mut self, event: &BridgeEventWire) -> Result<(), BridgeTransportError> {
        let response = self.post_json("publish_event", &self.config.event_path, event)?;
        let _ = self.ensure_success("publish_event", response)?;
        Ok(())
    }

    fn publish_response(
        &mut self,
        response: &BridgeWireResponse,
    ) -> Result<(), BridgeTransportError> {
        let response = self.post_json("publish_response", &self.config.response_path, response)?;
        let _ = self.ensure_success("publish_response", response)?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct SessionRunner {
    mode: BridgeMode,
    engine: QueryEngine,
}

impl SessionRunner {
    pub fn new(engine: QueryEngine, mode: BridgeMode) -> Self {
        Self { mode, engine }
    }

    pub fn mode(&self) -> BridgeMode {
        self.mode
    }

    pub fn engine(&self) -> &QueryEngine {
        &self.engine
    }

    pub fn engine_mut(&mut self) -> &mut QueryEngine {
        &mut self.engine
    }

    pub fn session_pointer(&self) -> SessionPointer {
        SessionPointer {
            session_id: self.engine.config().session_id.clone(),
            cwd: self.engine.config().cwd.clone(),
            transcript_entries: self.engine.state().resume_snapshot.transcript.len()
                + self.engine.state().session_persistence.transcript_entries,
            history_entries: self.engine.state().resume_snapshot.history.len()
                + self.engine.state().history_store.pending_count(),
        }
    }

    pub fn run(&mut self, request: BridgeRequest) -> BridgeTurn {
        match request.permission {
            PermissionCallback::AutoApprove => {
                self.submit_turn(request.transport_label, request.prompt, request.options)
            }
            PermissionCallback::AutoDeny { reason } => {
                self.permission_denied_turn(request.transport_label, reason)
            }
            PermissionCallback::Remote => self.permission_denied_turn(
                request.transport_label,
                String::from("remote permission callback requires remote transport"),
            ),
        }
    }

    pub fn run_remote(
        &mut self,
        request: BridgeRequest,
        transport: &mut impl RemoteBridgeTransport,
    ) -> Result<BridgeTurn, BridgeTransportError> {
        let initial_pointer = self.session_pointer();
        let request_id = self.request_id(&initial_pointer);
        let wire_request = BridgeWireRequest::from_request(
            request_id.clone(),
            self.mode,
            initial_pointer.clone(),
            &request,
        );
        transport.publish_request(&wire_request)?;

        let turn = match request.permission {
            PermissionCallback::AutoApprove => self.submit_turn_remote(
                request_id.clone(),
                request.transport_label,
                request.prompt,
                request.options,
                transport,
            )?,
            PermissionCallback::AutoDeny { reason } => {
                self.permission_denied_turn(request.transport_label, reason)
            }
            PermissionCallback::Remote => {
                let permission_request_id = format!("{request_id}:permission");
                let permission_request = PermissionRequestWire {
                    request_id: permission_request_id.clone(),
                    mode: self.mode,
                    transport_label: request.transport_label.clone(),
                    pointer: initial_pointer,
                    prompt: request.prompt.clone(),
                };
                let permission_response = transport.request_permission(&permission_request)?;
                if permission_response.request_id != permission_request_id {
                    return Err(BridgeTransportError::PermissionMismatch {
                        expected_request_id: permission_request_id,
                        actual_request_id: permission_response.request_id,
                    });
                }
                if permission_response.approved {
                    self.submit_turn_remote(
                        request_id.clone(),
                        request.transport_label,
                        request.prompt,
                        request.options,
                        transport,
                    )?
                } else {
                    self.permission_denied_turn(
                        request.transport_label,
                        permission_response.reason.unwrap_or_else(|| {
                            String::from("permission denied by remote transport")
                        }),
                    )
                }
            }
        };

        let wire_response = BridgeWireResponse::from_turn(request_id, &turn);
        transport.publish_response(&wire_response)?;
        Ok(turn)
    }

    fn submit_turn(
        &mut self,
        transport_label: String,
        prompt: String,
        options: SubmitMessageOptions,
    ) -> BridgeTurn {
        let plan = self.engine.submit_message(prompt, options);
        BridgeTurn {
            mode: self.mode,
            transport_label,
            pointer: self.session_pointer(),
            outcome: BridgeTurnOutcome::Submitted(plan),
        }
    }

    fn submit_turn_remote(
        &mut self,
        request_id: String,
        transport_label: String,
        prompt: String,
        options: SubmitMessageOptions,
        transport: &mut impl RemoteBridgeTransport,
    ) -> Result<BridgeTurn, BridgeTransportError> {
        let pointer = self.session_pointer();
        let mut sink = RemoteBridgeStreamSink::new(
            request_id,
            self.mode,
            transport_label.clone(),
            pointer,
            transport,
        );
        let plan = self
            .engine
            .submit_message_with_stream(prompt, options, &mut sink);
        if let Some(error) = sink.failure.take() {
            return Err(error);
        }
        if let Some(error) = plan.model_error.as_ref() {
            sink.publish_model_error(error)?;
        }
        Ok(BridgeTurn {
            mode: self.mode,
            transport_label,
            pointer: self.session_pointer(),
            outcome: BridgeTurnOutcome::Submitted(plan),
        })
    }

    fn permission_denied_turn(&self, transport_label: String, reason: String) -> BridgeTurn {
        BridgeTurn {
            mode: self.mode,
            transport_label,
            pointer: self.session_pointer(),
            outcome: BridgeTurnOutcome::PermissionDenied { reason },
        }
    }

    fn request_id(&self, pointer: &SessionPointer) -> String {
        format!(
            "{}-{}",
            pointer.session_id,
            pointer.transcript_entries + pointer.history_entries + 1
        )
    }
}

struct RemoteBridgeStreamSink<'a, T: RemoteBridgeTransport> {
    request_id: String,
    mode: BridgeMode,
    transport_label: String,
    pointer: SessionPointer,
    sequence: usize,
    transport: &'a mut T,
    failure: Option<BridgeTransportError>,
}

impl<'a, T: RemoteBridgeTransport> RemoteBridgeStreamSink<'a, T> {
    fn new(
        request_id: String,
        mode: BridgeMode,
        transport_label: String,
        pointer: SessionPointer,
        transport: &'a mut T,
    ) -> Self {
        Self {
            request_id,
            mode,
            transport_label,
            pointer,
            sequence: 0,
            transport,
            failure: None,
        }
    }

    fn publish_model_error(&mut self, error: &ModelError) -> Result<(), BridgeTransportError> {
        let event = BridgeEventWire::model_error(
            self.request_id.clone(),
            self.mode,
            self.transport_label.clone(),
            self.pointer.clone(),
            self.sequence,
            error,
        );
        self.sequence += 1;
        self.transport.publish_event(&event)
    }
}

impl<T: RemoteBridgeTransport> ModelStreamSink for RemoteBridgeStreamSink<'_, T> {
    fn push(&mut self, event: ModelStreamEvent) {
        if self.failure.is_some() {
            return;
        }
        let wire = BridgeEventWire::stream(
            self.request_id.clone(),
            self.mode,
            self.transport_label.clone(),
            self.pointer.clone(),
            self.sequence,
            &event,
        );
        self.sequence += 1;
        if let Err(error) = self.transport.publish_event(&wire) {
            self.failure = Some(error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BridgeEventPayloadWire, BridgeEventWire, BridgeMode, BridgeRequest, BridgeSubmittedWire,
        BridgeTransportError, BridgeTurnOutcome, BridgeTurnOutcomeWire, BridgeWireRequest,
        BridgeWireResponse, HttpRemoteBridgeAuth, HttpRemoteBridgeTransport,
        HttpRemoteBridgeTransportConfig, PermissionCallback, PermissionRequestWire,
        PermissionResponseWire, RemoteBridgeTransport, SessionRunner,
    };
    use crate::message::QueryMessage;
    use crate::provider::{
        ModelCallOutput, ModelError, ModelProvider, ModelRequest, ModelStreamSink,
    };
    use crate::query_deps::{CallModel, QueryDeps};
    use crate::query_engine::{QueryEngine, QueryEngineConfig, ThinkingMode};
    use crate::query_loop::TaskBudget;
    use crate::tool_registry::{ToolPermissionContext, ToolRuntimeMode};
    use serde_json::json;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc;
    use std::thread;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct CapturedHttpRequest {
        path: String,
        authorization: Option<String>,
        body: String,
    }

    #[derive(Default)]
    struct RecordingTransport {
        requests: Vec<BridgeWireRequest>,
        permission_requests: Vec<PermissionRequestWire>,
        events: Vec<BridgeEventWire>,
        responses: Vec<BridgeWireResponse>,
        next_permission: Option<PermissionResponseWire>,
    }

    #[derive(Debug)]
    struct StructuredCallModel;

    impl CallModel for StructuredCallModel {
        fn call_model(
            &self,
            request: &ModelRequest,
            _stream: &mut dyn ModelStreamSink,
        ) -> Result<ModelCallOutput, ModelError> {
            let selected_model = request
                .selection
                .selected_model()
                .ok_or_else(ModelError::no_model_configured)?;
            Ok(ModelCallOutput::new(
                request.selection.provider,
                selected_model,
                QueryMessage::assistant("{\"ok\":true,\"source\":\"bridge\"}"),
            )
            .with_response_result(json!({"ok": true, "source": "bridge"})))
        }
    }

    impl RemoteBridgeTransport for RecordingTransport {
        fn publish_request(
            &mut self,
            request: &BridgeWireRequest,
        ) -> Result<(), BridgeTransportError> {
            self.requests.push(request.clone());
            Ok(())
        }

        fn request_permission(
            &mut self,
            request: &PermissionRequestWire,
        ) -> Result<PermissionResponseWire, BridgeTransportError> {
            self.permission_requests.push(request.clone());
            self.next_permission.clone().ok_or_else(|| {
                BridgeTransportError::transport("request_permission", "missing mock response")
            })
        }

        fn publish_event(&mut self, event: &BridgeEventWire) -> Result<(), BridgeTransportError> {
            self.events.push(event.clone());
            Ok(())
        }

        fn publish_response(
            &mut self,
            response: &BridgeWireResponse,
        ) -> Result<(), BridgeTransportError> {
            self.responses.push(response.clone());
            Ok(())
        }
    }

    fn sample_config() -> QueryEngineConfig {
        QueryEngineConfig {
            cwd: String::from("/tmp"),
            session_id: String::from("bridge-session"),
            persist_session: false,
            persist_history: false,
            file_history_enabled: false,
            tools: vec![String::from("Read")],
            tool_runtime_mode: ToolRuntimeMode::Standard,
            tool_permission_context: ToolPermissionContext::default(),
            commands: vec![String::from("/help")],
            mcp_clients: Vec::new(),
            agents: vec![String::from("leader")],
            initial_messages: vec![QueryMessage::system("seed")],
            read_file_cache_entries: 0,
            custom_system_prompt: Some(String::from("custom")),
            append_system_prompt: None,
            model_provider: ModelProvider::Mock,
            user_specified_model: Some(String::from("sonnet")),
            fallback_model: Some(String::from("haiku")),
            model_reasoning_effort: None,
            thinking_mode: ThinkingMode::Adaptive,
            max_turns: Some(2),
            max_budget_usd: None,
            task_budget: Some(TaskBudget { total: 10_000 }),
            json_schema: None,
            verbose: false,
            replay_user_messages: false,
            include_partial_messages: false,
            stream_model_responses: true,
        }
    }

    fn read_http_request(stream: &mut TcpStream) -> CapturedHttpRequest {
        let mut reader = BufReader::new(stream);
        let mut start_line = String::new();
        reader
            .read_line(&mut start_line)
            .expect("request line should be readable");
        let path = start_line
            .split_whitespace()
            .nth(1)
            .expect("request path should exist")
            .to_string();

        let mut authorization = None;
        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            reader
                .read_line(&mut line)
                .expect("header line should be readable");
            if line == "\r\n" {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                let key = name.trim().to_ascii_lowercase();
                let value = value.trim().to_string();
                if key == "authorization" {
                    authorization = Some(value);
                } else if key == "content-length" {
                    content_length = value.parse::<usize>().expect("content-length should parse");
                }
            }
        }

        let mut body = vec![0u8; content_length];
        reader
            .read_exact(&mut body)
            .expect("request body should be readable");
        CapturedHttpRequest {
            path,
            authorization,
            body: String::from_utf8(body).expect("request body should be utf8"),
        }
    }

    fn write_http_response(stream: &mut TcpStream, status: &str, body: &str) {
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .expect("response should be written");
        stream.flush().expect("response should flush");
    }

    fn spawn_http_bridge_server(
        permission_response: PermissionResponseWire,
    ) -> (
        String,
        mpsc::Receiver<Vec<CapturedHttpRequest>>,
        thread::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let address = format!(
            "http://{}",
            listener.local_addr().expect("addr should resolve")
        );
        let (sender, receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            let mut requests = Vec::new();
            for _ in 0..6 {
                let (mut stream, _) = listener.accept().expect("server should accept");
                let request = read_http_request(&mut stream);
                let body = if request.path == "/v1/bridge/permission" {
                    permission_response
                        .to_json()
                        .expect("permission response should serialize")
                } else {
                    String::new()
                };
                let status = if request.path == "/v1/bridge/fail" {
                    "500 Internal Server Error"
                } else if request.path == "/v1/bridge/permission-denied" {
                    "403 Forbidden"
                } else {
                    "200 OK"
                };
                write_http_response(&mut stream, status, &body);
                requests.push(request);
            }
            sender.send(requests).expect("requests should send");
        });
        (address, receiver, handle)
    }

    fn spawn_http_error_server(status: &'static str) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let address = format!(
            "http://{}",
            listener.local_addr().expect("addr should resolve")
        );
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("server should accept");
            let _request = read_http_request(&mut stream);
            write_http_response(&mut stream, status, "{\"error\":\"bridge failed\"}");
        });
        (address, handle)
    }

    #[test]
    fn bridge_wire_request_round_trips_json() {
        let request = BridgeRequest::remote("bridge prompt", "remote-http");
        let wire = BridgeWireRequest::from_request(
            "req-1",
            BridgeMode::Remote,
            super::SessionPointer {
                session_id: String::from("session-1"),
                cwd: String::from("/tmp"),
                transcript_entries: 2,
                history_entries: 1,
            },
            &request,
        );

        let json = wire.to_json().expect("request json should serialize");
        let decoded = BridgeWireRequest::from_json(&json).expect("request json should decode");

        assert_eq!(decoded, wire);
        assert_eq!(decoded.clone().into_request().prompt, "bridge prompt");
        assert_eq!(decoded.permission, PermissionCallback::Remote);
    }

    #[test]
    fn bridge_wire_response_round_trips_json() {
        let engine = QueryEngine::new(sample_config());
        let mut runner = SessionRunner::new(engine, BridgeMode::LocalRepl);
        let turn = runner.run(BridgeRequest::approved("bridge prompt", "repl"));
        let wire = BridgeWireResponse::from_turn("req-1", &turn);

        let json = wire.to_json().expect("response json should serialize");
        let decoded = BridgeWireResponse::from_json(&json).expect("response json should decode");

        assert_eq!(decoded, wire);
        match decoded.turn.outcome {
            BridgeTurnOutcomeWire::Submitted { submission } => {
                assert_eq!(submission.response_id, "resp-1");
                assert_eq!(
                    submission.assistant_message.as_deref(),
                    Some("nocode response: bridge prompt")
                );
                assert_eq!(submission.response_result, None);
            }
            BridgeTurnOutcomeWire::PermissionDenied { .. } => {
                panic!("expected submitted wire turn")
            }
        }
    }

    #[test]
    fn bridge_submitted_wire_keeps_response_result() {
        let deps = QueryDeps::builder()
            .with_call_model(StructuredCallModel)
            .build();
        let engine = QueryEngine::with_deps(sample_config(), deps);
        let mut runner = SessionRunner::new(engine, BridgeMode::LocalRepl);
        let turn = runner.run(BridgeRequest::approved("bridge structured", "repl"));
        let wire = BridgeWireResponse::from_turn("req-structured", &turn);

        match wire.turn.outcome {
            BridgeTurnOutcomeWire::Submitted { submission } => {
                assert_eq!(
                    submission.response_result,
                    Some(json!({"ok": true, "source": "bridge"}))
                );
                assert_eq!(
                    submission.assistant_message.as_deref(),
                    Some("{\"ok\":true,\"source\":\"bridge\"}")
                );
            }
            BridgeTurnOutcomeWire::PermissionDenied { .. } => {
                panic!("expected submitted wire turn")
            }
        }
        assert!(turn.summary().contains("response-result=yes"));
    }

    #[test]
    fn bridge_submitted_wire_accepts_legacy_structured_output_alias() {
        let decoded: BridgeSubmittedWire = serde_json::from_str(
            r#"{
                "prompt":"bridge legacy",
                "prompt_uuid":null,
                "is_meta":false,
                "response_id":"resp-legacy",
                "assistant_message":"{\"ok\":true}",
                "structured_output":{"ok":true,"source":"legacy"},
                "tool_result_count":0,
                "output_tokens":12
            }"#,
        )
        .expect("legacy bridge submitted wire should decode");

        assert_eq!(
            decoded.response_result,
            Some(json!({"ok": true, "source": "legacy"}))
        );
    }

    #[test]
    fn http_remote_bridge_transport_posts_all_wire_messages() {
        let permission = PermissionResponseWire::approved("bridge-session-1:permission");
        let (base_url, receiver, handle) = spawn_http_bridge_server(permission);
        let mut transport = HttpRemoteBridgeTransport::new(HttpRemoteBridgeTransportConfig {
            base_url,
            request_path: String::from("/v1/bridge/request"),
            permission_path: String::from("/v1/bridge/permission"),
            event_path: String::from("/v1/bridge/event"),
            response_path: String::from("/v1/bridge/response"),
            auth: Some(HttpRemoteBridgeAuth::BearerToken(String::from(
                "bridge-token",
            ))),
            timeout_secs: 5,
        })
        .expect("http transport should build");
        let engine = QueryEngine::new(sample_config());
        let mut runner = SessionRunner::new(engine, BridgeMode::Remote);

        let turn = runner
            .run_remote(
                BridgeRequest::remote("remote bridge turn", "remote-http"),
                &mut transport,
            )
            .expect("remote HTTP bridge turn should succeed");

        match &turn.outcome {
            BridgeTurnOutcome::Submitted(plan) => {
                assert_eq!(plan.prompt, "remote bridge turn");
            }
            BridgeTurnOutcome::PermissionDenied { .. } => panic!("expected submission"),
        }

        let requests = receiver
            .recv()
            .expect("server should return captured requests");
        handle.join().expect("server thread should join");
        assert_eq!(requests.len(), 6);
        assert_eq!(requests[0].path, "/v1/bridge/request");
        assert_eq!(requests[1].path, "/v1/bridge/permission");
        assert_eq!(requests[2].path, "/v1/bridge/event");
        assert_eq!(requests[3].path, "/v1/bridge/event");
        assert_eq!(requests[4].path, "/v1/bridge/event");
        assert_eq!(requests[5].path, "/v1/bridge/response");
        for request in &requests {
            assert_eq!(
                request.authorization.as_deref(),
                Some("Bearer bridge-token")
            );
        }
        let published_request =
            BridgeWireRequest::from_json(&requests[0].body).expect("request wire should decode");
        assert_eq!(published_request.prompt, "remote bridge turn");
        let permission_request = PermissionRequestWire::from_json(&requests[1].body)
            .expect("permission request should decode");
        assert_eq!(permission_request.request_id, "bridge-session-1:permission");
        let event =
            BridgeEventWire::from_json(&requests[2].body).expect("event wire should decode");
        match event.payload {
            BridgeEventPayloadWire::Stream { event } => match event {
                crate::provider::ModelStreamEventWire::Start { provider, model } => {
                    assert_eq!(provider, "mock");
                    assert_eq!(model, "sonnet");
                }
                other => panic!("expected start event, got {other:?}"),
            },
            BridgeEventPayloadWire::ModelError { .. } => panic!("expected stream event"),
        }
        let published_response =
            BridgeWireResponse::from_json(&requests[5].body).expect("response wire should decode");
        match published_response.turn.outcome {
            BridgeTurnOutcomeWire::Submitted { submission } => {
                assert_eq!(submission.response_id, "resp-1");
                assert_eq!(submission.stream_events.len(), 3);
            }
            BridgeTurnOutcomeWire::PermissionDenied { .. } => {
                panic!("expected submitted response")
            }
        }
    }

    #[test]
    fn http_remote_bridge_transport_maps_http_failures() {
        let (base_url, handle) = spawn_http_error_server("500 Internal Server Error");
        let mut transport = HttpRemoteBridgeTransport::new(HttpRemoteBridgeTransportConfig {
            base_url,
            request_path: String::from("/v1/bridge/fail"),
            permission_path: String::from("/v1/bridge/permission"),
            event_path: String::from("/v1/bridge/event"),
            response_path: String::from("/v1/bridge/response"),
            auth: None,
            timeout_secs: 5,
        })
        .expect("http transport should build");
        let request = BridgeWireRequest::from_request(
            "req-1",
            BridgeMode::Remote,
            super::SessionPointer {
                session_id: String::from("session-1"),
                cwd: String::from("/tmp"),
                transcript_entries: 0,
                history_entries: 0,
            },
            &BridgeRequest::approved("bridge prompt", "remote-http"),
        );

        let error = transport
            .publish_request(&request)
            .expect_err("non-2xx publish_request should fail");
        handle.join().expect("server thread should join");

        assert_eq!(
            error,
            BridgeTransportError::TransportFailure {
                stage: "publish_request",
                message: String::from(
                    "unexpected status 500 Internal Server Error body={\"error\":\"bridge failed\"}"
                ),
            }
        );
    }

    #[test]
    fn session_runner_submits_prompt_when_permission_allows() {
        let engine = QueryEngine::new(sample_config());
        let mut runner = SessionRunner::new(engine, BridgeMode::LocalRepl);

        let turn = runner.run(BridgeRequest::approved("bridge prompt", "repl"));

        assert_eq!(turn.mode, BridgeMode::LocalRepl);
        assert_eq!(turn.transport_label, "repl");
        match &turn.outcome {
            BridgeTurnOutcome::Submitted(plan) => {
                assert_eq!(plan.prompt, "bridge prompt");
                assert_eq!(
                    plan.model_response
                        .final_assistant_message
                        .as_ref()
                        .map(|message| message.content.as_str()),
                    Some("nocode response: bridge prompt")
                );
            }
            BridgeTurnOutcome::PermissionDenied { .. } => panic!("expected submitted turn"),
        }
        assert!(turn.summary().contains("bridge-turn: mode=local-repl"));
        assert!(turn.summary().contains("response-result=no"));
    }

    #[test]
    fn session_runner_short_circuits_when_permission_denies() {
        let engine = QueryEngine::new(sample_config());
        let mut runner = SessionRunner::new(engine, BridgeMode::Remote);
        let request = BridgeRequest {
            prompt: String::from("blocked"),
            options: crate::query_engine::SubmitMessageOptions::default(),
            transport_label: String::from("remote-http"),
            permission: PermissionCallback::AutoDeny {
                reason: String::from("approval required"),
            },
        };

        let turn = runner.run(request);

        assert_eq!(turn.mode, BridgeMode::Remote);
        match &turn.outcome {
            BridgeTurnOutcome::PermissionDenied { reason } => {
                assert_eq!(reason, "approval required");
            }
            BridgeTurnOutcome::Submitted(_) => panic!("expected denial"),
        }
        assert_eq!(runner.engine().state().completed_turns.len(), 0);
        assert!(turn.summary().contains("denied=approval required"));
    }

    #[test]
    fn session_runner_remote_requests_permission_and_publishes_response() {
        let engine = QueryEngine::new(sample_config());
        let mut runner = SessionRunner::new(engine, BridgeMode::Remote);
        let mut transport = RecordingTransport {
            next_permission: Some(PermissionResponseWire::approved(
                "bridge-session-1:permission",
            )),
            ..RecordingTransport::default()
        };

        let turn = runner
            .run_remote(
                BridgeRequest::remote("remote bridge turn", "remote-http"),
                &mut transport,
            )
            .expect("remote bridge turn should succeed");

        assert_eq!(transport.requests.len(), 1);
        assert_eq!(transport.permission_requests.len(), 1);
        assert_eq!(transport.events.len(), 3);
        assert_eq!(transport.responses.len(), 1);
        assert_eq!(transport.requests[0].request_id, "bridge-session-1");
        assert_eq!(
            transport.permission_requests[0].request_id,
            "bridge-session-1:permission"
        );
        match &transport.events[0].payload {
            BridgeEventPayloadWire::Stream { event } => {
                assert!(matches!(
                    event,
                    crate::provider::ModelStreamEventWire::Start { .. }
                ));
            }
            BridgeEventPayloadWire::ModelError { .. } => panic!("expected stream event"),
        }
        match &turn.outcome {
            BridgeTurnOutcome::Submitted(plan) => {
                assert_eq!(plan.prompt, "remote bridge turn");
            }
            BridgeTurnOutcome::PermissionDenied { .. } => panic!("expected remote submission"),
        }
        match &transport.responses[0].turn.outcome {
            BridgeTurnOutcomeWire::Submitted { submission } => {
                assert_eq!(submission.response_id, "resp-1");
            }
            BridgeTurnOutcomeWire::PermissionDenied { .. } => {
                panic!("expected submitted response wire")
            }
        }
    }

    #[test]
    fn session_runner_remote_rejects_mismatched_permission_callback() {
        let engine = QueryEngine::new(sample_config());
        let mut runner = SessionRunner::new(engine, BridgeMode::Remote);
        let mut transport = RecordingTransport {
            next_permission: Some(PermissionResponseWire::approved("wrong-request-id")),
            ..RecordingTransport::default()
        };

        let error = runner
            .run_remote(
                BridgeRequest::remote("remote bridge turn", "remote-http"),
                &mut transport,
            )
            .expect_err("mismatched permission response should fail");

        assert_eq!(
            error,
            BridgeTransportError::PermissionMismatch {
                expected_request_id: String::from("bridge-session-1:permission"),
                actual_request_id: String::from("wrong-request-id"),
            }
        );
        assert_eq!(transport.responses.len(), 0);
    }

    #[test]
    fn session_runner_remote_publishes_model_error_event_surface() {
        let mut config = sample_config();
        config.json_schema = Some(String::from("{\"type\":\"object\"}"));
        let engine = QueryEngine::new(config);
        let mut runner = SessionRunner::new(engine, BridgeMode::Remote);
        let mut transport = RecordingTransport {
            next_permission: Some(PermissionResponseWire::approved(
                "bridge-session-1:permission",
            )),
            ..RecordingTransport::default()
        };

        let turn = runner
            .run_remote(
                BridgeRequest::remote("remote bridge error", "remote-http"),
                &mut transport,
            )
            .expect("remote bridge turn should complete with model error payload");

        assert_eq!(transport.events.len(), 1);
        match &transport.events[0].payload {
            BridgeEventPayloadWire::ModelError { error } => {
                assert_eq!(error.surface, "configuration");
                assert_eq!(error.kind, "configuration");
            }
            BridgeEventPayloadWire::Stream { .. } => panic!("expected model error event"),
        }
        match &turn.outcome {
            BridgeTurnOutcome::Submitted(plan) => {
                assert!(plan.model_error.is_some());
            }
            BridgeTurnOutcome::PermissionDenied { .. } => panic!("expected submitted turn"),
        }
        match &transport.responses[0].turn.outcome {
            BridgeTurnOutcomeWire::Submitted { submission } => {
                assert!(submission.model_error.is_some());
                assert!(submission.stream_events.is_empty());
            }
            BridgeTurnOutcomeWire::PermissionDenied { .. } => panic!("expected submitted turn"),
        }
    }
}
