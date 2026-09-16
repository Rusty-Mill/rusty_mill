//! `textDocument/formatting`, `rangeFormatting`, and `onTypeFormatting` types.

use super::base::{Range, TextDocumentIdentifier, TextDocumentPositionParams};
use super::progress::WorkDoneProgressParams;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Formatting preferences shared by all three formatting requests.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormattingOptions {
    /// The number of spaces a tab represents.
    pub tab_size: u32,
    /// Whether to indent with spaces instead of tabs.
    pub insert_spaces: bool,
    /// Any additional editor-specific options not modelled above.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Parameters of `textDocument/formatting`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentFormattingParams {
    /// The document to format.
    pub text_document: TextDocumentIdentifier,
    /// The client's formatting preferences.
    pub options: FormattingOptions,
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
}

/// Parameters of `textDocument/rangeFormatting`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentRangeFormattingParams {
    /// The document to format.
    pub text_document: TextDocumentIdentifier,
    /// The range to format.
    pub range: Range,
    /// The client's formatting preferences.
    pub options: FormattingOptions,
}

/// Parameters of `textDocument/onTypeFormatting`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentOnTypeFormattingParams {
    /// The document and position the triggering character was typed at.
    #[serde(flatten)]
    pub text_document_position: TextDocumentPositionParams,
    /// The character that was just typed, triggering this request.
    pub ch: String,
    /// The client's formatting preferences.
    pub options: FormattingOptions,
}

/// Options describing the server's `textDocument/onTypeFormatting` support.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentOnTypeFormattingOptions {
    /// The character that triggers on-type formatting (e.g. `"}"`).
    pub first_trigger_character: String,
    /// Additional trigger characters.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub more_trigger_character: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn formatting_options_preserves_editor_specific_extras() {
        let value = json!({"tabSize": 2, "insertSpaces": true, "trimTrailingWhitespace": true});
        let options: FormattingOptions = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(options.tab_size, 2);
        assert!(options.insert_spaces);
        assert_eq!(
            options.extra.get("trimTrailingWhitespace"),
            Some(&json!(true))
        );

        let round_tripped = serde_json::to_value(&options).unwrap();
        assert_eq!(round_tripped, value);
    }
}
