//! `textDocument/inlineCompletion` types (LSP 3.18, proposed): ghost-text
//! completions shown inline at the cursor, as popularised by AI code
//! assistants.

use super::base::{Range, TextDocumentPositionParams};
use super::code_action::Command;
use super::enums::InlineCompletionTriggerKind;
use super::progress::WorkDoneProgressParams;
use serde::{Deserialize, Serialize};

/// Parameters of `textDocument/inlineCompletion`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InlineCompletionParams {
    #[serde(flatten)]
    pub text_document_position: TextDocumentPositionParams,
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
    /// How and where the request was triggered.
    pub context: InlineCompletionContext,
}

/// The context an inline-completion request was made in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineCompletionContext {
    /// What triggered the request.
    pub trigger_kind: InlineCompletionTriggerKind,
    /// The currently selected item of an open (regular) completion popup,
    /// if one is showing — inline results must be consistent with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_completion_info: Option<SelectedCompletionInfo>,
}

/// The selected item of an open completion popup, alongside the range it
/// would replace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectedCompletionInfo {
    /// The range that would be replaced if the popup item were accepted.
    pub range: Range,
    /// The popup item's text.
    pub text: String,
}

/// The text an [`InlineCompletionItem`] inserts: a plain string or a snippet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InlineCompletionInsertText {
    /// Plain text, inserted verbatim.
    Plain(String),
    /// A snippet (with tab stops / placeholders), in LSP snippet syntax.
    Snippet(StringValue),
}

impl From<String> for InlineCompletionInsertText {
    fn from(s: String) -> Self {
        InlineCompletionInsertText::Plain(s)
    }
}

impl From<&str> for InlineCompletionInsertText {
    fn from(s: &str) -> Self {
        InlineCompletionInsertText::Plain(s.to_owned())
    }
}

/// A tagged string value; currently only the `snippet` kind exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StringValue {
    /// The value's kind; always `"snippet"`.
    pub kind: String,
    /// The snippet text.
    pub value: String,
}

impl StringValue {
    /// Build a snippet string value.
    pub fn snippet(value: impl Into<String>) -> Self {
        StringValue {
            kind: "snippet".to_owned(),
            value: value.into(),
        }
    }
}

/// One inline completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineCompletionItem {
    /// The text to insert.
    pub insert_text: InlineCompletionInsertText,
    /// Text used to decide whether the item is still valid as the user
    /// keeps typing; defaults to `insert_text`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_text: Option<String>,
    /// The range to replace; defaults to inserting at the request position.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<Range>,
    /// A command executed after the item is accepted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<Command>,
}

impl InlineCompletionItem {
    /// Build an item from its insert text.
    pub fn new(insert_text: impl Into<InlineCompletionInsertText>) -> Self {
        InlineCompletionItem {
            insert_text: insert_text.into(),
            filter_text: None,
            range: None,
            command: None,
        }
    }
}

/// Result of `textDocument/inlineCompletion`: a list object or a bare array.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InlineCompletionResponse {
    /// The list form.
    List(InlineCompletionList),
    /// The bare-array form.
    Items(Vec<InlineCompletionItem>),
}

/// The list form of an inline-completion result.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct InlineCompletionList {
    /// The completion items.
    pub items: Vec<InlineCompletionItem>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn item_serializes_plain_and_snippet_insert_text() {
        let plain = InlineCompletionItem::new("let x = 1;");
        assert_eq!(
            serde_json::to_value(&plain).unwrap(),
            json!({"insertText": "let x = 1;"})
        );

        let snippet = InlineCompletionItem::new(InlineCompletionInsertText::Snippet(
            StringValue::snippet("let ${1:name} = $0;"),
        ));
        assert_eq!(
            serde_json::to_value(&snippet).unwrap()["insertText"]["kind"],
            json!("snippet")
        );
    }

    #[test]
    fn params_flatten_position_and_context() {
        let params: InlineCompletionParams = serde_json::from_value(json!({
            "textDocument": {"uri": "file:///a"},
            "position": {"line": 0, "character": 4},
            "context": {"triggerKind": 2},
        }))
        .unwrap();
        assert_eq!(
            params.context.trigger_kind,
            InlineCompletionTriggerKind::Automatic
        );
    }

    #[test]
    fn response_forms_are_untagged() {
        let list = InlineCompletionResponse::List(InlineCompletionList {
            items: vec![InlineCompletionItem::new("x")],
        });
        let value = serde_json::to_value(&list).unwrap();
        assert_eq!(value["items"][0]["insertText"], json!("x"));

        let items = InlineCompletionResponse::Items(vec![InlineCompletionItem::new("y")]);
        let value = serde_json::to_value(&items).unwrap();
        assert_eq!(value[0]["insertText"], json!("y"));
    }
}
