//! Cross-origin resource sharing.
//!
//! Browsers reach MCP endpoints directly, so CORS is not decoration here: a
//! browser client cannot read `Mcp-Session-Id` off the initialize response
//! unless the server exposes it, and without the session id it cannot make a
//! second request. That is why the upstream MCP quickstart lists it under
//! `exposeHeaders`, and why this module keeps expose headers verbatim rather
//! than normalizing them away.

use agentgateway_config::CorsPolicy;
use http::{HeaderMap, HeaderValue, Method, Request, header};

/// A compiled CORS policy.
#[derive(Debug, Clone)]
pub struct CorsMatcher {
    allow_origins: Vec<String>,
    allow_any_origin: bool,
    allow_headers: Option<HeaderValue>,
    allow_methods: Option<HeaderValue>,
    expose_headers: Option<HeaderValue>,
    max_age: Option<HeaderValue>,
    allow_credentials: bool,
}

/// What to do with a request, once CORS has looked at it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorsDecision {
    /// Not a cross-origin request. Proceed untouched.
    NotCors,
    /// A preflight. Answer it here; do not call the backend.
    Preflight(HeaderMap),
    /// A real cross-origin request. Proceed, then merge these into the
    /// response.
    Simple(HeaderMap),
}

impl CorsMatcher {
    /// Compile a policy from configuration.
    pub fn new(policy: &CorsPolicy) -> Self {
        let allow_any_origin = policy.allow_origins.iter().any(|o| o == "*");
        CorsMatcher {
            allow_origins: policy
                .allow_origins
                .iter()
                .map(|o| o.to_ascii_lowercase())
                .collect(),
            allow_any_origin,
            allow_headers: join(&policy.allow_headers),
            allow_methods: join(&policy.allow_methods),
            expose_headers: join(&policy.expose_headers),
            max_age: policy
                .max_age
                .and_then(|d| HeaderValue::try_from(d.as_secs().to_string()).ok()),
            allow_credentials: policy.allow_credentials.unwrap_or(false),
        }
    }

    /// Classify a request and produce the headers its response needs.
    pub fn evaluate<B>(&self, request: &Request<B>) -> CorsDecision {
        let Some(origin) = request
            .headers()
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok())
        else {
            return CorsDecision::NotCors;
        };

        if !self.allows(origin) {
            // Answering without the allow-origin header is what a rejection
            // looks like over CORS: the browser blocks it. Returning 403 here
            // instead would break non-browser clients, which never asked.
            return CorsDecision::Simple(HeaderMap::new());
        }

        let mut headers = HeaderMap::new();

        // Echoing the origin rather than replying `*` is required whenever
        // credentials are allowed, and harmless otherwise -- but it makes the
        // response vary by origin, so caches must be told.
        let echo = if self.allow_any_origin && !self.allow_credentials {
            HeaderValue::from_static("*")
        } else {
            match HeaderValue::try_from(origin) {
                Ok(value) => value,
                Err(_) => return CorsDecision::Simple(HeaderMap::new()),
            }
        };
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, echo);
        headers.insert(header::VARY, HeaderValue::from_static("Origin"));

        if self.allow_credentials {
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
                HeaderValue::from_static("true"),
            );
        }

        let is_preflight = request.method() == Method::OPTIONS
            && request
                .headers()
                .contains_key(header::ACCESS_CONTROL_REQUEST_METHOD);

        if !is_preflight {
            if let Some(expose) = &self.expose_headers {
                headers.insert(header::ACCESS_CONTROL_EXPOSE_HEADERS, expose.clone());
            }
            return CorsDecision::Simple(headers);
        }

        // Fall back to echoing what the client asked for when the policy does
        // not enumerate methods or headers; an empty list would deny the
        // preflight outright, which is never what an unset field means.
        let requested_method = request
            .headers()
            .get(header::ACCESS_CONTROL_REQUEST_METHOD)
            .cloned();
        if let Some(methods) = self.allow_methods.clone().or(requested_method) {
            headers.insert(header::ACCESS_CONTROL_ALLOW_METHODS, methods);
        }

        let requested_headers = request
            .headers()
            .get(header::ACCESS_CONTROL_REQUEST_HEADERS)
            .cloned();
        if let Some(allow) = self.allow_headers.clone().or(requested_headers) {
            headers.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, allow);
        }

        if let Some(max_age) = &self.max_age {
            headers.insert(header::ACCESS_CONTROL_MAX_AGE, max_age.clone());
        }

        CorsDecision::Preflight(headers)
    }

    fn allows(&self, origin: &str) -> bool {
        self.allow_any_origin
            || self
                .allow_origins
                .iter()
                .any(|allowed| allowed == &origin.to_ascii_lowercase())
    }
}

fn join(values: &[String]) -> Option<HeaderValue> {
    if values.is_empty() {
        return None;
    }
    HeaderValue::try_from(values.join(", ")).ok()
}
