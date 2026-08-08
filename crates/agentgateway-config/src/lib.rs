//! Configuration model for rusty_agent_gateway.
//!
//! The types here are deliberately wire-compatible with [agentgateway]'s local
//! configuration file, so an existing `config.yaml` parses unmodified. The
//! shape is `binds` → `listeners` → `routes` → `backends`, with `policies`
//! attachable at the route level:
//!
//! ```yaml
//! binds:
//!   - port: 3000
//!     listeners:
//!       - routes:
//!           - policies:
//!               cors:
//!                 allowOrigins: ["*"]
//!             backends:
//!               - mcp:
//!                   targets:
//!                     - name: everything
//!                       stdio:
//!                         cmd: npx
//!                         args: ["@modelcontextprotocol/server-everything"]
//! ```
//!
//! Unknown fields are accepted rather than rejected: upstream adds fields
//! faster than we implement them, and refusing to boot on a field we merely do
//! not support yet would make this useless as a drop-in. [`Config::lint`]
//! reports what was understood but unimplemented, so the tolerance is visible
//! rather than silent.
//!
//! [agentgateway]: https://agentgateway.dev

mod oneof;

mod backend;
mod matcher;
mod methods;
mod policy;

pub use backend::{
    AiBackend, AiProvider, AiProviderParams, Backend, BackendTarget, FilterAction, McpBackend,
    McpTarget, McpTargetKind, NameMode, ServiceRef, SseTarget, StdioTarget, StreamableHttpTarget,
    ToolFilter,
};
pub use matcher::{
    HeaderMatch, HeaderMatchValue, PathMatch, QueryMatch, QueryMatchValue, RouteMatch,
};
pub use methods::{MCP_SERVED_METHODS, pattern_is_matchable, resolve};
pub use policy::{
    A2aPolicy, AgentCardPolicy, AiPolicy, AuthorizationRule, BackendAuth, Builtin, CorsPolicy,
    DurationString, ExtAuthzPolicy, FailureMode, GuardAction, GuardPattern, GuardRule,
    GuardWebhook, HeaderFilter, HeaderModifier, JwtAuth, JwtSource, LocalRateLimit,
    McpAuthorization, McpGuardrails, Moderation, ModerationPolicies, PathRewrite, Phase, Policies,
    Processor, PromptCaching, PromptGuard, PromptMessage, Prompts, RateLimitKind, RegexGuard,
    Rejection, RetryPolicy, TimeoutPolicy, UrlRewrite, WebhookTarget,
};

use std::path::Path;

use serde::{Deserialize, Serialize};

/// Failure to load or validate a configuration file.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file could not be read.
    #[error("reading {path}: {source}")]
    Io {
        /// Path we tried to read.
        path: String,
        /// Underlying I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// The file was not valid YAML, or did not match the schema.
    #[error("parsing {path}: {source}")]
    Parse {
        /// Path we tried to parse.
        path: String,
        /// Underlying deserialization failure.
        #[source]
        source: serde_yaml::Error,
    },
    /// The document parsed but describes something we cannot serve.
    #[error("invalid config: {0}")]
    Invalid(String),
}

/// A whole gateway configuration file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    /// Socket bindings, each owning a port and a set of listeners.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub binds: Vec<Bind>,

    /// Workload identity and telemetry settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<GlobalConfig>,
}

impl Config {
    /// Parse a configuration from a YAML or JSON string.
    ///
    /// JSON is a subset of YAML 1.2, so both go through the same parser.
    pub fn from_yaml(source: &str) -> Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(source)
    }

    /// Load and validate a configuration file from disk.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let display = path.display().to_string();
        let raw = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
            path: display.clone(),
            source,
        })?;
        let config = Self::from_yaml(&raw).map_err(|source| ConfigError::Parse {
            path: display,
            source,
        })?;
        config.validate()?;
        Ok(config)
    }

    /// Check the invariants the router relies on.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.binds.is_empty() {
            return Err(ConfigError::Invalid("no binds configured".into()));
        }
        for bind in &self.binds {
            if bind.port == 0 {
                return Err(ConfigError::Invalid("bind port must be non-zero".into()));
            }
            for listener in &bind.listeners {
                for route in &listener.routes {
                    route.validate()?;
                }
            }
        }
        Ok(())
    }

    /// Report configuration we parsed but do not act on yet.
    ///
    /// Deserialization is deliberately permissive so upstream files load; this
    /// is how that permissiveness stays honest. The gateway logs each entry at
    /// startup rather than pretending full coverage.
    pub fn lint(&self) -> Vec<String> {
        let mut findings = Vec::new();
        for (b, bind) in self.binds.iter().enumerate() {
            for (l, listener) in bind.listeners.iter().enumerate() {
                let at = format!("binds[{b}].listeners[{l}]");
                if listener.protocol == Protocol::Tls {
                    findings.push(format!(
                        "{at}: protocol TLS is terminated as HTTPS by this build; opaque TLS \
                         passthrough is not implemented"
                    ));
                }
                for (r, route) in listener.routes.iter().enumerate() {
                    route.lint(&format!("{at}.routes[{r}]"), &mut findings);
                }
            }
        }
        findings
    }
}

/// Global, non-per-listener settings.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalConfig {
    /// Address the admin/metrics server listens on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admin_addr: Option<String>,
    /// Address the Prometheus scrape endpoint listens on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats_addr: Option<String>,
    /// Logging configuration, as an `RUST_LOG`-style filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logging: Option<LoggingConfig>,

    /// OpenTelemetry export. An extension to upstream's schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracing: Option<TracingConfig>,

    /// Process-wide load shedding. An extension to upstream's schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<LimitsConfig>,
}

/// OpenTelemetry export settings.
///
/// Off unless present: an OTLP exporter pointed at nothing retries in the
/// background forever, which is a strange default to inflict on someone who
/// only wanted a gateway.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TracingConfig {
    /// OTLP/gRPC endpoint. Falls back to `OTEL_EXPORTER_OTLP_ENDPOINT`, then
    /// to the OpenTelemetry default of `http://localhost:4317`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,

    /// `service.name` on every span. The first thing anyone filters by.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_name: Option<String>,

    /// `service.version`, if releases should be distinguishable in traces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_version: Option<String>,

    /// Fraction of root traces to record, 0.0 to 1.0. Defaults to all.
    ///
    /// Only root traces: a caller that already sampled the trace is followed,
    /// because deciding independently is how traces end up half-recorded with
    /// gaps exactly where a service made its own choice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_ratio: Option<f64>,

    /// Whether to export metrics alongside spans. Defaults to true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metrics: Option<bool>,
}

/// Process-wide load shedding.
///
/// Both settings are off unless given. There is no value that is right for
/// everyone — a default of 100 concurrent requests would be a silent
/// regression for a gateway serving more than that today.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitsConfig {
    /// Requests in flight before the gateway sheds with `503`.
    ///
    /// Shedding, not queueing: a queue in front of an overloaded gateway turns
    /// a capacity problem into a latency problem, where every client waits
    /// longer, times out and retries — which is how a brief spike becomes a
    /// sustained outage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrent_requests: Option<usize>,

    /// Default budget for producing a response, when a route sets none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_timeout: Option<DurationString>,
}

/// Logging configuration.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoggingConfig {
    /// `tracing-subscriber` env filter, e.g. `info,agentgateway=debug`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
}

/// One listening socket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bind {
    /// TCP port to listen on.
    pub port: u16,

    /// Listeners multiplexed onto this port, selected by hostname.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub listeners: Vec<Listener>,
}

/// A server certificate and its private key, as PEM file paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TlsConfig {
    /// Path to the certificate chain, leaf first.
    pub cert: String,
    /// Path to the private key. PKCS#8, PKCS#1 and SEC1 are all accepted.
    pub key: String,
}

/// A protocol-specific listener on a [`Bind`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Listener {
    /// Optional name, used in logs and for route attachment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Hostname this listener serves. `None` matches any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,

    /// Wire protocol spoken on this listener.
    #[serde(default)]
    pub protocol: Protocol,

    /// Certificate and key, required when `protocol` terminates TLS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls: Option<TlsConfig>,

    /// Routes evaluated in order for requests arriving here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<Route>,
}

/// The wire protocol a [`Listener`] speaks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Protocol {
    /// Cleartext HTTP/1.1 and h2c.
    #[default]
    Http,
    /// HTTP over TLS.
    Https,
    /// Opaque TLS passthrough.
    Tls,
    /// Opaque TCP passthrough.
    Tcp,
    /// Istio HBONE.
    Hbone,
}

impl Protocol {
    /// Whether serving this protocol requires terminating or inspecting TLS.
    pub fn is_tls(self) -> bool {
        matches!(self, Protocol::Https | Protocol::Tls)
    }
}

/// A routing rule: match a request, apply policies, pick a backend.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Route {
    /// Optional name, used in logs and metrics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Hostnames this route serves. Empty matches any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hostnames: Vec<String>,

    /// Request predicates. Empty matches every request.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matches: Vec<RouteMatch>,

    /// Policies applied to requests taking this route.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policies: Option<Policies>,

    /// Weighted destinations for matched requests.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub backends: Vec<Backend>,
}

impl Route {
    fn validate(&self) -> Result<(), ConfigError> {
        for m in &self.matches {
            m.validate()?;
        }
        if self.backends.len() > 1 && self.backends.iter().any(|b| b.target.is_mcp()) {
            return Err(ConfigError::Invalid(
                "an MCP backend cannot be weighted against other backends on the same route; \
                 use a single mcp backend with multiple targets to federate"
                    .into(),
            ));
        }
        Ok(())
    }

    /// Report a `urlRewrite` that cannot act on this route's backends.
    ///
    /// An `mcp` backend terminates the protocol rather than forwarding a
    /// request line, so `urlRewrite` cannot mean here what it means on a
    /// `host` route. What it *can* mean is replacing parts of the one address
    /// the gateway dials: `path.full` and `authority` both do, and both only
    /// where there is a single Streamable HTTP target to be unambiguous about.
    fn lint_url_rewrite(&self, policies: &Policies, at: &str, findings: &mut Vec<String>) {
        let Some(rewrite) = &policies.url_rewrite else {
            return;
        };
        let mcp: Vec<&McpBackend> = self
            .backends
            .iter()
            .filter_map(|b| match &b.target {
                BackendTarget::Mcp(backend) => Some(backend),
                _ => None,
            })
            .collect();
        // A `host` backend rewrites per request and knows what that request
        // matched. `mcp` and `ai` both resolve one address at startup, before
        // any request exists, so both face the `prefix` question below.
        let resolves_at_startup = !mcp.is_empty()
            || self
                .backends
                .iter()
                .any(|b| matches!(b.target, BackendTarget::Ai(_)));
        if !resolves_at_startup {
            return;
        }

        let at = format!("{at}.policies.urlRewrite");

        // A `prefix` rewrite replaces whatever the route matched. Which prefix
        // a request matched is not knowable when the address is resolved --
        // unless the route offers exactly one.
        if matches!(rewrite.path, Some(PathRewrite::Prefix(_))) {
            let prefixes = self
                .matches
                .iter()
                .filter(|m| matches!(m.path, Some(PathMatch::PathPrefix(_))))
                .count();
            if prefixes != 1 {
                findings.push(format!(
                    "{at}.path.prefix: replacing the matched prefix needs the route to match \
                     on exactly one `pathPrefix`, and this one matches on {prefixes}; use \
                     `full` to set the upstream path outright"
                ));
            }
        }

        // The rest is about MCP targets specifically; an `ai` route has one
        // endpoint and no `stdio` variant to worry about.
        if mcp.is_empty() {
            return;
        }
        let targets: Vec<&McpTarget> = mcp.iter().flat_map(|b| &b.targets).collect();
        let addressable = targets
            .iter()
            .filter(|t| matches!(t.kind, McpTargetKind::Mcp(_)))
            .count();

        // A path rewrite transforms each target's own path and leaves its host
        // alone, so it generalises across a federation. An authority does not:
        // pointed at several targets it would make them all the same server,
        // and a target's address is what distinguishes it from the others.
        if rewrite.authority.is_some() && targets.len() != 1 {
            findings.push(format!(
                "{at}.authority: replacing the address of {} targets would point them all at \
                 one server rather than redirecting them, so it applies only to a federation \
                 with a single target",
                targets.len()
            ));
        }

        if rewrite.path.is_some() && addressable == 0 {
            findings.push(format!(
                "{at}.path: no target here has a path to rewrite; a `stdio` target speaks \
                 over a pipe"
            ));
        }
        if rewrite.authority.is_some() && targets.len() == 1 && addressable == 0 {
            findings.push(format!(
                "{at}.authority: only an `mcp:` target has an address to override; a `stdio` \
                 target speaks over a pipe"
            ));
        }
    }

    /// Report a `via` that would leave two targets indistinguishable.
    ///
    /// Collapsing a federation onto one address is the point of the field, but
    /// what tells the targets apart afterwards is only their paths. Two that
    /// end up at the same address *and* path are two connections to the same
    /// endpoint, federating the same tools twice.
    fn lint_via(&self, at: &str, findings: &mut Vec<String>) {
        for (i, backend) in self.backends.iter().enumerate() {
            let BackendTarget::Mcp(mcp) = &backend.target else {
                continue;
            };
            let Some(via) = &mcp.via else {
                continue;
            };
            let at = format!("{at}.backends[{i}].mcp.via");

            let addressed: Vec<(&str, u16, &str)> = mcp
                .targets
                .iter()
                .filter_map(|t| match &t.kind {
                    McpTargetKind::Mcp(http) => {
                        Some((t.name.as_str(), http.port, http.path.as_str()))
                    }
                    _ => None,
                })
                .collect();

            if addressed.is_empty() {
                findings.push(format!(
                    "{at}: no target here has an address to replace; a `stdio` target speaks \
                     over a pipe"
                ));
                continue;
            }

            // Which port a target keeps depends on whether `via` names one, so
            // the collision test has to ask the same question the dialler will.
            let shared_port = via
                .rsplit_once(':')
                .and_then(|(_, p)| p.parse::<u16>().ok());
            let mut seen: std::collections::BTreeMap<(u16, &str), &str> = Default::default();
            for (name, port, path) in addressed {
                let port = shared_port.unwrap_or(port);
                if let Some(first) = seen.insert((port, path), name) {
                    findings.push(format!(
                        "{at}: targets `{first}` and `{name}` would both be dialled at \
                         `{via}` port {port} path `{path}`, so they are the same endpoint \
                         federated twice; only their paths tell them apart once collapsed"
                    ));
                }
            }
        }
    }

    /// Report a `retry` on a route that terminates MCP.
    ///
    /// Retry applies wherever the gateway makes an HTTP request it could make
    /// again — a proxied upstream, an `a2a` agent, an `ai` provider. An MCP
    /// route makes no such request: it holds a session and sends a JSON-RPC
    /// message over it.
    ///
    /// Two things follow, and both are reasons not to quietly invent a meaning
    /// for the field. `codes` are HTTP statuses, and there is no HTTP response
    /// at that layer to read one from. And the safety rule the other paths rely
    /// on — only a *connect* failure is known never to have reached the
    /// upstream — has no equivalent: a transport error on an established
    /// session covers both "never sent" and "sent, reply lost". Replaying a
    /// `tools/call` under that ambiguity is how a tool whose whole purpose is a
    /// side effect performs it twice.
    fn lint_retry(&self, policies: &Policies, at: &str, findings: &mut Vec<String>) {
        if policies.retry.is_none() {
            return;
        }
        if !self.backends.iter().any(|b| b.target.is_mcp()) {
            return;
        }
        findings.push(format!(
            "{at}.policies.retry: an `mcp` backend holds a session rather than making a \
             request it could make again, so `codes` names statuses nothing here returns, and \
             replaying a `tools/call` after an ambiguous transport error would run the tool \
             twice; it is not applied"
        ));
    }

    /// Report a `type: tokens` limit on a route with nothing to count.
    ///
    /// A token bucket of this kind is charged the token count a model provider
    /// reports, which only an `ai` backend ever sees. Anywhere else the limit
    /// would never be charged, so it would sit at full capacity and refuse
    /// nothing — a rate limit that looks like protection and is not.
    fn lint_token_rate_limit(&self, policies: &Policies, at: &str, findings: &mut Vec<String>) {
        let tokens = policies
            .local_rate_limit
            .iter()
            .any(|limit| limit.kind == RateLimitKind::Tokens);
        if !tokens {
            return;
        }
        if self
            .backends
            .iter()
            .any(|b| matches!(b.target, BackendTarget::Ai(_)))
        {
            return;
        }
        findings.push(format!(
            "{at}.policies.localRateLimit[type=tokens]: only an `ai` backend reports a token \
             count to charge, so this bucket would never be spent; use `type: requests` to \
             limit this route"
        ));
    }

    fn lint(&self, at: &str, findings: &mut Vec<String>) {
        if let Some(policies) = &self.policies {
            policies.lint(at, findings);
            self.lint_url_rewrite(policies, at, findings);
            self.lint_retry(policies, at, findings);
            self.lint_token_rate_limit(policies, at, findings);
        }
        self.lint_via(at, findings);
        for (i, backend) in self.backends.iter().enumerate() {
            backend.lint(&format!("{at}.backends[{i}]"), findings);
        }
    }
}

#[cfg(test)]
mod tests;
