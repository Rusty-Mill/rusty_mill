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

    /// LLM-specific request shaping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai: Option<AiPolicy>,
}

impl Policies {
    pub(crate) fn lint(&self, at: &str, findings: &mut Vec<String>) {
        if let Some(ai) = &self.ai {
            ai.lint(&format!("{at}.policies.ai"), findings);
        }

        if let Some(guardrails) = &self.mcp_guardrails {
            guardrails.lint(&format!("{at}.policies.mcpGuardrails"), findings);
        }
    }
}

/// LLM-specific request shaping.
///
/// Everything upstream accepts here parses, and [`AiPolicy::lint`] names the
/// sub-policies this build does not act on **one at a time**. A single finding
/// for the whole of `ai` was accurate while none of it was implemented and
/// would be a lie now: an operator who reads "not enforced" and sees their
/// `prompts` working has no idea what else is silently ignored.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiPolicy {
    /// Messages added to every conversation on this route.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompts: Option<Prompts>,

    /// Request fields used when the caller did not set them.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub defaults: BTreeMap<String, serde_json::Value>,

    /// Request fields set whatever the caller asked for.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub overrides: BTreeMap<String, serde_json::Value>,

    /// Names a caller may use, mapped to the model actually requested.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub model_aliases: BTreeMap<String, String>,

    /// Where to place provider cache breakpoints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_caching: Option<PromptCaching>,

    /// Content rules applied to the prompt and to what comes back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_guard: Option<PromptGuard>,

    /// Everything else upstream accepts.
    ///
    /// Kept so an upstream config loads, and so the lint can name the exact
    /// sub-policy rather than the whole of `ai`.
    #[serde(flatten)]
    pub rest: BTreeMap<String, serde_json::Value>,
}

impl AiPolicy {
    pub(crate) fn lint(&self, at: &str, findings: &mut Vec<String>) {
        for key in self.rest.keys() {
            findings.push(format!("{at}.{key}: parsed but not enforced by this build"));
        }
        if let Some(guard) = &self.prompt_guard {
            guard.lint(&format!("{at}.promptGuard"), findings);
        }
    }
}

/// Content rules applied to the prompt and to what comes back.
///
/// Each phase is a list because a route usually wants several unrelated rules
/// — one for credentials, one for personal data — and each carries its own
/// refusal. The first rule to refuse ends the request.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptGuard {
    /// Rules applied to the prompt before it is sent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub request: Vec<GuardRule>,

    /// Rules applied to what the provider answered.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub response: Vec<GuardRule>,
}

impl PromptGuard {
    pub(crate) fn lint(&self, at: &str, findings: &mut Vec<String>) {
        for (phase, rules) in [("request", &self.request), ("response", &self.response)] {
            for (i, rule) in rules.iter().enumerate() {
                rule.lint(&format!("{at}.{phase}[{i}]"), findings);
            }
        }
    }
}

/// One rule in a `promptGuard` phase.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardRule {
    /// Patterns matched against the text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regex: Option<RegexGuard>,

    /// An external service asked about the text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook: Option<GuardWebhook>,

    /// What to answer with when this rule refuses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejection: Option<Rejection>,

    /// Other rule kinds upstream accepts, such as `openAIModeration`.
    #[serde(flatten)]
    pub rest: BTreeMap<String, serde_json::Value>,
}

/// An external service asked whether text may pass.
///
/// Where `regex` decides from a pattern written down in advance, this asks
/// something that can change its mind — a classifier, a policy service, a
/// model. The wire contract is upstream's: `POST /request` and `POST
/// /response`, each answered with one of pass, mask or reject.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardWebhook {
    /// Where to send it.
    pub target: WebhookTarget,

    /// Headers computed from CEL and set on the webhook request.
    ///
    /// Keys may be header names or the `:path`, `:method` and `:authority`
    /// pseudo-headers; setting `:path` replaces the default `/request` and
    /// `/response`. Expressions read the *client's* request, so `request.*`
    /// and `jwt.*` mean what the caller sent rather than what is being built.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,

    /// Which of the caller's own headers travel to the webhook.
    ///
    /// Empty forwards none. That is the opposite of `mcpGuardrails`, and
    /// deliberate: this body already carries the prompt, so a header list is
    /// extra reach rather than the point of the call.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forward_header_matches: Vec<String>,

    /// What to do when the webhook cannot be reached.
    #[serde(default)]
    pub failure_mode: FailureMode,
}

/// Where a guard webhook lives.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookTarget {
    /// `host:port`, as upstream spells it.
    #[serde(default)]
    pub host: String,
}

impl GuardRule {
    pub(crate) fn lint(&self, at: &str, findings: &mut Vec<String>) {
        for key in self.rest.keys() {
            findings.push(format!("{at}.{key}: parsed but not enforced by this build"));
        }
    }
}

/// Patterns matched against the text of a prompt or an answer.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegexGuard {
    /// What to do when a rule matches.
    #[serde(default)]
    pub action: GuardAction,

    /// The patterns. Any one matching is a match.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<GuardPattern>,
}

/// What to do when a `regex` rule matches.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GuardAction {
    /// Refuse, and answer with the rule's `rejection`.
    #[default]
    Reject,
    /// Replace the matched text and carry on.
    Mask,
}

one_of_enum! {
    /// One pattern in a `regex` rule.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum GuardPattern {
        /// A regular expression written by the operator.
        "pattern" => Pattern(String),

        /// One of the shapes this build already knows.
        "builtin" => Builtin(Builtin),
    }
}

/// A pattern this build ships rather than asking an operator to write.
///
/// These are the ones everybody writes the same way and gets subtly wrong the
/// same way, so shipping them beats every config carrying its own attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Builtin {
    /// An email address.
    Email,
    /// A telephone number.
    PhoneNumber,
    /// A US Social Security number.
    Ssn,
    /// A payment card number.
    CreditCard,
    /// A Canadian Social Insurance Number.
    CaSin,
}

/// What to answer with when a guard rule refuses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rejection {
    /// Status to answer with.
    #[serde(default = "default_rejection_status")]
    pub status: u16,

    /// Headers on the refusal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<HeaderModifier>,

    /// The body to answer with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

impl Default for Rejection {
    fn default() -> Self {
        Rejection {
            status: default_rejection_status(),
            headers: None,
            body: None,
        }
    }
}

/// The status a refusal uses when the rule does not name one.
///
/// `400`, not `403`: a content rule decides the request is unacceptable *for
/// this route*, which is what a bad request means. `403` would say the caller
/// lacks permission, sending them to check credentials that are fine.
fn default_rejection_status() -> u16 {
    400
}

/// Messages added to every conversation on a route.
///
/// A system prompt an operator wants on every call, without every client
/// having to send it — and, prepended, one a client cannot drop by not
/// sending it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Prompts {
    /// Messages placed before the caller's own.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prepend: Vec<PromptMessage>,

    /// Messages placed after the caller's own.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub append: Vec<PromptMessage>,
}

/// One message in a `prompts` list.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PromptMessage {
    /// `system`, `user`, `assistant`, or whatever the provider accepts.
    pub role: String,
    /// The message text.
    pub content: String,
}

/// Where to place provider cache breakpoints.
///
/// Only Anthropic takes these explicitly; OpenAI caches long prefixes on its
/// own and needs no configuration, so this is a documented no-op there.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptCaching {
    /// Mark the system prompt cacheable.
    #[serde(default)]
    pub cache_system: bool,

    /// Mark the conversation so far cacheable.
    #[serde(default)]
    pub cache_messages: bool,

    /// Mark the tool definitions cacheable.
    #[serde(default)]
    pub cache_tools: bool,

    /// Skip marking anything when the prompt is shorter than this.
    ///
    /// A provider will not cache a short prefix anyway, and a breakpoint it
    /// ignores costs nothing but noise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_tokens: Option<u64>,

    /// How many messages back from the end to place the message breakpoint.
    ///
    /// `0` marks the last message. A conversation that grows by one turn each
    /// call wants the breakpoint behind the part that changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_message_offset: Option<usize>,
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
                     never runs; served methods are {}",
                    crate::MCP_SERVED_METHODS.join(", ")
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

    /// Bytes of the request body to forward to the authorizer.
    ///
    /// A bound, not a target: a body larger than this is refused rather than
    /// truncated, so the authorizer never decides on a fragment.
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
