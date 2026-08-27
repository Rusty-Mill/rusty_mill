//! `textDocument/documentLink`, `documentColor`, and `colorPresentation`
//! types.

use super::base::{Range, TextDocumentIdentifier, Uri};
use super::progress::{PartialResultParams, WorkDoneProgressParams};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Parameters of `textDocument/documentLink`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentLinkParams {
    /// The document to find links in.
    pub text_document: TextDocumentIdentifier,
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result: PartialResultParams,
}

/// A clickable link anchored to a document range.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentLink {
    /// The range the link is anchored to.
    pub range: Range,
    /// The link's target. `None` until resolved, for servers that support
    /// `documentLink/resolve`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<Uri>,
    /// A tooltip shown when hovering the link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<String>,
    /// Opaque data round-tripped through `documentLink/resolve`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Options describing the server's `textDocument/documentLink` support.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentLinkOptions {
    /// Whether the server reports work-done progress for this provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_done_progress: Option<bool>,
    /// Whether the server supports `documentLink/resolve` for lazily filling
    /// in `target`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolve_provider: Option<bool>,
}

/// Parameters of `textDocument/documentColor`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentColorParams {
    /// The document to find color literals in.
    pub text_document: TextDocumentIdentifier,
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result: PartialResultParams,
}

/// An RGBA color, each component normalised to `0.0..=1.0`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Color {
    /// Red, `0.0..=1.0`.
    pub red: f64,
    /// Green, `0.0..=1.0`.
    pub green: f64,
    /// Blue, `0.0..=1.0`.
    pub blue: f64,
    /// Alpha, `0.0..=1.0`.
    pub alpha: f64,
}

/// A color literal found in a document, anchored to its range.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ColorInformation {
    /// The range of the color literal in the source.
    pub range: Range,
    /// The color it represents.
    pub color: Color,
}

/// Parameters of `textDocument/colorPresentation`: asks the server how a
/// given [`Color`] should be presented (and edited) in the source language.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorPresentationParams {
    /// The document the color literal is in.
    pub text_document: TextDocumentIdentifier,
    /// The color to present.
    pub color: Color,
    /// The range of the color literal being edited.
    pub range: Range,
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result: PartialResultParams,
}

/// One way to present/write a [`Color`] in the source language (e.g. as a
/// hex literal, an `rgb()` call, or a named color).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorPresentation {
    /// The presentation shown to the user (e.g. `"#FF0000"`).
    pub label: String,
    /// The edit to make when this presentation is chosen, if it differs from
    /// `label` itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_edit: Option<super::workspace::TextEdit>,
    /// Further edits to apply alongside `text_edit` (e.g. adding an import).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additional_text_edits: Option<Vec<super::workspace::TextEdit>>,
}

#[cfg(test)]
mod tests {
    use super::super::base::Position;
    use super::*;
    use serde_json::json;

    #[test]
    fn document_link_omits_unresolved_fields() {
        let link = DocumentLink {
            range: Range::new(Position::new(0, 0), Position::new(0, 1)),
            target: None,
            tooltip: None,
            data: None,
        };
        let value = serde_json::to_value(&link).unwrap();
        assert_eq!(
            value,
            json!({"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}}})
        );
    }

    #[test]
    fn color_round_trips() {
        let color = Color {
            red: 1.0,
            green: 0.0,
            blue: 0.0,
            alpha: 1.0,
        };
        let value = serde_json::to_value(color).unwrap();
        assert_eq!(
            value,
            json!({"red": 1.0, "green": 0.0, "blue": 0.0, "alpha": 1.0})
        );
    }

    #[test]
    fn document_link_options_advertise_work_done_progress() {
        let options = DocumentLinkOptions {
            work_done_progress: Some(true),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(&options).unwrap(),
            json!({"workDoneProgress": true})
        );
        assert_eq!(
            serde_json::to_value(DocumentLinkOptions::default()).unwrap(),
            json!({})
        );
    }
}
