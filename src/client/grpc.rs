//! An async client for calling A2A agents over the gRPC protocol binding
//! (spec Section 10).
//!
//! Mirrors [`super::A2aClient`]'s API one-for-one (same method names and
//! signatures) but speaks gRPC instead of JSON-RPC, wrapping the
//! `tonic`/`prost`-generated [`crate::pb::a2a_service_client::A2aServiceClient`]
//! with the same [`crate::types`] conversions the server uses (in the
//! opposite direction).
//!
//! ```no_run
//! # async fn run() -> rusty_a2a::client::Result<()> {
//! use rusty_a2a::client::GrpcClient;
//! use rusty_a2a::types::Message;
//!
//! let (client, _card) = GrpcClient::discover("https://agent.example.com").await?;
//! let result = client.send_message(Message::user_text("hello!"), None).await?;
//! println!("{result:?}");
//! # Ok(())
//! # }
//! ```
use std::pin::Pin;

use futures_core::Stream;
use futures_util::StreamExt;
use tonic::transport::{Channel, Endpoint};
use tonic::{Code, Request, Status};

use crate::error::A2aError;

use crate::grpc_convert::{
    our_cancel_task_request_to_pb, our_delete_push_notification_config_request_to_pb,
    our_get_extended_agent_card_request_to_pb, our_get_push_notification_config_request_to_pb,
    our_get_task_request_to_pb, our_list_push_notification_configs_request_to_pb,
    our_list_tasks_request_to_pb, our_push_config_to_pb, our_send_message_request_to_pb,
    our_subscribe_to_task_request_to_pb, pb_agent_card_to_ours,
    pb_list_push_notification_configs_response_to_ours, pb_list_tasks_response_to_ours,
    pb_push_config_to_ours, pb_send_message_response_to_ours, pb_stream_response_to_ours, pb_task_to_ours,
};
use crate::pb::a2a_service_client::A2aServiceClient;
use crate::types::{
    AgentCard, AgentInterface, CancelTaskRequest, DeleteTaskPushNotificationConfigRequest,
    GetExtendedAgentCardRequest, GetTaskPushNotificationConfigRequest, GetTaskRequest,
    ListTaskPushNotificationConfigsRequest, ListTaskPushNotificationConfigsResponse, ListTasksRequest,
    ListTasksResponse, Message, SendMessageConfiguration, SendMessageRequest, SendMessageResult,
    StreamResponse, SubscribeToTaskRequest, Task, TaskPushNotificationConfig,
};

use super::{A2aClient, ClientError, Result};

/// Best-effort mapping from a gRPC status back to an [`A2aError`],
/// mirroring `jsonrpc_error_to_a2a`/`rest::rest_error_to_a2a` for the gRPC
/// binding. Unlike those two, a `tonic::Status` this crate's gRPC server
/// sends carries only a [`Code`] and a message on the wire (no structured
/// error details), and several distinct `A2aError` variants share the same
/// gRPC code (spec Section 5.4's "gRPC Status" column) - so this can only
/// pick one representative variant per code, chosen to match the most
/// common cause. Codes with no A2A-specific meaning pass through as
/// [`ClientError::Grpc`] instead of guessing.
fn status_to_client_error(status: Status) -> ClientError {
    let message = status.message().to_string();
    let err = match status.code() {
        Code::NotFound => A2aError::TaskNotFound(message),
        Code::FailedPrecondition => A2aError::TaskNotCancelable(message),
        Code::InvalidArgument => A2aError::InvalidParams(message),
        Code::Unauthenticated => A2aError::Unauthenticated(message),
        Code::PermissionDenied => A2aError::PermissionDenied(message),
        Code::Unimplemented => A2aError::MethodNotFound(message),
        Code::Internal => A2aError::Internal(message),
        _ => return ClientError::Grpc(status),
    };
    ClientError::Protocol(err)
}

/// A client for one A2A agent interface, speaking the gRPC protocol
/// binding. Cheap to clone (the underlying [`Channel`] shares its
/// connection pool across clones).
#[derive(Clone)]
pub struct GrpcClient {
    inner: A2aServiceClient<Channel>,
    tenant: Option<String>,
    bearer_token: Option<String>,
    protocol_version: String,
    extensions: Vec<String>,
}

impl GrpcClient {
    /// Builds a client targeting the given gRPC endpoint (e.g.
    /// `"http://127.0.0.1:50051"`) directly. Prefer [`GrpcClient::discover`]
    /// or [`GrpcClient::from_agent_card`] when you have (or can fetch) the
    /// agent's `AgentCard`.
    ///
    /// The underlying [`Channel`] connects lazily (on the first RPC), so
    /// this never blocks and never fails for a merely-unreachable
    /// endpoint - only for a malformed URI.
    pub fn new(endpoint: impl Into<String>) -> Result<Self> {
        let endpoint = endpoint.into();
        let channel = Endpoint::from_shared(endpoint.clone())
            .map_err(|e| ClientError::GrpcConfig(format!("invalid gRPC endpoint {endpoint:?}: {e}")))?
            .connect_lazy();
        Ok(GrpcClient {
            inner: A2aServiceClient::new(channel),
            tenant: None,
            bearer_token: None,
            protocol_version: crate::PROTOCOL_VERSION.to_string(),
            extensions: Vec::new(),
        })
    }

    /// Builds a client for the first `GRPC` interface declared in
    /// `card.supportedInterfaces` (spec Section 8.3.2).
    pub fn from_agent_card(card: &AgentCard) -> Result<Self> {
        let interface = card
            .interface_for_binding(AgentInterface::GRPC)
            .ok_or(ClientError::NoGrpcInterface)?;
        let mut client = GrpcClient::new(interface.url.clone())?;
        client.tenant = interface.tenant.clone();
        Ok(client)
    }

    /// Fetches `{base_url}/.well-known/agent-card.json` (spec Section 8.2)
    /// over HTTP and builds a client from it. `base_url` should be the
    /// agent's origin, e.g. `https://agent.example.com` (no trailing
    /// slash required).
    pub async fn discover(base_url: &str) -> Result<(Self, AgentCard)> {
        let card = A2aClient::fetch_agent_card(base_url).await?;
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
    /// request (spec Section 3.2.6), as the `a2a-extensions` gRPC metadata
    /// entry.
    pub fn with_extensions(mut self, extensions: Vec<String>) -> Self {
        self.extensions = extensions;
        self
    }

    /// Overrides the `A2A-Version` service parameter (defaults to
    /// [`crate::PROTOCOL_VERSION`]), sent as the `a2a-version` gRPC
    /// metadata entry.
    pub fn with_protocol_version(mut self, version: impl Into<String>) -> Self {
        self.protocol_version = version.into();
        self
    }

    /// Wraps `message` in a [`Request`] with this client's service
    /// parameters attached as gRPC metadata - which, per gRPC convention
    /// (and unlike ordinary HTTP headers), only allows ASCII-lowercase
    /// keys (see `server::grpc::GrpcService::credentials`).
    fn request<T>(&self, message: T) -> Result<Request<T>> {
        let mut req = Request::new(message);
        let metadata = req.metadata_mut();
        metadata.insert(
            "a2a-version",
            self.protocol_version
                .parse()
                .map_err(|_| ClientError::GrpcConfig("invalid A2A-Version".to_string()))?,
        );
        if !self.extensions.is_empty() {
            metadata.insert(
                "a2a-extensions",
                self.extensions
                    .join(",")
                    .parse()
                    .map_err(|_| ClientError::GrpcConfig("invalid A2A-Extensions".to_string()))?,
            );
        }
        if let Some(token) = &self.bearer_token {
            metadata.insert(
                "authorization",
                format!("Bearer {token}")
                    .parse()
                    .map_err(|_| ClientError::GrpcConfig("invalid bearer token".to_string()))?,
            );
        }
        Ok(req)
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
        let request = self.request(our_send_message_request_to_pb(req))?;
        let response = self
            .inner
            .clone()
            .send_message(request)
            .await
            .map_err(status_to_client_error)?
            .into_inner();
        Ok(pb_send_message_response_to_ours(response)?)
    }

    /// `SendStreamingMessage` (spec Section 3.1.2): sends a message and
    /// streams `Task`/`Message`/status/artifact updates as the agent
    /// produces them.
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
        let request = self.request(our_send_message_request_to_pb(req))?;
        let stream = self
            .inner
            .clone()
            .send_streaming_message(request)
            .await
            .map_err(status_to_client_error)?
            .into_inner();
        Ok(Box::pin(stream.map(|item| match item {
            Ok(pb_item) => Ok(pb_stream_response_to_ours(pb_item)?),
            Err(status) => Err(status_to_client_error(status)),
        })))
    }

    /// `GetTask` (spec Section 3.1.3).
    pub async fn get_task(&self, id: impl Into<String>, history_length: Option<i32>) -> Result<Task> {
        let req = GetTaskRequest {
            tenant: self.tenant.clone(),
            id: id.into(),
            history_length,
        };
        let request = self.request(our_get_task_request_to_pb(req))?;
        let task = self
            .inner
            .clone()
            .get_task(request)
            .await
            .map_err(status_to_client_error)?
            .into_inner();
        Ok(pb_task_to_ours(task)?)
    }

    /// `ListTasks` (spec Section 3.1.4).
    pub async fn list_tasks(&self, mut req: ListTasksRequest) -> Result<ListTasksResponse> {
        req.tenant = self.tenant.clone();
        let request = self.request(our_list_tasks_request_to_pb(req))?;
        let res = self
            .inner
            .clone()
            .list_tasks(request)
            .await
            .map_err(status_to_client_error)?
            .into_inner();
        Ok(pb_list_tasks_response_to_ours(res)?)
    }

    /// `CancelTask` (spec Section 3.1.5).
    pub async fn cancel_task(&self, id: impl Into<String>) -> Result<Task> {
        let req = CancelTaskRequest {
            tenant: self.tenant.clone(),
            id: id.into(),
            metadata: None,
        };
        let request = self.request(our_cancel_task_request_to_pb(req))?;
        let task = self
            .inner
            .clone()
            .cancel_task(request)
            .await
            .map_err(status_to_client_error)?
            .into_inner();
        Ok(pb_task_to_ours(task)?)
    }

    /// `SubscribeToTask` (spec Section 3.1.6): streams updates for a task
    /// that is not (yet) in a terminal state. The canonical
    /// `SubscribeToTaskRequest` has no resume-point field (unlike SSE's
    /// `Last-Event-ID`, which the JSON-RPC/REST bindings support), so a
    /// gRPC (re)subscribe always replays this task's whole buffered event
    /// log, then continues live - see `server::engine::Engine::subscribe_to_task`.
    pub async fn subscribe_to_task(
        &self,
        id: impl Into<String>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamResponse>> + Send>>> {
        let req = SubscribeToTaskRequest {
            tenant: self.tenant.clone(),
            id: id.into(),
        };
        let request = self.request(our_subscribe_to_task_request_to_pb(req))?;
        let stream = self
            .inner
            .clone()
            .subscribe_to_task(request)
            .await
            .map_err(status_to_client_error)?
            .into_inner();
        Ok(Box::pin(stream.map(|item| match item {
            Ok(pb_item) => Ok(pb_stream_response_to_ours(pb_item)?),
            Err(status) => Err(status_to_client_error(status)),
        })))
    }

    /// `CreateTaskPushNotificationConfig` (spec Section 3.1.7).
    pub async fn create_push_notification_config(
        &self,
        mut config: TaskPushNotificationConfig,
    ) -> Result<TaskPushNotificationConfig> {
        config.tenant = self.tenant.clone();
        let request = self.request(our_push_config_to_pb(config))?;
        let created = self
            .inner
            .clone()
            .create_task_push_notification_config(request)
            .await
            .map_err(status_to_client_error)?
            .into_inner();
        Ok(pb_push_config_to_ours(created))
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
        let request = self.request(our_get_push_notification_config_request_to_pb(req))?;
        let config = self
            .inner
            .clone()
            .get_task_push_notification_config(request)
            .await
            .map_err(status_to_client_error)?
            .into_inner();
        Ok(pb_push_config_to_ours(config))
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
        let request = self.request(our_list_push_notification_configs_request_to_pb(req))?;
        let res = self
            .inner
            .clone()
            .list_task_push_notification_configs(request)
            .await
            .map_err(status_to_client_error)?
            .into_inner();
        Ok(pb_list_push_notification_configs_response_to_ours(res))
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
        let request = self.request(our_delete_push_notification_config_request_to_pb(req))?;
        self.inner
            .clone()
            .delete_task_push_notification_config(request)
            .await
            .map_err(status_to_client_error)?;
        Ok(())
    }

    /// `GetExtendedAgentCard` (spec Section 3.1.11).
    pub async fn get_extended_agent_card(&self) -> Result<AgentCard> {
        let req = GetExtendedAgentCardRequest {
            tenant: self.tenant.clone(),
        };
        let request = self.request(our_get_extended_agent_card_request_to_pb(req))?;
        let card = self
            .inner
            .clone()
            .get_extended_agent_card(request)
            .await
            .map_err(status_to_client_error)?
            .into_inner();
        Ok(pb_agent_card_to_ours(card)?)
    }
}
