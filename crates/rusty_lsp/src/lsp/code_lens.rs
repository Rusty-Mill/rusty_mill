//! `textDocument/codeLens` types.

use super::base::{Range, TextDocumentIdentifier};
use super::code_action::Command;
use super::progress::{PartialResultParams, WorkDoneProgressParams};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Parameters of `textDocument/codeLens`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeLensParams {
    /// The document to compute code lenses for.
    pub text_document: TextDocumentIdentifier,
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result: PartialResultParams,
}

/// An inline, actionable annotation anchored to a document range (e.g. a
/// "Run test" link above a test function).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeLens {
    /// The range the lens is anchored to.
    pub range: Range,
    /// The command to run when the lens is activated. `None` until resolved,
    /// for servers that support `codeLens/resolve`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<Command>,
    /// Opaque data round-tripped through `codeLens/resolve`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl CodeLens {
    /// Build a code lens from just its range, with no command yet (to be
    /// filled in by `codeLens/resolve`).
    pub fn new(range: Range) -> Self {
        CodeLens {
            range,
            command: None,
            data: None,
        }
    }
}

/// Options describing the server's `textDocument/codeLens` support.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeLensOptions {
    /// Whether the server reports work-done progress for this provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_done_progress: Option<bool>,
    /// Whether the server supports `codeLens/resolve` for lazily filling in
    /// `command`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolve_provider: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::super::base::Position;
    use super::*;
    use serde_json::json;

    #[test]
    fn code_lens_omits_absent_command_and_data() {
        let lens = CodeLens::new(Range::new(Position::new(0, 0), Position::new(0, 1)));
        let value = serde_json::to_value(&lens).unwrap();
        assert_eq!(
            value,
            json!({"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}}})
        );
    }

    #[test]
    fn code_lens_options_advertise_work_done_progress() {
        let options = CodeLensOptions {
            work_done_progress: Some(true),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(&options).unwrap(),
            json!({"workDoneProgress": true})
        );
        assert_eq!(
            serde_json::to_value(CodeLensOptions::default()).unwrap(),
            json!({})
        );
    }
}
