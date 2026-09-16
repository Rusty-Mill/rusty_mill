//! `window/*` parameters for surfacing messages, prompts, and documents to
//! the user.

use super::base::{Range, Uri};
use super::enums::MessageType;
use serde::{Deserialize, Serialize};

/// Parameters of `window/showMessage`: a message shown prominently to the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShowMessageParams {
    /// The message's severity.
    #[serde(rename = "type")]
    pub typ: MessageType,
    /// The message text.
    pub message: String,
}

/// Parameters of `window/logMessage`: a message routed to the client's log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogMessageParams {
    /// The message's severity.
    #[serde(rename = "type")]
    pub typ: MessageType,
    /// The message text.
    pub message: String,
}

/// Parameters of `window/showMessageRequest`: a message shown to the user
/// with a set of actions to choose from (e.g. `["Install", "Ignore"]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShowMessageRequestParams {
    /// The message's severity.
    #[serde(rename = "type")]
    pub typ: MessageType,
    /// The message text.
    pub message: String,
    /// The actions the user may choose between. `None`/empty means the
    /// message is just acknowledged (e.g. an OK button).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<MessageActionItem>>,
}

/// One action the user can choose in a `window/showMessageRequest` prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageActionItem {
    /// The action's label, shown to the user.
    pub title: String,
}

/// Parameters of `window/showDocument` (LSP 3.16): asks the client to open
/// or reveal a document, e.g. a generated file or an external URL.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShowDocumentParams {
    /// The document's URI. May be a `file://` URI or any external URI
    /// (e.g. `https://`) when [`external`](Self::external) is `true`.
    pub uri: Uri,
    /// Whether to open the URI externally (e.g. in a browser) rather than
    /// inside the editor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<bool>,
    /// Whether the client should give the opened document input focus.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub take_focus: Option<bool>,
    /// A range to select/reveal within the document, if applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<Range>,
}

/// Result of a `window/showDocument` request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShowDocumentResult {
    /// Whether the document was successfully shown.
    pub success: bool,
}
