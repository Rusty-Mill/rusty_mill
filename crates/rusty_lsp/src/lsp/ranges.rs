//! `textDocument/foldingRange` and `textDocument/selectionRange` types.

use super::base::{Position, TextDocumentIdentifier};
use serde::{Deserialize, Serialize};

/// Well-known [`FoldingRangeKind`] values from the spec. Like
/// [`crate::lsp::CodeActionKind`], this is an open string enum, so these are
/// plain constants rather than a closed Rust enum.
pub mod folding_range_kind {
    /// A comment block.
    pub const COMMENT: &str = "comment";
    /// A contiguous block of import/use statements.
    pub const IMPORTS: &str = "imports";
    /// A region marked by the language's region-comment convention.
    pub const REGION: &str = "region";
}

/// A [`FoldingRangeKind`] value, e.g. `"comment"` or `"region"`. An open
/// string enum per the spec (see the [`folding_range_kind`] module for
/// well-known values), not a closed Rust enum.
pub type FoldingRangeKind = String;

/// Parameters of `textDocument/foldingRange`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoldingRangeParams {
    /// The document to compute folding ranges for.
    pub text_document: TextDocumentIdentifier,
    #[serde(flatten)]
    pub work_done: super::progress::WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result: super::progress::PartialResultParams,
}

/// One collapsible region in a document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoldingRange {
    /// The zero-based line the folded range starts on.
    pub start_line: u32,
    /// The UTF-16 column the folded range starts at, on `start_line`. When
    /// omitted, the range extends to the end of `start_line`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_character: Option<u32>,
    /// The zero-based line the folded range ends on.
    pub end_line: u32,
    /// The UTF-16 column the folded range ends at, on `end_line`. When
    /// omitted, the range extends to the end of `end_line`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_character: Option<u32>,
    /// The range's kind (see [`folding_range_kind`]), used to pick an icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<FoldingRangeKind>,
    /// Text to show in place of the folded range, if the client supports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collapsed_text: Option<String>,
}

impl FoldingRange {
    /// Build a folding range from just its start/end lines.
    pub fn new(start_line: u32, end_line: u32) -> Self {
        FoldingRange {
            start_line,
            start_character: None,
            end_line,
            end_character: None,
            kind: None,
            collapsed_text: None,
        }
    }

    /// Set the range's kind.
    #[must_use]
    pub fn with_kind(mut self, kind: impl Into<FoldingRangeKind>) -> Self {
        self.kind = Some(kind.into());
        self
    }
}

/// Parameters of `textDocument/selectionRange`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionRangeParams {
    /// The document to compute selection ranges in.
    pub text_document: TextDocumentIdentifier,
    /// The cursor positions to compute a selection-range chain for, one per
    /// position (results are returned in the same order).
    pub positions: Vec<Position>,
    #[serde(flatten)]
    pub work_done: super::progress::WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result: super::progress::PartialResultParams,
}

/// A selection range and the chain of ever-larger enclosing ranges above it
/// (e.g. word → expression → statement → block), used for "expand
/// selection" commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionRange {
    /// This range.
    pub range: super::base::Range,
    /// The next-larger enclosing range, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<Box<SelectionRange>>,
}

#[cfg(test)]
mod tests {
    use super::super::base::Range;
    use super::*;
    use serde_json::json;

    #[test]
    fn folding_range_omits_absent_characters_and_kind() {
        let range = FoldingRange::new(1, 5);
        let value = serde_json::to_value(&range).unwrap();
        assert_eq!(value, json!({"startLine": 1, "endLine": 5}));
    }

    #[test]
    fn selection_range_nests_parent() {
        let inner = SelectionRange {
            range: Range::new(Position::new(0, 0), Position::new(0, 1)),
            parent: None,
        };
        let outer = SelectionRange {
            range: Range::new(Position::new(0, 0), Position::new(0, 5)),
            parent: Some(Box::new(inner)),
        };
        let value = serde_json::to_value(&outer).unwrap();
        assert_eq!(value["parent"]["range"]["end"]["character"], json!(1));
    }
}

/// Parameters of `textDocument/linkedEditingRange` (LSP 3.16).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkedEditingRangeParams {
    #[serde(flatten)]
    pub text_document_position: super::base::TextDocumentPositionParams,
    #[serde(flatten)]
    pub work_done: super::progress::WorkDoneProgressParams,
}

/// Result of `textDocument/linkedEditingRange`: ranges that should be edited
/// in lockstep (e.g. an HTML tag's opening and closing names).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedEditingRanges {
    /// The linked ranges. All have identical content, and none overlaps.
    pub ranges: Vec<super::base::Range>,
    /// A regex constraining what edited content keeps the link alive; the
    /// client falls back to a word-ish default when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word_pattern: Option<String>,
}
