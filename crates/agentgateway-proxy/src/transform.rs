//! Request and response rewriting.
//!
//! Everything here is pure: it takes head parts and mutates them, with no I/O
//! and no client. That is deliberate — the parts of proxying that are easy to
//! get subtly wrong (hop-by-hop headers, forwarded-for chains, prefix
//! rewriting) are exactly the parts worth testing without standing up a
//! socket.
//!
//! Patterns are compiled at startup, so an invalid header name in a config is
//! a boot failure rather than a header silently not being set.

use std::net::IpAddr;

use agentgateway_config::{BackendAuth, HeaderModifier, PathRewrite, UrlRewrite};
use http::{HeaderMap, HeaderName, HeaderValue, Uri, header, uri::Authority, uri::PathAndQuery};

/// The scheme a client used to reach the gateway.
///
/// A plain `&'static str` would do the job, but it makes the closures in the
/// serve loop higher-ranked over a lifetime and the inference falls over. A
/// `Copy` enum sidesteps that and rules out typos besides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    /// Cleartext.
    Http,
    /// TLS-terminated at this gateway.
    Https,
}

impl Scheme {
    /// The `X-Forwarded-Proto` value.
    pub fn as_str(self) -> &'static str {
        match self {
            Scheme::Http => "http",
            Scheme::Https => "https",
        }
    }
}

/// A header name or value in the configuration that HTTP cannot represent.
#[derive(Debug, thiserror::Error)]
#[error("{at}: `{value}` is not a valid HTTP {kind}")]
pub struct HeaderError {
    /// Where in the configuration it came from.
    pub at: String,
    /// The offending text.
    pub value: String,
    /// `header name` or `header value`.
    pub kind: &'static str,
}

/// Headers that describe a single connection, not the message.
///
/// Forwarding these is how a proxy corrupts a connection it does not own: a
/// relayed `Transfer-Encoding` desynchronises framing, and a relayed
/// `Connection: close` tears down the wrong hop. RFC 9110 §7.6.1 requires them
/// to be dropped.
const HOP_BY_HOP: [HeaderName; 8] = [
    header::CONNECTION,
    header::PROXY_AUTHENTICATE,
    header::PROXY_AUTHORIZATION,
    header::TE,
    header::TRAILER,
    header::TRANSFER_ENCODING,
    header::UPGRADE,
    // `Keep-Alive` has no constant in `http`.
    HeaderName::from_static("keep-alive"),
];

/// Strip hop-by-hop headers, including any the `Connection` header names.
///
/// The second part matters and is often missed: `Connection: x-custom` makes
/// `x-custom` hop-by-hop for that message, and a proxy that only removes the
/// fixed list happily forwards it.
pub fn strip_hop_by_hop(headers: &mut HeaderMap) {
    let named: Vec<HeaderName> = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|token| HeaderName::try_from(token.trim()).ok())
        .collect();

    for name in HOP_BY_HOP.iter().chain(named.iter()) {
        headers.remove(name);
    }
}

/// A compiled [`HeaderModifier`].
#[derive(Debug, Clone, Default)]
pub struct Headers {
    add: Vec<(HeaderName, HeaderValue)>,
    set: Vec<(HeaderName, HeaderValue)>,
    remove: Vec<HeaderName>,
}

impl Headers {
    /// Compile a modifier, reporting the first name or value HTTP rejects.
    pub fn new(modifier: &HeaderModifier, at: &str) -> Result<Self, HeaderError> {
        let pair = |name: &String, value: &String| -> Result<(HeaderName, HeaderValue), HeaderError> {
            let header = HeaderName::try_from(name.as_str()).map_err(|_| HeaderError {
                at: at.to_string(),
                value: name.clone(),
                kind: "header name",
            })?;
            let value = HeaderValue::try_from(value.as_str()).map_err(|_| HeaderError {
                at: at.to_string(),
                value: value.clone(),
                kind: "header value",
            })?;
            Ok((header, value))
        };

        let mut add = Vec::with_capacity(modifier.add.len());
        for (name, value) in &modifier.add {
            add.push(pair(name, value)?);
        }
        let mut set = Vec::with_capacity(modifier.set.len());
        for (name, value) in &modifier.set {
            set.push(pair(name, value)?);
        }
        let mut remove = Vec::with_capacity(modifier.remove.len());
        for name in &modifier.remove {
            remove.push(HeaderName::try_from(name.as_str()).map_err(|_| HeaderError {
                at: at.to_string(),
                value: name.clone(),
                kind: "header name",
            })?);
        }

        Ok(Headers { add, set, remove })
    }

    /// Apply the modifier.
    ///
    /// Order follows Gateway API: `set` replaces, `add` appends, `remove`
    /// wins over both. Removing last means a config that both sets and removes
    /// a header removes it, which is the reading that fails safe.
    pub fn apply(&self, headers: &mut HeaderMap) {
        for (name, value) in &self.set {
            headers.insert(name, value.clone());
        }
        for (name, value) in &self.add {
            headers.append(name, value.clone());
        }
        for name in &self.remove {
            headers.remove(name);
        }
    }
}

/// A compiled [`UrlRewrite`].
#[derive(Debug, Clone, Default)]
pub struct Rewrite {
    authority: Option<Authority>,
    path: Option<PathRewrite>,
}

/// Failure to compile a rewrite.
#[derive(Debug, thiserror::Error)]
#[error("{at}: `{value}` is not a valid authority")]
pub struct RewriteError {
    /// Where in the configuration it came from.
    pub at: String,
    /// The offending text.
    pub value: String,
}

impl Rewrite {
    /// Compile a rewrite policy.
    pub fn new(rewrite: &UrlRewrite, at: &str) -> Result<Self, RewriteError> {
        let authority = match &rewrite.authority {
            Some(value) => Some(Authority::try_from(value.as_str()).map_err(|_| RewriteError {
                at: at.to_string(),
                value: value.clone(),
            })?),
            None => None,
        };
        Ok(Rewrite {
            authority,
            path: rewrite.path.clone(),
        })
    }

    /// The authority this rewrite forces, if any.
    pub fn authority(&self) -> Option<&Authority> {
        self.authority.as_ref()
    }

    /// Rewrite a path, given the route prefix that matched.
    ///
    /// `matched_prefix` is what a `prefix` rewrite replaces; without it the
    /// rewrite has nothing to anchor on and the path is left alone rather than
    /// mangled.
    pub fn path(&self, path: &str, matched_prefix: Option<&str>) -> Option<String> {
        match self.path.as_ref()? {
            PathRewrite::Full(replacement) => Some(replacement.clone()),
            PathRewrite::Prefix(replacement) => {
                let prefix = matched_prefix?;
                let prefix = prefix.strip_suffix('/').unwrap_or(prefix);
                let rest = path.strip_prefix(prefix).unwrap_or(path);
                let replacement = replacement.strip_suffix('/').unwrap_or(replacement);

                let rewritten = match (replacement.is_empty(), rest.is_empty()) {
                    // Replacing `/api` with `/` on a request for `/api` must
                    // produce `/`, not an empty path, which is not a valid
                    // origin-form target.
                    (true, true) => "/".to_string(),
                    (true, false) => rest.to_string(),
                    (false, true) => replacement.to_string(),
                    (false, false) => format!("{replacement}{rest}"),
                };
                Some(rewritten)
            }
        }
    }
}

/// Build the upstream URI for a request.
pub fn upstream_uri(
    original: &Uri,
    authority: &Authority,
    rewritten_path: Option<String>,
) -> Result<Uri, http::Error> {
    let path_and_query = match rewritten_path {
        Some(path) => {
            let with_query = match original.query() {
                Some(query) => format!("{path}?{query}"),
                None => path,
            };
            PathAndQuery::try_from(with_query).ok()
        }
        None => original.path_and_query().cloned(),
    };

    Uri::builder()
        .scheme("http")
        .authority(authority.clone())
        .path_and_query(
            path_and_query
                .map(|pq| pq.as_str().to_string())
                .unwrap_or_else(|| "/".into()),
        )
        .build()
}

/// Record this hop in the forwarded-for chain.
///
/// Appended rather than replaced: the chain is the point, and overwriting it
/// erases every proxy before us — which is how the original client address
/// gets lost and every rate limit downstream starts counting the same IP.
pub fn add_forwarded(
    headers: &mut HeaderMap,
    peer: Option<IpAddr>,
    host: Option<&HeaderValue>,
    scheme: Scheme,
) {
    if let Some(peer) = peer {
        let chain = match headers.get(FORWARDED_FOR).and_then(|v| v.to_str().ok()) {
            Some(existing) => format!("{existing}, {peer}"),
            None => peer.to_string(),
        };
        if let Ok(value) = HeaderValue::try_from(chain) {
            headers.insert(FORWARDED_FOR, value);
        }
    }

    // The upstream's idea of its own name should be the name the client used,
    // not the backend address we happen to be dialling.
    if let Some(host) = host
        && !headers.contains_key(FORWARDED_HOST)
    {
        headers.insert(FORWARDED_HOST, host.clone());
    }

    if !headers.contains_key(FORWARDED_PROTO) {
        // The scheme the *client* used, which is the listener's, not the
        // upstream's. An upstream behind a TLS listener that generates
        // absolute URLs from this header would otherwise emit http:// links
        // into an https:// page and trip mixed-content blocking.
        let scheme = match scheme {
            Scheme::Https => HeaderValue::from_static("https"),
            Scheme::Http => HeaderValue::from_static("http"),
        };
        headers.insert(FORWARDED_PROTO, scheme);
    }
}

const FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");
const FORWARDED_HOST: HeaderName = HeaderName::from_static("x-forwarded-host");
const FORWARDED_PROTO: HeaderName = HeaderName::from_static("x-forwarded-proto");

/// Attach the backend credential.
///
/// `passthrough` leaves the client's own `Authorization` in place; anything
/// else replaces it, so a client cannot smuggle its own credential to a
/// backend that is supposed to see only ours.
pub fn apply_backend_auth(headers: &mut HeaderMap, auth: Option<&BackendAuth>) {
    match auth {
        None => {}
        Some(BackendAuth::Passthrough(true)) => {}
        Some(BackendAuth::Passthrough(false)) => {
            headers.remove(header::AUTHORIZATION);
        }
        Some(BackendAuth::Key(key)) => {
            if let Ok(value) = HeaderValue::try_from(format!("Bearer {key}")) {
                headers.insert(header::AUTHORIZATION, value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(
                HeaderName::try_from(*name).expect("test header name"),
                HeaderValue::try_from(*value).expect("test header value"),
            );
        }
        map
    }

    #[test]
    fn hop_by_hop_headers_are_stripped() {
        let mut map = headers(&[
            ("connection", "close"),
            ("keep-alive", "timeout=5"),
            ("transfer-encoding", "chunked"),
            ("upgrade", "websocket"),
            ("content-type", "application/json"),
        ]);
        strip_hop_by_hop(&mut map);

        assert!(map.get("connection").is_none());
        assert!(map.get("keep-alive").is_none());
        assert!(map.get("transfer-encoding").is_none());
        assert!(map.get("upgrade").is_none());
        assert_eq!(
            map.get("content-type").and_then(|v| v.to_str().ok()),
            Some("application/json"),
            "end-to-end headers must survive"
        );
    }

    #[test]
    fn headers_named_by_connection_are_also_stripped() {
        // The half that is easy to miss: `Connection` makes the headers it
        // names hop-by-hop for this message only.
        let mut map = headers(&[
            ("connection", "x-internal-token, x-hop"),
            ("x-internal-token", "secret"),
            ("x-hop", "1"),
            ("x-keep", "yes"),
        ]);
        strip_hop_by_hop(&mut map);

        assert!(
            map.get("x-internal-token").is_none(),
            "a header named by Connection must not be forwarded"
        );
        assert!(map.get("x-hop").is_none());
        assert!(map.get("x-keep").is_some());
    }

    fn modifier(add: &[(&str, &str)], set: &[(&str, &str)], remove: &[&str]) -> HeaderModifier {
        HeaderModifier {
            add: add
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<BTreeMap<_, _>>(),
            set: set
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<BTreeMap<_, _>>(),
            remove: remove.iter().map(|r| r.to_string()).collect(),
        }
    }

    #[test]
    fn set_replaces_and_add_appends() {
        let compiled = Headers::new(
            &modifier(&[("x-multi", "second")], &[("x-one", "new")], &[]),
            "test",
        )
        .expect("should compile");

        let mut map = headers(&[("x-one", "old"), ("x-multi", "first")]);
        compiled.apply(&mut map);

        assert_eq!(map.get_all("x-one").iter().count(), 1);
        assert_eq!(map.get("x-one").and_then(|v| v.to_str().ok()), Some("new"));
        assert_eq!(
            map.get_all("x-multi").iter().count(),
            2,
            "add appends rather than replacing"
        );
    }

    #[test]
    fn remove_wins_over_set() {
        // A config that both sets and removes a header should end up without
        // it: that is the reading that fails safe.
        let compiled = Headers::new(
            &modifier(&[], &[("x-secret", "value")], &["x-secret"]),
            "test",
        )
        .expect("should compile");

        let mut map = HeaderMap::new();
        compiled.apply(&mut map);
        assert!(map.get("x-secret").is_none());
    }

    #[test]
    fn an_invalid_header_name_fails_to_compile() {
        let err = Headers::new(&modifier(&[], &[("not a header", "v")], &[]), "route[0]")
            .expect_err("should not compile");
        assert!(err.to_string().contains("route[0]"), "got: {err}");
    }

    fn rewrite(authority: Option<&str>, path: Option<PathRewrite>) -> Rewrite {
        Rewrite::new(
            &UrlRewrite {
                authority: authority.map(|a| a.to_string()),
                path,
            },
            "test",
        )
        .expect("should compile")
    }

    #[test]
    fn a_full_path_rewrite_replaces_everything() {
        let r = rewrite(None, Some(PathRewrite::Full("/v2/thing".into())));
        assert_eq!(
            r.path("/api/v1/thing", Some("/api")).as_deref(),
            Some("/v2/thing")
        );
    }

    #[test]
    fn a_prefix_rewrite_replaces_only_the_matched_prefix() {
        let r = rewrite(None, Some(PathRewrite::Prefix("/internal".into())));
        assert_eq!(
            r.path("/api/v1/thing", Some("/api")).as_deref(),
            Some("/internal/v1/thing")
        );
    }

    #[test]
    fn a_prefix_rewrite_to_root_keeps_a_valid_path() {
        // `/api` -> `/` on a request for exactly `/api` must not produce an
        // empty target, which is not a valid origin-form request line.
        let r = rewrite(None, Some(PathRewrite::Prefix("/".into())));
        assert_eq!(r.path("/api", Some("/api")).as_deref(), Some("/"));
        assert_eq!(r.path("/api/x", Some("/api")).as_deref(), Some("/x"));
    }

    #[test]
    fn a_prefix_rewrite_without_a_matched_prefix_leaves_the_path_alone() {
        // An exact or regex match has no prefix to anchor on. Leaving the path
        // untouched beats guessing and mangling it.
        let r = rewrite(None, Some(PathRewrite::Prefix("/internal".into())));
        assert_eq!(r.path("/api/v1", None), None);
    }

    #[test]
    fn forwarded_for_appends_rather_than_replacing() {
        let mut map = headers(&[("x-forwarded-for", "203.0.113.1")]);
        add_forwarded(
            &mut map,
            Some("198.51.100.7".parse().expect("ip")),
            None,
            Scheme::Http,
        );

        assert_eq!(
            map.get("x-forwarded-for").and_then(|v| v.to_str().ok()),
            Some("203.0.113.1, 198.51.100.7"),
            "overwriting the chain erases every proxy before us"
        );
    }

    #[test]
    fn a_tls_listener_reports_https_upstream() {
        // An upstream generating absolute URLs from this header would emit
        // http:// links into an https:// page otherwise.
        let mut map = HeaderMap::new();
        add_forwarded(&mut map, None, None, Scheme::Https);
        assert_eq!(
            map.get("x-forwarded-proto").and_then(|v| v.to_str().ok()),
            Some("https")
        );
    }

    #[test]
    fn forwarded_host_records_the_name_the_client_used() {
        let mut map = HeaderMap::new();
        let host = HeaderValue::from_static("api.example.com");
        add_forwarded(&mut map, None, Some(&host), Scheme::Http);

        assert_eq!(
            map.get("x-forwarded-host").and_then(|v| v.to_str().ok()),
            Some("api.example.com")
        );
        assert_eq!(
            map.get("x-forwarded-proto").and_then(|v| v.to_str().ok()),
            Some("http")
        );
    }

    #[test]
    fn a_backend_key_replaces_the_clients_own_credential() {
        // Otherwise a client could smuggle its own credential to a backend
        // that is supposed to see only ours.
        let mut map = headers(&[("authorization", "Bearer client-token")]);
        apply_backend_auth(&mut map, Some(&BackendAuth::Key("backend-key".into())));

        assert_eq!(
            map.get("authorization").and_then(|v| v.to_str().ok()),
            Some("Bearer backend-key")
        );
    }

    #[test]
    fn passthrough_leaves_the_clients_credential_in_place() {
        let mut map = headers(&[("authorization", "Bearer client-token")]);
        apply_backend_auth(&mut map, Some(&BackendAuth::Passthrough(true)));
        assert_eq!(
            map.get("authorization").and_then(|v| v.to_str().ok()),
            Some("Bearer client-token")
        );

        let mut map = headers(&[("authorization", "Bearer client-token")]);
        apply_backend_auth(&mut map, Some(&BackendAuth::Passthrough(false)));
        assert!(
            map.get("authorization").is_none(),
            "passthrough: false strips it"
        );
    }

    #[test]
    fn the_upstream_uri_keeps_the_query_string() {
        let original: Uri = "/api/v1?debug=1&x=2".parse().expect("uri");
        let authority = Authority::try_from("backend:8080").expect("authority");
        let built = upstream_uri(&original, &authority, Some("/internal/v1".into()))
            .expect("should build");

        assert_eq!(built.to_string(), "http://backend:8080/internal/v1?debug=1&x=2");
    }

    #[test]
    fn an_unrewritten_uri_passes_through_unchanged() {
        let original: Uri = "/api/v1?a=b".parse().expect("uri");
        let authority = Authority::try_from("backend:8080").expect("authority");
        let built = upstream_uri(&original, &authority, None).expect("should build");
        assert_eq!(built.to_string(), "http://backend:8080/api/v1?a=b");
    }
}
