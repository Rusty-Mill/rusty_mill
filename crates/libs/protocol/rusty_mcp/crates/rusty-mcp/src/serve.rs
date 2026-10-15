//! Auth and load shedding for a blocking [`rusty_serve`] server.
//!
//! The same decisions as [`crate::auth::RequireAuthLayer`] and
//! [`crate::limits::LimitsLayer`], without `tower`: each is a wrapper around a
//! [`rusty_serve::SharedHandler`]. Compose them by nesting, outermost first, so
//! that shed requests never reach token validation:
//!
//! ```no_run
//! use std::sync::Arc;
//! use rusty_mcp::auth::{AuthConfig, StaticTokenValidator};
//! use rusty_mcp::serve::{AuthorizedHandler, LimitedHandler};
//! use rusty_serve::{Request, Response, Server};
//!
//! let config = AuthConfig::new(
//!     "https://mcp.example.com/mcp",
//!     Arc::new(StaticTokenValidator::new()),
//! )
//! .expect("valid canonical resource URI");
//!
//! let app = AuthorizedHandler::new(config, |_request: &Request<'_>, token: &_| {
//!     let _ = token;
//!     Response::json(rusty_http::StatusCode::OK, b"{}".to_vec())
//! });
//! let app = LimitedHandler::new(app, 256);
//!
//! let server = Server::bind_shared("127.0.0.1:0".parse().expect("addr"), app)
//!     .expect("bind");
//! # let _ = server;
//! ```
//!
//! Wrap the MCP route only. The Protected Resource Metadata document must stay
//! reachable without a token, so route it before the wrapped handler.
//!
//! # What differs from the tower layers
//!
//! - **No timeout.** A blocking handler cannot be cancelled from outside, so
//!   [`LimitedHandler`] bounds concurrency only. [`rusty_serve::Limits`]
//!   already bounds idle time and connections.
//! - **The validator is polled on the connection's thread.** It runs on a
//!   small `std` executor with no reactor, which is enough for a validator
//!   that answers from memory (such as
//!   [`StaticTokenValidator`](crate::auth::StaticTokenValidator)) and not for
//!   one that needs a `tokio` runtime, like the JWKS-backed one, whose fetch
//!   would never be driven. Use the tower layers for that, or validate before
//!   the request reaches this wrapper. A validator that never completes holds
//!   its connection thread.

use std::{
    future::Future,
    pin::pin,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
    thread::{self, Thread},
};

use rusty_http::StatusCode;
use rusty_json::json;
use rusty_serve::{Request, Response, SharedHandler};

use crate::{
    admission::{Admission, Rejection as Shed},
    auth::{
        AuthConfig, VerifiedToken,
        decision::{self, AuthorizationHeader, Refusal},
    },
};

/// Sheds requests over a concurrency limit; see the [module docs](self).
///
/// Over the limit a request gets `503` with `Retry-After` at once; it is never
/// queued. A permit is held while the inner handler runs and released when it
/// returns, so a streamed or deferred body is not counted. Clones share one
/// count.
pub struct LimitedHandler<H> {
    inner: H,
    admission: Admission,
}

impl<H: SharedHandler> LimitedHandler<H> {
    /// At most `max_concurrent` requests inside `inner` at once. Zero is
    /// treated as one.
    pub fn new(inner: H, max_concurrent: usize) -> Self {
        Self {
            inner,
            admission: Admission::new(max_concurrent),
        }
    }

    /// Slots free right now, for a health endpoint or a test.
    pub fn available(&self) -> usize {
        self.admission.available()
    }
}

impl<H: SharedHandler> SharedHandler for LimitedHandler<H> {
    fn handle(&self, request: &Request<'_>) -> Response {
        match self.admission.try_admit() {
            Ok(_permit) => self.inner.handle(request),
            Err(rejection) => {
                tracing::warn!("shedding a request: the concurrency limit is reached");
                shed_response(rejection)
            }
        }
    }
}

/// Requires a valid bearer token for the handler it wraps; see the
/// [module docs](self).
///
/// The inner function gets the request and the validated [`VerifiedToken`], so
/// a tool can check its own scopes.
pub struct AuthorizedHandler<F> {
    config: Arc<AuthConfig>,
    inner: F,
}

impl<F> AuthorizedHandler<F>
where
    F: Fn(&Request<'_>, &VerifiedToken) -> Response + Send + Sync,
{
    /// Enforce `config` in front of `inner`.
    pub fn new(config: AuthConfig, inner: F) -> Self {
        Self {
            config: Arc::new(config),
            inner,
        }
    }
}

impl<F> SharedHandler for AuthorizedHandler<F>
where
    F: Fn(&Request<'_>, &VerifiedToken) -> Response + Send + Sync,
{
    fn handle(&self, request: &Request<'_>) -> Response {
        match authorize(&self.config, request.authorization) {
            Ok(token) => (self.inner)(request, &token),
            Err(refusal) => refusal_response(refusal),
        }
    }
}

/// The resource-server check over an `Authorization` header value: the pure
/// steps are in [`decision`]; only the validator is polled here.
fn authorize(config: &AuthConfig, header: Option<&str>) -> Result<VerifiedToken, Refusal> {
    let header = header.map_or(AuthorizationHeader::Absent, AuthorizationHeader::Text);
    let token = decision::bearer_token(header).map_err(|r| r.refusal(config))?;
    let validated = block_on(config.validator.validate(token));
    let verified = decision::from_validation(validated).map_err(|r| r.refusal(config))?;
    decision::check_claims(config, verified).map_err(|r| r.refusal(config))
}

/// A refusal as a `rusty_serve` response, with its challenge if it has one.
fn refusal_response(refusal: Refusal) -> Response {
    let response = Response::json(
        StatusCode::from_u16(refusal.status.as_u16()),
        refusal.body.into_bytes(),
    );
    let Some(challenge) = refusal.challenge else {
        return response;
    };
    match response.with_header("WWW-Authenticate", &challenge) {
        Ok(response) => response,
        Err(err) => {
            // Should be unreachable: `Challenge` strips control characters.
            tracing::error!(%err, "built an unusable WWW-Authenticate value");
            Response::json(
                StatusCode::from_u16(refusal.status.as_u16()),
                json!({ "error": "unauthorized" })
                    .to_json_string()
                    .into_bytes(),
            )
        }
    }
}

/// A shed request as a `rusty_serve` response, in its `{"error": ...}` shape.
fn shed_response(rejection: Shed) -> Response {
    let body = json!({ "error": { "code": "overloaded", "message": rejection.body } });
    let response = Response::json(
        StatusCode::from_u16(rejection.status.as_u16()),
        body.to_json_string().into_bytes(),
    );
    match rejection.retry_after_secs {
        Some(secs) => response
            .with_header("Retry-After", &secs.to_string())
            .unwrap_or_else(|_| Response::internal_error()),
        None => response,
    }
}

/// Wakes the thread that is blocked in [`block_on`].
struct ThreadWaker(Thread);

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

/// Drive `future` to completion on this thread, parking while it is pending.
/// There is no reactor, so only a future that makes progress on its own (or is
/// woken by another thread) ever finishes.
fn block_on<T>(future: impl Future<Output = T>) -> T {
    let waker = Waker::from(Arc::new(ThreadWaker(thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => thread::park(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::atomic::{AtomicBool, Ordering},
        time::Duration,
    };

    use super::*;

    #[test]
    fn block_on_returns_a_ready_value() {
        assert_eq!(block_on(std::future::ready(7)), 7);
    }

    /// Pending until another thread flips a flag and wakes it.
    struct WokenByAnotherThread(Arc<AtomicBool>, bool);

    impl Future for WokenByAnotherThread {
        type Output = &'static str;

        fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            if self.0.load(Ordering::SeqCst) {
                return Poll::Ready("done");
            }
            if !self.1 {
                self.1 = true;
                let (flag, waker) = (Arc::clone(&self.0), cx.waker().clone());
                thread::spawn(move || {
                    thread::sleep(Duration::from_millis(20));
                    flag.store(true, Ordering::SeqCst);
                    waker.wake();
                });
            }
            Poll::Pending
        }
    }

    #[test]
    fn block_on_parks_until_the_future_is_woken() {
        let future = WokenByAnotherThread(Arc::new(AtomicBool::new(false)), false);
        assert_eq!(block_on(future), "done");
    }

    #[test]
    fn a_shed_response_is_json_with_a_retry_after() {
        let response = shed_response(Shed::OVERLOADED);
        assert_eq!(response.status.as_u16(), 503);
        assert_eq!(response.headers.get("retry-after"), Some("1"));
    }

    #[test]
    fn a_refusal_without_a_challenge_sets_no_www_authenticate() {
        let refusal = Refusal {
            status: rusty_http::StatusCode::SERVICE_UNAVAILABLE,
            challenge: None,
            body: "{}".to_owned(),
        };
        assert!(
            refusal_response(refusal)
                .headers
                .get("www-authenticate")
                .is_none()
        );
    }
}
