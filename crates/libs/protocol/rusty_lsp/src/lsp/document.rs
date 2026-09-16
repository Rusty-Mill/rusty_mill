//! Text-document synchronisation notification parameters.

use super::base::{
    Range, TextDocumentIdentifier, TextDocumentItem, VersionedTextDocumentIdentifier,
};
use super::enums::TextDocumentSaveReason;
use serde::{Deserialize, Serialize};

/// Parameters of `textDocument/didOpen`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DidOpenTextDocumentParams {
    /// The document that was opened.
    pub text_document: TextDocumentItem,
}

/// Parameters of `textDocument/didChange`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DidChangeTextDocumentParams {
    /// The document that changed, with its new version.
    pub text_document: VersionedTextDocumentIdentifier,
    /// The content changes, applied in order.
    pub content_changes: Vec<TextDocumentContentChangeEvent>,
}

/// A single change to a text document.
///
/// When [`range`](Self::range) is `Some`, only that range was replaced
/// (incremental sync). When it is `None`, [`text`](Self::text) is the entire
/// new document content (full sync).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextDocumentContentChangeEvent {
    /// The range that was replaced, or `None` for a full-document update.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<Range>,
    /// The new text for the range (or the whole document).
    pub text: String,
}

/// Parameters of `textDocument/didClose`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DidCloseTextDocumentParams {
    /// The document that was closed.
    pub text_document: TextDocumentIdentifier,
}

/// Parameters of `textDocument/didSave`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DidSaveTextDocumentParams {
    /// The document that was saved.
    pub text_document: TextDocumentIdentifier,
    /// The full document content, present only if the client is configured to
    /// include text on save.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// Parameters of `textDocument/willSave` and `textDocument/willSaveWaitUntil`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WillSaveTextDocumentParams {
    /// The document about to be saved.
    pub text_document: TextDocumentIdentifier,
    /// What triggered the save.
    pub reason: TextDocumentSaveReason,
}

/// The full form of the server's text-document-sync capability, advertised
/// in [`crate::lsp::ServerCapabilities::text_document_sync`] when a bare
/// [`TextDocumentSyncKind`](super::enums::TextDocumentSyncKind) is not
/// expressive enough — e.g. to request document text in `didSave`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextDocumentSyncOptions {
    /// Whether the server wants `didOpen`/`didClose` notifications.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_close: Option<bool>,
    /// How document changes are synced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<super::enums::TextDocumentSyncKind>,
    /// Whether the server wants `willSave` notifications.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub will_save: Option<bool>,
    /// Whether the server wants `willSaveWaitUntil` requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub will_save_wait_until: Option<bool>,
    /// Whether the server wants `didSave` notifications, optionally with
    /// the saved text included.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save: Option<SaveOptionsCapability>,
}

/// The `save` member of [`TextDocumentSyncOptions`]: a plain boolean or
/// [`SaveOptions`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SaveOptionsCapability {
    /// `true`/`false`: send (or don't send) `didSave`, without text.
    Simple(bool),
    /// Send `didSave` with the given options.
    Options(SaveOptions),
}

/// Options of the `didSave` notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveOptions {
    /// Ask the client to include the document's content in
    /// [`DidSaveTextDocumentParams::text`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_text: Option<bool>,
}

#[cfg(test)]
mod sync_options_tests {
    use super::super::enums::TextDocumentSyncKind;
    use super::*;
    use serde_json::json;

    #[test]
    fn sync_options_serialize_with_save_text() {
        let options = TextDocumentSyncOptions {
            open_close: Some(true),
            change: Some(TextDocumentSyncKind::Incremental),
            save: Some(SaveOptionsCapability::Options(SaveOptions {
                include_text: Some(true),
            })),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(options).unwrap(),
            json!({
                "openClose": true,
                "change": 2,
                "save": {"includeText": true},
            })
        );

        let simple: SaveOptionsCapability = serde_json::from_value(json!(true)).unwrap();
        assert_eq!(simple, SaveOptionsCapability::Simple(true));
    }
}

/// The value of [`crate::lsp::ServerCapabilities::text_document_sync`]: a
/// bare kind (the common case) or the full [`TextDocumentSyncOptions`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TextDocumentSyncCapability {
    /// Just the change-sync kind; open/close notifications implied.
    Kind(super::enums::TextDocumentSyncKind),
    /// The full options form.
    Options(TextDocumentSyncOptions),
}

impl From<super::enums::TextDocumentSyncKind> for TextDocumentSyncCapability {
    fn from(kind: super::enums::TextDocumentSyncKind) -> Self {
        TextDocumentSyncCapability::Kind(kind)
    }
}

impl From<TextDocumentSyncOptions> for TextDocumentSyncCapability {
    fn from(options: TextDocumentSyncOptions) -> Self {
        TextDocumentSyncCapability::Options(options)
    }
}
