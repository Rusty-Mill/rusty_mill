//! `textDocument/inlineValue` types (LSP 3.17): values a debugger-integrated
//! client shows inline next to the code while stopped at a breakpoint.

use super::base::{Range, TextDocumentIdentifier};
use super::progress::WorkDoneProgressParams;
use serde::{Deserialize, Serialize};

/// Parameters of `textDocument/inlineValue`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineValueParams {
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
    /// The document to compute inline values for.
    pub text_document: TextDocumentIdentifier,
    /// The document range for which inline values should be computed
    /// (typically the visible viewport).
    pub range: Range,
    /// Additional execution context from the debug session.
    pub context: InlineValueContext,
}

/// The debug-session context an inline-value request is evaluated in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineValueContext {
    /// The stack frame (as a DAP id) the values are for.
    pub frame_id: i32,
    /// The range covering the line where execution is currently stopped.
    pub stopped_location: Range,
}

/// One inline value: either literal text, a variable to look up in the
/// debugger, or an expression for the debugger to evaluate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InlineValue {
    /// Show `text` verbatim.
    Text(InlineValueText),
    /// Ask the underlying debugger to look up a variable by name.
    VariableLookup(InlineValueVariableLookup),
    /// Ask the underlying debugger to evaluate an expression.
    EvaluatableExpression(InlineValueEvaluatableExpression),
}

/// Literal inline-value text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineValueText {
    /// The range the value applies to.
    pub range: Range,
    /// The text to show.
    pub text: String,
}

/// An inline value resolved by variable lookup in the debug session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineValueVariableLookup {
    /// The range the value applies to; used to extract the variable name
    /// when [`variable_name`](Self::variable_name) is unset.
    pub range: Range,
    /// The variable to look up; defaults to the text within `range`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variable_name: Option<String>,
    /// Whether the lookup is case sensitive.
    pub case_sensitive_lookup: bool,
}

/// An inline value resolved by expression evaluation in the debug session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineValueEvaluatableExpression {
    /// The range the value applies to; used to extract the expression when
    /// [`expression`](Self::expression) is unset.
    pub range: Range,
    /// The expression to evaluate; defaults to the text within `range`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::Position;
    use serde_json::json;

    fn range() -> Range {
        Range::new(Position::new(0, 0), Position::new(0, 3))
    }

    #[test]
    fn inline_value_variants_round_trip_untagged() {
        let text = InlineValue::Text(InlineValueText {
            range: range(),
            text: "x = 1".to_owned(),
        });
        let value = serde_json::to_value(&text).unwrap();
        assert_eq!(value["text"], json!("x = 1"));
        assert_eq!(serde_json::from_value::<InlineValue>(value).unwrap(), text);

        let lookup = InlineValue::VariableLookup(InlineValueVariableLookup {
            range: range(),
            variable_name: None,
            case_sensitive_lookup: true,
        });
        let value = serde_json::to_value(&lookup).unwrap();
        assert_eq!(value["caseSensitiveLookup"], json!(true));
        assert_eq!(
            serde_json::from_value::<InlineValue>(value).unwrap(),
            lookup
        );

        let expr = InlineValue::EvaluatableExpression(InlineValueEvaluatableExpression {
            range: range(),
            expression: Some("a + b".to_owned()),
        });
        let value = serde_json::to_value(&expr).unwrap();
        assert_eq!(serde_json::from_value::<InlineValue>(value).unwrap(), expr);
    }
}
