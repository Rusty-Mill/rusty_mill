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
mod policy;

pub use backend::{
    AiBackend, AiProvider, AiProviderParams, Backend, BackendTarget, FilterAction, McpBackend,
    McpTarget, McpTargetKind, NameMode, ServiceRef, SseTarget, StdioTarget, StreamableHttpTarget,
    ToolFilter,
};
pub use matcher::{
    HeaderMatch, HeaderMatchValue, PathMatch, QueryMatch, QueryMatchValue, RouteMatch,
};
pub use policy::{
    BackendAuth, CorsPolicy, DurationString, HeaderModifier, JwtAuth, JwtSource, LocalRateLimit,
    McpAuthorization, PathRewrite, Policies, RateLimitKind, RetryPolicy, TimeoutPolicy, UrlRewrite,
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

    fn lint(&self, at: &str, findings: &mut Vec<String>) {
        if let Some(policies) = &self.policies {
            policies.lint(at, findings);
        }
        for (i, backend) in self.backends.iter().enumerate() {
            backend.lint(&format!("{at}.backends[{i}]"), findings);
        }
    }
}

#[cfg(test)]
mod tests;
