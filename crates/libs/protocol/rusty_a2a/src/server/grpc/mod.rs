//! Exposes an `Engine` over gRPC (spec Section 10) as a [`rusty_tonic`] [`Router`]: the eleven
//! RPCs of `A2AService`, with the messages of [`crate::pb`] (hand-written from the vendored
//! `spec/a2a.proto`; building this crate with the `grpc` feature needs no `protoc`).
//!
//! The router runs on `rusty_tokio`, the transport's runtime, while the engine runs on the
//! application's `tokio`: every handler hands its engine call across with `grpc_bridge`, and the
//! call stops when the client leaves. Runs as its own server rather than being merged into the
//! `axum::Router` the other bindings share - see [`AgentServices`](super::AgentServices) for
//! running it alongside them against the same agent state.

pub use crate::pb;

use std::pin::Pin;
use std::sync::Arc;

use futures_core::Stream;
use futures_util::StreamExt;
use rusty_tonic::client::{Request, Response};
use rusty_tonic::server::{Router, Sink};
use rusty_tonic::Status;

use super::auth::{extract_credentials, AuthContext, Credentials};
use super::engine::{check_version, parse_extensions_header, Engine};
use crate::grpc_bridge::server::{feed, on_app};
use crate::grpc_convert::*;
use crate::pb::path;

/// The `A2AService` gRPC service implementation, wrapping an `Engine`
/// shared with the other protocol bindings. Build one via
/// [`AgentServices::grpc_router`](super::AgentServices::grpc_router)
/// rather than directly.
pub struct GrpcService {
    engine: Arc<Engine>,
}

type Events = Pin<Box<dyn Stream<Item = Result<pb::StreamResponse, Status>> + Send>>;

impl GrpcService {
    pub(crate) fn new(engine: Arc<Engine>) -> Self {
        GrpcService { engine }
    }

    /// Extracts credentials for `AgentCard.securitySchemes` from `request`'s
    /// gRPC metadata (which, per gRPC convention, only has ASCII-lowercase
    /// keys - so a scheme's declared header/key `name` is lowercased
    /// before lookup).
    fn credentials<T>(&self, request: &Request<T>) -> Credentials {
        let metadata = &request.metadata;
        extract_credentials(
            &self.engine.card().security_schemes,
            |name| metadata.get(&name.to_ascii_lowercase()).map(str::to_string),
            None,
            self.engine.mtls_header(),
        )
    }

    /// Validates `request`'s `a2a-version` metadata entry (spec Section
    /// 3.2.6 / 3.6.2), the lowercased gRPC-metadata equivalent of the
    /// `A2A-Version` header the JSON-RPC/REST bindings read.
    fn check_version<T>(&self, request: &Request<T>) -> Result<(), Status> {
        check_version(request.metadata.get("a2a-version")).map_err(a2a_error_to_status)
    }

    /// Enforces `AgentCard.capabilities.extensions[].required` (spec
    /// Section 3.2.6 / 5.6) against `request`'s `a2a-extensions` metadata
    /// entry.
    fn check_required_extensions<T>(&self, request: &Request<T>) -> Result<(), Status> {
        let declared = parse_extensions_header(request.metadata.get("a2a-extensions"));
        self.engine
            .check_required_extensions(&declared)
            .map_err(a2a_error_to_status)
    }

    /// Validates the protocol version, enforces required extensions, then
    /// `AgentCard.securityRequirements` (spec Section 4.5), against
    /// `request`'s metadata.
    async fn authenticate<T>(&self, request: &Request<T>) -> Result<Option<AuthContext>, Status> {
        self.check_version(request)?;
        self.check_required_extensions(request)?;
        let credentials = self.credentials(request);
        self.engine
            .authenticate(&credentials)
            .await
            .map_err(a2a_error_to_status)
    }

    async fn send_message(
        &self,
        request: Request<pb::SendMessageRequest>,
    ) -> Result<pb::SendMessageResponse, Status> {
        let auth = self.authenticate(&request).await?;
        let req = pb_send_message_request_to_ours(request.message)?;
        let result = self
            .engine
            .send_message(req, auth.as_ref())
            .await
            .map_err(a2a_error_to_status)?;
        Ok(our_send_result_to_pb(result))
    }

    async fn send_streaming_message(
        &self,
        request: Request<pb::SendMessageRequest>,
    ) -> Result<Events, Status> {
        let auth = self.authenticate(&request).await?;
        let req = pb_send_message_request_to_ours(request.message)?;
        let stream = self
            .engine
            .send_streaming_message(req, auth.as_ref())
            .await
            .map_err(a2a_error_to_status)?;
        Ok(Box::pin(stream.map(|item| Ok(our_stream_response_to_pb(item)))))
    }

    async fn get_task(&self, request: Request<pb::GetTaskRequest>) -> Result<pb::Task, Status> {
        let auth = self.authenticate(&request).await?;
        let req = pb_get_task_request_to_ours(request.message);
        let task = self
            .engine
            .get_task(req, auth.as_ref())
            .await
            .map_err(a2a_error_to_status)?;
        Ok(our_task_to_pb(task))
    }

    async fn list_tasks(
        &self,
        request: Request<pb::ListTasksRequest>,
    ) -> Result<pb::ListTasksResponse, Status> {
        let auth = self.authenticate(&request).await?;
        let req = pb_list_tasks_request_to_ours(request.message);
        let res = self
            .engine
            .list_tasks(req, auth.as_ref())
            .await
            .map_err(a2a_error_to_status)?;
        Ok(our_list_tasks_response_to_pb(res))
    }

    async fn cancel_task(&self, request: Request<pb::CancelTaskRequest>) -> Result<pb::Task, Status> {
        let auth = self.authenticate(&request).await?;
        let req = pb_cancel_task_request_to_ours(request.message);
        let task = self
            .engine
            .cancel_task(req, auth.as_ref())
            .await
            .map_err(a2a_error_to_status)?;
        Ok(our_task_to_pb(task))
    }

    async fn subscribe_to_task(
        &self,
        request: Request<pb::SubscribeToTaskRequest>,
    ) -> Result<Events, Status> {
        let auth = self.authenticate(&request).await?;
        let req = pb_subscribe_to_task_request_to_ours(request.message);
        // The canonical `SubscribeToTaskRequest` has no resume-point
        // field (unlike SSE's `Last-Event-ID`, which the JSON-RPC/REST
        // bindings read from a request header), so a gRPC resubscribe
        // always replays this task's whole buffered event log, then
        // continues live - see `Engine::subscribe_to_task`.
        let stream = self
            .engine
            .subscribe_to_task(req, None, auth.as_ref())
            .await
            .map_err(a2a_error_to_status)?;
        Ok(Box::pin(
            stream.map(|(_, item)| Ok(our_stream_response_to_pb(item))),
        ))
    }

    async fn create_task_push_notification_config(
        &self,
        request: Request<pb::TaskPushNotificationConfig>,
    ) -> Result<pb::TaskPushNotificationConfig, Status> {
        let auth = self.authenticate(&request).await?;
        let config = pb_push_config_to_ours(request.message);
        let created = self
            .engine
            .create_push_notification_config(config, auth.as_ref())
            .await
            .map_err(a2a_error_to_status)?;
        Ok(our_push_config_to_pb(created))
    }

    async fn get_task_push_notification_config(
        &self,
        request: Request<pb::GetTaskPushNotificationConfigRequest>,
    ) -> Result<pb::TaskPushNotificationConfig, Status> {
        let auth = self.authenticate(&request).await?;
        let req = pb_get_push_notification_config_request_to_ours(request.message);
        let config = self
            .engine
            .get_push_notification_config(req, auth.as_ref())
            .await
            .map_err(a2a_error_to_status)?;
        Ok(our_push_config_to_pb(config))
    }

    async fn list_task_push_notification_configs(
        &self,
        request: Request<pb::ListTaskPushNotificationConfigsRequest>,
    ) -> Result<pb::ListTaskPushNotificationConfigsResponse, Status> {
        let auth = self.authenticate(&request).await?;
        let req = pb_list_push_notification_configs_request_to_ours(request.message);
        let res = self
            .engine
            .list_push_notification_configs(req, auth.as_ref())
            .await
            .map_err(a2a_error_to_status)?;
        Ok(our_list_push_notification_configs_response_to_pb(res))
    }

    async fn get_extended_agent_card(
        &self,
        request: Request<pb::GetExtendedAgentCardRequest>,
    ) -> Result<pb::AgentCard, Status> {
        self.check_version(&request)?;
        self.check_required_extensions(&request)?;
        let credentials = self.credentials(&request);
        let card = self
            .engine
            .get_extended_agent_card(&credentials)
            .await
            .map_err(a2a_error_to_status)?;
        Ok(our_agent_card_to_pb(card))
    }

    async fn delete_task_push_notification_config(
        &self,
        request: Request<pb::DeleteTaskPushNotificationConfigRequest>,
    ) -> Result<pb::Empty, Status> {
        let auth = self.authenticate(&request).await?;
        let req = pb_delete_push_notification_config_request_to_ours(request.message);
        self.engine
            .delete_push_notification_config(req, auth.as_ref())
            .await
            .map_err(a2a_error_to_status)?;
        Ok(pb::Empty)
    }

    /// The router for the eleven RPCs. Handlers run on the transport's runtime; each hands its
    /// engine call to `app`, the application's `tokio` runtime.
    pub(crate) fn router(self: Arc<Self>, app: tokio::runtime::Handle) -> Router {
        // One unary RPC: the engine call runs on `app`, and is aborted if the client leaves.
        macro_rules! unary {
            ($router:expr, $path:expr, $method:ident) => {{
                let (svc, app) = (self.clone(), app.clone());
                $router.unary($path, move |request| {
                    let (svc, app) = (svc.clone(), app.clone());
                    async move {
                        on_app(&app, async move { svc.$method(request).await })
                            .await
                            .map(Response::new)
                    }
                })
            }};
        }
        // One server-streaming RPC: the engine call and then the stream it returns both run on
        // `app`; replies cross a channel, and dropping the call stops them.
        macro_rules! streaming {
            ($router:expr, $path:expr, $method:ident) => {{
                let (svc, app) = (self.clone(), app.clone());
                $router.server_streaming($path, move |request, mut out: Sink<pb::StreamResponse>| {
                    let (svc, app) = (svc.clone(), app.clone());
                    async move {
                        let events = on_app(&app, async move { svc.$method(request).await }).await?;
                        let mut replies = feed(&app, events);
                        while let Some(reply) = replies.next().await {
                            out.send(&reply?).await?;
                        }
                        Ok(())
                    }
                })
            }};
        }
        let router = Router::new();
        let router = unary!(router, path::SEND_MESSAGE, send_message);
        let router = streaming!(router, path::SEND_STREAMING_MESSAGE, send_streaming_message);
        let router = unary!(router, path::GET_TASK, get_task);
        let router = unary!(router, path::LIST_TASKS, list_tasks);
        let router = unary!(router, path::CANCEL_TASK, cancel_task);
        let router = streaming!(router, path::SUBSCRIBE_TO_TASK, subscribe_to_task);
        let router = unary!(
            router,
            path::CREATE_TASK_PUSH_NOTIFICATION_CONFIG,
            create_task_push_notification_config
        );
        let router = unary!(
            router,
            path::GET_TASK_PUSH_NOTIFICATION_CONFIG,
            get_task_push_notification_config
        );
        let router = unary!(
            router,
            path::LIST_TASK_PUSH_NOTIFICATION_CONFIGS,
            list_task_push_notification_configs
        );
        let router = unary!(
            router,
            path::DELETE_TASK_PUSH_NOTIFICATION_CONFIG,
            delete_task_push_notification_config
        );
        unary!(router, path::GET_EXTENDED_AGENT_CARD, get_extended_agent_card)
    }
}
