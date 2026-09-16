//! `textDocument/rename` and `textDocument/prepareRename` types.

use super::base::{Range, TextDocumentPositionParams};
use serde::{Deserialize, Serialize};

/// Parameters of `textDocument/rename`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameParams {
    /// The document and position of the symbol to rename.
    #[serde(flatten)]
    pub text_document_position: TextDocumentPositionParams,
    /// The symbol's new name.
    pub new_name: String,
}

/// The result of a `textDocument/prepareRename` request: whether (and how)
/// the symbol under the cursor can be renamed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PrepareRenameResponse {
    /// The range that will be renamed (the client fills the rename input
    /// with this range's current text).
    Range(Range),
    /// The range to rename, plus an explicit placeholder string to
    /// pre-populate the rename input with (when it should differ from the
    /// range's literal text).
    RangeWithPlaceholder {
        /// The range that will be renamed.
        range: Range,
        /// The text to pre-populate the rename input with.
        placeholder: String,
    },
    /// The symbol can be renamed using the client's default word-boundary
    /// detection, with no explicit range.
    DefaultBehavior {
        /// Always `true`; present only to distinguish this variant on the
        /// wire.
        #[serde(rename = "defaultBehavior")]
        default_behavior: bool,
    },
}

/// Options describing the server's `textDocument/rename` support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameOptions {
    /// Whether the server reports work-done progress for this provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_done_progress: Option<bool>,
    /// Whether the server supports `textDocument/prepareRename`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prepare_provider: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn range() -> Range {
        Range::new(
            super::super::base::Position::new(0, 0),
            super::super::base::Position::new(0, 3),
        )
    }

    #[test]
    fn prepare_rename_response_variants_serialize_distinctly() {
        assert_eq!(
            serde_json::to_value(PrepareRenameResponse::Range(range())).unwrap(),
            json!({"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 3}})
        );

        let value = serde_json::to_value(PrepareRenameResponse::RangeWithPlaceholder {
            range: range(),
            placeholder: "foo".to_owned(),
        })
        .unwrap();
        assert_eq!(value["placeholder"], json!("foo"));

        let value = serde_json::to_value(PrepareRenameResponse::DefaultBehavior {
            default_behavior: true,
        })
        .unwrap();
        assert_eq!(value, json!({"defaultBehavior": true}));
    }

    #[test]
    fn rename_params_new_name_uses_camel_case() {
        let params = RenameParams {
            text_document_position: TextDocumentPositionParams {
                text_document: super::super::base::TextDocumentIdentifier {
                    uri: "file:///a".into(),
                },
                position: super::super::base::Position::new(0, 0),
            },
            new_name: "renamed".to_owned(),
        };
        let value = serde_json::to_value(&params).unwrap();
        assert_eq!(value["newName"], json!("renamed"));
        assert!(value.get("new_name").is_none());
    }

    #[test]
    fn rename_options_advertise_work_done_progress() {
        let options = RenameOptions {
            work_done_progress: Some(true),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(options).unwrap(),
            json!({"workDoneProgress": true})
        );
        assert_eq!(
            serde_json::to_value(RenameOptions::default()).unwrap(),
            json!({})
        );
    }
}
