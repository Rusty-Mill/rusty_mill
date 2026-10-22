//! `A2AService` on `tonic`, by hand: a codec that reads and writes the [`super`] messages with
//! `rusty_prost`, the server trait and service that routes the eleven RPCs, and the client.
//! The shapes follow what `tonic-prost-build` generated, so callers read the same.

use std::marker::PhantomData;

use bytes::{Buf, BufMut};
use rusty_prost::Message;
use tonic::codec::{BufferSettings, Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use tonic::Status;

/// The service's name in a request path.
const SERVICE: &str = "lf.a2a.v1.A2AService";

/// A `tonic` codec for `rusty_prost` messages: `T` is sent, `U` is received.
#[derive(Debug, Clone)]
pub(crate) struct WireCodec<T, U>(PhantomData<(T, U)>);

impl<T, U> Default for WireCodec<T, U> {
    fn default() -> Self {
        WireCodec(PhantomData)
    }
}

impl<T: Message + Send + 'static, U: Message + Send + 'static> Codec for WireCodec<T, U> {
    type Encode = T;
    type Decode = U;
    type Encoder = WireEncoder<T>;
    type Decoder = WireDecoder<U>;

    fn encoder(&mut self) -> WireEncoder<T> {
        WireEncoder(PhantomData)
    }

    fn decoder(&mut self) -> WireDecoder<U> {
        WireDecoder(PhantomData)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct WireEncoder<T>(PhantomData<T>);

impl<T: Message> Encoder for WireEncoder<T> {
    type Item = T;
    type Error = Status;

    fn encode(&mut self, item: T, dst: &mut EncodeBuf<'_>) -> Result<(), Status> {
        let bytes = item
            .encode_to_vec()
            .map_err(|e| Status::internal(format!("failed to encode a message: {e}")))?;
        dst.put_slice(&bytes);
        Ok(())
    }

    fn buffer_settings(&self) -> BufferSettings {
        BufferSettings::default()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct WireDecoder<U>(PhantomData<U>);

impl<U: Message> Decoder for WireDecoder<U> {
    type Item = U;
    type Error = Status;

    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<U>, Status> {
        let bytes = src.copy_to_bytes(src.remaining());
        U::decode(&bytes)
            .map(Some)
            .map_err(|e| Status::internal(format!("failed to decode a message: {e}")))
    }

    fn buffer_settings(&self) -> BufferSettings {
        BufferSettings::default()
    }
}

/// The server side: implement [`A2aService`](a2a_service_server::A2aService) and serve
/// [`A2aServiceServer`](a2a_service_server::A2aServiceServer) with `tonic`.
pub mod a2a_service_server {
    use std::convert::Infallible;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll};

    use tonic::codegen::{http, Body, StdError};
    use tonic::server::{Grpc, ServerStreamingService, UnaryService};
    use tonic::{async_trait, Request, Response, Status};

    use super::super::*;
    use super::{WireCodec, SERVICE};

    type Reply<T> = Result<Response<T>, Status>;
    type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

    /// The eleven RPCs of `A2AService`.
    #[async_trait]
    pub trait A2aService: Send + Sync + 'static {
        /// `SendMessage`.
        async fn send_message(&self, request: Request<SendMessageRequest>) -> Reply<SendMessageResponse>;

        /// The stream of `SendStreamingMessage`.
        type SendStreamingMessageStream: futures_core::Stream<Item = Result<StreamResponse, Status>>
            + Send
            + 'static;

        /// `SendStreamingMessage`.
        async fn send_streaming_message(
            &self,
            request: Request<SendMessageRequest>,
        ) -> Reply<Self::SendStreamingMessageStream>;

        /// `GetTask`.
        async fn get_task(&self, request: Request<GetTaskRequest>) -> Reply<Task>;

        /// `ListTasks`.
        async fn list_tasks(&self, request: Request<ListTasksRequest>) -> Reply<ListTasksResponse>;

        /// `CancelTask`.
        async fn cancel_task(&self, request: Request<CancelTaskRequest>) -> Reply<Task>;

        /// The stream of `SubscribeToTask`.
        type SubscribeToTaskStream: futures_core::Stream<Item = Result<StreamResponse, Status>>
            + Send
            + 'static;

        /// `SubscribeToTask`.
        async fn subscribe_to_task(
            &self,
            request: Request<SubscribeToTaskRequest>,
        ) -> Reply<Self::SubscribeToTaskStream>;

        /// `CreateTaskPushNotificationConfig`.
        async fn create_task_push_notification_config(
            &self,
            request: Request<TaskPushNotificationConfig>,
        ) -> Reply<TaskPushNotificationConfig>;

        /// `GetTaskPushNotificationConfig`.
        async fn get_task_push_notification_config(
            &self,
            request: Request<GetTaskPushNotificationConfigRequest>,
        ) -> Reply<TaskPushNotificationConfig>;

        /// `ListTaskPushNotificationConfigs`.
        async fn list_task_push_notification_configs(
            &self,
            request: Request<ListTaskPushNotificationConfigsRequest>,
        ) -> Reply<ListTaskPushNotificationConfigsResponse>;

        /// `GetExtendedAgentCard`.
        async fn get_extended_agent_card(
            &self,
            request: Request<GetExtendedAgentCardRequest>,
        ) -> Reply<AgentCard>;

        /// `DeleteTaskPushNotificationConfig`.
        async fn delete_task_push_notification_config(
            &self,
            request: Request<DeleteTaskPushNotificationConfigRequest>,
        ) -> Reply<Empty>;
    }

    /// Routes `A2AService` calls to a [`A2aService`].
    #[derive(Debug)]
    pub struct A2aServiceServer<T> {
        inner: Arc<T>,
    }

    impl<T> A2aServiceServer<T> {
        /// A server for `inner`.
        pub fn new(inner: T) -> Self {
            Self::from_arc(Arc::new(inner))
        }

        /// A server for a shared `inner`.
        pub fn from_arc(inner: Arc<T>) -> Self {
            A2aServiceServer { inner }
        }
    }

    impl<T> Clone for A2aServiceServer<T> {
        fn clone(&self) -> Self {
            A2aServiceServer {
                inner: self.inner.clone(),
            }
        }
    }

    impl<T> tonic::server::NamedService for A2aServiceServer<T> {
        const NAME: &'static str = SERVICE;
    }

    /// One unary RPC: an adapter from `UnaryService` to the trait method, and the call.
    macro_rules! unary {
        ($inner:expr, $req:expr, $method:ident, $rq:ty, $rs:ty) => {{
            struct Svc<T>(Arc<T>);
            impl<T: A2aService> UnaryService<$rq> for Svc<T> {
                type Response = $rs;
                type Future = BoxFuture<Reply<$rs>>;
                fn call(&mut self, request: Request<$rq>) -> Self::Future {
                    let inner = Arc::clone(&self.0);
                    Box::pin(async move { <T as A2aService>::$method(&inner, request).await })
                }
            }
            let svc = Svc(Arc::clone($inner));
            let req = $req;
            Box::pin(async move {
                let mut grpc = Grpc::new(WireCodec::<$rs, $rq>::default());
                Ok(grpc.unary(svc, req).await)
            })
        }};
    }

    /// One server-streaming RPC.
    macro_rules! streaming {
        ($inner:expr, $req:expr, $method:ident, $stream:ident, $rq:ty, $rs:ty) => {{
            struct Svc<T>(Arc<T>);
            impl<T: A2aService> ServerStreamingService<$rq> for Svc<T> {
                type Response = $rs;
                type ResponseStream = T::$stream;
                type Future = BoxFuture<Reply<T::$stream>>;
                fn call(&mut self, request: Request<$rq>) -> Self::Future {
                    let inner = Arc::clone(&self.0);
                    Box::pin(async move { <T as A2aService>::$method(&inner, request).await })
                }
            }
            let svc = Svc(Arc::clone($inner));
            let req = $req;
            Box::pin(async move {
                let mut grpc = Grpc::new(WireCodec::<$rs, $rq>::default());
                Ok(grpc.server_streaming(svc, req).await)
            })
        }};
    }

    impl<T, B> tonic::codegen::Service<http::Request<B>> for A2aServiceServer<T>
    where
        T: A2aService,
        B: Body + Send + 'static,
        B::Error: Into<StdError> + Send + 'static,
    {
        type Response = http::Response<tonic::body::Body>;
        type Error = Infallible;
        type Future = BoxFuture<Result<Self::Response, Infallible>>;

        fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, req: http::Request<B>) -> Self::Future {
            let inner = &self.inner;
            let path = req
                .uri()
                .path()
                .strip_prefix('/')
                .and_then(|p| p.strip_prefix(SERVICE));
            match path {
                Some("/SendMessage") => {
                    unary!(inner, req, send_message, SendMessageRequest, SendMessageResponse)
                }
                Some("/SendStreamingMessage") => streaming!(
                    inner,
                    req,
                    send_streaming_message,
                    SendStreamingMessageStream,
                    SendMessageRequest,
                    StreamResponse
                ),
                Some("/GetTask") => unary!(inner, req, get_task, GetTaskRequest, Task),
                Some("/ListTasks") => unary!(inner, req, list_tasks, ListTasksRequest, ListTasksResponse),
                Some("/CancelTask") => unary!(inner, req, cancel_task, CancelTaskRequest, Task),
                Some("/SubscribeToTask") => streaming!(
                    inner,
                    req,
                    subscribe_to_task,
                    SubscribeToTaskStream,
                    SubscribeToTaskRequest,
                    StreamResponse
                ),
                Some("/CreateTaskPushNotificationConfig") => unary!(
                    inner,
                    req,
                    create_task_push_notification_config,
                    TaskPushNotificationConfig,
                    TaskPushNotificationConfig
                ),
                Some("/GetTaskPushNotificationConfig") => unary!(
                    inner,
                    req,
                    get_task_push_notification_config,
                    GetTaskPushNotificationConfigRequest,
                    TaskPushNotificationConfig
                ),
                Some("/ListTaskPushNotificationConfigs") => unary!(
                    inner,
                    req,
                    list_task_push_notification_configs,
                    ListTaskPushNotificationConfigsRequest,
                    ListTaskPushNotificationConfigsResponse
                ),
                Some("/GetExtendedAgentCard") => {
                    unary!(
                        inner,
                        req,
                        get_extended_agent_card,
                        GetExtendedAgentCardRequest,
                        AgentCard
                    )
                }
                Some("/DeleteTaskPushNotificationConfig") => unary!(
                    inner,
                    req,
                    delete_task_push_notification_config,
                    DeleteTaskPushNotificationConfigRequest,
                    Empty
                ),
                _ => Box::pin(async move { Ok(Status::unimplemented("").into_http()) }),
            }
        }
    }
}

/// The client side: [`A2aServiceClient`](a2a_service_client::A2aServiceClient) over a `tonic`
/// channel.
pub mod a2a_service_client {
    use tonic::codegen::http::uri::PathAndQuery;
    use tonic::codegen::{http, Body, Bytes, InterceptedService, StdError};
    use tonic::{IntoRequest, Response, Status};

    use super::super::*;
    use super::{WireCodec, SERVICE};

    /// Calls `A2AService` over `T`, usually a `tonic::transport::Channel`.
    #[derive(Debug, Clone)]
    pub struct A2aServiceClient<T> {
        inner: tonic::client::Grpc<T>,
    }

    impl<T> A2aServiceClient<T>
    where
        T: tonic::client::GrpcService<tonic::body::Body>,
        T::Error: Into<StdError>,
        T::ResponseBody: Body<Data = Bytes> + Send + 'static,
        <T::ResponseBody as Body>::Error: Into<StdError> + Send,
    {
        /// A client over `inner`.
        pub fn new(inner: T) -> Self {
            A2aServiceClient {
                inner: tonic::client::Grpc::new(inner),
            }
        }

        /// A client whose calls pass through `interceptor` first (to add metadata, say).
        pub fn with_interceptor<F>(inner: T, interceptor: F) -> A2aServiceClient<InterceptedService<T, F>>
        where
            F: tonic::service::Interceptor,
            T::ResponseBody: Default,
            T: tonic::codegen::Service<
                http::Request<tonic::body::Body>,
                Response = http::Response<<T as tonic::client::GrpcService<tonic::body::Body>>::ResponseBody>,
            >,
            <T as tonic::codegen::Service<http::Request<tonic::body::Body>>>::Error:
                Into<StdError> + Send + Sync,
        {
            A2aServiceClient::new(InterceptedService::new(inner, interceptor))
        }

        /// Waits for the channel, then wraps `request` for `/{SERVICE}/{method}`.
        async fn prepare<Rq>(
            &mut self,
            method: &'static str,
            request: impl IntoRequest<Rq>,
        ) -> Result<(tonic::Request<Rq>, PathAndQuery), Status> {
            self.inner
                .ready()
                .await
                .map_err(|e| Status::unknown(format!("Service was not ready: {}", e.into())))?;
            let path = PathAndQuery::from_maybe_shared(format!("/{SERVICE}/{method}"))
                .map_err(|e| Status::internal(format!("invalid method path: {e}")))?;
            let mut request = request.into_request();
            request
                .extensions_mut()
                .insert(tonic::GrpcMethod::new(SERVICE, method));
            Ok((request, path))
        }
    }

    /// The RPCs. Each is `name(request) -> Response<reply>` (or a stream of replies).
    macro_rules! rpcs {
        ($($name:ident, $method:literal, $rq:ty => $rs:ty;)*) => {
            impl<T> A2aServiceClient<T>
            where
                T: tonic::client::GrpcService<tonic::body::Body>,
                T::Error: Into<StdError>,
                T::ResponseBody: Body<Data = Bytes> + Send + 'static,
                <T::ResponseBody as Body>::Error: Into<StdError> + Send,
            {
                $(
                    #[doc = concat!("`", $method, "`.")]
                    pub async fn $name(
                        &mut self,
                        request: impl IntoRequest<$rq>,
                    ) -> Result<Response<$rs>, Status> {
                        let (req, path) = self.prepare($method, request).await?;
                        self.inner.unary(req, path, WireCodec::<$rq, $rs>::default()).await
                    }
                )*
            }
        };
    }

    rpcs! {
        send_message, "SendMessage", SendMessageRequest => SendMessageResponse;
        get_task, "GetTask", GetTaskRequest => Task;
        list_tasks, "ListTasks", ListTasksRequest => ListTasksResponse;
        cancel_task, "CancelTask", CancelTaskRequest => Task;
        create_task_push_notification_config, "CreateTaskPushNotificationConfig",
            TaskPushNotificationConfig => TaskPushNotificationConfig;
        get_task_push_notification_config, "GetTaskPushNotificationConfig",
            GetTaskPushNotificationConfigRequest => TaskPushNotificationConfig;
        list_task_push_notification_configs, "ListTaskPushNotificationConfigs",
            ListTaskPushNotificationConfigsRequest => ListTaskPushNotificationConfigsResponse;
        get_extended_agent_card, "GetExtendedAgentCard", GetExtendedAgentCardRequest => AgentCard;
        delete_task_push_notification_config, "DeleteTaskPushNotificationConfig",
            DeleteTaskPushNotificationConfigRequest => Empty;
    }

    impl<T> A2aServiceClient<T>
    where
        T: tonic::client::GrpcService<tonic::body::Body>,
        T::Error: Into<StdError>,
        T::ResponseBody: Body<Data = Bytes> + Send + 'static,
        <T::ResponseBody as Body>::Error: Into<StdError> + Send,
    {
        /// `SendStreamingMessage`.
        pub async fn send_streaming_message(
            &mut self,
            request: impl IntoRequest<SendMessageRequest>,
        ) -> Result<Response<tonic::Streaming<StreamResponse>>, Status> {
            let (req, path) = self.prepare("SendStreamingMessage", request).await?;
            self.inner
                .server_streaming(
                    req,
                    path,
                    WireCodec::<SendMessageRequest, StreamResponse>::default(),
                )
                .await
        }

        /// `SubscribeToTask`.
        pub async fn subscribe_to_task(
            &mut self,
            request: impl IntoRequest<SubscribeToTaskRequest>,
        ) -> Result<Response<tonic::Streaming<StreamResponse>>, Status> {
            let (req, path) = self.prepare("SubscribeToTask", request).await?;
            self.inner
                .server_streaming(
                    req,
                    path,
                    WireCodec::<SubscribeToTaskRequest, StreamResponse>::default(),
                )
                .await
        }
    }
}
