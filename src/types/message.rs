//! Multimodal message types: [`Message`], [`MessagePart`], [`Role`] and part metadata.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::types::error::Error;

/// The default content type applied to a [`MessagePart`] when none is given.
pub const DEFAULT_CONTENT_TYPE: &str = "text/plain";

/// Sender of a [`Message`].
///
/// Serialized as the strings `user`, `agent`, or `agent/{agent_name}` per the
/// specification's `role` pattern.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Role {
    /// A message sent by an end user.
    User,
    /// A message sent by an anonymous agent.
    Agent,
    /// A message sent by the named agent.
    NamedAgent(String),
}

impl Role {
    /// Build the `agent/{name}` role for the given agent name.
    pub fn agent(name: impl Into<String>) -> Self {
        Role::NamedAgent(name.into())
    }

    /// Whether this role denotes an agent (named or anonymous).
    pub fn is_agent(&self) -> bool {
        matches!(self, Role::Agent | Role::NamedAgent(_))
    }

    /// The agent name, if this is a named agent role.
    pub fn agent_name(&self) -> Option<&str> {
        match self {
            Role::NamedAgent(name) => Some(name),
            _ => None,
        }
    }

    /// Parse a role from its wire representation.
    pub fn parse(value: &str) -> Result<Self, Error> {
        match value {
            "user" => Ok(Role::User),
            "agent" => Ok(Role::Agent),
            other => match other.strip_prefix("agent/") {
                Some(name)
                    if !name.is_empty()
                        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') =>
                {
                    Ok(Role::NamedAgent(name.to_string()))
                }
                _ => Err(Error::invalid_input(format!(
                    "invalid message role {value:?}: expected `user`, `agent`, or `agent/{{agent_name}}`"
                ))),
            },
        }
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Role::User => f.write_str("user"),
            Role::Agent => f.write_str("agent"),
            Role::NamedAgent(name) => write!(f, "agent/{name}"),
        }
    }
}

impl std::str::FromStr for Role {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Role::parse(s)
    }
}

impl Serialize for Role {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Role {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Role::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// How the `content` field of a [`MessagePart`] is encoded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContentEncoding {
    /// `content` holds the literal text.
    #[default]
    Plain,
    /// `content` holds standard base64 of the underlying bytes.
    Base64,
}

/// Metadata attached to a [`MessagePart`], discriminated by its `kind`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum PartMetadata {
    /// An inline citation pointing at an information source.
    Citation(CitationMetadata),
    /// A reasoning step or tool invocation in the agent's trajectory.
    Trajectory(TrajectoryMetadata),
}

impl From<CitationMetadata> for PartMetadata {
    fn from(value: CitationMetadata) -> Self {
        PartMetadata::Citation(value)
    }
}

impl From<TrajectoryMetadata> for PartMetadata {
    fn from(value: TrajectoryMetadata) -> Self {
        PartMetadata::Trajectory(value)
    }
}

/// An inline citation.
///
/// `start_index`/`end_index` count characters across all `text/*` parts of the
/// enclosing [`Message`]. When both are absent the citation renders at the
/// position of its own part.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CitationMetadata {
    /// Start of the cited text range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_index: Option<i64>,
    /// End of the cited text range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_index: Option<i64>,
    /// Source URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Source title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Description of the source, or the cited snippet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A reasoning step or tool execution in the agent's trajectory.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrajectoryMetadata {
    /// A reasoning step or thought.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Name of the tool that was executed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// Input parameters passed to the tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_input: Option<serde_json::Value>,
    /// Output returned by the tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_output: Option<serde_json::Value>,
}

fn default_content_type() -> String {
    DEFAULT_CONTENT_TYPE.to_string()
}

/// One part of a [`Message`].
///
/// A part carries a `content_type` plus *either* inline `content` *or* a
/// `content_url`, or neither. Constructing a part with both is invalid; use
/// [`MessagePart::validate`] to check a part received from the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessagePart {
    /// Optional name for the part, e.g. a file name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// MIME type of the content. Defaults to `text/plain`.
    #[serde(default = "default_content_type")]
    pub content_type: String,
    /// Inline content. Mutually exclusive with `content_url`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Encoding of `content`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_encoding: Option<ContentEncoding>,
    /// URL the content can be fetched from. Mutually exclusive with `content`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_url: Option<String>,
    /// Citation or trajectory metadata for this part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<PartMetadata>,
}

impl Default for MessagePart {
    fn default() -> Self {
        Self {
            name: None,
            content_type: default_content_type(),
            content: None,
            content_encoding: None,
            content_url: None,
            metadata: None,
        }
    }
}

impl MessagePart {
    /// A `text/plain` part with the given inline content.
    pub fn text(content: impl Into<String>) -> Self {
        Self { content: Some(content.into()), ..Default::default() }
    }

    /// An inline part with an explicit content type.
    pub fn inline(content_type: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            content_type: content_type.into(),
            content: Some(content.into()),
            ..Default::default()
        }
    }

    /// A part whose content lives at `url`.
    pub fn from_url(content_type: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            content_type: content_type.into(),
            content_url: Some(url.into()),
            ..Default::default()
        }
    }

    /// A part carrying only [`TrajectoryMetadata`] and no content.
    pub fn trajectory(trajectory: TrajectoryMetadata) -> Self {
        Self { metadata: Some(trajectory.into()), ..Default::default() }
    }

    /// Set the part name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the content encoding.
    pub fn with_encoding(mut self, encoding: ContentEncoding) -> Self {
        self.content_encoding = Some(encoding);
        self
    }

    /// Attach metadata to the part.
    pub fn with_metadata(mut self, metadata: impl Into<PartMetadata>) -> Self {
        self.metadata = Some(metadata.into());
        self
    }

    /// The effective content encoding, applying the `plain` default.
    pub fn encoding(&self) -> ContentEncoding {
        self.content_encoding.unwrap_or_default()
    }

    /// Whether the part carries `text/*` content.
    pub fn is_text(&self) -> bool {
        self.content_type.starts_with("text/")
    }

    /// The inline content when it is plain-encoded text.
    pub fn as_text(&self) -> Option<&str> {
        match (self.is_text(), self.encoding(), self.content.as_deref()) {
            (true, ContentEncoding::Plain, Some(content)) => Some(content),
            _ => None,
        }
    }

    /// Check the `content` / `content_url` exclusivity rule.
    pub fn validate(&self) -> Result<(), Error> {
        if self.content.is_some() && self.content_url.is_some() {
            return Err(Error::invalid_input(
                "message part must not set both `content` and `content_url`",
            ));
        }
        if self.content_type.is_empty() {
            return Err(Error::invalid_input("message part `content_type` must not be empty"));
        }
        Ok(())
    }
}

/// An ordered sequence of [`MessagePart`]s attributed to a [`Role`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    /// Who sent the message.
    pub role: Role,
    /// Ordered parts. Must contain at least one entry.
    pub parts: Vec<MessagePart>,
    /// When the message was created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
    /// When the message was completed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
}

impl Message {
    /// Build a message from a role and its parts.
    pub fn new(role: Role, parts: impl IntoIterator<Item = MessagePart>) -> Self {
        Self {
            role,
            parts: parts.into_iter().collect(),
            created_at: Some(Utc::now()),
            completed_at: None,
        }
    }

    /// A single-part `text/plain` message from the user.
    pub fn user(text: impl Into<String>) -> Self {
        Self::new(Role::User, [MessagePart::text(text)])
    }

    /// A single-part `text/plain` message from an anonymous agent.
    pub fn agent(text: impl Into<String>) -> Self {
        Self::new(Role::Agent, [MessagePart::text(text)])
    }

    /// Append a part.
    pub fn push(&mut self, part: MessagePart) {
        self.parts.push(part);
    }

    /// Concatenate the plain text of every `text/*` part.
    ///
    /// This mirrors the indexing model used by [`CitationMetadata`].
    pub fn text(&self) -> String {
        self.parts.iter().filter_map(MessagePart::as_text).collect()
    }

    /// Mark the message completed as of now.
    pub fn complete(&mut self) {
        self.completed_at = Some(Utc::now());
    }

    /// Check the `minItems: 1` constraint on `parts` and validate each part.
    pub fn validate(&self) -> Result<(), Error> {
        if self.parts.is_empty() {
            return Err(Error::invalid_input("message must contain at least one part"));
        }
        for part in &self.parts {
            part.validate()?;
        }
        Ok(())
    }
}

impl std::fmt::Display for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text())
    }
}
