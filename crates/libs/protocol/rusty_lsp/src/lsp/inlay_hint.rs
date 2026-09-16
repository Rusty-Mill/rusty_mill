//! `textDocument/inlayHint` types.

use super::base::{Location, Position, Range, TextDocumentIdentifier};
use super::code_action::Command;
use super::enums::InlayHintKind;
use super::progress::WorkDoneProgressParams;
use super::signature::Documentation;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Parameters of `textDocument/inlayHint`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlayHintParams {
    /// The document to compute inlay hints for.
    pub text_document: TextDocumentIdentifier,
    /// The range to compute hints within.
    pub range: Range,
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
}

/// An inline annotation shown alongside a document's text — a type hint, a
/// parameter name, or similar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlayHint {
    /// Where to render the hint.
    pub position: Position,
    /// The hint's text.
    pub label: InlayHintLabel,
    /// The hint's kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<InlayHintKind>,
    /// Edits that insert the hint's text literally into the document (used
    /// to support "accept this hint" editor actions).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_edits: Option<Vec<super::workspace::TextEdit>>,
    /// A tooltip shown when hovering the hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<Documentation>,
    /// Whether to render whitespace before the hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding_left: Option<bool>,
    /// Whether to render whitespace after the hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding_right: Option<bool>,
    /// Opaque data round-tripped through `inlayHint/resolve`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl InlayHint {
    /// Build an inlay hint from just its position and label text.
    pub fn new(position: Position, label: impl Into<String>) -> Self {
        InlayHint {
            position,
            label: InlayHintLabel::String(label.into()),
            kind: None,
            text_edits: None,
            tooltip: None,
            padding_left: None,
            padding_right: None,
            data: None,
        }
    }

    /// Set the hint's kind.
    #[must_use]
    pub fn with_kind(mut self, kind: InlayHintKind) -> Self {
        self.kind = Some(kind);
        self
    }
}

/// An [`InlayHint`]'s label: either plain text, or several clickable parts
/// (e.g. a type name that links to its declaration).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InlayHintLabel {
    /// Plain label text.
    String(String),
    /// Several concatenated, individually-actionable parts.
    Parts(Vec<InlayHintLabelPart>),
}

/// One part of a multi-part [`InlayHintLabel`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlayHintLabelPart {
    /// This part's text.
    pub value: String,
    /// A tooltip shown when hovering this part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<Documentation>,
    /// A location this part navigates to when clicked (e.g. a type's
    /// declaration).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
    /// A command to run when this part is clicked, instead of navigating.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<Command>,
}

/// Options describing the server's `textDocument/inlayHint` support.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlayHintOptions {
    /// Whether the server reports work-done progress for this provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_done_progress: Option<bool>,
    /// Whether the server supports `inlayHint/resolve` for lazily filling in
    /// `tooltip`/`text_edits`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolve_provider: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn inlay_hint_kind_encodes_as_integer() {
        assert_eq!(serde_json::to_value(InlayHintKind::Type).unwrap(), json!(1));
        assert_eq!(
            serde_json::to_value(InlayHintKind::Parameter).unwrap(),
            json!(2)
        );
    }

    #[test]
    fn inlay_hint_label_is_untagged_string_or_parts() {
        let hint = InlayHint::new(Position::new(0, 0), ": i32");
        assert_eq!(serde_json::to_value(&hint.label).unwrap(), json!(": i32"));

        let parts = InlayHintLabel::Parts(vec![InlayHintLabelPart {
            value: "i32".to_owned(),
            tooltip: None,
            location: None,
            command: None,
        }]);
        let value = serde_json::to_value(&parts).unwrap();
        assert_eq!(value[0]["value"], json!("i32"));
    }

    #[test]
    fn inlay_hint_options_advertise_work_done_progress() {
        let options = InlayHintOptions {
            work_done_progress: Some(true),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(&options).unwrap(),
            json!({"workDoneProgress": true})
        );
        assert_eq!(
            serde_json::to_value(InlayHintOptions::default()).unwrap(),
            json!({})
        );
    }
}
