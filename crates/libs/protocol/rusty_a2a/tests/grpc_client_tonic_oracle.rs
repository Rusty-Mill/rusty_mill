//! `GrpcClient` against a real `tonic` server speaking the A2A messages: the independent oracle
//! for the client's side of the wire. The server here is the `tonic`-shaped one in
//! `common/tonic_oracle.rs`, implementing just the RPCs these tests call.
#![allow(clippy::unwrap_used)]

mod common;

use std::pin::Pin;
use std::time::Duration;

use common::tonic_oracle as pb;
use futures_core::Stream;
use futures_util::StreamExt;
use pb::a2a_service_server::{A2aService, A2aServiceServer};
use rusty_a2a::client::{ClientError, GrpcClient};
use rusty_a2a::error::A2aError;
use rusty_a2a::types::{Message, StreamResponse};
use tonic::{async_trait, Request, Response, Status};

type Events = Pin<Box<dyn Stream<Item = std::result::Result<pb::StreamResponse, Status>> + Send>>;

fn task(id: &str) -> pb::Task {
    pb::Task {
        id: id.to_string(),
        status: Some(pb::TaskStatus {
            state: pb::TaskState::Completed as i32,
            ..Default::default()
        }),
        ..Default::default()
    }
}

struct Oracle;

#[async_trait]
impl A2aService for Oracle {
    async fn get_task(
        &self,
        request: Request<pb::GetTaskRequest>,
    ) -> std::result::Result<Response<pb::Task>, Status> {
        let id = request.into_inner().id;
        if id == "missing" {
            // What `tonic` sends for an error with details: a bare status plus the encoded
            // `google.rpc.Status` in `grpc-status-details-bin`.
            let info = rusty_prost::rpc::ErrorInfo {
                reason: "TASK_NOT_FOUND".to_string(),
                domain: "a2a-protocol.org".to_string(),
                metadata: Default::default(),
            };
            let details = rusty_prost::rpc::Status {
                code: tonic::Code::NotFound as i32,
                message: "no such task".to_string(),
                details: vec![rusty_prost::wkt::Any::pack(&info).unwrap()],
            };
            return Err(Status::with_details(
                tonic::Code::NotFound,
                "no such task",
                rusty_prost::Message::encode_to_vec(&details).unwrap().into(),
            ));
        }
        Ok(Response::new(task(&id)))
    }

    type SendStreamingMessageStream = Events;

    async fn send_streaming_message(
        &self,
        _request: Request<pb::SendMessageRequest>,
    ) -> std::result::Result<Response<Events>, Status> {
        let events = (0..3).map(|n| {
            Ok(pb::StreamResponse {
                payload: Some(pb::stream_response::Payload::Task(task(&format!("t{n}")))),
            })
        });
        Ok(Response::new(Box::pin(futures_util::stream::iter(events))))
    }

    type SubscribeToTaskStream = Events;
}

async fn serve() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(
        tonic::transport::Server::builder()
            .add_service(A2aServiceServer::new(Oracle))
            .serve_with_incoming(tokio_stream_incoming(listener)),
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    format!("http://{addr}")
}

/// The listener as the stream of connections `tonic` serves.
fn tokio_stream_incoming(
    listener: tokio::net::TcpListener,
) -> impl Stream<Item = std::io::Result<tokio::net::TcpStream>> {
    futures_util::stream::unfold(listener, |listener| async move {
        let accepted = listener.accept().await.map(|(stream, _)| stream);
        Some((accepted, listener))
    })
}

#[tokio::test]
async fn a_unary_reply_from_a_tonic_server_is_read() {
    let client = GrpcClient::new(serve().await).unwrap();
    let got = client.get_task("t-1", None).await.unwrap();
    assert_eq!(got.id, "t-1");
}

#[tokio::test]
async fn a_tonic_servers_error_details_pick_the_error() {
    let client = GrpcClient::new(serve().await).unwrap();
    match client.get_task("missing", None).await {
        Err(ClientError::Protocol(A2aError::TaskNotFound(message))) => {
            assert_eq!(message, "no such task");
        }
        other => panic!("expected TaskNotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn a_stream_from_a_tonic_server_is_read_in_order_and_ends() {
    let client = GrpcClient::new(serve().await).unwrap();
    let mut stream = client
        .send_streaming_message(Message::user_text("hi"), None)
        .await
        .unwrap();
    let mut ids = Vec::new();
    while let Some(item) = stream.next().await {
        match item.unwrap() {
            StreamResponse::Task { task } => ids.push(task.id),
            other => panic!("unexpected event {other:?}"),
        }
    }
    assert_eq!(ids, ["t0", "t1", "t2"]);
}

#[tokio::test]
async fn an_rpc_the_tonic_server_does_not_implement_is_an_unimplemented_error() {
    let client = GrpcClient::new(serve().await).unwrap();
    match client.get_extended_agent_card().await {
        Err(ClientError::Protocol(A2aError::MethodNotFound(_))) => {}
        other => panic!("expected MethodNotFound, got {other:?}"),
    }
}
