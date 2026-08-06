//! Backend destinations.
//!
//! A route names one or more backends. Plain HTTP backends (`host`, `service`)
//! are weighted and load-balanced; an `mcp` backend is different in kind — it
//! terminates the protocol rather than forwarding it, federating a set of
//! upstream MCP targets into one server. [`crate::Route::validate`] rejects
//! mixing the two on a route, because "50% of an MCP session" is not a thing.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::oneof::one_of_enum;

/// One weighted destination on a route.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Backend {
    /// Which kind of destination this is.
    #[serde(flatten)]
    pub target: BackendTarget,

    /// Relative share of traffic among the route's backends.
    #[serde(default = "default_weight")]
    pub weight: u32,
}

fn default_weight() -> u32 {
    1
}

one_of_enum! {
    /// The kinds of destination a route can name.
    #[derive(Debug, Clone, PartialEq)]
    pub enum BackendTarget {
        /// A literal `host:port` upstream, proxied over HTTP.
        "host" => Host(String),

        /// A named service, resolved by the control plane.
        "service" => Service(ServiceRef),

        /// A federated MCP server built from one or more upstream targets.
        "mcp" => Mcp(McpBackend),

        /// An LLM provider, exposed behind an OpenAI-compatible API.
        "ai" => Ai(AiBackend),

        /// A destination chosen per-request by an earlier policy.
        "dynamic" => Dynamic(BTreeMap<String, serde_json::Value>),
    }
}

impl BackendTarget {
    /// Whether this backend terminates MCP rather than forwarding bytes.
    pub fn is_mcp(&self) -> bool {
        matches!(self, BackendTarget::Mcp(_))
    }
}

impl Backend {
    pub(crate) fn lint(&self, at: &str, findings: &mut Vec<String>) {
        match &self.target {
            BackendTarget::Dynamic(_) => findings.push(format!(
                "{at}: `dynamic` backend parsed but dynamic resolution is not implemented yet"
            )),
            BackendTarget::Service(_) => findings.push(format!(
                "{at}: `service` backend parsed but service discovery is not implemented yet; \
                 use `host` for a literal address"
            )),
            BackendTarget::Host(_) | BackendTarget::Mcp(_) | BackendTarget::Ai(_) => {}
        }
    }
}

/// A control-plane service reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceRef {
    /// Service name, optionally `namespace/name`.
    pub name: String,
    /// Service port.
    pub port: u16,
}

/// A federated MCP server assembled from upstream targets.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpBackend {
    /// Upstream MCP servers to federate.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<McpTarget>,

    /// How to disambiguate tool names drawn from several targets.
    #[serde(default)]
    pub name_mode: NameMode,
}

/// How federated tool names are qualified.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NameMode {
    /// Expose tools as `<target>_<tool>`, so two targets can both export
    /// `search` without colliding. The default, and what upstream does.
    #[default]
    Prefix,
    /// Expose tool names unchanged. Only safe when names are known to be
    /// unique across targets; the router reports a collision at startup.
    Passthrough,
}

/// One upstream MCP server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpTarget {
    /// Name used to qualify this target's tools, and in logs.
    pub name: String,

    /// Transport used to reach it.
    #[serde(flatten)]
    pub kind: McpTargetKind,

    /// Optional allow/deny rules over this target's tools.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filters: Vec<ToolFilter>,
}

one_of_enum! {
    /// The transport used to reach an upstream MCP server.
    #[derive(Debug, Clone, PartialEq)]
    pub enum McpTargetKind {
        /// Launch a subprocess and speak newline-delimited JSON-RPC over its
        /// stdio.
        "stdio" => Stdio(StdioTarget),

        /// Speak Streamable HTTP to a remote endpoint.
        "mcp" => Mcp(StreamableHttpTarget),

        /// Speak the deprecated 2024-11-05 HTTP+SSE transport.
        "sse" => Sse(SseTarget),
    }
}

/// A subprocess MCP server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StdioTarget {
    /// Executable to run.
    pub cmd: String,

    /// Arguments passed to it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,

    /// Extra environment variables for the child.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

/// A remote MCP server speaking Streamable HTTP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamableHttpTarget {
    /// Upstream hostname.
    pub host: String,

    /// Upstream port.
    #[serde(default = "default_http_port")]
    pub port: u16,

    /// Path of the MCP endpoint.
    #[serde(default = "default_mcp_path")]
    pub path: String,
}

/// A remote MCP server speaking the deprecated HTTP+SSE transport.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SseTarget {
    /// Upstream hostname.
    pub host: String,

    /// Upstream port.
    #[serde(default = "default_http_port")]
    pub port: u16,

    /// Path of the SSE endpoint.
    #[serde(default = "default_sse_path")]
    pub path: String,
}

fn default_http_port() -> u16 {
    80
}

fn default_mcp_path() -> String {
    "/mcp".into()
}

fn default_sse_path() -> String {
    "/sse".into()
}

/// An allow or deny rule over a target's tools.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolFilter {
    /// Whether matching tools are kept or dropped.
    pub action: FilterAction,

    /// Unanchored regular expression over the unqualified tool name.
    pub matcher: String,
}

/// What a [`ToolFilter`] does with the tools it matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FilterAction {
    /// Keep only tools matching this rule.
    Allow,
    /// Drop tools matching this rule.
    Deny,
}

/// An LLM provider backend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiBackend {
    /// Which provider to route to.
    pub provider: AiProvider,
}

one_of_enum! {
    /// The supported LLM providers.
    #[derive(Debug, Clone, PartialEq)]
    pub enum AiProvider {
        /// OpenAI or an OpenAI-compatible endpoint. Spelled `openAI` upstream,
        /// which is why the key is written out rather than derived.
        "openAI" => OpenAi(AiProviderParams),
        /// Anthropic.
        "anthropic" => Anthropic(AiProviderParams),
        /// Google Gemini.
        "gemini" => Gemini(AiProviderParams),
        /// Google Vertex AI.
        "vertex" => Vertex(AiProviderParams),
        /// AWS Bedrock.
        "bedrock" => Bedrock(AiProviderParams),
    }
}

/// Provider-specific settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderParams {
    /// Model to route to, overriding whatever the request asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Override for the provider's base URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_override: Option<String>,
}
