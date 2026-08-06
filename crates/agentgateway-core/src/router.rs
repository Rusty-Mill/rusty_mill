//! Request routing.
//!
//! Building a [`Router`] compiles every regex and sorts every route into
//! precedence order once, so [`Router::select`] walks a pre-ordered list and
//! returns the first entry that matches. Nothing is compiled, allocated per
//! pattern, or re-sorted on the request path.
//!
//! Ordering follows Gateway API's precedence rules, which exist so that route
//! selection does not depend on the order rules happen to appear in a file:
//! an exact path beats a prefix, a longer prefix beats a shorter one, and
//! method, header and query predicates break the remaining ties. Regex matches
//! are ranked between prefix and no-path — Gateway API leaves that to the
//! implementation, and putting them below prefixes keeps the common case
//! predictable.

use agentgateway_config::{
    Backend, Config, HeaderMatchValue, PathMatch, Policies, Protocol, QueryMatchValue, RouteMatch,
};
use http::{Method, Request};

use crate::hostname::HostnamePattern;

/// Failure to build a router from a configuration.
#[derive(Debug, thiserror::Error)]
pub enum RouterError {
    /// A pattern that [`agentgateway_config::Config::validate`] would also
    /// reject. Surfaced here too, since a router may be built from a config
    /// that arrived over the wire rather than off disk.
    #[error("{at}: invalid regex `{pattern}`: {source}")]
    Regex {
        /// Where in the configuration the pattern came from.
        at: String,
        /// The pattern that failed to compile.
        pattern: String,
        /// Why it failed.
        #[source]
        source: Box<regex::Error>,
    },
    /// An HTTP method that is not a valid token.
    #[error("{at}: invalid HTTP method `{method}`")]
    Method {
        /// Where in the configuration the method came from.
        at: String,
        /// The offending value.
        method: String,
    },
}

/// A compiled routing table.
#[derive(Debug)]
pub struct Router {
    binds: Vec<CompiledBind>,
    routes: usize,
}

impl Router {
    /// Compile a configuration into a routing table.
    pub fn build(config: &Config) -> Result<Self, RouterError> {
        let mut binds = Vec::with_capacity(config.binds.len());
        let mut next_id = 0usize;
        for (b, bind) in config.binds.iter().enumerate() {
            let mut listeners = Vec::with_capacity(bind.listeners.len());
            for (l, listener) in bind.listeners.iter().enumerate() {
                let at = format!("binds[{b}].listeners[{l}]");
                listeners.push(CompiledListener::build(listener, &at, &mut next_id)?);
            }
            binds.push(CompiledBind {
                port: bind.port,
                listeners,
            });
        }
        Ok(Self {
            binds,
            routes: next_id,
        })
    }

    /// How many routes this router holds.
    ///
    /// Every [`CompiledRoute::id`] is less than this, so a caller can size a
    /// side table of per-route state by it.
    pub fn route_count(&self) -> usize {
        self.routes
    }

    /// Every route in the table, in id order.
    pub fn routes(&self) -> impl Iterator<Item = &CompiledRoute> {
        self.binds
            .iter()
            .flat_map(|bind| bind.listeners.iter())
            .flat_map(|listener| listener.routes.iter())
    }

    /// Every port this router expects a socket on.
    pub fn ports(&self) -> impl Iterator<Item = u16> + '_ {
        self.binds.iter().map(|bind| bind.port)
    }

    /// The bind serving `port`, if any.
    pub fn bind(&self, port: u16) -> Option<&CompiledBind> {
        self.binds.iter().find(|bind| bind.port == port)
    }

    /// Pick the route that should serve `request` on `port`.
    ///
    /// Returns `None` when the port is unknown, no listener claims the
    /// request's hostname, or no route on the chosen listener matches.
    pub fn select<B>(&self, port: u16, request: &Request<B>) -> Option<Selection<'_>> {
        let bind = self.bind(port)?;
        let host = request_host(request);

        // A listener whose hostname is more specific wins the request, even if
        // a less specific listener also could have served it.
        let listener = bind
            .listeners
            .iter()
            .filter(|listener| listener.hostname.matches(host))
            .max_by_key(|listener| listener.hostname.specificity())?;

        listener.select(request, host)
    }
}

/// One port, and the listeners multiplexed onto it.
#[derive(Debug)]
pub struct CompiledBind {
    /// The port this bind listens on.
    pub port: u16,
    /// Listeners selected by hostname.
    pub listeners: Vec<CompiledListener>,
}

/// A listener and its precedence-ordered routes.
#[derive(Debug)]
pub struct CompiledListener {
    /// Optional listener name, for logs.
    pub name: Option<String>,
    /// Hostname this listener claims.
    pub hostname: HostnamePattern,
    /// Wire protocol.
    pub protocol: Protocol,
    /// Routes, in configuration order.
    pub routes: Vec<CompiledRoute>,
    /// `(route, matcher)` pairs sorted most-specific first. A `None` matcher is
    /// a route with no predicates, which matches everything.
    order: Vec<Entry>,
}

#[derive(Debug)]
struct Entry {
    route: usize,
    matcher: Option<usize>,
    precedence: Precedence,
}

/// How specific a route entry is. Compared field by field, higher wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Precedence {
    host: u8,
    path_kind: u8,
    path_len: usize,
    method: u8,
    headers: usize,
    query: usize,
}

impl CompiledListener {
    fn build(
        listener: &agentgateway_config::Listener,
        at: &str,
        next_id: &mut usize,
    ) -> Result<CompiledListener, RouterError> {
        let mut routes = Vec::with_capacity(listener.routes.len());
        for (r, route) in listener.routes.iter().enumerate() {
            let id = *next_id;
            *next_id += 1;
            routes.push(CompiledRoute::build(route, &format!("{at}.routes[{r}]"), id)?);
        }

        let mut order = Vec::new();
        for (index, route) in routes.iter().enumerate() {
            let host = route
                .hostnames
                .iter()
                .map(HostnamePattern::specificity)
                .max()
                .unwrap_or(0);

            if route.matches.is_empty() {
                order.push(Entry {
                    route: index,
                    matcher: None,
                    precedence: Precedence {
                        host,
                        path_kind: 0,
                        path_len: 0,
                        method: 0,
                        headers: 0,
                        query: 0,
                    },
                });
                continue;
            }

            for (m, matcher) in route.matches.iter().enumerate() {
                order.push(Entry {
                    route: index,
                    matcher: Some(m),
                    precedence: matcher.precedence(host),
                });
            }
        }

        // Descending: the most specific entry is examined first. `sort_by` is
        // stable, so entries of equal precedence keep configuration order,
        // which is the documented tiebreak of last resort.
        order.sort_by(|a, b| b.precedence.cmp(&a.precedence));

        Ok(CompiledListener {
            name: listener.name.clone(),
            hostname: listener
                .hostname
                .as_deref()
                .map_or(HostnamePattern::Any, HostnamePattern::parse),
            protocol: listener.protocol,
            routes,
            order,
        })
    }

    fn select<B>(&self, request: &Request<B>, host: &str) -> Option<Selection<'_>> {
        let path = request.uri().path();
        let query = Query::parse(request.uri().query());

        for entry in &self.order {
            let route = &self.routes[entry.route];
            if !route.serves_host(host) {
                continue;
            }
            match entry.matcher {
                None => {
                    return Some(Selection {
                        listener: self,
                        route,
                        matched_prefix: None,
                    });
                }
                Some(index) => {
                    let matcher = &route.matches[index];
                    if let Some(matched_prefix) = matcher.matches(request, path, &query) {
                        return Some(Selection {
                            listener: self,
                            route,
                            matched_prefix,
                        });
                    }
                }
            }
        }
        None
    }
}

/// The route chosen for a request, and what matching learned along the way.
#[derive(Debug)]
pub struct Selection<'a> {
    /// The listener that claimed the request.
    pub listener: &'a CompiledListener,
    /// The route that matched.
    pub route: &'a CompiledRoute,
    /// The path prefix that matched, when the route matched by prefix. This is
    /// what a `prefix` URL rewrite replaces, so it has to survive matching.
    pub matched_prefix: Option<String>,
}

/// A route with its patterns compiled.
#[derive(Debug)]
pub struct CompiledRoute {
    /// Stable index into a caller's per-route side table. Assigned in build
    /// order and dense, so `Vec` indexing works.
    pub id: usize,
    /// Route name, for logs and metrics.
    pub name: Option<String>,
    /// Hostnames this route serves. Empty means any.
    pub hostnames: Vec<HostnamePattern>,
    /// Compiled predicates.
    pub matches: Vec<RouteMatcher>,
    /// Policies to apply.
    pub policies: Policies,
    /// Destinations.
    pub backends: Vec<Backend>,
}

impl CompiledRoute {
    fn build(
        route: &agentgateway_config::Route,
        at: &str,
        id: usize,
    ) -> Result<Self, RouterError> {
        let mut matches = Vec::with_capacity(route.matches.len());
        for (m, matcher) in route.matches.iter().enumerate() {
            matches.push(RouteMatcher::build(matcher, &format!("{at}.matches[{m}]"))?);
        }
        Ok(CompiledRoute {
            id,
            name: route.name.clone(),
            hostnames: route
                .hostnames
                .iter()
                .map(|h| HostnamePattern::parse(h))
                .collect(),
            matches,
            policies: route.policies.clone().unwrap_or_default(),
            backends: route.backends.clone(),
        })
    }

    fn serves_host(&self, host: &str) -> bool {
        self.hostnames.is_empty() || self.hostnames.iter().any(|h| h.matches(host))
    }
}

/// A compiled conjunction of request predicates.
#[derive(Debug)]
pub struct RouteMatcher {
    path: CompiledPath,
    method: Option<Method>,
    headers: Vec<CompiledHeader>,
    query: Vec<CompiledQuery>,
}

#[derive(Debug)]
enum CompiledPath {
    Exact(String),
    Prefix(String),
    Regex(regex::Regex),
}

#[derive(Debug)]
struct CompiledHeader {
    name: http::HeaderName,
    value: CompiledValue,
}

#[derive(Debug)]
struct CompiledQuery {
    name: String,
    value: CompiledValue,
}

#[derive(Debug)]
enum CompiledValue {
    Exact(String),
    Regex(regex::Regex),
    Present(bool),
}

impl CompiledValue {
    fn build(pattern: &str, at: &str) -> Result<regex::Regex, RouterError> {
        regex::Regex::new(pattern).map_err(|source| RouterError::Regex {
            at: at.to_string(),
            pattern: pattern.to_string(),
            source: Box::new(source),
        })
    }

    fn matches(&self, value: Option<&str>) -> bool {
        match (self, value) {
            (CompiledValue::Present(expected), found) => found.is_some() == *expected,
            (_, None) => false,
            (CompiledValue::Exact(expected), Some(found)) => expected == found,
            (CompiledValue::Regex(re), Some(found)) => re.is_match(found),
        }
    }
}

impl RouteMatcher {
    fn build(matcher: &RouteMatch, at: &str) -> Result<Self, RouterError> {
        let path = match matcher.path.clone().unwrap_or_default() {
            PathMatch::Exact(p) => CompiledPath::Exact(p),
            PathMatch::PathPrefix(p) => CompiledPath::Prefix(p),
            PathMatch::Regex(p) => {
                CompiledPath::Regex(CompiledValue::build(&p, &format!("{at}.path"))?)
            }
        };

        let method = match &matcher.method {
            None => None,
            Some(m) => Some(
                Method::try_from(m.as_str()).map_err(|_| RouterError::Method {
                    at: format!("{at}.method"),
                    method: m.clone(),
                })?,
            ),
        };

        let mut headers = Vec::with_capacity(matcher.headers.len());
        for header in &matcher.headers {
            let name = http::HeaderName::try_from(header.name.as_str()).map_err(|_| {
                RouterError::Method {
                    at: format!("{at}.headers"),
                    method: header.name.clone(),
                }
            })?;
            let value = match &header.value {
                HeaderMatchValue::Exact(v) => CompiledValue::Exact(v.clone()),
                HeaderMatchValue::Present(v) => CompiledValue::Present(*v),
                HeaderMatchValue::Regex(v) => CompiledValue::Regex(CompiledValue::build(
                    v,
                    &format!("{at}.headers.{}", header.name),
                )?),
            };
            headers.push(CompiledHeader { name, value });
        }

        let mut query = Vec::with_capacity(matcher.query.len());
        for param in &matcher.query {
            let value = match &param.value {
                QueryMatchValue::Exact(v) => CompiledValue::Exact(v.clone()),
                QueryMatchValue::Present(v) => CompiledValue::Present(*v),
                QueryMatchValue::Regex(v) => CompiledValue::Regex(CompiledValue::build(
                    v,
                    &format!("{at}.query.{}", param.name),
                )?),
            };
            query.push(CompiledQuery {
                name: param.name.clone(),
                value,
            });
        }

        Ok(RouteMatcher {
            path,
            method,
            headers,
            query,
        })
    }

    fn precedence(&self, host: u8) -> Precedence {
        let (path_kind, path_len) = match &self.path {
            CompiledPath::Exact(p) => (3, p.len()),
            CompiledPath::Prefix(p) => (2, p.len()),
            CompiledPath::Regex(re) => (1, re.as_str().len()),
        };
        Precedence {
            host,
            path_kind,
            path_len,
            method: u8::from(self.method.is_some()),
            headers: self.headers.len(),
            query: self.query.len(),
        }
    }

    /// Returns `Some(matched_prefix)` when every predicate holds.
    ///
    /// The nested `Option` is load-bearing: the outer one says whether the
    /// match succeeded, the inner one carries the prefix that matched, which
    /// only exists for prefix matches.
    fn matches<B>(
        &self,
        request: &Request<B>,
        path: &str,
        query: &Query,
    ) -> Option<Option<String>> {
        if let Some(expected) = &self.method
            && request.method() != expected
        {
            return None;
        }

        let matched_prefix = match &self.path {
            CompiledPath::Exact(expected) => {
                if path != expected {
                    return None;
                }
                None
            }
            CompiledPath::Prefix(prefix) => {
                if !prefix_matches(path, prefix) {
                    return None;
                }
                Some(prefix.clone())
            }
            CompiledPath::Regex(re) => {
                if !re.is_match(path) {
                    return None;
                }
                None
            }
        };

        for header in &self.headers {
            let found = request
                .headers()
                .get(&header.name)
                .and_then(|v| v.to_str().ok());
            if !header.value.matches(found) {
                return None;
            }
        }

        for param in &self.query {
            if !param.value.matches(query.get(&param.name)) {
                return None;
            }
        }

        Some(matched_prefix)
    }
}

/// Whether `path` sits under `prefix`, matching whole segments only.
///
/// `/api` covers `/api` and `/api/v1` but not `/apixyz`. Gateway API is
/// explicit about this, and a raw `starts_with` here is a classic way to route
/// `/admin-public` into `/admin`.
fn prefix_matches(path: &str, prefix: &str) -> bool {
    let prefix = prefix.strip_suffix('/').unwrap_or(prefix);
    if prefix.is_empty() {
        return true;
    }
    let Some(rest) = path.strip_prefix(prefix) else {
        return false;
    };
    rest.is_empty() || rest.starts_with('/')
}

/// The request's hostname, from `Host` or the absolute-form URI.
fn request_host<B>(request: &Request<B>) -> &str {
    request
        .uri()
        .host()
        .or_else(|| {
            request
                .headers()
                .get(http::header::HOST)
                .and_then(|v| v.to_str().ok())
        })
        .unwrap_or("")
}

/// A parsed query string.
///
/// Parsing is done once per request rather than once per predicate, and only
/// when the request actually carries a query.
#[derive(Debug, Default)]
struct Query {
    params: Vec<(String, String)>,
}

impl Query {
    fn parse(raw: Option<&str>) -> Self {
        let Some(raw) = raw else {
            return Query::default();
        };
        let params = raw
            .split('&')
            .filter(|pair| !pair.is_empty())
            .map(|pair| match pair.split_once('=') {
                Some((name, value)) => (percent_decode(name), percent_decode(value)),
                None => (percent_decode(pair), String::new()),
            })
            .collect();
        Query { params }
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// Decode `%XX` escapes and `+`, leaving invalid escapes as written.
///
/// Comparing an encoded value against a config-file literal would make
/// `?tenant=a%20b` fail to match `exact: "a b"`, so decoding is not optional.
fn percent_decode(raw: &str) -> String {
    if !raw.contains('%') && !raw.contains('+') {
        return raw.to_string();
    }

    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = &raw[i + 1..i + 3];
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    // Not a real escape; keep it verbatim rather than guessing.
                    Err(_) => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }

    String::from_utf8_lossy(&out).into_owned()
}
