//! Forward-proxy support: [`Proxy`] (an explicit `http://host[:port]`
//! proxy every request can be routed through) and [`NoProxyRules`]
//! (`NO_PROXY`-style bypass rules for specific hosts).
//!
//! An `http://` request is forwarded in cleartext (absolute-form
//! request-target, RFC 7230 §5.3.2, `Host` still naming the origin) --
//! the proxy can read and rewrite it. An `https://` request instead asks
//! the proxy to open an opaque `CONNECT` tunnel to the origin (see
//! `crate::client::establish`/`connect_tunnel`), since the proxy can't
//! (and shouldn't be able to) read encrypted traffic; the TLS handshake
//! and the real request both happen *through* that tunnel, invisible to
//! the proxy. Either way the proxy itself is reached over plain
//! `http://` -- [`Proxy::http`] rejects an `https://` proxy URL, since
//! this crate has no way to open the first (unencrypted) hop to such a
//! proxy in a way that would let it understand a `CONNECT` request.

use crate::error::{Error, Result};
use crate::pool::PoolKey;
use rusty_http::Url;

/// An HTTP forward proxy: requests go to `host:port` instead of the
/// request's own origin. An `http://` request uses the origin's full
/// URL as the absolute-form request-target (RFC 7230 §5.3.2) so the
/// proxy knows where to forward it, with `Host` still naming the origin;
/// an `https://` request instead tunnels through the proxy via `CONNECT`
/// (see the module docs).
#[derive(Debug, Clone)]
pub struct Proxy {
    pub(crate) host: String,
    pub(crate) port: u16,
}

impl Proxy {
    /// Parses `proxy_url` (`http://host[:port]`) as the proxy to route
    /// requests through. Rejects an `https://` proxy URL: reaching the
    /// proxy itself always happens over plain `http://`, whether the
    /// request being routed through it is `http://` or `https://` (see
    /// the module docs) -- a `CONNECT` tunnel changes how traffic to the
    /// *origin* is protected, not how the client talks to the proxy.
    pub fn http(proxy_url: &str) -> Result<Proxy> {
        let url = Url::parse(proxy_url)?;
        if url.scheme != "http" {
            return Err(Error::UnsupportedScheme(url.scheme));
        }
        Ok(Proxy {
            host: url.host,
            port: url.port,
        })
    }

    /// A distinct pool key namespace (`"proxy"` isn't a real URL scheme)
    /// so a pooled connection *to* a proxy, used for plain `http://`
    /// forwarding, can never collide with a pooled direct connection
    /// that coincidentally targets the same host:port.
    pub(crate) fn pool_key(&self) -> PoolKey {
        (
            "proxy".to_string(),
            self.host.to_ascii_lowercase(),
            self.port,
        )
    }

    /// The pool key for a `CONNECT`-tunneled `https://` connection
    /// through this proxy to `url`'s origin. Distinct from both
    /// [`Proxy::pool_key`] (this proxy's shared plain-forwarding key)
    /// and a direct connection's own key: once a tunnel is up it's a
    /// private, origin-specific channel -- physically routed via the
    /// proxy, but not shareable across origins the way plain forwarding
    /// is -- so reusing a pooled tunneled connection for a *different*
    /// origin than the one it was tunneled to would silently send that
    /// request to the wrong place.
    pub(crate) fn tunnel_pool_key(&self, url: &Url) -> PoolKey {
        (
            "connect-tunnel".to_string(),
            format!(
                "{}:{}->{}",
                self.host.to_ascii_lowercase(),
                self.port,
                url.host.to_ascii_lowercase()
            ),
            url.port,
        )
    }
}

/// Which [`Proxy`], if any, each request scheme routes through: an
/// explicit [`crate::ClientBuilder::proxy`] covers both, while the
/// environment can name a different one per scheme (see [`from_env`]).
#[derive(Debug, Clone, Default)]
pub(crate) struct Proxies {
    pub(crate) http: Option<Proxy>,
    pub(crate) https: Option<Proxy>,
}

impl Proxies {
    /// One proxy for every scheme.
    pub(crate) fn all(proxy: Proxy) -> Self {
        Proxies {
            http: Some(proxy.clone()),
            https: Some(proxy),
        }
    }

    /// The proxy a request to `url` routes through, before bypass rules.
    pub(crate) fn for_url(&self, url: &Url) -> Option<&Proxy> {
        match url.scheme.as_str() {
            "http" => self.http.as_ref(),
            "https" => self.https.as_ref(),
            _ => None,
        }
    }
}

/// A proxy configuration read from environment variables: each part is
/// `None` when its variables are unset, so it leaves that part alone.
#[derive(Debug, Default)]
pub(crate) struct EnvProxy {
    pub(crate) http: Option<Proxy>,
    pub(crate) https: Option<Proxy>,
    pub(crate) bypass: Option<NoProxyRules>,
}

/// Reads the curl/`requests` proxy variables through `var` (a variable's
/// value, or `None` when unset): `HTTP_PROXY`/`http_proxy` for `http://`
/// requests, `HTTPS_PROXY`/`https_proxy` for `https://` ones, with
/// `ALL_PROXY`/`all_proxy` as the fallback for both, and
/// `NO_PROXY`/`no_proxy` for bypass rules. Empty values count as unset.
///
/// httpoxy mitigation: the uppercase `HTTP_PROXY` is ignored whenever
/// `REQUEST_METHOD` is set, since a CGI handler maps an inbound `Proxy:`
/// header onto it. A value without a scheme is taken as `http://`. An
/// invalid value is an error naming the variable but never its value,
/// which may carry credentials.
pub(crate) fn from_env(var: impl Fn(&str) -> Option<String>) -> Result<EnvProxy> {
    let set = |name: &str| {
        var(name)
            .filter(|v| !v.is_empty())
            .map(|v| (name.to_string(), v))
    };
    let first = |names: &[&str]| names.iter().find_map(|name| set(name));
    let in_cgi_context = var("REQUEST_METHOD").is_some();
    let all = first(&["ALL_PROXY", "all_proxy"]);
    let http = if in_cgi_context {
        first(&["http_proxy"])
    } else {
        first(&["HTTP_PROXY", "http_proxy"])
    };
    let https = first(&["HTTPS_PROXY", "https_proxy"]);
    Ok(EnvProxy {
        http: http.or_else(|| all.clone()).map(env_proxy).transpose()?,
        https: https.or(all).map(env_proxy).transpose()?,
        bypass: first(&["NO_PROXY", "no_proxy"]).map(|(_, spec)| NoProxyRules::parse(&spec)),
    })
}

fn env_proxy((name, value): (String, String)) -> Result<Proxy> {
    let url = if value.contains("://") {
        value
    } else {
        format!("http://{value}")
    };
    Proxy::http(&url)
        .map_err(|_| Error::InvalidUrl(format!("`{name}` is not an http:// proxy URL")))
}

/// `NO_PROXY`-style bypass rules: a request to a matching host skips
/// the configured [`Proxy`] and connects directly. Each entry matches
/// either exactly or as a dot-boundary suffix (`example.com` also
/// matches `www.example.com`, the same convention `crate::cookie`'s
/// `Domain` matching already uses) -- except the single entry `*`,
/// which bypasses the proxy for every host.
#[derive(Debug, Clone, Default)]
pub(crate) struct NoProxyRules {
    entries: Vec<String>,
}

impl NoProxyRules {
    pub(crate) fn from_hosts<I, S>(hosts: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        NoProxyRules {
            entries: hosts
                .into_iter()
                .map(|h| {
                    h.as_ref()
                        .trim()
                        .trim_start_matches('.')
                        .to_ascii_lowercase()
                })
                .filter(|h| !h.is_empty())
                .collect(),
        }
    }

    /// Parses a comma-separated `NO_PROXY`-style spec (whitespace around
    /// entries is trimmed).
    pub(crate) fn parse(spec: &str) -> Self {
        Self::from_hosts(spec.split(','))
    }

    pub(crate) fn bypasses(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        self.entries
            .iter()
            .any(|e| e == "*" || host == *e || host.ends_with(&format!(".{e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let vars: Vec<(String, String)> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name| vars.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
    }

    fn host(proxy: &Option<Proxy>) -> Option<&str> {
        proxy.as_ref().map(|p| p.host.as_str())
    }

    #[test]
    fn env_proxies_are_chosen_per_scheme() {
        let read = from_env(env(&[
            ("HTTP_PROXY", "http://plain.example:3128"),
            ("HTTPS_PROXY", "http://tls.example:3128"),
        ]))
        .unwrap();
        assert_eq!(host(&read.http), Some("plain.example"));
        assert_eq!(host(&read.https), Some("tls.example"));
        assert!(read.bypass.is_none());
    }

    #[test]
    fn an_http_proxy_alone_leaves_https_direct() {
        let read = from_env(env(&[("http_proxy", "http://plain.example")])).unwrap();
        assert_eq!(host(&read.http), Some("plain.example"));
        assert!(read.https.is_none());
    }

    #[test]
    fn all_proxy_is_the_fallback_for_both_schemes() {
        let read = from_env(env(&[
            ("all_proxy", "http://all.example"),
            ("https_proxy", "http://tls.example"),
        ]))
        .unwrap();
        assert_eq!(host(&read.http), Some("all.example"));
        assert_eq!(host(&read.https), Some("tls.example"));
        let read = from_env(env(&[("ALL_PROXY", "http://all.example")])).unwrap();
        assert_eq!(host(&read.http), Some("all.example"));
        assert_eq!(host(&read.https), Some("all.example"));
    }

    #[test]
    fn uppercase_http_proxy_is_ignored_in_a_cgi_context() {
        let read = from_env(env(&[
            ("REQUEST_METHOD", "GET"),
            ("HTTP_PROXY", "http://attacker.example"),
            ("HTTPS_PROXY", "http://tls.example"),
        ]))
        .unwrap();
        assert!(read.http.is_none());
        assert_eq!(host(&read.https), Some("tls.example"));
    }

    #[test]
    fn env_no_proxy_sets_bypass_rules_and_empty_values_are_unset() {
        let read = from_env(env(&[
            ("HTTPS_PROXY", ""),
            ("no_proxy", "internal.example"),
        ]))
        .unwrap();
        assert!(read.https.is_none());
        assert!(read
            .bypass
            .is_some_and(|rules| rules.bypasses("api.internal.example")));
    }

    #[test]
    fn an_env_proxy_without_a_scheme_is_http() {
        let read = from_env(env(&[("HTTPS_PROXY", "tls.example:3128")])).unwrap();
        let proxy = read.https.unwrap();
        assert_eq!((proxy.host.as_str(), proxy.port), ("tls.example", 3128));
    }

    #[test]
    fn an_invalid_env_proxy_names_the_variable_but_not_its_value() {
        let secret = "s3cr3t-proxy-password";
        let err = from_env(env(&[(
            "HTTPS_PROXY",
            &format!("http://user:{secret}@proxy.example"),
        )]))
        .unwrap_err()
        .to_string();
        assert!(err.contains("HTTPS_PROXY"), "{err}");
        assert!(!err.contains(secret), "{err}");
    }

    #[test]
    fn proxies_route_by_url_scheme() {
        let read = from_env(env(&[("HTTPS_PROXY", "http://tls.example")])).unwrap();
        let proxies = Proxies {
            http: read.http,
            https: read.https,
        };
        let plain = Url::parse("http://origin.example/").unwrap();
        let tls = Url::parse("https://origin.example/").unwrap();
        assert!(proxies.for_url(&plain).is_none());
        assert_eq!(
            proxies.for_url(&tls).map(|p| p.host.as_str()),
            Some("tls.example")
        );
        let both = Proxies::all(Proxy::http("http://one.example").unwrap());
        assert!(both.for_url(&plain).is_some() && both.for_url(&tls).is_some());
    }

    #[test]
    fn http_proxy_parses_host_and_port() {
        let p = Proxy::http("http://proxy.example:8080").unwrap();
        assert_eq!(p.host, "proxy.example");
        assert_eq!(p.port, 8080);
    }

    #[test]
    fn http_proxy_rejects_https_scheme() {
        assert!(matches!(
            Proxy::http("https://proxy.example"),
            Err(Error::UnsupportedScheme(_))
        ));
    }

    #[test]
    fn http_proxy_rejects_invalid_url() {
        assert!(Proxy::http("not a url").is_err());
    }

    #[test]
    fn distinct_proxies_get_distinct_pool_keys() {
        let a = Proxy::http("http://a.example:8080").unwrap();
        let b = Proxy::http("http://b.example:8080").unwrap();
        assert_ne!(a.pool_key(), b.pool_key());
    }

    #[test]
    fn tunnel_pool_key_differs_from_forwarding_key_and_by_origin() {
        let proxy = Proxy::http("http://proxy.example:8080").unwrap();
        let a = Url::parse("https://a.example/").unwrap();
        let b = Url::parse("https://b.example/").unwrap();

        // A tunnel to origin A must not collide with a tunnel to origin
        // B through the same proxy...
        assert_ne!(proxy.tunnel_pool_key(&a), proxy.tunnel_pool_key(&b));
        // ...nor with this proxy's own plain-forwarding key.
        assert_ne!(proxy.tunnel_pool_key(&a), proxy.pool_key());
    }

    #[test]
    fn no_proxy_rules_match_exact_host() {
        let rules = NoProxyRules::parse("example.com");
        assert!(rules.bypasses("example.com"));
        assert!(rules.bypasses("EXAMPLE.COM"));
        assert!(!rules.bypasses("other.com"));
    }

    #[test]
    fn no_proxy_rules_match_subdomains() {
        let rules = NoProxyRules::parse("example.com");
        assert!(rules.bypasses("www.example.com"));
        assert!(!rules.bypasses("notexample.com"));
    }

    #[test]
    fn no_proxy_rules_handle_leading_dot_and_whitespace() {
        let rules = NoProxyRules::parse(" .example.com , other.com ");
        assert!(rules.bypasses("example.com"));
        assert!(rules.bypasses("sub.other.com"));
    }

    #[test]
    fn no_proxy_star_bypasses_everything() {
        let rules = NoProxyRules::parse("*");
        assert!(rules.bypasses("anything.at.all"));
    }

    #[test]
    fn empty_no_proxy_rules_bypass_nothing() {
        let rules = NoProxyRules::default();
        assert!(!rules.bypasses("example.com"));
    }
}
