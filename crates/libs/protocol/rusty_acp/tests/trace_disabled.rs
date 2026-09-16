//! The `trace` feature off means no header on the wire.
//!
//! The other half of `tests/trace_context.rs`, which is gated the opposite way.
//! Between them one of the two always runs, and neither can pass for the other
//! one's reason.
//!
//! Worth having because a feature that gates a *behaviour* rather than a
//! dependency can be got wrong silently: the code compiles either way and the
//! header keeps going out. Nothing else here would notice — the enabled tests
//! pass because the feature is enabled, and the disabled build has no
//! assertions of its own.
//!
//! CI reaches this through the `Test (1.86)` job, which builds default features
//! only. The `--all-features` jobs run the other file.

#![cfg(all(feature = "client", not(feature = "trace")))]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::routing::get;
use rusty_acp::client::AcpClient;

#[tokio::test]
async fn no_traceparent_is_sent_when_the_feature_is_off() {
    let seen = Arc::new(AtomicBool::new(false));
    let recorder = Arc::clone(&seen);

    let app = axum::Router::new().route(
        "/ping",
        get(move |headers: axum::http::HeaderMap| {
            let recorder = Arc::clone(&recorder);
            async move {
                recorder.store(headers.contains_key("traceparent"), Ordering::SeqCst);
                axum::Json(serde_json::json!({}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    AcpClient::new(format!("http://{addr}")).unwrap().ping().await.unwrap();

    assert!(
        !seen.load(Ordering::SeqCst),
        "a traceparent went out with the `trace` feature disabled, so the gate gates nothing"
    );
}
