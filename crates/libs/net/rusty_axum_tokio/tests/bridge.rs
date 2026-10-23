//! Handlers and SSE bodies that use tokio, served from `rusty_tokio`.

#![allow(clippy::unwrap_used)]

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use futures_core::Stream;
use rusty_axum::response::sse::{Event, Sse};
use rusty_axum::routing::get;
use rusty_axum::{IntoResponse, Method, Request, Router};
use rusty_axum_tokio::TokioBridge;

/// Uses a tokio timer, `spawn` and a oneshot channel, as a `reqwest` call
/// would use tokio's reactor.
async fn handler() -> String {
    tokio::time::sleep(Duration::from_millis(5)).await;
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move { tx.send("from tokio").unwrap() });
    rx.await.unwrap().to_string()
}

/// An SSE body whose every item waits on a tokio timer: polled after the
/// handler has returned.
async fn events() -> impl IntoResponse {
    Sse::new(Ticks {
        left: 3,
        sleep: None,
    })
}

struct Ticks {
    left: u32,
    sleep: Option<Pin<Box<tokio::time::Sleep>>>,
}

impl Stream for Ticks {
    type Item = Result<Event, Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.left == 0 {
            return Poll::Ready(None);
        }
        let sleep = self
            .sleep
            .get_or_insert_with(|| Box::pin(tokio::time::sleep(Duration::from_millis(2))));
        if sleep.as_mut().poll(cx).is_pending() {
            return Poll::Pending;
        }
        self.sleep = None;
        self.left -= 1;
        Poll::Ready(Some(Ok(Event::default().data(self.left.to_string()))))
    }
}

fn tokio_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap()
}

fn on_rusty_tokio<F: Future>(f: F) -> F::Output {
    rusty_tokio::Builder::new_multi_thread()
        .worker_threads(2)
        .build()
        .unwrap()
        .block_on(f)
}

fn app(bridge: Option<TokioBridge>) -> Router {
    let router = Router::new()
        .route("/", get(handler))
        .route("/events", get(events));
    match bridge {
        Some(bridge) => router.layer(bridge),
        None => router,
    }
}

#[test]
fn handlers_and_streams_reach_tokio_through_the_bridge() {
    let tokio = tokio_runtime();
    let app = app(Some(TokioBridge::new(tokio.handle().clone())));
    on_rusty_tokio(async move {
        for _ in 0..50 {
            let r = app.call(Request::new(Method::Get, "/")).await;
            assert_eq!(r.body.collect().await.unwrap(), b"from tokio");
        }
        let r = app.call(Request::new(Method::Get, "/events")).await;
        let body = r.body.collect().await.unwrap();
        assert_eq!(body, b"data: 2\n\ndata: 1\n\ndata: 0\n\n");
    });
}

/// The failure the bridge exists for: without it, the handler's first tokio
/// call has no runtime.
#[test]
#[should_panic(expected = "Tokio 1.x runtime")]
fn without_the_bridge_tokio_panics() {
    let _tokio = tokio_runtime();
    let app = app(None);
    on_rusty_tokio(async move {
        let _ = app.call(Request::new(Method::Get, "/")).await;
    });
}
