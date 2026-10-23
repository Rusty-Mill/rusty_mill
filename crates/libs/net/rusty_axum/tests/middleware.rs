//! Layers in process: order, short-circuiting, 404s, and CORS as `rk-app`
//! configures it (one exact origin, GET and POST, three headers).

#![allow(clippy::unwrap_used)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use rusty_axum::cors::{Allow, Cors};
use rusty_axum::middleware::{from_fn, Next};
use rusty_axum::routing::{get, post};
use rusty_axum::{IntoResponse, Method, Request, Response, Router, StatusCode};

async fn tag(req: Request, next: Next, name: &'static str) -> Response {
    let response = next.run(req).await;
    let seen = response.headers.get("x-order").unwrap_or("").to_string();
    response.with_header("x-order", &format!("{seen}{name}"))
}

#[rusty_tokio::test]
async fn last_layer_added_runs_first_and_wraps_404() {
    let app = Router::<()>::new()
        .route("/", get(|| async { "root" }))
        .layer(from_fn(|req, next| tag(req, next, "inner")))
        .layer(from_fn(|req, next| tag(req, next, "outer")));
    let r = app.call(Request::new(Method::Get, "/")).await;
    // Responses unwind inside out: inner tags first.
    assert_eq!(r.headers.get("x-order"), Some("innerouter"));
    let missing = app.call(Request::new(Method::Get, "/nope")).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(missing.headers.get("x-order"), Some("innerouter"));
}

#[rusty_tokio::test]
async fn a_layer_can_answer_without_the_handler() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let app = Router::<()>::new()
        .route(
            "/secret",
            get(move || {
                let counted = counted.clone();
                async move {
                    counted.fetch_add(1, Ordering::SeqCst);
                    "secret"
                }
            }),
        )
        .layer(from_fn(|req: Request, next: Next| async move {
            if req.headers().get("authorization") != Some("Bearer t") {
                return StatusCode::UNAUTHORIZED.into_response();
            }
            next.run(req).await
        }));
    let denied = app.call(Request::new(Method::Get, "/secret")).await;
    assert_eq!(denied.status, StatusCode::UNAUTHORIZED);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let req = Request::new(Method::Get, "/secret")
        .with_header("authorization", "Bearer t")
        .unwrap();
    assert_eq!(app.call(req).await.status, StatusCode::OK);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

fn gateway() -> Router {
    let cors = Cors::new()
        .allow_origin(Allow::List(vec!["https://app.example".into()]))
        .allow_methods(Allow::List(vec![Method::Get, Method::Post]))
        .allow_headers(Allow::List(vec![
            "authorization".into(),
            "content-type".into(),
            "x-session-id".into(),
        ]));
    Router::new()
        .route("/chat", post(|| async { "reply" }))
        .layer(cors)
}

#[rusty_tokio::test]
async fn cors_preflight_is_answered_before_routing() {
    let req = Request::new(Method::Options, "/chat")
        .with_header("origin", "https://app.example")
        .unwrap()
        .with_header("access-control-request-method", "POST")
        .unwrap();
    // `/chat` has no OPTIONS handler: a 200 shows the handler never ran.
    let r = gateway().call(req).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(
        r.headers.get("access-control-allow-origin"),
        Some("https://app.example")
    );
    assert_eq!(
        r.headers.get("access-control-allow-methods"),
        Some("GET,POST")
    );
}

#[rusty_tokio::test]
async fn cors_marks_only_allowed_origins() {
    let from = |origin: &str| {
        Request::new(Method::Post, "/chat")
            .with_header("origin", origin)
            .unwrap()
    };
    let ok = gateway().call(from("https://app.example")).await;
    assert_eq!(
        ok.headers.get("access-control-allow-origin"),
        Some("https://app.example")
    );
    assert!(ok.headers.get("vary").is_some());
    let other = gateway().call(from("https://evil.example")).await;
    assert_eq!(
        other.status,
        StatusCode::OK,
        "CORS is enforced by the browser"
    );
    assert_eq!(other.headers.get("access-control-allow-origin"), None);
}

#[test]
#[should_panic(expected = "before adding its layers")]
fn merging_a_layered_router_is_refused() {
    let layered = Router::<()>::new()
        .route("/b", get(|| async {}))
        .layer(Cors::permissive());
    let _ = Router::<()>::new().merge(layered);
}
