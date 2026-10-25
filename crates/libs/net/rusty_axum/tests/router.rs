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
use rusty_axum::middleware::{from_fn, Next};
use rusty_axum::response::sse::{Event, Sse};
use rusty_axum::routing::{any, get, post};
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

/// Panics if polled: proves a HEAD answer never reads its stream.
struct Untouchable;

impl Stream for Untouchable {
    type Item = Result<Event, Infallible>;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        panic!("a HEAD response body was polled");
    }
}

#[rusty_tokio::test]
async fn head_keeps_status_and_headers_but_no_body() {
    let app = Router::<()>::new()
        .route("/text", get(|| async { "twelve bytes" }))
        .route("/events", get(|| async { Sse::new(Untouchable) }))
        .route("/json", get(|Json(v): Json<Value>| async move { Json(v) }));

    let r = app.call(Request::new(Method::Head, "/text")).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(
        r.headers.get("content-type"),
        Some("text/plain; charset=utf-8")
    );
    assert_eq!(r.headers.get("content-length"), Some("12"));
    assert_eq!(r.body.as_bytes(), Some(&b""[..]));

    let r = app.call(Request::new(Method::Head, "/events")).await;
    assert_eq!(r.headers.get("content-type"), Some("text/event-stream"));
    assert_eq!(r.headers.get("content-length"), None);
    assert_eq!(r.body.collect().await.unwrap(), b"");

    let r = app.call(Request::new(Method::Head, "/json")).await;
    assert_eq!(r.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert!(r.headers.get("content-length").is_some());
    assert_eq!(r.body.as_bytes(), Some(&b""[..]));

    let r = app.call(Request::new(Method::Get, "/text")).await;
    assert_eq!(r.body.as_bytes(), Some(&b"twelve bytes"[..]));
}

#[rusty_tokio::test]
async fn any_takes_the_methods_a_path_has_no_handler_for() {
    let app = Router::new().route("/mcp", get(|| async { "get" })).route(
        "/mcp",
        any(|method: Method| async move { method.as_str().to_string() }),
    );
    for (method, expected) in [
        (Method::Get, "get"),
        (Method::Post, "POST"),
        (Method::Delete, "DELETE"),
    ] {
        let response = app.call(Request::new(method, "/mcp")).await;
        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(text(response).await, expected);
    }
    let other = app.call(Request::new(Method::Post, "/elsewhere")).await;
    assert_eq!(other.status, StatusCode::NOT_FOUND);
}

#[rusty_tokio::test]
async fn fallback_answers_unmatched_paths_with_state_and_layers() {
    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .fallback(
            |State(name): State<&'static str>, req: Request| async move {
                format!("{name} {}", req.target())
            },
        )
        .with_state("fb")
        .layer(from_fn(|req: Request, next: Next| async move {
            next.run(req).await.with_header("x-layer", "1")
        }));
    let hit = app.call(Request::new(Method::Get, "/health")).await;
    assert_eq!(text(hit).await, "ok");
    let missed = app.call(Request::new(Method::Post, "/a/b?q=1")).await;
    assert_eq!(missed.status, StatusCode::OK);
    assert_eq!(missed.headers.get("x-layer"), Some("1"));
    assert_eq!(text(missed).await, "fb /a/b?q=1");
}

#[rusty_tokio::test]
async fn merge_takes_the_other_routers_fallback() {
    let app = Router::<()>::new()
        .route("/a", get(|| async { "a" }))
        .merge(Router::new().fallback(|| async { "fallback" }));
    let missed = app.call(Request::new(Method::Get, "/b")).await;
    assert_eq!(text(missed).await, "fallback");
}

#[test]
#[should_panic(expected = "both routers have a fallback")]
fn merge_refuses_two_fallbacks() {
    let _ = Router::<()>::new()
        .fallback(|| async {})
        .merge(Router::new().fallback(|| async {}));
}
