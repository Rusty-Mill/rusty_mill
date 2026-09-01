//! `Message`, `Part` and `Role` (spec Section 4.1.4-4.1.6 / proto `Message`,
//! `Part`, `Role`).

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use rusty_uuid::Uuid;

/// Identifies the sender of a [`Message`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    #[serde(rename = "ROLE_UNSPECIFIED")]
    Unspecified,
    /// The message is from the client to the server.
    #[serde(rename = "ROLE_USER")]
    User,
    /// The message is from the server to the client.
    #[serde(rename = "ROLE_AGENT")]
    Agent,
}

/// The content union of a [`Part`] (proto `oneof content`). Exactly one
/// variant is present in any given `Part`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum PartContent {
    /// Plain text content.
    Text { text: String },
    /// Raw file bytes, base64-encoded on the wire.
    Raw {
        #[serde(with = "crate::codec::base64_bytes")]
        raw: Vec<u8>,
    },
    /// A URL pointing to the file's content.
    Url { url: String },
    /// Arbitrary structured JSON data.
    Data { data: Value },
}

/// Deriving `Deserialize` directly on an untagged enum, as `PartContent`
/// otherwise would, doesn't enforce spec Section 4.1.6's "A Part MUST
/// contain exactly one of the following: text, raw, url, data" - serde
/// tries each variant in turn and accepts the first one that matches,
/// silently ignoring any of the other three keys that also happen to be
/// present rather than rejecting the input. This mirror type keeps that
/// same derived (and therefore reused, not reimplemented) per-variant
/// deserialization logic, but only after [`PartContent`]'s own
/// `Deserialize` impl below has confirmed exactly one of the four keys is
/// present.
#[derive(Deserialize)]
#[serde(untagged)]
enum PartContentRepr {
    Text {
        text: String,
    },
    Raw {
        #[serde(with = "crate::codec::base64_bytes")]
        raw: Vec<u8>,
    },
    Url {
        url: String,
    },
    Data {
        data: Value,
    },
}

impl From<PartContentRepr> for PartContent {
    fn from(repr: PartContentRepr) -> Self {
        match repr {
            PartContentRepr::Text { text } => PartContent::Text { text },
            PartContentRepr::Raw { raw } => PartContent::Raw { raw },
            PartContentRepr::Url { url } => PartContent::Url { url },
            PartContentRepr::Data { data } => PartContent::Data { data },
        }
    }
}

impl<'de> Deserialize<'de> for PartContent {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let obj = value
            .as_object()
            .ok_or_else(|| serde::de::Error::custom("a Part must be a JSON object"))?;
        const KEYS: [&str; 4] = ["text", "raw", "url", "data"];
        let present: Vec<&str> = KEYS.iter().copied().filter(|k| obj.contains_key(*k)).collect();
        if present.len() != 1 {
            return Err(serde::de::Error::custom(format!(
                "a Part must contain exactly one of `text`, `raw`, `url`, `data` (spec Section \
                 4.1.6); found {} ({present:?})",
                present.len()
            )));
        }
        serde_json::from_value::<PartContentRepr>(value)
            .map(Into::into)
            .map_err(serde::de::Error::custom)
    }
}

/// A container for a section of communication content: text, a file (by
/// raw bytes or URL), or structured data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Part {
    #[serde(flatten)]
    pub content: PartContent,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub metadata: Option<Map<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub filename: Option<String>,
    #[serde(rename = "mediaType", skip_serializing_if = "Option::is_none", default)]
    pub media_type: Option<String>,
}

impl Part {
    pub fn text(text: impl Into<String>) -> Self {
        Part {
            content: PartContent::Text { text: text.into() },
            metadata: None,
            filename: None,
            media_type: None,
        }
    }

    pub fn data(data: Value) -> Self {
        Part {
            content: PartContent::Data { data },
            metadata: None,
            filename: None,
            media_type: None,
        }
    }

    pub fn url(url: impl Into<String>) -> Self {
        Part {
            content: PartContent::Url { url: url.into() },
            metadata: None,
            filename: None,
            media_type: None,
        }
    }

    pub fn raw(bytes: impl Into<Vec<u8>>) -> Self {
        Part {
            content: PartContent::Raw { raw: bytes.into() },
            metadata: None,
            filename: None,
            media_type: None,
        }
    }

    pub fn with_media_type(mut self, media_type: impl Into<String>) -> Self {
        self.media_type = Some(media_type.into());
        self
    }

    pub fn with_filename(mut self, filename: impl Into<String>) -> Self {
        self.filename = Some(filename.into());
        self
    }

    /// Returns the text content of this part, if it is a text part.
    pub fn as_text(&self) -> Option<&str> {
        match &self.content {
            PartContent::Text { text } => Some(text),
            _ => None,
        }
    }
}

/// One unit of communication between client and server (spec Section
/// 4.1.4). May be associated with a `contextId` and/or `taskId`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    #[serde(rename = "messageId")]
    pub message_id: String,
    #[serde(rename = "contextId", skip_serializing_if = "Option::is_none", default)]
    pub context_id: Option<String>,
    #[serde(rename = "taskId", skip_serializing_if = "Option::is_none", default)]
    pub task_id: Option<String>,
    pub role: Role,
    pub parts: Vec<Part>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub metadata: Option<Map<String, Value>>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub extensions: Vec<String>,
    #[serde(rename = "referenceTaskIds", skip_serializing_if = "Vec::is_empty", default)]
    pub reference_task_ids: Vec<String>,
}

impl Message {
    /// Builds a new message with a freshly generated `messageId`.
    pub fn new(role: Role, parts: Vec<Part>) -> Self {
        Message {
            message_id: Uuid::new_v4().to_string(),
            context_id: None,
            task_id: None,
            role,
            parts,
            metadata: None,
            extensions: Vec::new(),
            reference_task_ids: Vec::new(),
        }
    }

    /// Convenience constructor for a single-text-part user message.
    pub fn user_text(text: impl Into<String>) -> Self {
        Message::new(Role::User, vec![Part::text(text)])
    }

    /// Convenience constructor for a single-text-part agent message.
    pub fn agent_text(text: impl Into<String>) -> Self {
        Message::new(Role::Agent, vec![Part::text(text)])
    }

    pub fn with_context_id(mut self, context_id: impl Into<String>) -> Self {
        self.context_id = Some(context_id.into());
        self
    }

    pub fn with_task_id(mut self, task_id: impl Into<String>) -> Self {
        self.task_id = Some(task_id.into());
        self
    }

    /// Concatenates the text of all text parts, space-separated. Convenient
    /// for agents that only care about the textual content of a message.
    pub fn text(&self) -> String {
        self.parts
            .iter()
            .filter_map(|p| p.as_text())
            .collect::<Vec<_>>()
            .join(" ")
    }
}
