//! `GrpcClient` over the first-party transport (`rusty_tonic` on `rusty_tokio`), called from a
//! `tokio` application, against servers built from `rusty_tonic` itself: TLS (which the old
//! `tonic` build could not do at all), a hostile reply, cancellation, backpressure and a
//! runtime that has one thread. The behaviour against a real `tonic` server is in
//! `grpc_client.rs`.
#![allow(clippy::unwrap_used)]

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use rcgen::{BasicConstraints, Certificate, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair};
use rustls::pki_types::CertificateDer;
use rusty_a2a::client::{ClientError, GrpcClient};
use rusty_a2a::error::A2aError;
use rusty_a2a::pb;
use rusty_hyper::{serve, ConnectOptions, HeaderField, ServeOptions};
use rusty_tls::{TlsAcceptor, TrustPolicy};
use rusty_tonic::client::{ChannelOptions, Request, Response};
use rusty_tonic::server::{Router, Sink};
use rusty_tonic::Code;

const GET_TASK: &str = "/lf.a2a.v1.A2AService/GetTask";
const STREAM: &str = "/lf.a2a.v1.A2AService/SendStreamingMessage";

/// A server on its own `rusty_tokio` runtime, kept alive by this value.
struct Served {
    _runtime: rusty_tokio::Runtime,
    server: rusty_hyper::Server,
}

impl Served {
    fn addr(&self) -> SocketAddr {
        self.server.local_addr()
    }
}

fn serve_router(router: Router, tls: Option<TlsAcceptor>, options: ServeOptions) -> Served {
    let runtime = rusty_tokio::Runtime::new().unwrap();
    let _guard = runtime.enter();
    let listener =
        rusty_tokio::io::TcpListener::from_std(std::net::TcpListener::bind("127.0.0.1:0").unwrap()).unwrap();
    let server = router.serve(listener, tls, options).unwrap();
    drop(_guard);
    Served {
        _runtime: runtime,
        server,
    }
}

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

fn get_task_router() -> Router {
    Router::new().unary(GET_TASK, |req: Request<pb::GetTaskRequest>| async move {
        Ok(Response::new(task(&req.message.id)))
    })
}

fn event(n: usize) -> pb::StreamResponse {
    pb::StreamResponse {
        payload: Some(pb::stream_response::Payload::Task(task(&format!("t{n}")))),
    }
}

/// Sets a flag when dropped: the handler's future ended, however it ended. Unwinding from a
/// panic does not count: a handler that fell over proves nothing about cancellation.
struct Ended(Arc<AtomicBool>);

impl Drop for Ended {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            self.0.store(true, Ordering::SeqCst);
        }
    }
}

async fn soon(flag: &AtomicBool) -> bool {
    for _ in 0..200 {
        if flag.load(Ordering::SeqCst) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    false
}

// ---- TLS ------------------------------------------------------------------------------------

/// One CA that signs the server's certificate.
struct Pki {
    ca: Certificate,
    ca_key: KeyPair,
}

impl Pki {
    fn new() -> Pki {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "a2a test CA");
        params.distinguished_name = dn;
        let ca_key = KeyPair::generate().unwrap();
        let ca = params.self_signed(&ca_key).unwrap();
        Pki { ca, ca_key }
    }

    fn acceptor(&self) -> TlsAcceptor {
        let params = CertificateParams::new(vec!["localhost".to_string(), "127.0.0.1".to_string()]).unwrap();
        let key = KeyPair::generate().unwrap();
        let cert = params.signed_by(&key, &self.ca, &self.ca_key).unwrap();
        TlsAcceptor::new_with_alpn(
            vec![cert.der().to_vec()],
            key.serialize_der(),
            vec![b"h2".to_vec()],
        )
        .unwrap()
    }

    fn trusting(&self) -> ChannelOptions {
        ChannelOptions {
            connect: ConnectOptions {
                trust: TrustPolicy::PinnedAnchors(vec![CertificateDer::from(self.ca.der().to_vec())]),
                ..ConnectOptions::default()
            },
            ..ChannelOptions::default()
        }
    }
}

#[tokio::test]
async fn an_https_agent_is_reached_when_its_ca_is_trusted() {
    let pki = Pki::new();
    let served = serve_router(get_task_router(), Some(pki.acceptor()), ServeOptions::default());
    let client =
        GrpcClient::with_channel_options(format!("https://{}", served.addr()), pki.trusting()).unwrap();
    let got = client.get_task("t-tls", None).await.unwrap();
    assert_eq!(got.id, "t-tls");
}

#[tokio::test]
async fn an_https_agent_with_an_untrusted_certificate_is_refused() {
    let pki = Pki::new();
    let served = serve_router(get_task_router(), Some(pki.acceptor()), ServeOptions::default());
    // The default trust policy is the operating system's anchors, which do not include this CA.
    let client = GrpcClient::new(format!("https://{}", served.addr())).unwrap();
    match client.get_task("t", None).await {
        Err(ClientError::Grpc(status)) => assert_eq!(status.code(), Code::Unavailable),
        other => panic!("expected an unavailable agent, got {other:?}"),
    }
}

// ---- a hostile reply ------------------------------------------------------------------------

#[tokio::test]
async fn a_malformed_status_details_header_is_an_error_and_not_a_panic() {
    // `tonic` unwraps the base64 of `grpc-status-details-bin`, so a server could panic its
    // client's call with this one header. The status keeps its code and message.
    let runtime = rusty_tokio::Runtime::new().unwrap();
    let _guard = runtime.enter();
    let listener =
        rusty_tokio::io::TcpListener::from_std(std::net::TcpListener::bind("127.0.0.1:0").unwrap()).unwrap();
    let server = serve(
        listener,
        None,
        ServeOptions::default(),
        |_req, respond| async move {
            let headers = vec![
                HeaderField::new(":status", "200"),
                HeaderField::new("content-type", "application/grpc"),
                HeaderField::new("grpc-status", "5"),
                HeaderField::new("grpc-message", "nope"),
                HeaderField::new("grpc-status-details-bin", "%%% not base64 %%%"),
            ];
            let _ = respond.send_response(headers, true).await;
        },
    )
    .unwrap();
    drop(_guard);
    let client = GrpcClient::new(format!("http://{}", server.local_addr())).unwrap();
    match client.get_task("missing", None).await {
        Err(ClientError::Protocol(A2aError::TaskNotFound(message))) => {
            assert_eq!(message, "nope");
        }
        other => panic!("expected TaskNotFound, got {other:?}"),
    }
    drop(runtime);
}

// ---- cancellation and backpressure ----------------------------------------------------------

#[tokio::test]
async fn dropping_a_response_stream_cancels_the_call() {
    let ended = Arc::new(AtomicBool::new(false));
    let flag = ended.clone();
    let router = Router::new().server_streaming(
        STREAM,
        move |_req: Request<pb::SendMessageRequest>, mut out: Sink<pb::StreamResponse>| {
            let guard = Ended(flag.clone());
            async move {
                let _guard = guard;
                loop {
                    out.send(&event(0)).await?;
                    rusty_tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        },
    );
    let served = serve_router(router, None, ServeOptions::default());
    let client = GrpcClient::new(format!("http://{}", served.addr())).unwrap();
    let mut stream = client
        .send_streaming_message(rusty_a2a::types::Message::user_text("hi"), None)
        .await
        .unwrap();
    assert!(stream.next().await.unwrap().is_ok());
    drop(stream);
    assert!(
        soon(&ended).await,
        "the server kept sending after the client dropped the stream"
    );
}

#[tokio::test]
async fn dropping_a_stream_the_server_has_gone_quiet_on_frees_its_stream_slot() {
    // Nothing more arrives, so only dropping the stream itself can end the call: the transport
    // task is parked reading the next reply, and it is aborted, not told by a closed channel.
    let mut h2 = rusty_h2::server::builder::Builder::new();
    h2.max_concurrent_streams(1);
    let options = ServeOptions {
        h2,
        ..ServeOptions::default()
    };
    let router = get_task_router().server_streaming(
        STREAM,
        |_req: Request<pb::SendMessageRequest>, mut out: Sink<pb::StreamResponse>| async move {
            out.send(&event(0)).await?;
            rusty_tokio::time::sleep(Duration::from_secs(30)).await;
            Ok(())
        },
    );
    let served = serve_router(router, None, options);
    let client = GrpcClient::new(format!("http://{}", served.addr())).unwrap();
    let mut stream = client
        .send_streaming_message(rusty_a2a::types::Message::user_text("hi"), None)
        .await
        .unwrap();
    assert!(stream.next().await.unwrap().is_ok());
    drop(stream);
    let next = tokio::time::timeout(Duration::from_secs(5), client.get_task("after", None))
        .await
        .expect("the dropped stream still held the only stream slot")
        .unwrap();
    assert_eq!(next.id, "after");
}

#[tokio::test]
async fn a_stream_that_fails_yields_its_events_then_one_error_then_ends() {
    let router = Router::new().server_streaming(
        STREAM,
        |_req: Request<pb::SendMessageRequest>, mut out: Sink<pb::StreamResponse>| async move {
            out.send(&event(0)).await?;
            out.send(&event(1)).await?;
            Err(rusty_tonic::Status::aborted("boom"))
        },
    );
    let served = serve_router(router, None, ServeOptions::default());
    let client = GrpcClient::new(format!("http://{}", served.addr())).unwrap();
    let mut stream = client
        .send_streaming_message(rusty_a2a::types::Message::user_text("hi"), None)
        .await
        .unwrap();
    assert!(stream.next().await.unwrap().is_ok());
    assert!(stream.next().await.unwrap().is_ok());
    match stream.next().await {
        Some(Err(ClientError::Grpc(status))) => assert_eq!(status.code(), Code::Aborted),
        other => panic!("expected the stream's error, got {other:?}"),
    }
    assert!(stream.next().await.is_none(), "the stream should have ended");
}

#[tokio::test]
async fn a_stream_the_server_refuses_at_the_start_fails_the_call_with_its_status() {
    let router = Router::new().server_streaming(
        "/lf.a2a.v1.A2AService/SubscribeToTask",
        |_req: Request<pb::SubscribeToTaskRequest>, _out: Sink<pb::StreamResponse>| async move {
            Err(rusty_tonic::Status::not_found("no such task"))
        },
    );
    let served = serve_router(router, None, ServeOptions::default());
    let client = GrpcClient::new(format!("http://{}", served.addr())).unwrap();
    match client.subscribe_to_task("missing").await {
        Err(ClientError::Protocol(A2aError::TaskNotFound(message))) => {
            assert_eq!(message, "no such task");
        }
        other => panic!("expected TaskNotFound, got {:?}", other.err()),
    }
}

#[tokio::test]
async fn a_status_with_error_info_is_mapped_by_its_reason_not_its_bare_code() {
    // FAILED_PRECONDITION alone would read as "task not cancelable"; the reason says otherwise.
    let router = Router::new().unary(GET_TASK, |_req: Request<pb::GetTaskRequest>| async move {
        Err::<Response<pb::Task>, _>(rusty_tonic::Status::with_error_info(
            Code::FailedPrecondition,
            "too new",
            "VERSION_NOT_SUPPORTED",
            "a2a-protocol.org",
            Default::default(),
        ))
    });
    let served = serve_router(router, None, ServeOptions::default());
    let client = GrpcClient::new(format!("http://{}", served.addr())).unwrap();
    match client.get_task("t", None).await {
        Err(ClientError::Protocol(A2aError::VersionNotSupported(message))) => {
            assert_eq!(message, "too new");
        }
        other => panic!("expected VersionNotSupported, got {other:?}"),
    }
}

#[tokio::test]
async fn a_slow_consumer_loses_no_events_and_sees_them_in_order() {
    const N: usize = 300;
    let router = Router::new().server_streaming(
        STREAM,
        |_req: Request<pb::SendMessageRequest>, mut out: Sink<pb::StreamResponse>| async move {
            for n in 0..N {
                out.send(&event(n)).await?;
            }
            Ok(())
        },
    );
    let served = serve_router(router, None, ServeOptions::default());
    let client = GrpcClient::new(format!("http://{}", served.addr())).unwrap();
    let mut stream = client
        .send_streaming_message(rusty_a2a::types::Message::user_text("hi"), None)
        .await
        .unwrap();
    let mut ids = Vec::new();
    while let Some(item) = stream.next().await {
        match item.unwrap() {
            rusty_a2a::types::StreamResponse::Task { task } => ids.push(task.id),
            other => panic!("unexpected event {other:?}"),
        }
        if ids.len() % 50 == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
    let expected: Vec<String> = (0..N).map(|n| format!("t{n}")).collect();
    assert_eq!(ids, expected);
}

#[tokio::test]
async fn a_call_dropped_by_its_caller_gives_up_its_stream_slot() {
    let mut h2 = rusty_h2::server::builder::Builder::new();
    h2.max_concurrent_streams(1);
    let options = ServeOptions {
        h2,
        ..ServeOptions::default()
    };
    let router = Router::new().unary(GET_TASK, |req: Request<pb::GetTaskRequest>| async move {
        if req.message.id == "slow" {
            rusty_tokio::time::sleep(Duration::from_secs(30)).await;
        }
        Ok(Response::new(task(&req.message.id)))
    });
    let served = serve_router(router, None, options);
    let client = GrpcClient::new(format!("http://{}", served.addr())).unwrap();
    let slow = client.get_task("slow", None);
    assert!(tokio::time::timeout(Duration::from_millis(300), slow)
        .await
        .is_err());
    let started = Instant::now();
    let fast = client.get_task("fast", None).await.unwrap();
    assert_eq!(fast.id, "fast");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the abandoned call still held the only stream slot"
    );
}

// ---- the application's runtime --------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn many_calls_at_once_work_from_a_runtime_with_one_thread() {
    let served = serve_router(get_task_router(), None, ServeOptions::default());
    let client = GrpcClient::new(format!("http://{}", served.addr())).unwrap();
    let calls = (0..40).map(|n| {
        let client = client.clone();
        async move { client.get_task(format!("t{n}"), None).await.unwrap().id }
    });
    let ids = futures_util::future::join_all(calls).await;
    let expected: Vec<String> = (0..40).map(|n| format!("t{n}")).collect();
    assert_eq!(ids, expected);
}

#[tokio::test]
async fn an_agent_that_is_not_there_is_unavailable_and_not_a_hang() {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let client = GrpcClient::new(format!("http://127.0.0.1:{port}")).unwrap();
    let started = Instant::now();
    match client.get_task("t", None).await {
        Err(ClientError::Grpc(status)) => assert_eq!(status.code(), Code::Unavailable),
        other => panic!("expected an unavailable agent, got {other:?}"),
    }
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[tokio::test]
async fn an_endpoint_without_a_scheme_is_refused_up_front_with_a_clear_error() {
    // The `tonic` client parsed `host:port` and then failed its first call with "transport error".
    // Whether a scheme-less endpoint means plaintext or TLS is a choice this client does not make.
    match GrpcClient::new("127.0.0.1:50051") {
        Err(ClientError::GrpcConfig(message)) => {
            assert!(message.contains("invalid gRPC endpoint"), "{message}");
        }
        other => panic!("expected a configuration error, got {:?}", other.err()),
    }
}
