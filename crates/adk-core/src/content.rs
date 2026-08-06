//! Multimodal content types.
//!
//! Mirrors the ADK `Content` / `Part` model, which in turn follows the
//! `google.genai` content shape used across every ADK language SDK.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The role that authored a piece of [`Content`].
///
/// ADK uses the two `google.genai` roles. Tool results are reported with the
/// `user` role carrying a [`Part::FunctionResponse`], matching the other SDKs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Input from the end user, and the carrier role for function responses.
    User,
    /// Output produced by the model.
    Model,
}

impl Role {
    /// The wire representation of this role.
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Model => "model",
        }
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A model-issued request to invoke a tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionCall {
    /// Correlation id, echoed back on the matching [`FunctionResponse`].
    ///
    /// Optional on the wire because some providers omit it for single calls,
    /// but ADK assigns one before dispatch so tools always see a stable id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Name of the tool to invoke.
    pub name: String,
    /// Arguments, keyed by parameter name.
    #[serde(default)]
    pub args: Map<String, Value>,
}

impl FunctionCall {
    /// Builds a call with a freshly generated id.
    pub fn new(name: impl Into<String>, args: Map<String, Value>) -> Self {
        Self {
            id: Some(crate::new_id("call")),
            name: name.into(),
            args,
        }
    }
}

/// The result of a tool invocation, returned to the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionResponse {
    /// Correlation id matching the originating [`FunctionCall::id`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Name of the tool that produced this response.
    pub name: String,
    /// The tool's return payload.
    ///
    /// ADK's convention is a JSON object; scalar returns are wrapped under a
    /// single `result` key by [`crate::wrap_tool_result`].
    #[serde(default)]
    pub response: Value,
}

/// Inline binary data (images, audio, arbitrary blobs).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Blob {
    /// IANA media type, e.g. `image/png`.
    pub mime_type: String,
    /// Base64-encoded payload, matching the `google.genai` wire format.
    pub data: String,
}

/// A reference to data held outside the message itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileData {
    /// IANA media type of the referenced file.
    pub mime_type: String,
    /// URI the runtime can resolve to fetch the bytes.
    pub file_uri: String,
}

/// One element of a [`Content`] payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Part {
    /// Plain text.
    Text(String),
    /// Model reasoning emitted by a thinking-capable model.
    ///
    /// Kept distinct from [`Part::Text`] so the runtime can exclude it from
    /// user-facing output while still persisting it in session history.
    Thought(String),
    /// Inline bytes.
    InlineData(Blob),
    /// An out-of-band file reference.
    FileData(FileData),
    /// A request to run a tool.
    FunctionCall(FunctionCall),
    /// The result of running a tool.
    FunctionResponse(FunctionResponse),
}

impl Part {
    /// Convenience constructor for a text part.
    pub fn text(s: impl Into<String>) -> Self {
        Part::Text(s.into())
    }

    /// Returns the text payload, if this is a [`Part::Text`].
    ///
    /// Deliberately excludes [`Part::Thought`] — callers assembling a user
    /// facing answer should not pick up reasoning traces.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Part::Text(t) => Some(t),
            _ => None,
        }
    }

    /// Returns the function call, if this is a [`Part::FunctionCall`].
    pub fn as_function_call(&self) -> Option<&FunctionCall> {
        match self {
            Part::FunctionCall(c) => Some(c),
            _ => None,
        }
    }

    /// Returns the function response, if this is a [`Part::FunctionResponse`].
    pub fn as_function_response(&self) -> Option<&FunctionResponse> {
        match self {
            Part::FunctionResponse(r) => Some(r),
            _ => None,
        }
    }
}

/// A role-tagged sequence of [`Part`]s — the unit of conversation history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Content {
    /// Who authored these parts.
    pub role: Role,
    /// The payload.
    #[serde(default)]
    pub parts: Vec<Part>,
}

impl Content {
    /// Builds content from an explicit role and parts.
    pub fn new(role: Role, parts: Vec<Part>) -> Self {
        Self { role, parts }
    }

    /// Builds single-part user text.
    pub fn user_text(text: impl Into<String>) -> Self {
        Self::new(Role::User, vec![Part::text(text)])
    }

    /// Builds single-part model text.
    pub fn model_text(text: impl Into<String>) -> Self {
        Self::new(Role::Model, vec![Part::text(text)])
    }

    /// Concatenates every [`Part::Text`] in order.
    ///
    /// Returns an empty string when there is no text, which keeps call sites
    /// free of `Option` handling; use [`Content::parts`] directly when the
    /// distinction between "no text" and "empty text" matters.
    pub fn text(&self) -> String {
        self.parts
            .iter()
            .filter_map(Part::as_text)
            .collect::<Vec<_>>()
            .concat()
    }

    /// All function calls carried by this content, in order.
    pub fn function_calls(&self) -> Vec<&FunctionCall> {
        self.parts
            .iter()
            .filter_map(Part::as_function_call)
            .collect()
    }

    /// All function responses carried by this content, in order.
    pub fn function_responses(&self) -> Vec<&FunctionResponse> {
        self.parts
            .iter()
            .filter_map(Part::as_function_response)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_concatenates_only_text_parts() {
        let c = Content::new(
            Role::Model,
            vec![
                Part::text("Hello, "),
                Part::Thought("the user greeted me".into()),
                Part::text("world"),
            ],
        );
        assert_eq!(c.text(), "Hello, world");
    }

    #[test]
    fn function_call_and_response_accessors() {
        let call = FunctionCall::new("get_weather", Map::new());
        let id = call.id.clone();
        let c = Content::new(Role::Model, vec![Part::FunctionCall(call)]);
        assert_eq!(c.function_calls().len(), 1);
        assert!(c.function_responses().is_empty());
        assert!(id.is_some());
    }

    #[test]
    fn role_round_trips_through_json() {
        let json = serde_json::to_string(&Role::Model).unwrap();
        assert_eq!(json, "\"model\"");
        assert_eq!(serde_json::from_str::<Role>(&json).unwrap(), Role::Model);
    }
}
