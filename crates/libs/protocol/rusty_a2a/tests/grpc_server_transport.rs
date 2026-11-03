//! The gRPC server's behaviour at the transport, driven by a real `tonic` client (the oracle in
//! `common/tonic_oracle.rs`): deadlines, message limits, a client that leaves, and a port that is
//! taken. These hold whichever stack the server is built on.
#![allow(clippy::unwrap_used)]

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use common::tonic_oracle as pb;
use rusty_a2a::error::Result;
use rusty_a2a::server::{AgentExecutor, AgentServer, EventSink, RequestContext};
use rusty_a2a::types::{AgentCard, AgentInterface, TaskState};
use tonic::transport::Channel;
use tonic::{Code, Request};

/// Accepts a task and then never finishes it, so a blocking `SendMessage` waits for ever.
struct Stalls;

#[async_trait]
impl AgentExecutor for Stalls {
    async fn execute(&self, _ctx: RequestContext, events: EventSink) -> Result<()> {
        events.status(TaskState::Working);
        std::future::pending::<()>().await;
        Ok(())
    }
}

fn card(port: u16) -> AgentCard {
    AgentCard::new(
        "gRPC transport test agent",
        "Stalls on purpose.",
        "0.0.0",
        AgentInterface::json_rpc(format!("http://127.0.0.1:{port}")),
    )
}

/// A listener on a port of its own, bound before serving so no other test can take the port.
fn listen() -> (std::net::TcpListener, u16) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    (listener, port)
}

/// Serves a stalling agent on a port of its own and returns its URL.
async fn start() -> String {
    let (listener, port) = listen();
    let services = AgentServer::new(card(port), Arc::new(Stalls)).build();
    tokio::spawn(async move { services.serve_grpc_on(listener).await.unwrap() });
    format!("http://127.0.0.1:{port}")
}

async fn connect(url: String) -> pb::a2a_service_client::A2aServiceClient<Channel> {
    let channel = Channel::from_shared(url).unwrap().connect().await.unwrap();
    pb::a2a_service_client::A2aServiceClient::new(channel)
}

fn versioned<T>(message: T) -> Request<T> {
    let mut request = Request::new(message);
    request
        .metadata_mut()
        .insert("a2a-version", "1.0".parse().unwrap());
    request
}

fn send_request(text: &str) -> pb::SendMessageRequest {
    pb::SendMessageRequest {
        message: Some(pb::Message {
            message_id: "m1".to_string(),
            role: pb::Role::User as i32,
            parts: vec![pb::Part {
                content: Some(pb::part::Content::Text(text.to_string())),
                ..Default::default()
            }],
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// One call made with a bare HTTP/2 client, which enforces no deadline of its own (a `tonic`
/// client also reads `grpc-timeout` and cuts the call off itself, so it cannot show the server's
/// answer). Returns the call's status and how long it took.
fn raw_call_status(url: &str, timeout: Duration) -> (rusty_tonic::Status, Duration) {
    let addr = url.trim_start_matches("http://").to_string();
    let runtime = rusty_tokio::Runtime::new().unwrap();
    runtime.block_on(async move {
        let mut head = rusty_tonic::RequestHead::new(rusty_a2a::pb::path::SEND_MESSAGE, "localhost");
        head.timeout = Some(timeout);
        head.metadata.insert("a2a-version", "1.0").unwrap();
        let headers: Vec<_> = head
            .to_headers()
            .iter()
            .map(|(n, v)| rusty_hyper::HeaderField::new(n.as_str(), v.as_slice()))
            .collect();
        let started = Instant::now();
        let conn = rusty_hyper::connect(&format!("http://{addr}"), rusty_hyper::ConnectOptions::default())
            .await
            .unwrap();
        let (mut send, response) = conn.client().send_request(headers, false).await.unwrap();
        send.send_data(rusty_tonic::encode_message(&send_request("wait")).unwrap(), true)
            .await
            .unwrap();
        let mut response = response.await.unwrap();
        let mut trailers = None;
        while let Some(piece) = response.body.next().await {
            if let rusty_hyper::Body::Trailers(t) = piece.unwrap() {
                trailers = Some(t);
            }
        }
        // A reply that ended in its first headers carries the status there.
        let fields = trailers.unwrap_or(response.headers);
        let pairs: Vec<(&str, &[u8])> = fields
            .iter()
            .filter_map(|h| Some((std::str::from_utf8(&h.name).ok()?, h.value.as_slice())))
            .collect();
        (
            rusty_tonic::Status::from_headers(pairs).unwrap(),
            started.elapsed(),
        )
    })
}

#[tokio::test]
async fn a_call_past_its_grpc_timeout_ends_deadline_exceeded() {
    let url = start().await;
    let (status, took) =
        tokio::task::spawn_blocking(move || raw_call_status(&url, Duration::from_millis(300)))
            .await
            .unwrap();
    assert_eq!(status.code(), rusty_tonic::Code::DeadlineExceeded, "{status:?}");
    assert!(
        took >= Duration::from_millis(250) && took < Duration::from_secs(5),
        "{took:?}"
    );
}

#[tokio::test]
async fn a_request_over_the_size_limit_is_resource_exhausted() {
    let mut client = connect(start().await).await;
    let status = client
        .send_message(versioned(send_request(&"x".repeat(5 * 1024 * 1024))))
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::ResourceExhausted);
}

#[tokio::test]
async fn a_client_that_gives_up_on_a_waiting_call_leaves_the_server_serving() {
    let mut client = connect(start().await).await;
    let waiting = client.send_message(versioned(send_request("wait")));
    assert!(tokio::time::timeout(Duration::from_millis(300), waiting)
        .await
        .is_err());
    // The same connection, straight afterwards.
    let status = client
        .get_task(versioned(pb::GetTaskRequest {
            id: "no-such-task".to_string(),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::NotFound);
}

#[tokio::test]
async fn serving_on_an_address_that_is_taken_fails() {
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let services = AgentServer::new(card(port), Arc::new(Stalls)).build();
    let served = tokio::time::timeout(
        Duration::from_secs(5),
        services.serve_grpc(([127, 0, 0, 1], port)),
    )
    .await
    .expect("serve_grpc should fail at once, not run");
    assert!(served.is_err());
}

/// Waits (up to five seconds) for the port to be held (`held`) or free (`!held`).
async fn port_becomes(port: u16, held: bool) -> bool {
    for _ in 0..250 {
        let is_held = std::net::TcpListener::bind(("127.0.0.1", port)).is_err();
        if is_held == held {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

#[tokio::test]
async fn dropping_serve_grpc_closes_the_listener() {
    let (listener, port) = listen();
    let services = AgentServer::new(card(port), Arc::new(Stalls)).build();
    let task = tokio::spawn(async move { services.serve_grpc_on(listener).await });
    // A call answered proves the server took the listener over, so dropping it is what is tested.
    let status = connect(format!("http://127.0.0.1:{port}"))
        .await
        .get_task(versioned(pb::GetTaskRequest {
            id: "no-such-task".to_string(),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::NotFound);
    task.abort();
    let _ = task.await;
    // The runtime that held the listener is stopped on a thread of its own: allow it a moment.
    assert!(
        port_becomes(port, false).await,
        "the port was still held after serve_grpc was dropped"
    );
}

/// The application runtime stops while the transport stays up: a stream already begun must end
/// in an error, never in a clean `OK` that would pass a truncated stream off as complete.
#[test]
fn a_stream_cut_short_by_the_application_stopping_ends_in_an_error() {
    let app = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let transport = rusty_tokio::Runtime::new().unwrap();
    let (listener, port) = listen();
    let router = {
        let _enter = app.enter();
        AgentServer::new(card(port).with_streaming(true), Arc::new(Stalls))
            .build()
            .grpc_router(app.handle().clone())
    };
    let _server = {
        let _enter = transport.enter();
        rusty_tokio::io::TcpListener::from_std(listener)
            .and_then(|listener| router.serve(listener, None, rusty_hyper::ServeOptions::default()))
            .unwrap()
    };
    let client_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let ended = client_runtime.block_on(async {
        let mut client = connect(format!("http://127.0.0.1:{port}")).await;
        let mut stream = client
            .send_streaming_message(versioned(send_request("stream")))
            .await
            .unwrap()
            .into_inner();
        // The agent accepts the task (a snapshot, then `Working`) and then stalls.
        for _ in 0..2 {
            let event = tokio::time::timeout(Duration::from_secs(5), stream.message())
                .await
                .expect("the opening events should arrive");
            assert!(event.unwrap().is_some());
        }
        app.shutdown_background();
        tokio::time::timeout(Duration::from_secs(5), stream.message())
            .await
            .expect("a stopped application must end the stream, not hang it")
    });
    let status = ended.expect_err("the stream was reported complete though it was cut short");
    assert_eq!(status.code(), Code::Unavailable, "{status:?}");
}
