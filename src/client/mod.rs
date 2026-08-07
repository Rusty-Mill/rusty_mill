//! An async client for calling A2A agents over the JSON-RPC 2.0 protocol
//! binding (spec Section 9), including SSE streaming.
//!
//! ```no_run
//! # async fn run() -> rusty_a2a::client::Result<()> {
//! use rusty_a2a::client::A2aClient;
//! use rusty_a2a::types::Message;
//!
//! let (client, _card) = A2aClient::discover("https://agent.example.com").await?;
//! let result = client.send_message(Message::user_text("hello!"), None).await?;
//! println!("{result:?}");
//! # Ok(())
//! # }
//! ```
use std::pin::Pin;
use std::sync::atomic::{AtomicI64, Ordering};

use eventsource_stream::Eventsource;
use futures_core::Stream;
use futures_util::StreamExt;
use reqwest::RequestBuilder;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::error::A2aError;
use crate::types::jsonrpc::{methods, JsonRpcRequest, JsonRpcResponse, RequestId};
use crate::types::{
    AgentCard, AgentInterface, CancelTaskRequest, DeleteTaskPushNotificationConfigRequest,
    GetExtendedAgentCardRequest, GetTaskPushNotificationConfigRequest, GetTaskRequest,
    ListTaskPushNotificationConfigsRequest, ListTaskPushNotificationConfigsResponse, ListTasksRequest,
    ListTasksResponse, Message, SendMessageConfiguration, SendMessageRequest, SendMessageResult,
    StreamResponse, SubscribeToTaskRequest, Task, TaskPushNotificationConfig,
};

pub mod rest;
pub use rest::RestClient;

#[cfg(feature = "grpc")]
pub mod grpc;
#[cfg(feature = "grpc")]
pub use grpc::GrpcClient;

/// Errors that can occur while acting as an A2A client: transport-level
/// failures (network, JSON encoding) as well as [`A2aError`]s returned by
/// the remote agent.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("HTTP transport error: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("A2A protocol error: {0}")]
    Protocol(#[from] A2aError),
    #[error("failed to (de)serialize JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("event stream error: {0}")]
    Stream(String),
    #[error("agent card declares no JSONRPC interface")]
    NoJsonRpcInterface,
    #[error("agent card declares no HTTP+JSON interface")]
    NoRestInterface,
    #[cfg(feature = "grpc")]
    #[error("agent card declares no GRPC interface")]
    NoGrpcInterface,
    #[cfg(feature = "grpc")]
    #[error("invalid gRPC client configuration: {0}")]
    GrpcConfig(String),
    #[cfg(feature = "grpc")]
    #[error("gRPC error: {0}")]
    Grpc(#[from] tonic::Status),
    #[error("unexpected response (HTTP {status}): {body}")]
    UnexpectedResponse { status: u16, body: String },
}

pub type Result<T> = std::result::Result<T, ClientError>;

/// A client for one A2A agent interface, speaking the JSON-RPC 2.0
/// protocol binding.
pub struct A2aClient {
    http: reqwest::Client,
    endpoint: String,
    tenant: Option<String>,
    bearer_token: Option<String>,
    protocol_version: String,
    extensions: Vec<String>,
    next_id: AtomicI64,
}

impl A2aClient {
    /// Builds a client targeting the given JSON-RPC endpoint URL directly.
    /// Prefer [`A2aClient::discover`] or [`A2aClient::from_agent_card`]
    /// when you have (or can fetch) the agent's `AgentCard`.
    pub fn new(endpoint: impl Into<String>) -> Self {
        A2aClient {
            http: reqwest::Client::new(),
            endpoint: endpoint.into(),
            tenant: None,
            bearer_token: None,
            protocol_version: crate::PROTOCOL_VERSION.to_string(),
            extensions: Vec::new(),
            next_id: AtomicI64::new(1),
        }
    }

    /// Like [`A2aClient::new`], using a caller-provided [`reqwest::Client`]
    /// (e.g. to share connection pools, or configure timeouts/proxies).
    pub fn with_http_client(endpoint: impl Into<String>, http: reqwest::Client) -> Self {
        A2aClient {
            http,
            ..A2aClient::new(endpoint)
        }
    }

    /// Builds a client for the first `JSONRPC` interface declared in
    /// `card.supportedInterfaces` (spec Section 8.3.2).
    pub fn from_agent_card(card: &AgentCard) -> Result<Self> {
        let interface = card
            .interface_for_binding(AgentInterface::JSONRPC)
            .ok_or(ClientError::NoJsonRpcInterface)?;
        let mut client = A2aClient::new(interface.url.clone());
        client.tenant = interface.tenant.clone();
        Ok(client)
    }

    /// Fetches `{base_url}/.well-known/agent-card.json` (spec Section 8.2)
    /// and builds a client from it. `base_url` should be the agent's
    /// origin, e.g. `https://agent.example.com` (no trailing slash
    /// required).
    pub async fn discover(base_url: &str) -> Result<(Self, AgentCard)> {
        let card = Self::fetch_agent_card(base_url).await?;
        let client = Self::from_agent_card(&card)?;
        Ok((client, card))
    }

    /// Fetches an `AgentCard` from its well-known discovery URI without
    /// building a client.
    pub async fn fetch_agent_card(base_url: &str) -> Result<AgentCard> {
        let url = format!(
            "{}{}",
            base_url.trim_end_matches('/'),
            crate::AGENT_CARD_WELL_KNOWN_PATH
        );
        let resp = reqwest::Client::new().get(url).send().await?;
        Self::parse_json_response(resp).await
    }

    async fn parse_json_response<T: DeserializeOwned>(resp: reqwest::Response) -> Result<T> {
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ClientError::UnexpectedResponse {
                status: status.as_u16(),
                body,
            });
        }
        let bytes = resp.bytes().await?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub fn with_bearer_token(mut self, token: impl Into<String>) -> Self {
        self.bearer_token = Some(token.into());
        self
    }

    pub fn with_tenant(mut self, tenant: impl Into<String>) -> Self {
        self.tenant = Some(tenant.into());
        self
    }

    /// Sets the `A2A-Extensions` service parameter sent with every
    /// request (spec Section 3.2.6).
    pub fn with_extensions(mut self, extensions: Vec<String>) -> Self {
        self.extensions = extensions;
        self
    }

    /// Overrides the `A2A-Version` service parameter (defaults to
    /// [`crate::PROTOCOL_VERSION`]).
    pub fn with_protocol_version(mut self, version: impl Into<String>) -> Self {
        self.protocol_version = version.into();
        self
    }

    fn apply_headers(&self, mut builder: RequestBuilder) -> RequestBuilder {
        builder = builder.header("A2A-Version", &self.protocol_version);
        if !self.extensions.is_empty() {
            builder = builder.header("A2A-Extensions", self.extensions.join(","));
        }
        if let Some(token) = &self.bearer_token {
            builder = builder.bearer_auth(token);
        }
        builder
    }

    fn next_request_id(&self) -> RequestId {
        RequestId::Number(self.next_id.fetch_add(1, Ordering::Relaxed))
    }

    async fn call<P: Serialize, R: DeserializeOwned>(&self, method: &str, params: P) -> Result<R> {
        let id = self.next_request_id();
        let request = JsonRpcRequest::new(id, method, serde_json::to_value(params)?);
        let builder = self.apply_headers(self.http.post(&self.endpoint).json(&request));
        let resp = builder.send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ClientError::UnexpectedResponse {
                status: status.as_u16(),
                body,
            });
        }
        let body: JsonRpcResponse = resp.json().await?;
        let value = body.into_result()?;
        Ok(serde_json::from_value(value)?)
    }

    async fn call_streaming(
        &self,
        method: &str,
        params: impl Serialize,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamResponse>> + Send>>> {
        let id = self.next_request_id();
        let request = JsonRpcRequest::new(id, method, serde_json::to_value(params)?);
        let builder = self.apply_headers(self.http.post(&self.endpoint).json(&request));
        let resp = builder.send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ClientError::UnexpectedResponse {
                status: status.as_u16(),
                body,
            });
        }

        // A request that fails before streaming begins (e.g. `streaming`
        // capability not declared, or a task that's already terminal) comes
        // back as an ordinary JSON-RPC error response, not an SSE stream
        // (spec Section 9.2 gives no explicit rule here; this mirrors how
        // this crate's own server behaves - see `router::jsonrpc_handler`).
        // Surface that as an immediate error rather than silently handing
        // an SSE parser a non-SSE body, which would just look like an
        // empty stream.
        let is_event_stream = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|ct| ct.starts_with("text/event-stream"));
        if !is_event_stream {
            let body: JsonRpcResponse = Self::parse_json_response(resp).await?;
            return match body.into_result() {
                Ok(_) => Err(ClientError::UnexpectedResponse {
                    status: status.as_u16(),
                    body: "expected an SSE stream but got a non-streaming success response".to_string(),
                }),
                Err(e) => Err(ClientError::Protocol(e)),
            };
        }

        let events = resp.bytes_stream().eventsource();
        Ok(Box::pin(events.filter_map(|event| async move {
            let event = match event {
                Ok(e) => e,
                Err(e) => return Some(Err(ClientError::Stream(e.to_string()))),
            };
            if event.data.is_empty() {
                return None;
            }
            let parsed: JsonRpcResponse = match serde_json::from_str(&event.data) {
                Ok(v) => v,
                Err(e) => return Some(Err(ClientError::Json(e))),
            };
            match parsed.into_result() {
                Ok(value) => match serde_json::from_value::<StreamResponse>(value) {
                    Ok(sr) => Some(Ok(sr)),
                    Err(e) => Some(Err(ClientError::Json(e))),
                },
                Err(e) => Some(Err(ClientError::Protocol(e))),
            }
        })))
    }

    /// `SendMessage` (spec Section 3.1.1). Blocks until the task reaches a
    /// terminal/interrupted state, unless `configuration.returnImmediately`
    /// is set.
    pub async fn send_message(
        &self,
        message: Message,
        configuration: Option<SendMessageConfiguration>,
    ) -> Result<SendMessageResult> {
        let req = SendMessageRequest {
            tenant: self.tenant.clone(),
            message,
            configuration,
            metadata: None,
        };
        self.call(methods::SEND_MESSAGE, req).await
    }

    /// `SendStreamingMessage` (spec Section 3.1.2): sends a message and
    /// streams `Task`/`Message`/status/artifact updates via SSE as the
    /// agent produces them.
    pub async fn send_streaming_message(
        &self,
        message: Message,
        configuration: Option<SendMessageConfiguration>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamResponse>> + Send>>> {
        let req = SendMessageRequest {
            tenant: self.tenant.clone(),
            message,
            configuration,
            metadata: None,
        };
        self.call_streaming(methods::SEND_STREAMING_MESSAGE, req).await
    }

    /// `GetTask` (spec Section 3.1.3).
    pub async fn get_task(&self, id: impl Into<String>, history_length: Option<i32>) -> Result<Task> {
        let req = GetTaskRequest {
            tenant: self.tenant.clone(),
            id: id.into(),
            history_length,
        };
        self.call(methods::GET_TASK, req).await
    }

    /// `ListTasks` (spec Section 3.1.4).
    pub async fn list_tasks(&self, mut req: ListTasksRequest) -> Result<ListTasksResponse> {
        req.tenant = self.tenant.clone();
        self.call(methods::LIST_TASKS, req).await
    }

    /// `CancelTask` (spec Section 3.1.5).
    pub async fn cancel_task(&self, id: impl Into<String>) -> Result<Task> {
        let req = CancelTaskRequest {
            tenant: self.tenant.clone(),
            id: id.into(),
            metadata: None,
        };
        self.call(methods::CANCEL_TASK, req).await
    }

    /// `SubscribeToTask` (spec Section 3.1.6): streams updates for a task
    /// that is not (yet) in a terminal state.
    pub async fn subscribe_to_task(
        &self,
        id: impl Into<String>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamResponse>> + Send>>> {
        let req = SubscribeToTaskRequest {
            tenant: self.tenant.clone(),
            id: id.into(),
        };
        self.call_streaming(methods::SUBSCRIBE_TO_TASK, req).await
    }

    /// `CreateTaskPushNotificationConfig` (spec Section 3.1.7).
    pub async fn create_push_notification_config(
        &self,
        mut config: TaskPushNotificationConfig,
    ) -> Result<TaskPushNotificationConfig> {
        config.tenant = self.tenant.clone();
        self.call(methods::CREATE_TASK_PUSH_NOTIFICATION_CONFIG, config)
            .await
    }

    /// `GetTaskPushNotificationConfig` (spec Section 3.1.8).
    pub async fn get_push_notification_config(
        &self,
        task_id: impl Into<String>,
        id: impl Into<String>,
    ) -> Result<TaskPushNotificationConfig> {
        let req = GetTaskPushNotificationConfigRequest {
            tenant: self.tenant.clone(),
            task_id: task_id.into(),
            id: id.into(),
        };
        self.call(methods::GET_TASK_PUSH_NOTIFICATION_CONFIG, req).await
    }

    /// `ListTaskPushNotificationConfigs` (spec Section 3.1.9).
    pub async fn list_push_notification_configs(
        &self,
        task_id: impl Into<String>,
    ) -> Result<ListTaskPushNotificationConfigsResponse> {
        let req = ListTaskPushNotificationConfigsRequest {
            tenant: self.tenant.clone(),
            task_id: task_id.into(),
            page_size: None,
            page_token: None,
        };
        self.call(methods::LIST_TASK_PUSH_NOTIFICATION_CONFIGS, req).await
    }

    /// `DeleteTaskPushNotificationConfig` (spec Section 3.1.10).
    pub async fn delete_push_notification_config(
        &self,
        task_id: impl Into<String>,
        id: impl Into<String>,
    ) -> Result<()> {
        let req = DeleteTaskPushNotificationConfigRequest {
            tenant: self.tenant.clone(),
            task_id: task_id.into(),
            id: id.into(),
        };
        self.call(methods::DELETE_TASK_PUSH_NOTIFICATION_CONFIG, req)
            .await
    }

    /// `GetExtendedAgentCard` (spec Section 3.1.11).
    pub async fn get_extended_agent_card(&self) -> Result<AgentCard> {
        let req = GetExtendedAgentCardRequest {
            tenant: self.tenant.clone(),
        };
        self.call(methods::GET_EXTENDED_AGENT_CARD, req).await
    }
}
