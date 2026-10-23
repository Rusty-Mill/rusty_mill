//! Handlers and SSE bodies that use tokio, served from `rusty_tokio`, and
//! tokio resources dropped inside the runtime however a request ends.

#![allow(clippy::unwrap_used)]

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use futures_core::Stream;
use rusty_axum::response::sse::{Event, Sse};
use rusty_axum::routing::get;
use rusty_axum::{IntoResponse, Method, Request, Router, State};
use rusty_axum_tokio::TokioBridge;
use tokio::runtime::{Handle, RuntimeFlavor};

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

/// Records, when dropped, which kind of tokio runtime was current, if any.
/// The bridge in these tests is a current-thread runtime, so its handle is
/// told apart from an outer multi-thread one.
#[derive(Clone)]
struct Probe(Arc<Mutex<Vec<Seen>>>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Seen {
    NoRuntime,
    /// A current-thread runtime: the bridge's.
    Bridged,
    Other,
}

impl Drop for Probe {
    fn drop(&mut self) {
        let seen = match Handle::try_current().map(|h| h.runtime_flavor()) {
            Err(_) => Seen::NoRuntime,
            Ok(RuntimeFlavor::CurrentThread) => Seen::Bridged,
            Ok(_) => Seen::Other,
        };
        // No unwrap: a panic in a drop during a failed assertion aborts.
        let mut log = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        log.push(seen);
    }
}

/// A body holding a probe: never yields (`open`), or ends at once.
struct Held {
    _probe: Probe,
    open: bool,
}

impl Stream for Held {
    type Item = Result<Event, Infallible>;

    fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.open {
            Poll::Pending
        } else {
            Poll::Ready(None)
        }
    }
}

fn probed_app(bridge: TokioBridge, probe: Probe) -> Router {
    Router::new()
        .route(
            "/pending",
            get(|State(probe): State<Probe>| async move {
                let _probe = probe;
                std::future::pending::<()>().await;
            }),
        )
        .route(
            "/done",
            get(|State(probe): State<Probe>| async move {
                let _probe = probe;
                "done"
            }),
        )
        .route(
            "/open",
            get(|State(probe): State<Probe>| async move {
                Sse::new(Held {
                    _probe: probe,
                    open: true,
                })
            }),
        )
        .route(
            "/closed",
            get(|State(probe): State<Probe>| async move {
                Sse::new(Held {
                    _probe: probe,
                    open: false,
                })
            }),
        )
        .layer(bridge)
        .with_state(probe)
}

/// Polls once, off any runtime: what a server does when it drives a future.
fn poll_once<F: Future + Unpin>(future: &mut F) -> Poll<F::Output> {
    Pin::new(future).poll(&mut Context::from_waker(Waker::noop()))
}

fn ready<F: Future + Unpin>(mut future: F) -> F::Output {
    match poll_once(&mut future) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("expected a ready future"),
    }
}

fn get_req(path: &str) -> Request {
    Request::new(Method::Get, path)
}

#[test]
fn tokio_resources_are_dropped_inside_the_runtime() {
    let bridged = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    let app = probed_app(
        TokioBridge::new(bridged.handle().clone()),
        Probe(log.clone()),
    );
    // The probes `case` dropped, each of which must have seen the bridge's
    // runtime; afterwards the caller's (empty) context is back.
    let case = |name: &str, act: &dyn Fn()| {
        let before = log.lock().unwrap().len();
        act();
        let dropped = log.lock().unwrap()[before..].to_vec();
        assert!(!dropped.is_empty(), "{name}: no probe was dropped");
        assert!(
            dropped.iter().all(|seen| *seen == Seen::Bridged),
            "{name}: dropped outside the bridged runtime: {dropped:?}"
        );
        assert!(
            Handle::try_current().is_err(),
            "{name}: runtime context leaked"
        );
    };

    case("cancelled before its first poll", &|| {
        drop(app.call(get_req("/pending")));
    });
    case("cancelled after Pending", &|| {
        let mut call = app.call(get_req("/pending"));
        assert!(poll_once(&mut call).is_pending());
        drop(call);
    });
    case("completed", &|| {
        assert_eq!(ready(app.call(get_req("/done"))).status.as_u16(), 200);
    });
    case("streamed body cancelled", &|| {
        let response = ready(app.call(get_req("/open")));
        let mut body = Box::pin(response.body.collect());
        assert!(poll_once(&mut body).is_pending());
        drop(body);
    });
    case("streamed body suppressed for HEAD", &|| {
        let response = ready(app.call(Request::new(Method::Head, "/open")));
        assert!(response.body.as_bytes().is_some_and(<[u8]>::is_empty));
    });
    case("streamed body completed", &|| {
        let response = ready(app.call(get_req("/closed")));
        assert!(ready(Box::pin(response.body.collect())).unwrap().is_empty());
    });
}

#[test]
fn the_callers_runtime_context_is_restored() {
    let bridged = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let outer = tokio_runtime();
    let log = Arc::new(Mutex::new(Vec::new()));
    let app = probed_app(
        TokioBridge::new(bridged.handle().clone()),
        Probe(log.clone()),
    );
    let _outer = outer.enter();

    let mut call = app.call(get_req("/pending"));
    assert!(poll_once(&mut call).is_pending());
    drop(call);

    assert_eq!(log.lock().unwrap().last().copied(), Some(Seen::Bridged));
    assert_eq!(
        Handle::current().runtime_flavor(),
        RuntimeFlavor::MultiThread
    );
}
