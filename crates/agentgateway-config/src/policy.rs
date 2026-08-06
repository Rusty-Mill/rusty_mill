//! Route policies.
//!
//! Policies attach to a route and shape the request on its way through:
//! CORS, authentication, header rewriting, timeouts, retries, rate limits.
//! Everything upstream accepts parses here. What this gateway does not yet
//! enforce is reported by [`Policies::lint`] rather than silently ignored —
//! a policy that parses but does nothing is worse than one that fails to load,
//! because it looks like security.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::oneof::one_of_enum;

/// The policy bundle attached to a route.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Policies {
    /// Cross-origin resource sharing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cors: Option<CorsPolicy>,

    /// JWT bearer token validation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jwt_auth: Option<JwtAuth>,

    /// Tool-level authorization for MCP backends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_authorization: Option<McpAuthorization>,

    /// Credential attached when calling the backend.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend_auth: Option<BackendAuth>,

    /// Request and backend timeouts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<TimeoutPolicy>,

    /// Retry behaviour for failed backend attempts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryPolicy>,

    /// Mutations applied to the request headers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_header_modifier: Option<HeaderModifier>,

    /// Mutations applied to the response headers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_header_modifier: Option<HeaderModifier>,

    /// Rewrites applied to the request line before proxying.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url_rewrite: Option<UrlRewrite>,

    /// Token-bucket rate limits enforced in this process.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub local_rate_limit: Vec<LocalRateLimit>,

    /// External authorization service.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ext_authz: Option<BTreeMap<String, serde_json::Value>>,

    /// Agent-to-agent protocol handling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub a2a: Option<BTreeMap<String, serde_json::Value>>,

    /// LLM-specific policies such as prompt guards and model routing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai: Option<BTreeMap<String, serde_json::Value>>,
}

impl Policies {
    pub(crate) fn lint(&self, at: &str, findings: &mut Vec<String>) {
        let unimplemented: [(&str, bool); 5] = [
            ("extAuthz", self.ext_authz.is_some()),
            ("a2a", self.a2a.is_some()),
            ("ai", self.ai.is_some()),
            (
                "mcpAuthorization.rules",
                self.mcp_authorization
                    .as_ref()
                    .is_some_and(|a| !a.rules.is_empty()),
            ),
            (
                "localRateLimit[type=tokens]",
                self.local_rate_limit
                    .iter()
                    .any(|limit| limit.kind == RateLimitKind::Tokens),
            ),
        ];
        for (name, present) in unimplemented {
            if present {
                findings.push(format!(
                    "{at}.policies.{name}: parsed but not enforced by this build"
                ));
            }
        }
    }
}

/// Cross-origin resource sharing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorsPolicy {
    /// Permitted origins. `*` permits any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_origins: Vec<String>,

    /// Request headers a client may send.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_headers: Vec<String>,

    /// Methods a client may use.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_methods: Vec<String>,

    /// Response headers a browser may read. `Mcp-Session-Id` belongs here for
    /// any MCP route a browser talks to, or the client cannot resume sessions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expose_headers: Vec<String>,

    /// How long a preflight result may be cached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_age: Option<DurationString>,

    /// Whether credentialed requests are permitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_credentials: Option<bool>,
}

/// JWT bearer token validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JwtAuth {
    /// Expected `iss` claim.
    pub issuer: String,

    /// Accepted `aud` values. Empty accepts any audience.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audiences: Vec<String>,

    /// Where to find the signing keys.
    pub jwks: JwtSource,
}

one_of_enum! {
    /// Where a JWKS is loaded from.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum JwtSource {
        /// A JWKS document on local disk.
        "file" => File(String),
        /// A JWKS endpoint fetched over HTTP and refreshed periodically.
        "url" => Url(String),
    }
}

/// Tool-level authorization for MCP backends.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpAuthorization {
    /// Upstream policy expressions. Accepted for compatibility; this build
    /// does not evaluate them, and [`Policies::lint`] says so at startup.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<String>,

    /// Tools callable through this route, as unanchored regexes over the
    /// federated (prefixed) name. Empty allows all not denied below.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_tools: Vec<String>,

    /// Tools rejected on this route. Denies win over allows.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny_tools: Vec<String>,
}

one_of_enum! {
    /// A credential attached when calling the backend.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum BackendAuth {
        /// A literal key sent as a bearer token.
        "key" => Key(String),
        /// Forward the client's own `Authorization` header unchanged.
        "passthrough" => Passthrough(bool),
    }
}

/// Request and backend timeouts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeoutPolicy {
    /// Budget for the whole request, including retries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_timeout: Option<DurationString>,

    /// Budget for a single backend attempt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend_request_timeout: Option<DurationString>,
}

/// Retry behaviour for failed backend attempts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryPolicy {
    /// How many times to retry after the first attempt.
    #[serde(default)]
    pub attempts: u32,

    /// Base delay between attempts, doubled each time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backoff: Option<DurationString>,

    /// Response statuses that count as retryable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub codes: Vec<u16>,
}

/// Mutations applied to a header map.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeaderModifier {
    /// Headers appended, keeping any existing values.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub add: BTreeMap<String, String>,

    /// Headers written, replacing any existing values.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub set: BTreeMap<String, String>,

    /// Header names stripped entirely.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remove: Vec<String>,
}

/// Rewrites applied to the request line before proxying.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UrlRewrite {
    /// Replacement `Host` authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority: Option<String>,

    /// Path rewrite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathRewrite>,
}

one_of_enum! {
    /// How to rewrite the request path.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum PathRewrite {
        /// Replace the whole path.
        "full" => Full(String),
        /// Replace the segment prefix that the route matched.
        "prefix" => Prefix(String),
    }
}

/// A token-bucket rate limit enforced in this process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalRateLimit {
    /// Bucket capacity, and so the largest burst allowed.
    pub max_tokens: u64,

    /// Tokens added each `fillInterval`.
    pub tokens_per_fill: u64,

    /// How often the bucket refills.
    pub fill_interval: DurationString,

    /// What each token represents.
    #[serde(default, rename = "type")]
    pub kind: RateLimitKind,
}

/// What a rate-limit token represents.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RateLimitKind {
    /// One token per request.
    #[default]
    Requests,
    /// One token per LLM token consumed.
    Tokens,
}

/// A duration written the way humans and upstream configs write it: `5s`,
/// `100ms`, `2m`, `1h`.
///
/// Stored as a [`Duration`] but serialized back to the compact string form, so
/// a round-trip through this type does not rewrite a user's config file into
/// `{secs: 5, nanos: 0}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DurationString(pub Duration);

impl From<DurationString> for Duration {
    fn from(value: DurationString) -> Self {
        value.0
    }
}

impl std::ops::Deref for DurationString {
    type Target = Duration;
    fn deref(&self) -> &Duration {
        &self.0
    }
}

impl std::fmt::Display for DurationString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ms = self.0.as_millis();
        if ms == 0 {
            return write!(f, "0s");
        }
        if ms % 3_600_000 == 0 {
            write!(f, "{}h", ms / 3_600_000)
        } else if ms % 60_000 == 0 {
            write!(f, "{}m", ms / 60_000)
        } else if ms % 1_000 == 0 {
            write!(f, "{}s", ms / 1_000)
        } else {
            write!(f, "{ms}ms")
        }
    }
}

impl std::str::FromStr for DurationString {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        // Longest suffix first: `ms` must win over `m`.
        let (value, scale_ms) = if let Some(v) = s.strip_suffix("ms") {
            (v, 1u64)
        } else if let Some(v) = s.strip_suffix('s') {
            (v, 1_000)
        } else if let Some(v) = s.strip_suffix('m') {
            (v, 60_000)
        } else if let Some(v) = s.strip_suffix('h') {
            (v, 3_600_000)
        } else {
            return Err(format!(
                "`{s}` is not a duration; expected a number with a `ms`, `s`, `m` or `h` suffix"
            ));
        };

        let value: u64 = value
            .trim()
            .parse()
            .map_err(|_| format!("`{s}` is not a duration; `{value}` is not a whole number"))?;

        value
            .checked_mul(scale_ms)
            .map(|ms| DurationString(Duration::from_millis(ms)))
            .ok_or_else(|| format!("`{s}` overflows a duration"))
    }
}

impl Serialize for DurationString {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for DurationString {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}
