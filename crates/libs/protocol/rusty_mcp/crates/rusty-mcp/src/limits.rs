//! Shedding load before it costs anything.
//!
//! [`crate::auth::RequireAuthLayer`] rejects requests that are *unauthorized*.
//! It does nothing about requests that are merely *too many* — including too
//! many unauthorized ones, each of which still costs a token validation, and
//! with [`crate::auth::JwtValidator`] can reach for the JWKS on an unknown
//! `kid`. This module is the other half: a bound on how much work is in flight
//! at once, and on how long any of it may take.
//!
//! ```
//! use std::time::Duration;
//! use rusty_mcp::limits::LimitsLayer;
//!
//! let limits = LimitsLayer::new()
//!     .with_max_concurrent(256)
//!     .with_timeout(Duration::from_secs(30));
//! # let _ = limits;
//! ```
//!
//! Set it on [`crate::HttpConfig::limits`] and [`crate::serve`] mounts it in
//! the right place.
//!
//! # Shedding, not queueing
//!
//! Over the limit, a request gets `503` with `Retry-After` immediately. It is
//! not queued. A queue in front of an overloaded server converts a capacity
//! problem into a latency problem: every client waits longer, times out, and
//! retries, which is how a brief spike becomes a sustained outage. Refusing
//! quickly lets a client back off while the requests already accepted still
//! finish on time.
//!
//! # Off by default
//!
//! Unlike the `Host` allow-list, there is no value that is right for everyone —
//! it depends entirely on what your tools do. A default of 100 would be a
//! silent regression for a server handling more than that today. So this must
//! be turned on deliberately.
//!
//! # What the timeout actually bounds
//!
//! Time to *produce a response*, not time to finish streaming one. That
//! distinction is what keeps a long-lived `subscriptions/listen` alive: the
//! transport returns the SSE response promptly and streams events afterwards,
//! so the part this wraps is short even though the request lives for hours.
//! There is a test holding a subscription open across a timeout far shorter
//! than its lifetime.
//!
//! A tool running **inline** — no tasks extension — does hold the response
//! open for its whole duration, and will be cut off. That is the point; it is
//! also the reason to reach for [`crate::tasks`] for anything slow.

use std::{
    convert::Infallible,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use axum::response::{IntoResponse, Response};
use http::{Request, StatusCode, header::RETRY_AFTER};
use tokio::sync::Semaphore;

/// `Retry-After` sent with a shed response, in seconds.
///
/// Short on purpose: the point is a brief pause while the backlog clears, not
/// a client that goes away for a minute.
const RETRY_AFTER_SECS: u32 = 1;

/// Bounds concurrent requests and how long each may take.
///
/// Cheap to clone; the permit pool is shared, which is the whole point — a
/// limit that each clone counted separately would bound nothing.
#[derive(Debug, Clone, Default)]
pub struct LimitsLayer {
    permits: Option<Arc<Semaphore>>,
    max_concurrent: Option<usize>,
    timeout: Option<Duration>,
}

impl LimitsLayer {
    /// No limits at all. Add them with the `with_*` methods.
    pub fn new() -> Self {
        Self::default()
    }

    /// At most `max` requests in flight; the rest get `503`.
    ///
    /// Counts **requests**, not work. A tool handed off to
    /// [`crate::tasks::TaskSupport`] releases its permit as soon as the task
    /// handle goes back to the client, which is correct: the point of a task
    /// is that the work outlives the request, and counting it here would let
    /// a handful of long tasks close the server to everyone.
    ///
    /// Zero is treated as one, since a limit of zero would refuse everything.
    pub fn with_max_concurrent(mut self, max: usize) -> Self {
        let max = max.max(1);
        self.max_concurrent = Some(max);
        self.permits = Some(Arc::new(Semaphore::new(max)));
        self
    }

    /// Give up on a request that has not produced a response within `timeout`.
    ///
    /// Bounds time to first response, not stream duration — see the module
    /// docs. The client gets `504`.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// The concurrency limit, if one is set.
    pub fn max_concurrent(&self) -> Option<usize> {
        self.max_concurrent
    }

    /// The request timeout, if one is set.
    pub fn timeout(&self) -> Option<Duration> {
        self.timeout
    }

    /// Permits available right now, for a health endpoint or a test.
    pub fn available(&self) -> Option<usize> {
        self.permits.as_ref().map(|p| p.available_permits())
    }
}

impl<S> tower_layer::Layer<S> for LimitsLayer {
    type Service = Limited<S>;

    fn layer(&self, inner: S) -> Self::Service {
        Limited {
            inner,
            permits: self.permits.clone(),
            timeout: self.timeout,
        }
    }
}

/// Service produced by [`LimitsLayer`].
#[derive(Debug, Clone)]
pub struct Limited<S> {
    inner: S,
    permits: Option<Arc<Semaphore>>,
    timeout: Option<Duration>,
}

impl<S, ReqBody> tower_service::Service<Request<ReqBody>> for Limited<S>
where
    S: tower_service::Service<Request<ReqBody>, Error = Infallible> + Clone + Send + 'static,
    S::Response: IntoResponse,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
{
    type Response = Response;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Response, Infallible>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // Deliberately *not* where the limit is applied. Returning `Pending`
        // here is how `tower`'s own ConcurrencyLimit backpressures, and
        // backpressure means queueing — the caller waits instead of being told
        // no. Shedding has to happen in `call`.
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<ReqBody>) -> Self::Future {
        // Swap in the readied service: `self.inner` is the one that passed
        // `poll_ready`, and the fresh clone may not be ready yet.
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        let permits = self.permits.clone();
        let timeout = self.timeout;

        Box::pin(async move {
            // `try_acquire_owned` rather than `acquire_owned`: the former
            // refuses, the latter waits. Waiting is the failure mode this
            // whole module exists to avoid.
            let _permit = match &permits {
                Some(semaphore) => match Arc::clone(semaphore).try_acquire_owned() {
                    Ok(permit) => Some(permit),
                    Err(_) => {
                        tracing::warn!("shedding a request: the concurrency limit is reached");
                        return Ok(overloaded());
                    }
                },
                None => None,
            };

            let call = inner.call(request);

            let Some(timeout) = timeout else {
                return Ok(call.await?.into_response());
            };

            match tokio::time::timeout(timeout, call).await {
                Ok(response) => Ok(response?.into_response()),
                Err(_) => {
                    tracing::warn!(
                        timeout_ms = timeout.as_millis() as u64,
                        "a request produced no response within the timeout"
                    );
                    Ok(timed_out())
                }
            }
            // The permit drops here, after the response exists — so the count
            // tracks requests being handled, not requests being streamed.
        })
    }
}

fn overloaded() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(RETRY_AFTER, RETRY_AFTER_SECS.to_string())],
        "Service Unavailable: too many requests in flight",
    )
        .into_response()
}

fn timed_out() -> Response {
    (
        StatusCode::GATEWAY_TIMEOUT,
        "Gateway Timeout: the server took too long to respond",
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    use tower_layer::Layer as _;
    use tower_service::Service as _;

    /// A service that waits `delay` before answering.
    #[derive(Clone)]
    struct Slow(Duration);

    impl<B: Send + 'static> tower_service::Service<Request<B>> for Slow {
        type Response = Response;
        type Error = Infallible;
        type Future = Pin<Box<dyn Future<Output = Result<Response, Infallible>> + Send>>;

        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, _request: Request<B>) -> Self::Future {
            let delay = self.0;
            Box::pin(async move {
                tokio::time::sleep(delay).await;
                Ok(StatusCode::OK.into_response())
            })
        }
    }

    fn request() -> Request<axum::body::Body> {
        Request::builder()
            .body(axum::body::Body::empty())
            .expect("request")
    }

    #[tokio::test]
    async fn no_limits_configured_changes_nothing() {
        let mut service = LimitsLayer::new().layer(Slow(Duration::ZERO));
        let response = service.call(request()).await.expect("infallible");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn a_request_over_the_limit_is_shed_rather_than_queued() {
        let layer = LimitsLayer::new().with_max_concurrent(1);
        let mut first = layer.layer(Slow(Duration::from_millis(200)));
        let mut second = layer.layer(Slow(Duration::from_millis(200)));

        let held = tokio::spawn(async move { first.call(request()).await });
        // Let the first request take the only permit.
        tokio::time::sleep(Duration::from_millis(50)).await;

        let started = std::time::Instant::now();
        let shed = second.call(request()).await.expect("infallible");
        let waited = started.elapsed();

        assert_eq!(shed.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            shed.headers()
                .get(RETRY_AFTER)
                .and_then(|v| v.to_str().ok()),
            Some("1")
        );
        // The point of shedding: refused now, not queued behind the first.
        assert!(
            waited < Duration::from_millis(100),
            "the request waited {waited:?}, so it queued instead of shedding"
        );

        let first = held.await.expect("join").expect("infallible");
        assert_eq!(first.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn a_permit_is_released_when_the_response_is_produced() {
        let layer = LimitsLayer::new().with_max_concurrent(1);

        for _ in 0..3 {
            let mut service = layer.layer(Slow(Duration::ZERO));
            let response = service.call(request()).await.expect("infallible");
            assert_eq!(
                response.status(),
                StatusCode::OK,
                "a leaked permit would shed this"
            );
        }

        assert_eq!(layer.available(), Some(1));
    }

    #[tokio::test]
    async fn a_shed_request_does_not_consume_a_permit() {
        // Otherwise being overloaded would make the server progressively worse
        // at recovering from being overloaded.
        let layer = LimitsLayer::new().with_max_concurrent(1);
        let mut first = layer.layer(Slow(Duration::from_millis(150)));
        let mut second = layer.layer(Slow(Duration::ZERO));

        let held = tokio::spawn(async move { first.call(request()).await });
        tokio::time::sleep(Duration::from_millis(30)).await;

        for _ in 0..5 {
            let shed = second.call(request()).await.expect("infallible");
            assert_eq!(shed.status(), StatusCode::SERVICE_UNAVAILABLE);
        }

        let _ = held.await.expect("join");
        assert_eq!(
            layer.available(),
            Some(1),
            "the shed requests should not have taken permits"
        );
    }

    #[tokio::test]
    async fn a_slow_request_times_out() {
        let mut service = LimitsLayer::new()
            .with_timeout(Duration::from_millis(50))
            .layer(Slow(Duration::from_secs(30)));

        let response = service.call(request()).await.expect("infallible");
        assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    }

    #[tokio::test]
    async fn a_request_inside_the_timeout_is_untouched() {
        let mut service = LimitsLayer::new()
            .with_timeout(Duration::from_secs(5))
            .layer(Slow(Duration::from_millis(10)));

        let response = service.call(request()).await.expect("infallible");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn a_timed_out_request_releases_its_permit() {
        // A timeout that leaked permits would turn one slow client into a
        // permanent outage.
        let layer = LimitsLayer::new()
            .with_max_concurrent(1)
            .with_timeout(Duration::from_millis(30));

        let mut slow = layer.layer(Slow(Duration::from_secs(30)));
        let timed_out = slow.call(request()).await.expect("infallible");
        assert_eq!(timed_out.status(), StatusCode::GATEWAY_TIMEOUT);

        assert_eq!(layer.available(), Some(1));

        let mut fast = layer.layer(Slow(Duration::ZERO));
        let response = fast.call(request()).await.expect("infallible");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn clones_share_one_permit_pool() {
        // A per-clone limit would bound nothing: Streamable HTTP builds a
        // fresh service per request.
        let layer = LimitsLayer::new().with_max_concurrent(1);
        let cloned = layer.clone();

        let mut first = layer.layer(Slow(Duration::from_millis(150)));
        let mut second = cloned.layer(Slow(Duration::ZERO));

        let held = tokio::spawn(async move { first.call(request()).await });
        tokio::time::sleep(Duration::from_millis(30)).await;

        let shed = second.call(request()).await.expect("infallible");
        assert_eq!(shed.status(), StatusCode::SERVICE_UNAVAILABLE);

        let _ = held.await.expect("join");
    }

    #[test]
    fn a_limit_of_zero_is_clamped_to_one() {
        let layer = LimitsLayer::new().with_max_concurrent(0);
        assert_eq!(layer.max_concurrent(), Some(1));
        assert_eq!(layer.available(), Some(1));
    }

    #[test]
    fn nothing_is_configured_by_default() {
        let layer = LimitsLayer::new();
        assert_eq!(layer.max_concurrent(), None);
        assert_eq!(layer.timeout(), None);
        assert_eq!(layer.available(), None);
    }
}
