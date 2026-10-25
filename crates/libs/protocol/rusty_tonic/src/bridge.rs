//! Runs the transport for callers on another runtime.
//!
//! A [`client::Channel`](crate::client::Channel) is async on `rusty_tokio`. A caller on `tokio`
//! (or anything else that polls a plain `Future`) does not enter that runtime: it hands the work
//! to a process-wide `rusty_tokio` runtime with [`spawn`] and awaits the result. The two meet only
//! through the returned future, which a `std` waker wakes, so neither side needs the other's
//! runtime context.
//!
//! Dropping the future aborts the work, which drops the call and so resets its HTTP/2 stream:
//! cancelling a call from the caller cancels it on the wire.
//!
//! This is a bridge for callers that have not moved to `rusty_tokio` yet and goes away with them.

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;
use std::task::{Context, Poll};

use crate::Status;

/// Threads of the transport's runtime: it moves bytes and frames, nothing heavier.
const WORKERS: usize = 2;

/// The process-wide runtime the transport runs on, started on first use.
fn runtime() -> Result<&'static rusty_tokio::Runtime, Status> {
    static RUNTIME: OnceLock<Result<rusty_tokio::Runtime, String>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            rusty_tokio::Runtime::builder()
                .worker_threads(WORKERS)
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| Status::unavailable(format!("could not start the gRPC transport: {e}")))
}

/// Work running on the transport's runtime, awaited from anywhere. Dropping it aborts the work.
///
/// Resolves to the work's output, or to `INTERNAL` if the work panicked or was cancelled.
#[must_use = "the work is cancelled when this is dropped"]
pub struct Detached<T>(rusty_tokio::task::JoinHandle<T>);

impl<T> Drop for Detached<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl<T> Future for Detached<T> {
    type Output = Result<T, Status>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.0).poll(cx).map(|joined| {
            joined.map_err(|e| Status::internal(format!("the gRPC transport task failed: {e}")))
        })
    }
}

/// Starts `work` on the transport's runtime, which is started on first use.
///
/// Fails with `UNAVAILABLE` only if that runtime cannot be started.
pub fn spawn<F, T>(work: F) -> Result<Detached<T>, Status>
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    Ok(Detached(runtime()?.spawn(work)))
}
