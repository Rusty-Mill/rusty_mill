//! Load shedding at the server, before a request's body is read.
//!
//! The server reads a body whole before routing, so a limit applied as
//! middleware would count a request only after its upload had been received
//! and held in memory. An [`Admission`] is checked as soon as the request head
//! arrives instead: over the limit the body is never read, and the deadline
//! covers reading it as well as producing the response.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use rusty_tokio::sync::{OwnedSemaphorePermit, Semaphore};
use rusty_tokio::time::{timeout_at, Instant};

use crate::response::Response;
use rusty_http::StatusCode;

/// `Retry-After` on a shed request, in seconds: a brief pause while the
/// backlog clears.
const RETRY_AFTER_SECS: &str = "1";

/// A bound on requests in flight and on how long each may take, checked
/// before a request's body is read. Off until set with the `with_*` methods.
///
/// Cheap to clone, and clones share one count, so servers on several ports
/// can share one limit. Over the limit a request gets `503` with
/// `Retry-After` at once; it is not queued. A request that has no response by
/// its deadline gets `504`.
///
/// A request holds its place until its response head is ready, so a
/// streamed body (SSE) does not count once it has started.
#[derive(Clone, Default)]
pub struct Admission {
    permits: Option<Arc<Semaphore>>,
    timeout: Option<Duration>,
}

impl Admission {
    /// No limits.
    pub fn new() -> Self {
        Self::default()
    }

    /// At most `max` requests in flight; zero is treated as one.
    pub fn with_max_in_flight(mut self, max: usize) -> Self {
        let max = max.clamp(1, Semaphore::MAX_PERMITS);
        self.permits = Some(Arc::new(Semaphore::new(max)));
        self
    }

    /// Answer `504` to a request whose body has not been read and response
    /// produced within `timeout` of its head arriving.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// A place for the request whose head just arrived, or the `503` that
    /// refuses it.
    pub(crate) fn admit(&self) -> Result<Ticket, Response> {
        let permit = match &self.permits {
            None => None,
            Some(permits) => match permits.clone().try_acquire_owned() {
                Ok(permit) => Some(permit),
                Err(_) => return Err(overloaded()),
            },
        };
        Ok(Ticket {
            _permit: permit,
            deadline: self.timeout.map(|timeout| Instant::now() + timeout),
        })
    }
}

impl std::fmt::Debug for Admission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Admission")
            .field(
                "in_flight_free",
                &self.permits.as_ref().map(|p| p.available_permits()),
            )
            .field("timeout", &self.timeout)
            .finish()
    }
}

/// An admitted request's place, held until it is dropped.
pub(crate) struct Ticket {
    _permit: Option<OwnedSemaphorePermit>,
    deadline: Option<Instant>,
}

impl Ticket {
    /// A place under no limits.
    pub(crate) fn unlimited() -> Self {
        Ticket {
            _permit: None,
            deadline: None,
        }
    }

    /// `work`, or `None` once the deadline passes.
    pub(crate) async fn within<T>(&self, work: impl Future<Output = T>) -> Option<T> {
        match self.deadline {
            None => Some(work.await),
            Some(deadline) => timeout_at(deadline, work).await.ok(),
        }
    }
}

fn overloaded() -> Response {
    Response::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "Service Unavailable: too many requests in flight",
    )
    .with_header("content-type", "text/plain; charset=utf-8")
    .with_header("retry-after", RETRY_AFTER_SECS)
}

/// The answer to a request that passed its deadline.
pub(crate) fn timed_out() -> Response {
    Response::new(
        StatusCode::GATEWAY_TIMEOUT,
        "Gateway Timeout: the server took too long to respond",
    )
    .with_header("content-type", "text/plain; charset=utf-8")
}
