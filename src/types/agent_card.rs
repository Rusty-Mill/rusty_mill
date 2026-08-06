//! The Agent Card: an agent's self-describing manifest (spec Section 4.4 /
//! 8, proto `AgentCard` and friends).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::security::{SecurityRequirement, SecurityScheme};

/// A target URL, transport and protocol version for interacting with an
/// agent. An `AgentCard` may declare several, letting the agent expose the
/// same functionality over multiple protocol bindings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInterface {
    pub url: String,
    /// The protocol binding at this URL. The core officially-supported
    /// values are `"JSONRPC"`, `"GRPC"` and `"HTTP+JSON"`; this crate
    /// currently implements the `JSONRPC` binding.
    #[serde(rename = "protocolBinding")]
    pub protocol_binding: String,
    /// Opaque routing identifier for multi-tenant deployments. When set,
    /// clients MUST echo it in the `tenant` field of every request sent to
    /// this interface.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tenant: Option<String>,
    /// The `Major.Minor` A2A protocol version this interface exposes, e.g.
    /// `"1.0"`.
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
}

impl AgentInterface {
    pub const JSONRPC: &'static str = "JSONRPC";

    pub fn json_rpc(url: impl Into<String>) -> Self {
        AgentInterface {
            url: url.into(),
            protocol_binding: Self::JSONRPC.to_string(),
            tenant: None,
            protocol_version: crate::PROTOCOL_VERSION.to_string(),
        }
    }
}

/// The service provider of an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProvider {
    pub url: String,
    pub organization: String,
}

/// Optional capabilities supported by an agent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentCapabilities {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub streaming: Option<bool>,
    #[serde(
        rename = "pushNotifications",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub push_notifications: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub extensions: Vec<AgentExtension>,
    #[serde(
        rename = "extendedAgentCard",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub extended_agent_card: Option<bool>,
}

/// A declaration of a protocol extension supported by an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentExtension {
    pub uri: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub params: Option<Map<String, Value>>,
}

/// A distinct capability or function an agent can perform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSkill {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub examples: Vec<String>,
    #[serde(rename = "inputModes", skip_serializing_if = "Vec::is_empty", default)]
    pub input_modes: Vec<String>,
    #[serde(rename = "outputModes", skip_serializing_if = "Vec::is_empty", default)]
    pub output_modes: Vec<String>,
    #[serde(
        rename = "securityRequirements",
        skip_serializing_if = "Vec::is_empty",
        default
    )]
    pub security_requirements: Vec<SecurityRequirement>,
}

impl AgentSkill {
    pub fn new(id: impl Into<String>, name: impl Into<String>, description: impl Into<String>) -> Self {
        AgentSkill {
            id: id.into(),
            name: name.into(),
            description: description.into(),
            tags: Vec::new(),
            examples: Vec::new(),
            input_modes: Vec::new(),
            output_modes: Vec::new(),
            security_requirements: Vec::new(),
        }
    }

    pub fn with_tags(mut self, tags: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.tags = tags.into_iter().map(Into::into).collect();
        self
    }
}

/// A JSON Web Signature (RFC 7515) over an `AgentCard`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCardSignature {
    /// Base64url-encoded protected JWS header.
    pub protected: String,
    /// Base64url-encoded signature.
    pub signature: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub header: Option<Map<String, Value>>,
}

/// A self-describing manifest for an agent: identity, capabilities,
/// skills, supported communication methods, and security requirements
/// (spec Section 4.4.1 / 8).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCard {
    pub name: String,
    pub description: String,
    /// Ordered list of supported interfaces; the first entry is preferred.
    #[serde(rename = "supportedInterfaces")]
    pub supported_interfaces: Vec<AgentInterface>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub provider: Option<AgentProvider>,
    pub version: String,
    #[serde(rename = "documentationUrl", skip_serializing_if = "Option::is_none", default)]
    pub documentation_url: Option<String>,
    pub capabilities: AgentCapabilities,
    #[serde(
        rename = "securitySchemes",
        skip_serializing_if = "HashMap::is_empty",
        default
    )]
    pub security_schemes: HashMap<String, SecurityScheme>,
    #[serde(
        rename = "securityRequirements",
        skip_serializing_if = "Vec::is_empty",
        default
    )]
    pub security_requirements: Vec<SecurityRequirement>,
    #[serde(rename = "defaultInputModes")]
    pub default_input_modes: Vec<String>,
    #[serde(rename = "defaultOutputModes")]
    pub default_output_modes: Vec<String>,
    pub skills: Vec<AgentSkill>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub signatures: Vec<AgentCardSignature>,
    #[serde(rename = "iconUrl", skip_serializing_if = "Option::is_none", default)]
    pub icon_url: Option<String>,
}

impl AgentCard {
    /// Minimal builder for a single-interface (JSON-RPC), non-streaming
    /// agent card. Use the setter methods to declare capabilities and
    /// skills.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        version: impl Into<String>,
        interface: AgentInterface,
    ) -> Self {
        AgentCard {
            name: name.into(),
            description: description.into(),
            supported_interfaces: vec![interface],
            provider: None,
            version: version.into(),
            documentation_url: None,
            capabilities: AgentCapabilities::default(),
            security_schemes: HashMap::new(),
            security_requirements: Vec::new(),
            default_input_modes: vec!["text/plain".to_string()],
            default_output_modes: vec!["text/plain".to_string()],
            skills: Vec::new(),
            signatures: Vec::new(),
            icon_url: None,
        }
    }

    pub fn with_skill(mut self, skill: AgentSkill) -> Self {
        self.skills.push(skill);
        self
    }

    pub fn with_streaming(mut self, streaming: bool) -> Self {
        self.capabilities.streaming = Some(streaming);
        self
    }

    pub fn with_push_notifications(mut self, supported: bool) -> Self {
        self.capabilities.push_notifications = Some(supported);
        self
    }

    /// The preferred (first) interface, if any is declared.
    pub fn preferred_interface(&self) -> Option<&AgentInterface> {
        self.supported_interfaces.first()
    }

    /// The first declared interface using the given `protocolBinding`
    /// value (case-sensitive, per spec).
    pub fn interface_for_binding(&self, binding: &str) -> Option<&AgentInterface> {
        self.supported_interfaces
            .iter()
            .find(|i| i.protocol_binding == binding)
    }
}
