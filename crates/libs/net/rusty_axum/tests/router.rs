//! The router end to end, in process: the shapes `rk-app`'s gateway uses
//! (state, headers, query, JSON in and out, SSE) plus path parameters.

#![allow(clippy::unwrap_used)]

use std::collections::HashMap;
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

use futures_core::Stream;
use rusty_axum::response::sse::{Event, Sse};
use rusty_axum::routing::{get, post};
use rusty_axum::{
    HeaderMap, IntoResponse, Json, Method, Path, Query, Request, Response, Router, State,
    StatusCode,
};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Default)]
struct Gateway {
    turns: AtomicUsize,
}

type Gw = State<Arc<Gateway>>;

async fn health(State(gw): Gw, headers: HeaderMap) -> impl IntoResponse {
    if headers.get("authorization") != Some("Bearer t") {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(json!({ "status": "ok", "turns": gw.turns.load(Ordering::SeqCst) })).into_response()
}

async fn chat(State(gw): Gw, Json(body): Json<Value>) -> impl IntoResponse {
    match body.get("message").and_then(Value::as_str) {
        Some(m) => {
            gw.turns.fetch_add(1, Ordering::SeqCst);
            Ok(Json(json!({ "reply": m.to_uppercase() })))
        }
        None => Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "no message" })),
        )),
    }
}

async fn stream(Query(params): Query<HashMap<String, String>>) -> impl IntoResponse {
    let session = params.get("session").cloned().unwrap_or_default();
    let events = vec![
        Ok::<_, Infallible>(Event::default().event("start").id("0").data(session)),
        Ok(Event::default().event("token").id("1").data("hi")),
    ];
    Sse::new(Iter(events.into_iter()))
}

#[derive(Deserialize)]
struct TaskPath {
    tenant: String,
    id: u32,
}

async fn task(Path(p): Path<TaskPath>) -> String {
    format!("{}:{}", p.tenant, p.id)
}

async fn new_task() -> &'static str {
    "new"
}

fn app() -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/chat", post(chat))
        .route("/stream", get(stream))
        .route("/{tenant}/tasks/{id}", get(task))
        .route("/{tenant}/tasks/new", get(new_task))
        .with_state(Arc::new(Gateway::default()))
}

/// A stream over an iterator, for the test only.
struct Iter<I>(I);

impl<I: Iterator + Unpin> Stream for Iter<I> {
    type Item = I::Item;

    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<I::Item>> {
        Poll::Ready(self.0.next())
    }
}

async fn text(response: Response) -> String {
    String::from_utf8(response.body.collect().await.unwrap()).unwrap()
}

fn json_post(target: &str, body: &str) -> Request {
    Request::new(Method::Post, target)
        .with_header("content-type", "application/json")
        .unwrap()
        .with_body(body.as_bytes().to_vec())
}

#[rusty_tokio::test]
async fn state_and_headers() {
    let app = app();
    let denied = app.call(Request::new(Method::Get, "/health")).await;
    assert_eq!(denied.status, StatusCode::UNAUTHORIZED);
    let req = Request::new(Method::Get, "/health")
        .with_header("Authorization", "Bearer t")
        .unwrap();
    let ok = app.call(req).await;
    assert_eq!(ok.status, StatusCode::OK);
    assert_eq!(text(ok).await, r#"{"status":"ok","turns":0}"#);
}

#[rusty_tokio::test]
async fn json_in_and_out_shares_state() {
    let app = app();
    let reply = app.call(json_post("/chat", r#"{"message":"hey"}"#)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(text(reply).await, r#"{"reply":"HEY"}"#);
    let bad = app.call(json_post("/chat", "{}")).await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    let req = Request::new(Method::Get, "/health")
        .with_header("authorization", "Bearer t")
        .unwrap();
    assert_eq!(
        text(app.call(req).await).await,
        r#"{"status":"ok","turns":1}"#
    );
}

#[rusty_tokio::test]
async fn sse_streams_named_events() {
    let response = app()
        .call(Request::new(Method::Get, "/stream?session=s%201"))
        .await;
    assert_eq!(
        response.headers.get("content-type"),
        Some("text/event-stream")
    );
    assert!(response.body.as_bytes().is_none(), "SSE must stream");
    assert_eq!(
        text(response).await,
        "event: start\nid: 0\ndata: s 1\n\nevent: token\nid: 1\ndata: hi\n\n"
    );
}

#[rusty_tokio::test]
async fn path_parameters_and_static_precedence() {
    let app = app();
    let r = app
        .call(Request::new(Method::Get, "/acme%20co/tasks/7"))
        .await;
    assert_eq!(text(r).await, "acme co:7");
    let r = app.call(Request::new(Method::Get, "/acme/tasks/new")).await;
    assert_eq!(text(r).await, "new");
    let r = app
        .call(Request::new(Method::Get, "/acme/tasks/seven"))
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
}

#[rusty_tokio::test]
async fn not_found_method_not_allowed_and_head() {
    let app = app();
    assert_eq!(
        app.call(Request::new(Method::Get, "/nope")).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.call(Request::new(Method::Get, "/health/")).await.status,
        StatusCode::NOT_FOUND
    );
    let r = app.call(Request::new(Method::Delete, "/chat")).await;
    assert_eq!(r.status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(r.headers.get("allow"), Some("POST"));
    let r = app.call(Request::new(Method::Head, "/stream")).await;
    assert_eq!(r.status, StatusCode::OK);
}

#[rusty_tokio::test]
async fn rejection_answers_before_the_handler() {
    let r = app()
        .call(Request::new(Method::Post, "/chat").with_body(b"{}".to_vec()))
        .await;
    assert_eq!(r.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

#[rusty_tokio::test]
async fn merge_combines_routers() {
    let a = Router::<()>::new().route("/a", get(|| async { "a" }));
    let b = Router::<()>::new()
        .route("/a", post(|| async { "posted" }))
        .route("/b", get(|| async { "b" }));
    let app = a.merge(b);
    assert_eq!(
        text(app.call(Request::new(Method::Post, "/a")).await).await,
        "posted"
    );
    assert_eq!(
        text(app.call(Request::new(Method::Get, "/b")).await).await,
        "b"
    );
}
