//! `textDocument/signatureHelp` types.

use super::base::TextDocumentPositionParams;
use super::enums::{MarkupKind, SignatureHelpTriggerKind};
use super::features::MarkupContent;
use super::progress::WorkDoneProgressParams;
use serde::{Deserialize, Serialize};

/// Parameters of `textDocument/signatureHelp`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignatureHelpParams {
    /// The document and position signature help was requested at.
    #[serde(flatten)]
    pub text_document_position: TextDocumentPositionParams,
    /// Additional information about what triggered this request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<SignatureHelpContext>,
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
}

/// Additional signature-help-trigger context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureHelpContext {
    /// How signature help was triggered.
    pub trigger_kind: SignatureHelpTriggerKind,
    /// The trigger character, when `trigger_kind` is
    /// [`SignatureHelpTriggerKind::TriggerCharacter`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_character: Option<String>,
    /// Whether this re-triggers signature help that was already showing.
    pub is_retrigger: bool,
    /// The signature help that was already showing, when `is_retrigger` is
    /// `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_signature_help: Option<SignatureHelp>,
}

/// The result of a `textDocument/signatureHelp` request.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureHelp {
    /// The candidate signatures (a call may be overloaded).
    pub signatures: Vec<SignatureInformation>,
    /// The index into `signatures` that is currently active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_signature: Option<u32>,
    /// The index into the active signature's `parameters` that is currently
    /// active (i.e. which parameter the cursor is in).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_parameter: Option<u32>,
}

/// One candidate signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureInformation {
    /// The signature's label, e.g. `"fn foo(x: i32, y: i32) -> i32"`.
    pub label: String,
    /// Documentation for the signature as a whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation: Option<Documentation>,
    /// The signature's parameters, in order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Vec<ParameterInformation>>,
    /// Overrides [`SignatureHelp::active_parameter`] for this signature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_parameter: Option<u32>,
}

/// One parameter within a [`SignatureInformation`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParameterInformation {
    /// The parameter's label: either its literal text, or a `[start, end)`
    /// UTF-16 code-unit range into the owning signature's `label`.
    pub label: ParameterLabel,
    /// Documentation for this parameter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation: Option<Documentation>,
}

/// A [`ParameterInformation`]'s label, either standalone text or a range
/// into the signature's label string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParameterLabel {
    /// The parameter's text, repeated standalone (not a substring reference).
    Simple(String),
    /// A `[start, end)` UTF-16 code-unit offset pair into the signature's
    /// `label`.
    Range(u32, u32),
}

/// Documentation text, either plain or Markdown-rendered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Documentation {
    /// Plain text with no markup.
    String(String),
    /// Markup-rendered content.
    Markup(MarkupContent),
}

impl Documentation {
    /// Build plain-text documentation.
    pub fn plain_text(value: impl Into<String>) -> Self {
        Documentation::String(value.into())
    }

    /// Build Markdown documentation.
    pub fn markdown(value: impl Into<String>) -> Self {
        Documentation::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: value.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parameter_label_is_untagged_string_or_range() {
        assert_eq!(
            serde_json::to_value(ParameterLabel::Simple("x".into())).unwrap(),
            json!("x")
        );
        assert_eq!(
            serde_json::to_value(ParameterLabel::Range(0, 3)).unwrap(),
            json!([0, 3])
        );
        let label: ParameterLabel = serde_json::from_value(json!([1, 4])).unwrap();
        assert_eq!(label, ParameterLabel::Range(1, 4));
    }

    #[test]
    fn documentation_is_untagged_string_or_markup() {
        assert_eq!(
            serde_json::to_value(Documentation::plain_text("hi")).unwrap(),
            json!("hi")
        );
        let markup = Documentation::markdown("**hi**");
        let value = serde_json::to_value(&markup).unwrap();
        assert_eq!(value["kind"], json!("markdown"));
        assert_eq!(value["value"], json!("**hi**"));
    }
}
