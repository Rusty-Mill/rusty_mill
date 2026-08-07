//! Exposes an `Engine` over gRPC (spec Section
//! 10), implementing the `A2AService` service generated from the vendored
//! `spec/a2a.proto` by `build.rs` (via `tonic-prost-build`; requires a
//! `protoc` binary on `PATH` to build this crate with the `grpc` feature
//! at all).
//!
//! Runs as its own [`tonic`] server rather than being merged into the
//! `axum::Router` the other bindings share - see
//! [`AgentServices`](super::AgentServices) for running it alongside them
//! against the same agent state.

pub mod pb {
    #![allow(clippy::doc_lazy_continuation)]
    tonic::include_proto!("lf.a2a.v1");
}

mod convert;

use std::pin::Pin;
use std::sync::Arc;

use futures_core::Stream;
use futures_util::StreamExt;
use tonic::{Request, Response, Status};

use super::auth::{extract_credentials, Credentials};
use super::engine::{parse_extensions_header, Engine};
use convert::*;
use pb::a2a_service_server::A2aService;

/// The `A2AService` gRPC service implementation, wrapping an `Engine`
/// shared with the other protocol bindings. Build one via
/// [`AgentServices::grpc_service`](super::AgentServices::grpc_service)
/// rather than directly.
pub struct GrpcService {
    engine: Arc<Engine>,
}

impl GrpcService {
    pub(crate) fn new(engine: Arc<Engine>) -> Self {
        GrpcService { engine }
    }

    /// Extracts credentials for `AgentCard.securitySchemes` from `request`'s
    /// gRPC metadata (which, per gRPC convention, only has ASCII-lowercase
    /// keys - so a scheme's declared header/key `name` is lowercased
    /// before lookup).
    fn credentials<T>(&self, request: &Request<T>) -> Credentials {
        let metadata = request.metadata();
        extract_credentials(
            &self.engine.card().security_schemes,
            |name| {
                metadata
                    .get(name.to_ascii_lowercase())
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string)
            },
            None,
        )
    }

    /// Enforces `AgentCard.capabilities.extensions[].required` (spec
    /// Section 3.2.6 / 5.6) against `request`'s `a2a-extensions` metadata
    /// entry.
    fn check_required_extensions<T>(&self, request: &Request<T>) -> Result<(), Status> {
        let declared = parse_extensions_header(
            request
                .metadata()
                .get("a2a-extensions")
                .and_then(|v| v.to_str().ok()),
        );
        self.engine
            .check_required_extensions(&declared)
            .map_err(a2a_error_to_status)
    }

    /// Enforces required extensions, then `AgentCard.securityRequirements`
    /// (spec Section 4.5), against `request`'s metadata.
    async fn authenticate<T>(&self, request: &Request<T>) -> Result<(), Status> {
        self.check_required_extensions(request)?;
        let credentials = self.credentials(request);
        self.engine
            .authenticate(&credentials)
            .await
            .map(|_| ())
            .map_err(a2a_error_to_status)
    }
}

type ResponseStream = Pin<Box<dyn Stream<Item = Result<pb::StreamResponse, Status>> + Send>>;

#[tonic::async_trait]
impl A2aService for GrpcService {
    async fn send_message(
        &self,
        request: Request<pb::SendMessageRequest>,
    ) -> Result<Response<pb::SendMessageResponse>, Status> {
        self.authenticate(&request).await?;
        let req = pb_send_message_request_to_ours(request.into_inner())?;
        let result = self.engine.send_message(req).await.map_err(a2a_error_to_status)?;
        Ok(Response::new(our_send_result_to_pb(result)))
    }

    type SendStreamingMessageStream = ResponseStream;

    async fn send_streaming_message(
        &self,
        request: Request<pb::SendMessageRequest>,
    ) -> Result<Response<Self::SendStreamingMessageStream>, Status> {
        self.authenticate(&request).await?;
        let req = pb_send_message_request_to_ours(request.into_inner())?;
        let stream = self
            .engine
            .send_streaming_message(req)
            .await
            .map_err(a2a_error_to_status)?;
        let mapped = stream.map(|item| Ok(our_stream_response_to_pb(item)));
        Ok(Response::new(Box::pin(mapped)))
    }

    async fn get_task(&self, request: Request<pb::GetTaskRequest>) -> Result<Response<pb::Task>, Status> {
        self.authenticate(&request).await?;
        let req = pb_get_task_request_to_ours(request.into_inner());
        let task = self.engine.get_task(req).await.map_err(a2a_error_to_status)?;
        Ok(Response::new(our_task_to_pb(task)))
    }

    async fn list_tasks(
        &self,
        request: Request<pb::ListTasksRequest>,
    ) -> Result<Response<pb::ListTasksResponse>, Status> {
        self.authenticate(&request).await?;
        let req = pb_list_tasks_request_to_ours(request.into_inner());
        let res = self.engine.list_tasks(req).await.map_err(a2a_error_to_status)?;
        Ok(Response::new(our_list_tasks_response_to_pb(res)))
    }

    async fn cancel_task(
        &self,
        request: Request<pb::CancelTaskRequest>,
    ) -> Result<Response<pb::Task>, Status> {
        self.authenticate(&request).await?;
        let req = pb_cancel_task_request_to_ours(request.into_inner());
        let task = self.engine.cancel_task(req).await.map_err(a2a_error_to_status)?;
        Ok(Response::new(our_task_to_pb(task)))
    }

    type SubscribeToTaskStream = ResponseStream;

    async fn subscribe_to_task(
        &self,
        request: Request<pb::SubscribeToTaskRequest>,
    ) -> Result<Response<Self::SubscribeToTaskStream>, Status> {
        self.authenticate(&request).await?;
        let req = pb_subscribe_to_task_request_to_ours(request.into_inner());
        // The canonical `SubscribeToTaskRequest` has no resume-point
        // field (unlike SSE's `Last-Event-ID`, which the JSON-RPC/REST
        // bindings read from a request header), so a gRPC resubscribe
        // always replays this task's whole buffered event log, then
        // continues live - see `Engine::subscribe_to_task`.
        let stream = self
            .engine
            .subscribe_to_task(req, None)
            .await
            .map_err(a2a_error_to_status)?;
        let mapped = stream.map(|(_, item)| Ok(our_stream_response_to_pb(item)));
        Ok(Response::new(Box::pin(mapped)))
    }

    async fn create_task_push_notification_config(
        &self,
        request: Request<pb::TaskPushNotificationConfig>,
    ) -> Result<Response<pb::TaskPushNotificationConfig>, Status> {
        self.authenticate(&request).await?;
        let config = pb_push_config_to_ours(request.into_inner());
        let created = self
            .engine
            .create_push_notification_config(config)
            .await
            .map_err(a2a_error_to_status)?;
        Ok(Response::new(our_push_config_to_pb(created)))
    }

    async fn get_task_push_notification_config(
        &self,
        request: Request<pb::GetTaskPushNotificationConfigRequest>,
    ) -> Result<Response<pb::TaskPushNotificationConfig>, Status> {
        self.authenticate(&request).await?;
        let req = pb_get_push_notification_config_request_to_ours(request.into_inner());
        let config = self
            .engine
            .get_push_notification_config(req)
            .await
            .map_err(a2a_error_to_status)?;
        Ok(Response::new(our_push_config_to_pb(config)))
    }

    async fn list_task_push_notification_configs(
        &self,
        request: Request<pb::ListTaskPushNotificationConfigsRequest>,
    ) -> Result<Response<pb::ListTaskPushNotificationConfigsResponse>, Status> {
        self.authenticate(&request).await?;
        let req = pb_list_push_notification_configs_request_to_ours(request.into_inner());
        let res = self
            .engine
            .list_push_notification_configs(req)
            .await
            .map_err(a2a_error_to_status)?;
        Ok(Response::new(our_list_push_notification_configs_response_to_pb(
            res,
        )))
    }

    async fn get_extended_agent_card(
        &self,
        request: Request<pb::GetExtendedAgentCardRequest>,
    ) -> Result<Response<pb::AgentCard>, Status> {
        self.check_required_extensions(&request)?;
        let credentials = self.credentials(&request);
        let card = self
            .engine
            .get_extended_agent_card(&credentials)
            .await
            .map_err(a2a_error_to_status)?;
        Ok(Response::new(our_agent_card_to_pb(card)))
    }

    async fn delete_task_push_notification_config(
        &self,
        request: Request<pb::DeleteTaskPushNotificationConfigRequest>,
    ) -> Result<Response<()>, Status> {
        self.authenticate(&request).await?;
        let req = pb_delete_push_notification_config_request_to_ours(request.into_inner());
        self.engine
            .delete_push_notification_config(req)
            .await
            .map_err(a2a_error_to_status)?;
        Ok(Response::new(()))
    }
}
