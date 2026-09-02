//! Phase 14 gateway contract test (status codes, auth, SSE frames, multi-session
//! isolation). Uses `tower::ServiceExt::oneshot` — no port bind, fully offline.
#![cfg(feature = "gateway")]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use rk_app::gateway::{Gateway, Mode};
use rk_config::Config;
use rk_kernel::fake::{FakeLanguageModel, Scripted};
use tower::ServiceExt; // oneshot

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

async fn body_string(resp: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_and_health_round_trip() {
    let dir = tmp("chat");
    let model = FakeLanguageModel::new(vec![vec![Scripted::Text("hello from gateway".into())]]);
    let gw = Arc::new(Gateway::new(config_at(&dir), model));
    let app = gw.router();

    // /health
    let resp = app
        .clone()
        .oneshot(Request::get("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(body_string(resp).await.contains("\"status\":\"ok\""));

    // /ready — readiness probe: model + workspace + SQLite round-trip.
    let resp = app
        .clone()
        .oneshot(Request::get("/ready").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let rb = body_string(resp).await;
    assert!(rb.contains("\"ready\":true"), "expected ready: {rb}");

    // /chat
    let resp = app
        .oneshot(
            Request::post("/chat")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"message":"hi"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let b = body_string(resp).await;
    assert!(b.contains("hello from gateway"));
    assert!(b.contains("\"verified\":true"));

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
    let gw = Arc::new(Gateway::new(config, model));
    let app = gw.router();

    // Drive one turn so the exporter accumulates.
    let resp = app
        .clone()
        .oneshot(
            Request::post("/chat")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"message":"hi"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Pull telemetry: a host-boundary scrape, no push from inside the turn.
    let resp = app
        .oneshot(Request::get("/metrics").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let b = body_string(resp).await;
    assert!(b.contains("\"enabled\":true"), "expected enabled: {b}");
    assert!(b.contains("\"turns\":1"), "expected one turn: {b}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bearer_auth_gates_requests() {
    let dir = tmp("auth");
    let model = FakeLanguageModel::new(vec![]);
    let gw = Arc::new(Gateway::new(config_at(&dir), model).with_secret(Some("s3cr3t".into())));
    let app = gw.router();

    // No token → 401.
    let resp = app
        .clone()
        .oneshot(Request::get("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // Correct token → 200.
    let resp = app
        .oneshot(
            Request::get("/health")
                .header("authorization", "Bearer s3cr3t")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stream_emits_named_sse_frames() {
    let dir = tmp("stream");
    let model = FakeLanguageModel::new(vec![vec![Scripted::Text("streamed".into())]]);
    let gw = Arc::new(Gateway::new(config_at(&dir), model));
    let app = gw.router();

    let resp = app
        .oneshot(
            Request::get("/stream?message=hi")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let b = body_string(resp).await;
    assert!(b.contains("event: turn_start"));
    // Token-level streaming: the reply arrives as a live `token` frame.
    assert!(b.contains("event: token"), "expected token frames:\n{b}");
    assert!(
        b.contains("streamed"),
        "expected the streamed reply text:\n{b}"
    );
    assert!(b.contains("event: turn_complete"));
    assert!(b.contains("event: done"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn multi_mode_requires_session_id_and_isolates() {
    let dir = tmp("multi");
    let model = FakeLanguageModel::new(vec![
        vec![Scripted::Text("a-reply".into())],
        vec![Scripted::Text("b-reply".into())],
    ]);
    let gw = Arc::new(Gateway::new(config_at(&dir), model).with_mode(Mode::Multi));
    let app = gw.router();

    // No session id → 400.
    let resp = app
        .clone()
        .oneshot(
            Request::post("/chat")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"message":"hi"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Two distinct session ids each get a working, isolated session.
    for (sid, expect) in [("alice", "a-reply"), ("bob", "b-reply")] {
        let resp = app
            .clone()
            .oneshot(
                Request::post("/chat")
                    .header("content-type", "application/json")
                    .header("x-session-id", sid)
                    .body(Body::from(r#"{"message":"hi"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(body_string(resp).await.contains(expect));
    }

    let _ = std::fs::remove_dir_all(&dir);
}
