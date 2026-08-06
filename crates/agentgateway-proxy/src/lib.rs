//! HTTP reverse proxying for `host` backends.
//!
//! A `host` backend is the opposite of an `mcp` one: it forwards bytes rather
//! than terminating a protocol. That makes it the place where the policies
//! modelled but unenforced until now finally do something — header modifiers,
//! `urlRewrite`, `backendAuth`, and the per-attempt half of `timeout`.
//!
//! The pure parts live in [`transform`] and [`balance`] so the fiddly bits
//! (hop-by-hop headers, forwarded chains, prefix rewriting, weighted
//! selection) are testable without a socket.

mod balance;
mod transform;

use std::net::IpAddr;
use std::time::Duration;

use agentgateway_config::{Backend, BackendAuth, BackendTarget, Policies};
use http::{HeaderValue, Request, Response, StatusCode, header};
use hyper::body::Incoming;
use hyper_util::client::legacy::{Client, connect::HttpConnector};
use hyper_util::rt::TokioExecutor;

pub use balance::{BalanceError, Endpoints};
pub use transform::{HeaderError, Headers, RewriteError, Rewrite};

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
}

/// A proxying `host` backend.
pub struct HostProxy {
    endpoints: Endpoints,
    client: Client<HttpConnector, Incoming>,
    rewrite: Option<Rewrite>,
    request_headers: Option<Headers>,
    response_headers: Option<Headers>,
    backend_auth: Option<BackendAuth>,
    /// Budget for a single upstream attempt.
    timeout: Option<Duration>,
}

impl std::fmt::Debug for HostProxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostProxy")
            .field("endpoints", &self.endpoints)
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl HostProxy {
    /// Build a proxy for a route's `host` backends.
    pub fn new(backends: &[Backend], policies: &Policies, at: &str) -> Result<Self, ProxyError> {
        let hosts = backends.iter().filter_map(|backend| match &backend.target {
            BackendTarget::Host(host) => Some((host.as_str(), backend.weight)),
            _ => None,
        });
        let endpoints = Endpoints::new(hosts, at)?;

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
        let response_headers = match policies.response_header_modifier.as_ref() {
            Some(modifier) => Some(Headers::new(
                modifier,
                &format!("{at}.responseHeaderModifier"),
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
            endpoints,
            client,
            rewrite,
            request_headers,
            response_headers,
            backend_auth: policies.backend_auth.clone(),
            timeout,
        })
    }

    /// How many endpoints can receive traffic.
    pub fn endpoint_count(&self) -> usize {
        self.endpoints.len()
    }

    /// Forward a request upstream and return the response.
    ///
    /// `matched_prefix` is the route prefix that matched, which a `prefix`
    /// rewrite replaces. `peer` is the client address, recorded in the
    /// forwarded-for chain.
    pub async fn proxy(
        &self,
        request: Request<Incoming>,
        matched_prefix: Option<&str>,
        peer: Option<IpAddr>,
    ) -> Response<ProxyBody> {
        let (mut parts, body) = request.into_parts();

        let host = parts.headers.get(header::HOST).cloned();

        // Strip before mutating: a policy that adds a header should not have
        // it removed again by hop-by-hop handling, and a client must not be
        // able to make one of our own headers hop-by-hop by naming it in
        // `Connection`.
        transform::strip_hop_by_hop(&mut parts.headers);
        transform::add_forwarded(&mut parts.headers, peer, host.as_ref());
        transform::apply_backend_auth(&mut parts.headers, self.backend_auth.as_ref());
        if let Some(headers) = &self.request_headers {
            headers.apply(&mut parts.headers);
        }

        let authority = match self.rewrite.as_ref().and_then(Rewrite::authority) {
            Some(forced) => forced.clone(),
            None => self.endpoints.next().clone(),
        };

        let rewritten_path = self
            .rewrite
            .as_ref()
            .and_then(|rewrite| rewrite.path(parts.uri.path(), matched_prefix));

        let uri = match transform::upstream_uri(&parts.uri, &authority, rewritten_path) {
            Ok(uri) => uri,
            Err(err) => {
                tracing::warn!(%err, "could not build the upstream URI");
                return error(StatusCode::INTERNAL_SERVER_ERROR, "invalid upstream URI");
            }
        };

        // The upstream's `Host` must name the upstream, not us, or a
        // name-based virtual host upstream serves the wrong site.
        if let Ok(value) = HeaderValue::try_from(authority.as_str()) {
            parts.headers.insert(header::HOST, value);
        }
        parts.uri = uri;

        let upstream = Request::from_parts(parts, body);
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
                    return error(
                        StatusCode::GATEWAY_TIMEOUT,
                        "the upstream did not respond within its budget",
                    );
                }
            },
            None => call.await,
        };

        let response = match result {
            Ok(response) => response,
            Err(err) => {
                // 502, not 500: the gateway is fine, the upstream is not, and
                // conflating the two sends people debugging the wrong process.
                tracing::warn!(upstream = %authority, %err, "upstream request failed");
                return error(StatusCode::BAD_GATEWAY, "the upstream could not be reached");
            }
        };

        let (mut parts, body) = response.into_parts();
        transform::strip_hop_by_hop(&mut parts.headers);
        if let Some(headers) = &self.response_headers {
            headers.apply(&mut parts.headers);
        }

        Response::from_parts(parts, ProxyBody::Upstream(body))
    }
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
