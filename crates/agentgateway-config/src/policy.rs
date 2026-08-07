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
    pub ext_authz: Option<ExtAuthzPolicy>,

    /// External MCP policy processors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_guardrails: Option<McpGuardrails>,

    /// Agent-to-agent protocol handling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub a2a: Option<A2aPolicy>,

    /// LLM-specific policies such as prompt guards and model routing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai: Option<BTreeMap<String, serde_json::Value>>,
}

impl Policies {
    pub(crate) fn lint(&self, at: &str, findings: &mut Vec<String>) {
        let unimplemented: [(&str, bool); 3] = [
            (
                "extAuthz.includeBody",
                self.ext_authz
                    .as_ref()
                    .is_some_and(|e| e.include_body.is_some()),
            ),
            ("ai", self.ai.is_some()),
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

        if let Some(guardrails) = &self.mcp_guardrails {
            guardrails.lint(&format!("{at}.policies.mcpGuardrails"), findings);
        }
    }
}

/// External MCP policy processors.
///
/// Each processor is an MCP-aware policy service the gateway consults before
/// forwarding a call and after receiving its result — Envoy's `ext_authz`
/// shape, moved down to the MCP method layer, and able to rewrite as well as
/// refuse.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpGuardrails {
    /// Processors applied to matched methods, in order.
    ///
    /// The first to refuse short-circuits the chain.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub processors: Vec<Processor>,
}

impl McpGuardrails {
    /// Report processors that parse but cannot run in this build.
    fn lint(&self, at: &str, findings: &mut Vec<String>) {
        for (i, processor) in self.processors.iter().enumerate() {
            let at = format!("{at}.processors[{i}]");
            if processor.host.is_none() {
                findings.push(format!(
                    "{at}: only `host` is supported; `backend` and `service` need a backend \
                     registry this build does not have, so this processor will not run"
                ));
            }
            for pattern in processor.methods.keys() {
                if !crate::pattern_is_matchable(pattern) {
                    findings.push(format!(
                        "{at}.methods: `{pattern}` can never match; use an exact method, \
                         `prefix/*`, `*/suffix`, or `*`"
                    ));
                }
            }

            // A processor keyed only on methods this gateway never serves is
            // indistinguishable, in production, from one that always passes.
            let reaches_something = crate::MCP_SERVED_METHODS
                .iter()
                .any(|method| crate::resolve(method, &processor.methods) != Phase::Off);
            let has_usable_pattern = processor
                .methods
                .keys()
                .any(|pattern| crate::pattern_is_matchable(pattern));
            if has_usable_pattern && !reaches_something {
                findings.push(format!(
                    "{at}.methods: matches no method this gateway serves, so this processor \
                     never runs; only {} are served",
                    crate::MCP_SERVED_METHODS.join(" and ")
                ));
            }
        }
    }
}

/// One external policy service in an `mcpGuardrails` chain.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Processor {
    /// Which methods run through this processor, and at which phase.
    ///
    /// An allow-list: a method matching no key bypasses the processor
    /// entirely. Keys may be exact (`tools/call`), a prefix wildcard
    /// (`tools/*`), a suffix wildcard (`*/list`), or `*` for everything.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub methods: BTreeMap<String, Phase>,

    /// Discriminator. Only `remote` exists; accepted and ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,

    /// `host:port` of the policy service.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,

    /// A backend named in the top-level `backends` list. Not supported here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,

    /// A service named in the top-level `services` list. Not supported here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<serde_json::Value>,

    /// What to do when the processor is unreachable or answers unusably.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_mode: Option<FailureMode>,

    /// Budget for one call to the processor. Defaults to 10s, matching upstream.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<DurationString>,

    /// CEL expressions evaluated per call and sent as `metadata_context`.
    ///
    /// One entry per key. The context is `jwt` — the verified token's claims —
    /// and `request`, carrying `method`, `path` and `headers`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,

    /// Which incoming request headers are forwarded to the processor.
    #[serde(default, skip_serializing_if = "HeaderFilter::is_default")]
    pub request_headers: HeaderFilter,

    /// Backend policies used when connecting. Accepted; nothing is applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policies: Option<serde_json::Value>,
}

/// Which side (or sides) of a call a processor sees.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    /// Not consulted.
    #[default]
    Off,
    /// Before the call is forwarded.
    Request,
    /// After the result comes back.
    Response,
    /// Both.
    Full,
}

impl Phase {
    /// Whether this phase runs before the call is forwarded.
    pub fn runs_request(self) -> bool {
        matches!(self, Phase::Request | Phase::Full)
    }

    /// Whether this phase runs after the result comes back.
    pub fn runs_response(self) -> bool {
        matches!(self, Phase::Response | Phase::Full)
    }
}

/// What to do when a processor cannot answer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FailureMode {
    /// Refuse the call. The default: a policy service that is down must not
    /// become an open door.
    #[default]
    FailClosed,
    /// Serve the call anyway.
    FailOpen,
}

/// Which request headers a processor is shown.
///
/// An empty `allowed` forwards everything, matching upstream — the opposite of
/// `extAuthz.includeHeaders`, whose empty list forwards nothing. The
/// difference is upstream's, not a choice made here. `disallowed` always wins.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeaderFilter {
    /// Headers to forward; empty forwards all of them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed: Vec<String>,

    /// Headers to drop, ahead of the allow list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disallowed: Vec<String>,
}

impl HeaderFilter {
    fn is_default(&self) -> bool {
        self.allowed.is_empty() && self.disallowed.is_empty()
    }

    /// Whether a header is shown to the processor.
    ///
    /// Names are compared case-insensitively, since HTTP header names are.
    pub fn allows(&self, name: &str) -> bool {
        let matches = |list: &[String]| list.iter().any(|n| n.eq_ignore_ascii_case(name));
        if matches(&self.disallowed) {
            return false;
        }
        self.allowed.is_empty() || matches(&self.allowed)
    }
}

/// An external authorization service consulted before a request is served.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtAuthzPolicy {
    /// Base URL of the authorization service.
    ///
    /// The original request path is appended, so the service sees the path
    /// being authorized and can route on it — the same shape as Envoy's
    /// `http_service` with a `path_prefix`.
    pub target: String,

    /// Budget for the authorization call. Defaults to 250ms.
    ///
    /// Short on purpose: this sits in front of every request on the route, so
    /// a slow authorizer is a slow gateway.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<DurationString>,

    /// Request headers forwarded to the authorizer.
    ///
    /// An allow-list rather than everything: the authorizer has no need for
    /// the cookies and payload headers of every request, and sending them
    /// widens what a compromised authorizer can read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include_headers: Vec<String>,

    /// Headers from the authorizer's response copied onto the upstream
    /// request — how an authorizer passes down a resolved identity.
    ///
    /// Also an allow-list: without one, an authorizer could set any header the
    /// upstream trusts, which turns an authorization service into an
    /// impersonation service.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_upstream_headers: Vec<String>,

    /// Whether to serve the request when the authorizer cannot be reached.
    ///
    /// Defaults to `false`. An authorization service that is down must not
    /// become an open door, so this has to be opted into deliberately.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fail_open: Option<bool>,

    /// Bytes of the request body to forward. Accepted for compatibility;
    /// [`Policies::lint`] reports that this build does not send a body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_body: Option<usize>,
}

/// Agent-to-agent protocol handling.
///
/// Marks a route as carrying A2A traffic, which lets the gateway gate the
/// JSON-RPC methods a caller may invoke and serve a merged agent card for the
/// agents behind it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct A2aPolicy {
    /// Methods callable through this route, as unanchored regexes over the
    /// JSON-RPC method name. Empty allows everything not denied.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_methods: Vec<String>,

    /// Methods refused on this route. Denies win over allows.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny_methods: Vec<String>,

    /// Serve an agent card for the agents behind this route.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_card: Option<AgentCardPolicy>,
}

/// How the gateway presents the agents behind a route.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCardPolicy {
    /// The URL clients should call — the gateway's, not the agents'.
    ///
    /// This is the field that makes a card served through a gateway usable: an
    /// upstream agent advertises its own address, and a client that reads it
    /// verbatim goes around the gateway entirely.
    pub url: String,

    /// Name for the merged card. Defaults to the sole agent's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Description for the merged card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
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

/// One CEL rule in an `mcpAuthorization` policy.
///
/// A bare string is an [`AuthorizationRule::Allow`], which is how upstream's
/// own examples are written; the map forms exist for the other two modes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorizationRule {
    /// Permit the call when this expression is true.
    Allow(String),

    /// Refuse the call when this expression is true.
    ///
    /// Read the warning on [`AuthorizationRule`]'s `deny` arm in the config
    /// docs before reaching for it: an expression that fails to evaluate
    /// counts as false, so a `deny` that errors lets the call through.
    Deny(String),

    /// Refuse the call unless this expression is true.
    ///
    /// The safe way to express "deny": a `require` that fails to evaluate
    /// refuses, where a `deny` that fails to evaluate permits.
    Require(String),
}

impl AuthorizationRule {
    /// The expression text, whichever mode this is.
    pub fn expression(&self) -> &str {
        match self {
            AuthorizationRule::Allow(text)
            | AuthorizationRule::Deny(text)
            | AuthorizationRule::Require(text) => text,
        }
    }
}

/// The wire shape: either a bare string or a single-key map.
///
/// Untagged over a struct of optional fields rather than an externally-tagged
/// enum, because `serde_yaml` 0.9 encodes those as YAML tags (`!allow expr`)
/// and rejects the map form upstream's configs actually use.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum RuleRepr {
    Bare(String),
    Tagged {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        allow: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        deny: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        require: Option<String>,
    },
}

impl<'de> Deserialize<'de> for AuthorizationRule {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        match RuleRepr::deserialize(deserializer)? {
            RuleRepr::Bare(text) => Ok(AuthorizationRule::Allow(text)),
            RuleRepr::Tagged {
                allow,
                deny,
                require,
            } => {
                // Two modes in one rule has no single meaning, and guessing
                // one would quietly enforce something nobody wrote.
                let mut found = [
                    allow.map(AuthorizationRule::Allow),
                    deny.map(AuthorizationRule::Deny),
                    require.map(AuthorizationRule::Require),
                ]
                .into_iter()
                .flatten();

                let first = found.next().ok_or_else(|| {
                    D::Error::custom("a rule needs one of `allow`, `deny` or `require`")
                })?;
                if found.next().is_some() {
                    return Err(D::Error::custom(
                        "a rule names only one of `allow`, `deny` or `require`",
                    ));
                }
                Ok(first)
            }
        }
    }
}

impl Serialize for AuthorizationRule {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let repr = match self {
            AuthorizationRule::Allow(text) => RuleRepr::Bare(text.clone()),
            AuthorizationRule::Deny(text) => RuleRepr::Tagged {
                allow: None,
                deny: Some(text.clone()),
                require: None,
            },
            AuthorizationRule::Require(text) => RuleRepr::Tagged {
                allow: None,
                deny: None,
                require: Some(text.clone()),
            },
        };
        repr.serialize(serializer)
    }
}

/// Tool-level authorization for MCP backends.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpAuthorization {
    /// CEL expressions evaluated against each tool call.
    ///
    /// The context is `mcp.tool.name`, `mcp.tool.target` and `jwt`, the
    /// verified token's claims.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<AuthorizationRule>,

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
        if ms.is_multiple_of(3_600_000) {
            write!(f, "{}h", ms / 3_600_000)
        } else if ms.is_multiple_of(60_000) {
            write!(f, "{}m", ms / 60_000)
        } else if ms.is_multiple_of(1_000) {
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
