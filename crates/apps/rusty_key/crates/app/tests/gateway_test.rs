//! Phase 14 gateway contract test (status codes, auth, SSE frames, multi-session
//! isolation). Requests go through `Router::call` in-process — no port bind,
//! fully offline — except the last test, which serves over a real socket on
//! `rusty_tokio` the way `gateway::serve` does.
#![cfg(feature = "gateway")]

use std::io::{Read, Write};
use std::sync::Arc;

use rk_app::gateway::{Gateway, Mode};
use rk_config::Config;
use rk_kernel::fake::{FakeLanguageModel, Scripted};
use rusty_axum::{Method, Request, Router, StatusCode};

fn config_at(ws: &std::path::Path) -> Config {
    let s = ws.to_string_lossy().into_owned();
    Config::resolve(move |k| match k {
        "RUSTYKEYS_MODEL" => Some("fake".into()),
        "RUSTYKEYS_WORKSPACE" => Some(s.clone()),
        _ => None,
    })
    .unwrap()
}

fn tmp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("rk-gw-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn router<M>(gw: Gateway<M>) -> Router
where
    M: aisdk::core::language_model::LanguageModel
        + aisdk::core::capabilities::TextInputSupport
        + aisdk::core::capabilities::ToolCallSupport
        + Clone
        + Send
        + Sync
        + 'static,
{
    Arc::new(gw).router(tokio::runtime::Handle::current())
}

/// Sends `req`; returns the status and the whole body as text.
async fn send(app: &Router, req: Request) -> (StatusCode, String) {
    let resp = app.call(req).await;
    let bytes = resp.body.collect().await.unwrap();
    (resp.status, String::from_utf8_lossy(&bytes).into_owned())
}

fn get(path: &str) -> Request {
    Request::new(Method::Get, path)
}

fn post_json(path: &str, json: &str) -> Request {
    Request::new(Method::Post, path)
        .with_header("content-type", "application/json")
        .unwrap()
        .with_body(json.as_bytes().to_vec())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_and_health_round_trip() {
    let dir = tmp("chat");
    let model = FakeLanguageModel::new(vec![vec![Scripted::Text("hello from gateway".into())]]);
    let app = router(Gateway::new(config_at(&dir), model));

    let (status, body) = send(&app, get("/health")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"status\":\"ok\""));

    // /ready — readiness probe: model + workspace + SQLite round-trip.
    let (status, body) = send(&app, get("/ready")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"ready\":true"), "expected ready: {body}");

    let (status, body) = send(&app, post_json("/chat", r#"{"message":"hi"}"#)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("hello from gateway"));
    assert!(body.contains("\"verified\":true"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn metrics_pull_reflects_turn_telemetry() {
    let dir = tmp("metrics");
    let s = dir.to_string_lossy().into_owned();
    let config = Config::resolve(move |k| match k {
        "RUSTYKEYS_MODEL" => Some("fake".into()),
        "RUSTYKEYS_WORKSPACE" => Some(s.clone()),
        "RUSTYKEYS_OTLP_ENDPOINT" => Some("http://localhost:4317".into()),
        _ => None,
    })
    .unwrap();
    let model = FakeLanguageModel::new(vec![vec![Scripted::Text("hi back".into())]]);
    let app = router(Gateway::new(config, model));

    // Drive one turn so the exporter accumulates.
    let (status, _) = send(&app, post_json("/chat", r#"{"message":"hi"}"#)).await;
    assert_eq!(status, StatusCode::OK);

    // Pull telemetry: a host-boundary scrape, no push from inside the turn.
    let (status, body) = send(&app, get("/metrics")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("\"enabled\":true"),
        "expected enabled: {body}"
    );
    assert!(body.contains("\"turns\":1"), "expected one turn: {body}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bearer_auth_gates_requests() {
    let dir = tmp("auth");
    let model = FakeLanguageModel::new(vec![]);
    let app = router(Gateway::new(config_at(&dir), model).with_secret(Some("s3cr3t".into())));

    // No token → 401.
    let (status, _) = send(&app, get("/health")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Correct token → 200.
    let authed = get("/health")
        .with_header("authorization", "Bearer s3cr3t")
        .unwrap();
    let (status, _) = send(&app, authed).await;
    assert_eq!(status, StatusCode::OK);

    let _ = std::fs::remove_dir_all(&dir);
}

fn assert_turn_frames(body: &str) {
    assert!(body.contains("event: turn_start"));
    // Token-level streaming: the reply arrives as a live `token` frame.
    assert!(
        body.contains("event: token"),
        "expected token frames:\n{body}"
    );
    assert!(
        body.contains("streamed"),
        "expected the streamed reply text:\n{body}"
    );
    assert!(body.contains("event: turn_complete"));
    assert!(body.contains("event: done"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stream_emits_named_sse_frames() {
    let dir = tmp("stream");
    let model = FakeLanguageModel::new(vec![vec![Scripted::Text("streamed".into())]]);
    let app = router(Gateway::new(config_at(&dir), model));

    let (status, body) = send(&app, get("/stream?message=hi")).await;
    assert_eq!(status, StatusCode::OK);
    assert_turn_frames(&body);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn multi_mode_requires_session_id_and_isolates() {
    let dir = tmp("multi");
    let model = FakeLanguageModel::new(vec![
        vec![Scripted::Text("a-reply".into())],
        vec![Scripted::Text("b-reply".into())],
    ]);
    let app = router(Gateway::new(config_at(&dir), model).with_mode(Mode::Multi));

    // No session id → 400.
    let (status, _) = send(&app, post_json("/chat", r#"{"message":"hi"}"#)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Two distinct session ids each get a working, isolated session.
    for (sid, expect) in [("alice", "a-reply"), ("bob", "b-reply")] {
        let req = post_json("/chat", r#"{"message":"hi"}"#)
            .with_header("x-session-id", sid)
            .unwrap();
        let (status, body) = send(&app, req).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains(expect));
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_refuses_to_start_without_gateway_secret() {
    // `RUSTYKEYS_GATEWAY_SECRET` is not set anywhere in this test suite's
    // process environment — startup must refuse to bind a network-exposed,
    // unauthenticated gateway rather than silently disabling auth (round-4
    // finding 1b).
    let dir = tmp("no-secret");
    let model = FakeLanguageModel::new(vec![]);
    let err = rk_app::gateway::serve(config_at(&dir), model, "127.0.0.1:0")
        .await
        .expect_err("gateway must refuse to start without a configured secret");
    assert!(
        err.to_string().contains("RUSTYKEYS_GATEWAY_SECRET"),
        "unexpected error: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gateway_session_gates_new_file_writes_behind_an_approval_gate() {
    let dir = tmp("approval-gate");
    let model = FakeLanguageModel::new(vec![
        vec![Scripted::ToolCall {
            name: "write_file".into(),
            args: serde_json::json!({"path": "unapproved.txt", "content": "hi"}),
        }],
        vec![Scripted::Text("done".into())],
    ]);
    let app = router(Gateway::new(config_at(&dir), model));

    let (status, _) = send(&app, post_json("/chat", r#"{"message":"write a file"}"#)).await;
    assert_eq!(status, StatusCode::OK);

    // The gateway has no interactive adapter to answer an approval prompt, so
    // `Gateway::build_session` must wire the same `ApprovalGate` ACP/desktop
    // use — it fails closed, blocking the first new-file write rather than
    // executing it (round-4 finding 1d). Pre-fix, `build_session` called the
    // ungated `Session::new`, and this file would exist.
    assert!(
        !dir.join("unapproved.txt").exists(),
        "write_file must be blocked by the gateway's ApprovalGate, not executed"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// The production shape: the server on its own `rusty_tokio` runtime, the
/// session's tokio work on the test's tokio runtime through the bridge, and
/// the SSE body polled by the server after `/stream`'s handler has returned
/// (its turn runs on a `tokio::spawn`ed task).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stream_over_a_socket_on_rusty_tokio() {
    let dir = tmp("socket");
    let model = FakeLanguageModel::new(vec![vec![Scripted::Text("streamed".into())]]);
    let app = router(Gateway::new(config_at(&dir), model).with_secret(Some("s3cr3t".into())));

    let response = tokio::task::spawn_blocking(move || {
        let runtime = rusty_tokio::Builder::new_multi_thread().build().unwrap();
        runtime.block_on(async move {
            let listener =
                rusty_tokio::io::TcpListener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
            let server =
                rusty_axum::serve(listener, None, rusty_axum::ServeOptions::default(), app)
                    .unwrap();
            let addr = server.local_addr();
            // A blocking client off the runtime's workers.
            let response = rusty_tokio::spawn_blocking(move || {
                let mut socket = std::net::TcpStream::connect(addr).unwrap();
                socket
                    .write_all(
                        b"GET /stream?message=hi HTTP/1.1\r\nhost: x\r\n\
                          authorization: Bearer s3cr3t\r\nconnection: close\r\n\r\n",
                    )
                    .unwrap();
                let mut response = String::new();
                socket.read_to_string(&mut response).unwrap();
                response
            })
            .await
            .unwrap();
            server.shutdown().await;
            response
        })
    })
    .await
    .unwrap();

    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains("text/event-stream"), "{response}");
    assert_turn_frames(&response);

    let _ = std::fs::remove_dir_all(&dir);
}
