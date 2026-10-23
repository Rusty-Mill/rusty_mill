//! A migration bridge for `rusty_axum` consumers whose handlers still call
//! `tokio` libraries (`reqwest`, `sqlx`, `tonic`, `tokio::time`).
//!
//! `rusty_axum` serves on `rusty_tokio`. A tokio resource used there panics
//! ("no reactor") unless a tokio runtime is entered while it is polled, and
//! some need one when dropped too (a pooled connection's `Drop` spawns its
//! cleanup). [`TokioBridge`] enters one on **every poll and on drop**, of
//! the handler's future and of a streamed response body (SSE), which is
//! polled after the handler has returned. Drop covers cancellation: a
//! client that disconnects, a HEAD request's suppressed body, shutdown. The
//! tokio runtime must be multi-threaded and kept running by the consumer:
//! its threads drive tokio's I/O and timers.
//!
//! Add it as the router's last layer, so that it is outermost and every
//! other layer runs inside it:
//!
//! ```ignore
//! let app = Router::new()
//!     .route("/chat", post(chat))
//!     .layer(cors)
//!     .layer(TokioBridge::new(tokio_handle));
//! ```
//!
//! The cost is two runtimes in one process while a consumer migrates. Each
//! consumer drops this layer when it leaves tokio; the crate goes with the
//! last one.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_core::Stream;
use rusty_axum::middleware::{Middleware, Next};
use rusty_axum::{BodyStream, BoxError, BoxFuture, Request, Response};
use tokio::runtime::Handle;

/// Enters a tokio runtime around every poll of the requests it wraps. See
/// the crate docs.
#[derive(Debug, Clone)]
pub struct TokioBridge {
    handle: Handle,
}

impl TokioBridge {
    /// Bridges to the runtime `handle` belongs to.
    pub fn new(handle: Handle) -> Self {
        TokioBridge { handle }
    }
}

impl Middleware for TokioBridge {
    fn call(&self, req: Request, next: Next) -> BoxFuture<'static, Response> {
        let handle = self.handle.clone();
        // Entered while the chain builds its futures too: an extractor or a
        // handler may create a tokio resource before its first poll.
        let inner = {
            let _entered = handle.enter();
            next.run(req)
        };
        // Wrapped before it is returned, so a request cancelled before its
        // first poll still drops `inner` inside the runtime.
        let inner = Entered::new(handle.clone(), inner);
        Box::pin(async move {
            let mut response = inner.await;
            response.body = response
                .body
                .map_stream(|stream| Box::pin(Entered::new(handle, stream)) as BodyStream);
            response
        })
    }
}

/// A future or stream polled and dropped with `handle` entered.
struct Entered<T> {
    handle: Handle,
    /// `None` only inside `drop`, which takes it to drop it while entered:
    /// fields drop after `Drop::drop` returns, by when the guard is gone.
    inner: Option<T>,
}

impl<T> Entered<T> {
    fn new(handle: Handle, inner: T) -> Self {
        Entered {
            handle,
            inner: Some(inner),
        }
    }
}

impl<T> Drop for Entered<T> {
    fn drop(&mut self) {
        let _entered = self.handle.enter();
        drop(self.inner.take());
    }
}

impl Future for Entered<BoxFuture<'static, Response>> {
    type Output = Response;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Response> {
        let this = &mut *self;
        let _entered = this.handle.enter();
        match this.inner.as_mut() {
            Some(inner) => inner.as_mut().poll(cx),
            None => Poll::Pending,
        }
    }
}

impl Stream for Entered<BodyStream> {
    type Item = Result<Vec<u8>, BoxError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = &mut *self;
        let _entered = this.handle.enter();
        match this.inner.as_mut() {
            Some(inner) => inner.as_mut().poll_next(cx),
            None => Poll::Ready(None),
        }
    }
}
