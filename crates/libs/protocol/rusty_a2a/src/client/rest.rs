//! An async client for calling A2A agents over the HTTP+JSON/REST protocol
//! binding (spec Section 11), including SSE streaming.
//!
//! Mirrors [`super::A2aClient`]'s API one-for-one (same method names and
//! signatures) but speaks REST instead of JSON-RPC: real HTTP status codes,
//! the `google.rpc.Status` JSON error shape, and raw (non-enveloped)
//! `StreamResponse` SSE events.
//!
//! ```no_run
//! # async fn run() -> rusty_a2a::client::Result<()> {
//! use rusty_a2a::client::RestClient;
//! use rusty_a2a::types::Message;
//!
//! let (client, _card) = RestClient::discover("https://agent.example.com").await?;
//! let result = client.send_message(Message::user_text("hello!"), None).await?;
//! println!("{result:?}");
//! # Ok(())
//! # }
//! ```
use std::pin::Pin;

use eventsource_stream::Eventsource;
use futures_core::Stream;
use futures_util::StreamExt;
use reqwest::{RequestBuilder, StatusCode};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::A2aError;
use crate::types::{
    AgentCard, AgentInterface, ListTaskPushNotificationConfigsResponse, ListTasksRequest, ListTasksResponse,
    Message, SendMessageConfiguration, SendMessageRequest, SendMessageResult, StreamResponse, Task,
    TaskPushNotificationConfig,
};

use super::{A2aClient, ClientError, Result};

/// A client for one A2A agent interface, speaking the HTTP+JSON/REST
/// protocol binding.
pub struct RestClient {
    http: reqwest::Client,
    /// The interface's base URL (its origin), with no trailing slash -
    /// every route in spec Section 11.3 is resolved relative to it.
    base_url: String,
    tenant: Option<String>,
    bearer_token: Option<String>,
    protocol_version: String,
    extensions: Vec<String>,
}

/// The `google.rpc.Status` JSON error shape a REST agent responds with
/// (spec Section 11.6).
#[derive(Debug, Deserialize)]
struct RestErrorBody {
    error: RestErrorInner,
}

#[derive(Debug, Deserialize)]
struct RestErrorInner {
    message: String,
    #[serde(default)]
    details: Vec<RestErrorDetail>,
}

#[derive(Debug, Deserialize)]
struct RestErrorDetail {
    #[serde(default)]
    reason: Option<String>,
}

/// Best-effort mapping from the `google.rpc.Status` error shape back to an
/// [`A2aError`], mirroring
/// [`jsonrpc_error_to_a2a`](crate::types::jsonrpc::jsonrpc_error_to_a2a) for
/// the REST binding: the nine A2A-specific errors round-trip exactly via
/// `details[].reason` (spec Section 11.6); the remaining generic
/// categories fall back to the HTTP status code.
fn rest_error_to_a2a(status: StatusCode, body: RestErrorBody) -> A2aError {
    let reason = body.error.details.first().and_then(|d| d.reason.as_deref());
    let message = body.error.message;
    match reason {
        Some("TASK_NOT_FOUND") => A2aError::TaskNotFound(message),
        Some("TASK_NOT_CANCELABLE") => A2aError::TaskNotCancelable(message),
        Some("PUSH_NOTIFICATION_NOT_SUPPORTED") => A2aError::PushNotificationNotSupported,
        Some("UNSUPPORTED_OPERATION") => A2aError::UnsupportedOperation(message),
        Some("CONTENT_TYPE_NOT_SUPPORTED") => A2aError::ContentTypeNotSupported(message),
        Some("INVALID_AGENT_RESPONSE") => A2aError::InvalidAgentResponse(message),
        Some("EXTENDED_AGENT_CARD_NOT_CONFIGURED") => A2aError::ExtendedAgentCardNotConfigured,
        Some("EXTENSION_SUPPORT_REQUIRED") => A2aError::ExtensionSupportRequired(message),
        Some("VERSION_NOT_SUPPORTED") => A2aError::VersionNotSupported(message),
        _ => match status {
            StatusCode::UNAUTHORIZED => A2aError::Unauthenticated(message),
            StatusCode::FORBIDDEN => A2aError::PermissionDenied(message),
            StatusCode::NOT_FOUND => A2aError::MethodNotFound(message),
            StatusCode::BAD_REQUEST => A2aError::InvalidRequest(message),
            _ => A2aError::Internal(message),
        },
    }
}

impl RestClient {
    /// Builds a client targeting the given REST interface base URL (its
    /// origin) directly. Prefer [`RestClient::discover`] or
    /// [`RestClient::from_agent_card`] when you have (or can fetch) the
    /// agent's `AgentCard`.
    pub fn new(base_url: impl Into<String>) -> Self {
        RestClient {
            http: reqwest::Client::new(),
            base_url: base_url.into(),
            tenant: None,
            bearer_token: None,
            protocol_version: crate::PROTOCOL_VERSION.to_string(),
            extensions: Vec::new(),
        }
    }

    /// Like [`RestClient::new`], using a caller-provided [`reqwest::Client`]
    /// (e.g. to share connection pools, or configure timeouts/proxies).
    pub fn with_http_client(base_url: impl Into<String>, http: reqwest::Client) -> Self {
        RestClient {
            http,
            ..RestClient::new(base_url)
        }
    }

    /// Builds a client for the first `HTTP+JSON` interface declared in
    /// `card.supportedInterfaces` (spec Section 8.3.2).
    pub fn from_agent_card(card: &AgentCard) -> Result<Self> {
        let interface = card
            .interface_for_binding(AgentInterface::HTTP_JSON)
            .ok_or(ClientError::NoRestInterface)?;
        let mut client = RestClient::new(interface.url.clone());
        client.tenant = interface.tenant.clone();
        Ok(client)
    }

    /// Fetches `{base_url}/.well-known/agent-card.json` (spec Section 8.2)
    /// and builds a client from it. `base_url` should be the agent's
    /// origin, e.g. `https://agent.example.com` (no trailing slash
    /// required).
    pub async fn discover(base_url: &str) -> Result<(Self, AgentCard)> {
        let card = A2aClient::fetch_agent_card(base_url).await?;
        let client = Self::from_agent_card(&card)?;
        Ok((client, card))
    }

    /// Like [`RestClient::discover`], but additionally verifies the fetched
    /// `AgentCard` against `trusted_keys` (spec Section 8.4) before
    /// returning it, failing closed - an unsigned card, a card signed only
    /// by an untrusted key, or a tampered card are all rejected with
    /// [`ClientError::AgentCardSignatureInvalid`].
    #[cfg(feature = "signing")]
    pub async fn discover_and_verify<'a>(
        base_url: &str,
        trusted_keys: impl IntoIterator<Item = &'a crate::signing::VerifyingKey>,
    ) -> Result<(Self, AgentCard)> {
        let card = A2aClient::fetch_agent_card(base_url).await?;
        super::verify_any_signature(&card, trusted_keys)?;
        let client = Self::from_agent_card(&card)?;
        Ok((client, card))
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

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url.trim_end_matches('/'))
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

    /// Parses a non-streaming REST response: `T` on any 2xx status, or the
    /// `google.rpc.Status` error shape mapped to a [`ClientError::Protocol`]
    /// otherwise.
    async fn parse_response<T: DeserializeOwned>(resp: reqwest::Response) -> Result<T> {
        let status = resp.status();
        let bytes = resp.bytes().await?;
        if !status.is_success() {
            return Err(match serde_json::from_slice::<RestErrorBody>(&bytes) {
                Ok(body) => ClientError::Protocol(rest_error_to_a2a(status, body)),
                Err(_) => ClientError::UnexpectedResponse {
                    status: status.as_u16(),
                    body: String::from_utf8_lossy(&bytes).into_owned(),
                },
            });
        }
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Like [`RestClient::parse_response`], but for endpoints with no
    /// response body on success (`DELETE .../pushNotificationConfigs/{id}`,
    /// which returns `204 No Content`).
    async fn parse_empty_response(resp: reqwest::Response) -> Result<()> {
        let status = resp.status();
        if status.is_success() {
            return Ok(());
        }
        let bytes = resp.bytes().await?;
        Err(match serde_json::from_slice::<RestErrorBody>(&bytes) {
            Ok(body) => ClientError::Protocol(rest_error_to_a2a(status, body)),
            Err(_) => ClientError::UnexpectedResponse {
                status: status.as_u16(),
                body: String::from_utf8_lossy(&bytes).into_owned(),
            },
        })
    }

    /// Issues a request with a JSON body and no query parameters, parsing
    /// the response as `R`.
    async fn post<P: Serialize, R: DeserializeOwned>(&self, path: &str, body: &P) -> Result<R> {
        let builder = self.apply_headers(self.http.post(self.url(path)).json(body));
        let resp = builder.send().await?;
        Self::parse_response(resp).await
    }

    /// Like [`RestClient::post`], but the request begins an SSE stream of
    /// raw (non-enveloped) `StreamResponse` events (spec Section 11.7) on
    /// success instead of returning a single JSON body.
    async fn post_streaming<P: Serialize>(
        &self,
        path: &str,
        body: &P,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamResponse>> + Send>>> {
        let builder = self.apply_headers(self.http.post(self.url(path)).json(body));
        let resp = builder.send().await?;
        let is_event_stream = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|ct| ct.starts_with("text/event-stream"));
        if !is_event_stream {
            Self::parse_response::<serde_json::Value>(resp).await?;
            return Err(ClientError::UnexpectedResponse {
                status: 200,
                body: "expected an SSE stream but got a non-streaming success response".to_string(),
            });
        }
        Ok(sse_stream(resp))
    }

    fn tenant_query(&self) -> Vec<(&'static str, &str)> {
        match &self.tenant {
            Some(t) => vec![("tenant", t.as_str())],
            None => Vec::new(),
        }
    }

    /// `SendMessage` (spec Section 3.1.1 / `POST /message:send`). Blocks
    /// until the task reaches a terminal/interrupted state, unless
    /// `configuration.returnImmediately` is set.
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
        self.post("/message:send", &req).await
    }

    /// `SendStreamingMessage` (spec Section 3.1.2 / `POST /message:stream`):
    /// sends a message and streams `Task`/`Message`/status/artifact
    /// updates via SSE as the agent produces them.
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
        self.post_streaming("/message:stream", &req).await
    }

    /// `GetTask` (spec Section 3.1.3 / `GET /tasks/{id}`).
    pub async fn get_task(&self, id: impl Into<String>, history_length: Option<i32>) -> Result<Task> {
        let id = id.into();
        let mut query = self.tenant_query();
        let history_length_str = history_length.map(|n| n.to_string());
        if let Some(v) = &history_length_str {
            query.push(("historyLength", v));
        }
        let builder = self.apply_headers(self.http.get(self.url(&format!("/tasks/{id}"))).query(&query));
        Self::parse_response(builder.send().await?).await
    }

    /// `ListTasks` (spec Section 3.1.4 / `GET /tasks`).
    pub async fn list_tasks(&self, mut req: ListTasksRequest) -> Result<ListTasksResponse> {
        req.tenant = self.tenant.clone();
        let builder = self.apply_headers(self.http.get(self.url("/tasks")).query(&req));
        Self::parse_response(builder.send().await?).await
    }

    /// `CancelTask` (spec Section 3.1.5 / `POST /tasks/{id}:cancel`).
    pub async fn cancel_task(&self, id: impl Into<String>) -> Result<Task> {
        let id = id.into();
        let builder = self
            .apply_headers(self.http.post(self.url(&format!("/tasks/{id}:cancel"))))
            .query(&self.tenant_query());
        Self::parse_response(builder.send().await?).await
    }

    /// `SubscribeToTask` (spec Section 3.1.6 / `GET /tasks/{id}:subscribe`):
    /// streams updates for a task that is not (yet) in a terminal state.
    ///
    /// Uses the spec-literal `GET` binding. This crate's own server also
    /// accepts `POST` on the same path, which is what this client used to
    /// send — but another SDK's server has no reason to, so `GET` is the only
    /// method that can be relied on across implementations.
    pub async fn subscribe_to_task(
        &self,
        id: impl Into<String>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamResponse>> + Send>>> {
        let id = id.into();
        let builder = self
            .apply_headers(self.http.get(self.url(&format!("/tasks/{id}:subscribe"))))
            .query(&self.tenant_query());
        let resp = builder.send().await?;
        let is_event_stream = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|ct| ct.starts_with("text/event-stream"));
        if !is_event_stream {
            Self::parse_response::<serde_json::Value>(resp).await?;
            return Err(ClientError::UnexpectedResponse {
                status: 200,
                body: "expected an SSE stream but got a non-streaming success response".to_string(),
            });
        }
        Ok(sse_stream(resp))
    }

    /// `CreateTaskPushNotificationConfig` (spec Section 3.1.7 / `POST
    /// /tasks/{taskId}/pushNotificationConfigs`). `config.taskId` selects
    /// the URL path and must be set.
    pub async fn create_push_notification_config(
        &self,
        mut config: TaskPushNotificationConfig,
    ) -> Result<TaskPushNotificationConfig> {
        let task_id = config.task_id.clone().ok_or_else(|| {
            ClientError::Protocol(A2aError::InvalidParams(
                "task_id is required to create a push notification config over REST".to_string(),
            ))
        })?;
        config.tenant = self.tenant.clone();
        self.post(&format!("/tasks/{task_id}/pushNotificationConfigs"), &config)
            .await
    }

    /// `GetTaskPushNotificationConfig` (spec Section 3.1.8 / `GET
    /// /tasks/{taskId}/pushNotificationConfigs/{id}`).
    pub async fn get_push_notification_config(
        &self,
        task_id: impl Into<String>,
        id: impl Into<String>,
    ) -> Result<TaskPushNotificationConfig> {
        let (task_id, id) = (task_id.into(), id.into());
        let builder = self
            .apply_headers(
                self.http
                    .get(self.url(&format!("/tasks/{task_id}/pushNotificationConfigs/{id}"))),
            )
            .query(&self.tenant_query());
        Self::parse_response(builder.send().await?).await
    }

    /// `ListTaskPushNotificationConfigs` (spec Section 3.1.9 / `GET
    /// /tasks/{taskId}/pushNotificationConfigs`).
    pub async fn list_push_notification_configs(
        &self,
        task_id: impl Into<String>,
    ) -> Result<ListTaskPushNotificationConfigsResponse> {
        let task_id = task_id.into();
        let builder = self
            .apply_headers(
                self.http
                    .get(self.url(&format!("/tasks/{task_id}/pushNotificationConfigs"))),
            )
            .query(&self.tenant_query());
        Self::parse_response(builder.send().await?).await
    }

    /// `DeleteTaskPushNotificationConfig` (spec Section 3.1.10 / `DELETE
    /// /tasks/{taskId}/pushNotificationConfigs/{id}`).
    pub async fn delete_push_notification_config(
        &self,
        task_id: impl Into<String>,
        id: impl Into<String>,
    ) -> Result<()> {
        let (task_id, id) = (task_id.into(), id.into());
        let builder = self
            .apply_headers(
                self.http
                    .delete(self.url(&format!("/tasks/{task_id}/pushNotificationConfigs/{id}"))),
            )
            .query(&self.tenant_query());
        Self::parse_empty_response(builder.send().await?).await
    }

    /// `GetExtendedAgentCard` (spec Section 3.1.11 / `GET
    /// /extendedAgentCard`).
    pub async fn get_extended_agent_card(&self) -> Result<AgentCard> {
        let builder = self
            .apply_headers(self.http.get(self.url("/extendedAgentCard")))
            .query(&self.tenant_query());
        Self::parse_response(builder.send().await?).await
    }
}

/// Parses a REST SSE response body into a stream of raw (non-enveloped)
/// `StreamResponse` events (spec Section 11.7).
fn sse_stream(resp: reqwest::Response) -> Pin<Box<dyn Stream<Item = Result<StreamResponse>> + Send>> {
    let events = resp.bytes_stream().eventsource();
    Box::pin(events.filter_map(|event| async move {
        let event = match event {
            Ok(e) => e,
            Err(e) => return Some(Err(ClientError::Stream(e.to_string()))),
        };
        if event.data.is_empty() {
            return None;
        }
        match serde_json::from_str::<StreamResponse>(&event.data) {
            Ok(sr) => Some(Ok(sr)),
            Err(e) => Some(Err(ClientError::Json(e))),
        }
    }))
}
