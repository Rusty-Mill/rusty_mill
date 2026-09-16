//! Agent identity and discovery: [`AgentName`], [`AgentManifest`] and its metadata.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};

use crate::types::error::Error;

/// A unique agent identifier following the RFC 1123 DNS label convention.
///
/// Lowercase alphanumerics and `-`, must start and end with an alphanumeric,
/// 1 to 63 characters.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct AgentName(String);

impl AgentName {
    /// Maximum length of an agent name.
    pub const MAX_LEN: usize = 63;

    /// Validate and wrap an agent name.
    pub fn new(name: impl Into<String>) -> Result<Self, Error> {
        let name = name.into();
        Self::validate(&name)?;
        Ok(Self(name))
    }

    /// Check a string against the RFC 1123 DNS label rules used by ACP.
    pub fn validate(name: &str) -> Result<(), Error> {
        let invalid = |reason: &str| {
            Err(Error::invalid_input(format!("invalid agent name {name:?}: {reason}")))
        };
        if name.is_empty() {
            return invalid("must not be empty");
        }
        if name.len() > Self::MAX_LEN {
            return invalid(&format!("must be at most {} characters", Self::MAX_LEN));
        }
        if !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
            return invalid("may only contain lowercase alphanumerics and `-`");
        }
        let starts_ok = name.chars().next().is_some_and(|c| c.is_ascii_alphanumeric());
        let ends_ok = name.chars().next_back().is_some_and(|c| c.is_ascii_alphanumeric());
        if !starts_ok || !ends_ok {
            return invalid("must start and end with an alphanumeric character");
        }
        Ok(())
    }

    /// The name as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume the wrapper, yielding the inner [`String`].
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl std::fmt::Display for AgentName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for AgentName {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        AgentName::new(s)
    }
}

impl AsRef<str> for AgentName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::borrow::Borrow<str> for AgentName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for AgentName {
    type Error = Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        AgentName::new(value)
    }
}

impl TryFrom<&str> for AgentName {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        AgentName::new(value)
    }
}

impl<'de> Deserialize<'de> for AgentName {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        AgentName::new(raw).map_err(serde::de::Error::custom)
    }
}

/// The manifest describing an agent, returned by discovery endpoints.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentManifest {
    /// Unique agent identifier.
    pub name: AgentName,
    /// Human-readable description of what the agent does.
    pub description: String,
    /// MIME content types the agent accepts on input.
    pub input_content_types: Vec<String>,
    /// MIME content types the agent may produce on output.
    pub output_content_types: Vec<String>,
    /// Static details for discovery, classification and cataloging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
    /// Real-time metrics reported by the hosting system.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
}

impl AgentManifest {
    /// A manifest accepting and producing `text/plain`.
    pub fn new(name: AgentName, description: impl Into<String>) -> Self {
        Self {
            name,
            description: description.into(),
            input_content_types: vec![super::message::DEFAULT_CONTENT_TYPE.to_string()],
            output_content_types: vec![super::message::DEFAULT_CONTENT_TYPE.to_string()],
            metadata: None,
            status: None,
        }
    }

    /// Replace the accepted input content types.
    pub fn with_input_content_types<S: Into<String>>(
        mut self,
        types: impl IntoIterator<Item = S>,
    ) -> Self {
        self.input_content_types = types.into_iter().map(Into::into).collect();
        self
    }

    /// Replace the produced output content types.
    pub fn with_output_content_types<S: Into<String>>(
        mut self,
        types: impl IntoIterator<Item = S>,
    ) -> Self {
        self.output_content_types = types.into_iter().map(Into::into).collect();
        self
    }

    /// Attach discovery metadata.
    pub fn with_metadata(mut self, metadata: Metadata) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Attach runtime status metrics.
    pub fn with_status(mut self, status: Status) -> Self {
        self.status = Some(status);
        self
    }

    /// Check the `minItems: 1` constraints on the content type lists.
    pub fn validate(&self) -> Result<(), Error> {
        if self.input_content_types.is_empty() {
            return Err(Error::invalid_input(format!(
                "agent {}: `input_content_types` must list at least one MIME type",
                self.name
            )));
        }
        if self.output_content_types.is_empty() {
            return Err(Error::invalid_input(format!(
                "agent {}: `output_content_types` must list at least one MIME type",
                self.name
            )));
        }
        Ok(())
    }

    /// Whether `content_type` is matched by the agent's accepted input types,
    /// honouring `*/*` and `type/*` wildcards.
    pub fn accepts_input(&self, content_type: &str) -> bool {
        self.input_content_types.iter().any(|pattern| content_type_matches(pattern, content_type))
    }

    /// Whether `content_type` is matched by the agent's declared output types.
    pub fn produces_output(&self, content_type: &str) -> bool {
        self.output_content_types.iter().any(|pattern| content_type_matches(pattern, content_type))
    }
}

/// Match a MIME type against a pattern supporting `*/*` and `type/*` wildcards.
pub fn content_type_matches(pattern: &str, content_type: &str) -> bool {
    if pattern == "*/*" || pattern == "*" {
        return true;
    }
    let strip = |value: &str| value.split(';').next().unwrap_or(value).trim().to_ascii_lowercase();
    let (pattern, content_type) = (strip(pattern), strip(content_type));
    if let Some(prefix) = pattern.strip_suffix("/*") {
        return content_type.split('/').next().is_some_and(|actual| actual == prefix);
    }
    pattern == content_type
}

/// Real-time metrics about an agent, supplied by the managing system.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Status {
    /// Average tokens consumed per run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_run_tokens: Option<f64>,
    /// Average wall-clock duration of a run, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_run_time_seconds: Option<f32>,
    /// Percentage of successful runs, 0 to 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success_rate: Option<f64>,
}

/// A classification tag. Well-known values are listed by the specification but
/// any string is permitted.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Tag(pub String);

impl Tag {
    /// Conversational agents.
    pub const CHAT: &'static str = "Chat";
    /// Retrieval-augmented generation agents.
    pub const RAG: &'static str = "RAG";
    /// Canvas agents.
    pub const CANVAS: &'static str = "Canvas";
    /// Code-focused agents.
    pub const CODE: &'static str = "Code";
    /// Research agents.
    pub const RESEARCHER: &'static str = "Researcher";
    /// Agents that orchestrate other agents.
    pub const ORCHESTRATOR: &'static str = "Orchestrator";
}

impl<S: Into<String>> From<S> for Tag {
    fn from(value: S) -> Self {
        Tag(value.into())
    }
}

impl std::fmt::Display for Tag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A named capability the agent exposes, intended to be readable by both humans
/// and language models.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capability {
    /// Human-readable capability name.
    pub name: String,
    /// What the capability provides or enables.
    pub description: String,
}

impl Capability {
    /// Build a capability entry.
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self { name: name.into(), description: description.into() }
    }
}

/// A person credited on an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Person {
    /// Full name.
    pub name: String,
    /// Contact email address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// Personal or organisational URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl Person {
    /// A person with only a name.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), email: None, url: None }
    }
}

/// Kind of resource a [`Link`] points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LinkType {
    /// Source repository.
    SourceCode,
    /// Published container image.
    ContainerImage,
    /// Project homepage.
    Homepage,
    /// Documentation site.
    Documentation,
}

/// A typed link in the agent metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    /// What the link points at.
    #[serde(rename = "type")]
    pub link_type: LinkType,
    /// The URL.
    pub url: String,
}

impl Link {
    /// Build a link.
    pub fn new(link_type: LinkType, url: impl Into<String>) -> Self {
        Self { link_type, url: url.into() }
    }
}

/// Kind of resource an [`AgentDependency`] refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DependencyType {
    /// Another agent.
    Agent,
    /// An external tool.
    Tool,
    /// A specific model.
    Model,
}

/// **Experimental.** An external resource the agent relies on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDependency {
    /// The kind of dependency.
    #[serde(rename = "type")]
    pub dependency_type: DependencyType,
    /// Identifier of the dependency.
    pub name: String,
}

impl AgentDependency {
    /// Build a dependency entry.
    pub fn new(dependency_type: DependencyType, name: impl Into<String>) -> Self {
        Self { dependency_type, name: name.into() }
    }
}

/// Static details about an agent, used for discovery and cataloging.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Metadata {
    /// Free-form key/value annotations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<serde_json::Map<String, serde_json::Value>>,
    /// Full agent documentation in Markdown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
    /// SPDX license identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Implementation language.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub programming_language: Option<String>,
    /// Supported human languages as ISO 639-1 codes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub natural_languages: Option<Vec<String>>,
    /// Agent framework, e.g. `BeeAI` or `crewAI`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub framework: Option<String>,
    /// Capabilities the agent exposes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Vec<Capability>>,
    /// Functional domains the agent applies to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domains: Option<Vec<String>>,
    /// Classification tags.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<Tag>>,
    /// When the agent was first published.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
    /// When the agent was last updated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
    /// Primary author.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Person>,
    /// Additional contributors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contributors: Option<Vec<Person>>,
    /// Related links.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub links: Option<Vec<Link>>,
    /// External resources the agent depends on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependencies: Option<Vec<AgentDependency>>,
    /// Models recommended for running this agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended_models: Option<Vec<String>>,
}

impl Metadata {
    /// An empty metadata block.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the SPDX license identifier.
    pub fn with_license(mut self, license: impl Into<String>) -> Self {
        self.license = Some(license.into());
        self
    }

    /// Set the implementation language.
    pub fn with_programming_language(mut self, language: impl Into<String>) -> Self {
        self.programming_language = Some(language.into());
        self
    }

    /// Set the agent framework.
    pub fn with_framework(mut self, framework: impl Into<String>) -> Self {
        self.framework = Some(framework.into());
        self
    }

    /// Set the Markdown documentation.
    pub fn with_documentation(mut self, documentation: impl Into<String>) -> Self {
        self.documentation = Some(documentation.into());
        self
    }

    /// Set the classification tags.
    pub fn with_tags<T: Into<Tag>>(mut self, tags: impl IntoIterator<Item = T>) -> Self {
        self.tags = Some(tags.into_iter().map(Into::into).collect());
        self
    }

    /// Set the exposed capabilities.
    pub fn with_capabilities(mut self, capabilities: impl IntoIterator<Item = Capability>) -> Self {
        self.capabilities = Some(capabilities.into_iter().collect());
        self
    }

    /// Set the related links.
    pub fn with_links(mut self, links: impl IntoIterator<Item = Link>) -> Self {
        self.links = Some(links.into_iter().collect());
        self
    }

    /// Set the primary author.
    pub fn with_author(mut self, author: Person) -> Self {
        self.author = Some(author);
        self
    }
}

/// Response body of `GET /agents`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentsListResponse {
    /// The page of agent manifests.
    pub agents: Vec<AgentManifest>,
}
