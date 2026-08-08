//! HTTP reverse proxying for `host` and `service` backends.
//!
//! Both forward bytes rather than terminating a protocol, which is what makes
//! them one kind here: a `service` is resolved to addresses before this sees
//! it, so the only difference by the time a request is dialled is how many
//! endpoints there are. That makes this the place where the policies modelled
//! but unenforced until now finally do something — header modifiers,
//! `urlRewrite`, `backendAuth`, and the per-attempt half of `timeout`.
//!
//! The pure parts live in [`transform`] and [`balance`] so the fiddly bits
//! (hop-by-hop headers, forwarded chains, prefix rewriting, weighted
//! selection) are testable without a socket.

mod balance;
mod dynamic;
mod retry;
mod transform;

use std::net::IpAddr;
use std::time::Duration;

use agentgateway_config::{BackendAuth, Policies};
use agentgateway_core::Endpoint;
use http::{HeaderValue, Request, Response, StatusCode, header};
use http_body_util::BodyExt as _;
use hyper::body::Incoming;
use hyper_util::client::legacy::{Client, connect::HttpConnector};
use hyper_util::rt::TokioExecutor;

pub use balance::{BalanceError, Endpoints};
pub use dynamic::{Dynamic, TargetError};
// Re-exported so the proxy's own callers need not learn where this moved.
pub use agentgateway_core::Retry;
pub use retry::{MAX_REPLAY_BYTES, RequestBody};
pub use transform::{HeaderError, Headers, Rewrite, RewriteError, Scheme};

/// Failure to build a proxy from configuration.
#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    /// An endpoint could not be resolved.
    #[error(transparent)]
    Balance(#[from] BalanceError),

    /// A header modifier held something HTTP cannot represent.
    #[error(transparent)]
    Header(#[from] HeaderError),

    /// A rewrite held an invalid authority.
    #[error(transparent)]
    Rewrite(#[from] RewriteError),

    /// A `service` backend named something the inventory does not hold.
    #[error(transparent)]
    Registry(#[from] agentgateway_core::RegistryError),

    /// A `dynamic` backend's `target` will not compile.
    #[error(transparent)]
    Target(#[from] TargetError),
}

/// Where a route sends a request.
#[derive(Debug)]
enum Destination {
    /// A weighted ring, resolved at startup.
    Fixed(Endpoints),
    /// Taken from the request. See [`dynamic`].
    PerRequest(Dynamic),
}

/// A proxying `host`, `service` or `dynamic` backend.
pub struct HostProxy {
    destination: Destination,
    client: Client<HttpConnector, RequestBody>,
    rewrite: Option<Rewrite>,
    request_headers: Option<Headers>,
    backend_auth: Option<BackendAuth>,
    /// Budget for a single upstream attempt.
    timeout: Option<Duration>,
    retry: Option<Retry>,
}

impl std::fmt::Debug for HostProxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostProxy")
            .field("destination", &self.destination)
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl HostProxy {
    /// Build a proxy that takes its destination from each request.
    ///
    /// Separate from [`HostProxy::new`] because there is nothing to resolve:
    /// the address is not in the configuration at all. See [`dynamic`] for what
    /// that means for anyone deploying one.
    pub fn dynamic(
        backend: &agentgateway_config::DynamicBackend,
        policies: &Policies,
        at: &str,
    ) -> Result<Self, ProxyError> {
        let mut proxy = Self::build(
            Destination::PerRequest(Dynamic::new(backend, at)?),
            policies,
            at,
        )?;
        // The rewrite's authority would override the whole point of the
        // backend, so it is dropped rather than silently winning. A path
        // rewrite still applies: it does not choose the upstream.
        if let Some(rewrite) = proxy.rewrite.as_ref()
            && rewrite.authority().is_some()
        {
            tracing::warn!(
                route = %at,
                "ignoring `urlRewrite.authority` on a `dynamic` route: the request chooses the \
                 upstream, and a fixed authority would mean it never does"
            );
            proxy.rewrite = proxy.rewrite.take().map(Rewrite::without_authority);
        }
        Ok(proxy)
    }

    /// Build a proxy for a route's already-resolved endpoints.
    ///
    /// Resolution happens before this so that a `service` backend's failure to
    /// resolve is reported as what it is — an inventory problem — rather than
    /// as an empty endpoint set. See [`agentgateway_core::resolve_backends`].
    pub fn new(resolved: &[Endpoint], policies: &Policies, at: &str) -> Result<Self, ProxyError> {
        let hosts = resolved
            .iter()
            .map(|endpoint| (endpoint.authority.as_str(), endpoint.weight));
        Self::build(Destination::Fixed(Endpoints::new(hosts, at)?), policies, at)
    }

    /// Everything both constructors share: the policies applied per request.
    fn build(destination: Destination, policies: &Policies, at: &str) -> Result<Self, ProxyError> {
        let rewrite = match policies.url_rewrite.as_ref() {
            Some(rewrite) => Some(Rewrite::new(rewrite, &format!("{at}.urlRewrite"))?),
            None => None,
        };
        let request_headers = match policies.request_header_modifier.as_ref() {
            Some(modifier) => Some(Headers::new(
                modifier,
                &format!("{at}.requestHeaderModifier"),
            )?),
            None => None,
        };
        let timeout = policies
            .timeout
            .as_ref()
            .and_then(|t| t.backend_request_timeout)
            .map(Duration::from);

        let client = Client::builder(TokioExecutor::new())
            // Upstreams are long-lived; re-dialling per request would put a
            // TCP handshake in front of every call for no reason.
            .pool_idle_timeout(Duration::from_secs(30))
            .build(HttpConnector::new());

        Ok(HostProxy {
            destination,
            client,
            rewrite,
            request_headers,
            backend_auth: policies.backend_auth.clone(),
            timeout,
            retry: policies.retry.as_ref().and_then(Retry::new),
        })
    }

    /// How many endpoints can receive traffic.
    ///
    /// Zero for a `dynamic` route: its destination is not known until a
    /// request names one.
    pub fn endpoint_count(&self) -> usize {
        match &self.destination {
            Destination::Fixed(endpoints) => endpoints.len(),
            Destination::PerRequest(_) => 0,
        }
    }

    /// Whether this route takes its destination from the request.
    pub fn is_dynamic(&self) -> bool {
        matches!(self.destination, Destination::PerRequest(_))
    }

    /// Forward a request upstream and return the response.
    ///
    /// `matched_prefix` is the route prefix that matched, which a `prefix`
    /// rewrite replaces. `peer` is the client address, recorded in the
    /// forwarded-for chain, and `scheme` is the listener's, reported as
    /// `X-Forwarded-Proto`.
    pub async fn proxy(
        &self,
        request: Request<RequestBody>,
        matched_prefix: Option<&str>,
        peer: Option<IpAddr>,
        scheme: Scheme,
    ) -> Response<ProxyBody> {
        let (mut parts, body) = request.into_parts();

        let host = parts.headers.get(header::HOST).cloned();

        // Strip before mutating: a policy that adds a header should not have
        // it removed again by hop-by-hop handling, and a client must not be
        // able to make one of our own headers hop-by-hop by naming it in
        // `Connection`.
        transform::strip_hop_by_hop(&mut parts.headers);
        transform::add_forwarded(&mut parts.headers, peer, host.as_ref(), scheme);
        transform::apply_backend_auth(&mut parts.headers, self.backend_auth.as_ref());
        if let Some(headers) = &self.request_headers {
            headers.apply(&mut parts.headers);
        }

        let rewritten_path = self
            .rewrite
            .as_ref()
            .and_then(|rewrite| rewrite.path(parts.uri.path(), matched_prefix));

        // Buffer only when the size is known up front and small enough.
        // Reading to find out would leave a stream that can be neither
        // replayed nor forwarded intact.
        let mut body = match &self.retry {
            Some(_) if retry::is_replayable(&body) => match body.collect().await {
                Ok(collected) => RequestBody::Buffered(collected.to_bytes()),
                Err(err) => {
                    tracing::warn!(%err, "reading the request body failed");
                    return error(StatusCode::BAD_REQUEST, "could not read the request body");
                }
            },
            _ => body,
        };

        let attempts = match (&self.retry, &body) {
            (Some(retry), RequestBody::Buffered(_)) => retry.max_attempts(),
            // A streaming body cannot be replayed, so there is exactly one
            // attempt however the policy is written.
            _ => 1,
        };

        let mut last_response = None;

        for attempt in 0..attempts {
            if attempt > 0
                && let Some(retry) = &self.retry
                && let Some(wait) = retry.backoff(attempt)
            {
                tokio::time::sleep(wait).await;
            }

            // A fresh endpoint each attempt: retrying the instance that just
            // failed is the least likely way to succeed.
            let authority = match self.rewrite.as_ref().and_then(Rewrite::authority) {
                Some(forced) => forced.clone(),
                None => match &self.destination {
                    Destination::Fixed(endpoints) => endpoints.next().clone(),
                    // A `dynamic` route has one destination per request, so
                    // every attempt goes back to the same place -- there is no
                    // second instance to fail over to, and inventing one would
                    // mean dialling somewhere the request did not name.
                    Destination::PerRequest(dynamic) => {
                        match dynamic.authority(&Request::from_parts(parts.clone(), ())) {
                            Some(authority) => authority,
                            None => {
                                return error(
                                    StatusCode::BAD_REQUEST,
                                    "this route takes its upstream from the request, and this \
                                     request names none",
                                );
                            }
                        }
                    }
                },
            };

            let uri = match transform::upstream_uri(&parts.uri, &authority, rewritten_path.clone())
            {
                Ok(uri) => uri,
                Err(err) => {
                    tracing::warn!(%err, "could not build the upstream URI");
                    return error(StatusCode::INTERNAL_SERVER_ERROR, "invalid upstream URI");
                }
            };

            let mut attempt_parts = parts.clone();
            // The version the *client* used is a property of its connection,
            // not the upstream's. A client arriving over HTTP/2 -- which is
            // what ALPN negotiates on a TLS listener -- would otherwise make
            // us attempt h2 prior-knowledge against a cleartext upstream that
            // only speaks HTTP/1.1, and every such request 502s.
            attempt_parts.version = http::Version::HTTP_11;
            // The upstream's `Host` must name the upstream, not us, or a
            // name-based virtual host upstream serves the wrong site.
            if let Ok(value) = HeaderValue::try_from(authority.as_str()) {
                attempt_parts.headers.insert(header::HOST, value);
            }
            attempt_parts.uri = uri;

            // Keep a replay for the next attempt before handing this one over.
            let replay = body.replay();
            let sending = std::mem::replace(&mut body, RequestBody::Buffered(bytes::Bytes::new()));
            let upstream = Request::from_parts(attempt_parts, sending);

            let call = self.client.request(upstream);
            let result = match self.timeout {
                Some(budget) => match tokio::time::timeout(budget, call).await {
                    Ok(result) => result,
                    Err(_) => {
                        tracing::warn!(
                            upstream = %authority,
                            timeout_ms = budget.as_millis() as u64,
                            "upstream exceeded its budget"
                        );
                        // Not retried: a timeout is ambiguous. The upstream may
                        // have received and processed the request, with only the
                        // response lost, and replaying would double the work.
                        return error(
                            StatusCode::GATEWAY_TIMEOUT,
                            "the upstream did not respond within its budget",
                        );
                    }
                },
                None => call.await,
            };

            let retryable_left = attempt + 1 < attempts;

            match result {
                Ok(response) => {
                    let status = response.status().as_u16();
                    let retry_this = retryable_left
                        && self
                            .retry
                            .as_ref()
                            .is_some_and(|r| r.retries_status(status));
                    if !retry_this {
                        return finish(response);
                    }
                    tracing::debug!(
                        upstream = %authority,
                        status,
                        attempt = attempt + 1,
                        "retrying an upstream response"
                    );
                    last_response = Some(response);
                }
                Err(err) => {
                    // 502, not 500: the gateway is fine, the upstream is not,
                    // and conflating the two sends people debugging the wrong
                    // process.
                    tracing::warn!(upstream = %authority, %err, "upstream request failed");

                    // Only a connect failure is known never to have reached the
                    // upstream. Any other transport error may have been received
                    // and processed, with the response lost on the way back, so
                    // replaying it could double a write.
                    if !(retryable_left && err.is_connect()) {
                        return error(StatusCode::BAD_GATEWAY, "the upstream could not be reached");
                    }
                }
            }

            match replay {
                Some(replay) => body = replay,
                // Nothing left to send; stop rather than replay an empty body.
                None => break,
            }
        }

        let Some(response) = last_response else {
            return error(StatusCode::BAD_GATEWAY, "the upstream could not be reached");
        };

        let (mut parts, body) = response.into_parts();
        transform::strip_hop_by_hop(&mut parts.headers);

        Response::from_parts(parts, ProxyBody::Upstream(body))
    }
}

/// Apply response policies and hand the body back to the caller.
/// Hand an upstream response on, minus the headers that do not cross a hop.
///
/// A route's `responseHeaderModifier` used to be applied here. It moved to the
/// gateway, where every backend kind converges, so that one description of the
/// policy is true of `ai` and `a2a` routes too rather than only of proxied
/// ones.
fn finish(response: Response<Incoming>) -> Response<ProxyBody> {
    let (mut parts, body) = response.into_parts();
    transform::strip_hop_by_hop(&mut parts.headers);
    Response::from_parts(parts, ProxyBody::Upstream(body))
}

/// The body a proxied response carries.
///
/// Upstream bodies are streamed through rather than collected: a proxy that
/// buffers turns a download into a memory limit.
pub enum ProxyBody {
    /// Streamed from the upstream.
    Upstream(Incoming),
    /// Generated here, for an error the upstream never saw.
    Message(bytes::Bytes),
}

impl http_body::Body for ProxyBody {
    type Data = bytes::Bytes;
    type Error = hyper::Error;

    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        // Safe because neither variant is structurally pinned: `Incoming` is
        // `Unpin`, and `Bytes` holds no self-references.
        match self.get_mut() {
            ProxyBody::Upstream(body) => std::pin::Pin::new(body).poll_frame(cx),
            ProxyBody::Message(bytes) => {
                if bytes.is_empty() {
                    std::task::Poll::Ready(None)
                } else {
                    let chunk = std::mem::take(bytes);
                    std::task::Poll::Ready(Some(Ok(http_body::Frame::data(chunk))))
                }
            }
        }
    }

    fn size_hint(&self) -> http_body::SizeHint {
        match self {
            ProxyBody::Upstream(body) => body.size_hint(),
            ProxyBody::Message(bytes) => http_body::SizeHint::with_exact(bytes.len() as u64),
        }
    }
}

fn error(code: StatusCode, message: &str) -> Response<ProxyBody> {
    let mut response = Response::new(ProxyBody::Message(bytes::Bytes::from(message.to_string())));
    *response.status_mut() = code;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
}
