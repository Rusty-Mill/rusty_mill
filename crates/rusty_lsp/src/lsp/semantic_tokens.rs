//! `textDocument/semanticTokens/{full,full/delta,range}` types.
//!
//! Semantic tokens let a server highlight code more accurately than a
//! TextMate grammar (which only sees syntax) by classifying each token with
//! real semantic information (e.g. "this identifier is a mutable local
//! variable"). The wire format encodes each token as 5 integers relative to
//! the previous one — see [`SemanticTokens::data`] — which this module does
//! not interpret; servers typically build it with a small local encoder
//! keyed by their own token/modifier tables.

use super::base::{Range, TextDocumentIdentifier};
use super::progress::{PartialResultParams, WorkDoneProgressParams};
use serde::{Deserialize, Serialize};

/// Declares the token types and modifiers a server's semantic-token indices
/// refer to. Sent once, in
/// [`crate::lsp::ServerCapabilities::semantic_tokens_provider`]; every
/// token's `tokenType`/`tokenModifiers` index into these arrays.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticTokensLegend {
    /// Token type names, indexed by a token's type field.
    pub token_types: Vec<String>,
    /// Token modifier names, indexed by the set bits of a token's modifiers
    /// bitmask.
    pub token_modifiers: Vec<String>,
}

/// Parameters of `textDocument/semanticTokens/full`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticTokensParams {
    /// The document to compute tokens for.
    pub text_document: TextDocumentIdentifier,
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result: PartialResultParams,
}

/// Parameters of `textDocument/semanticTokens/full/delta`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticTokensDeltaParams {
    /// The document to compute a token delta for.
    pub text_document: TextDocumentIdentifier,
    /// The [`SemanticTokens::result_id`] of the previous full result this
    /// delta is relative to.
    pub previous_result_id: String,
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result: PartialResultParams,
}

/// Parameters of `textDocument/semanticTokens/range`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticTokensRangeParams {
    /// The document to compute tokens for.
    pub text_document: TextDocumentIdentifier,
    /// The range to compute tokens within.
    pub range: Range,
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result: PartialResultParams,
}

/// The full set of semantic tokens for a document (or range).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticTokens {
    /// An opaque id for this result, passed back in a later
    /// [`SemanticTokensDeltaParams::previous_result_id`] to request a delta
    /// instead of a full re-scan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_id: Option<String>,
    /// The encoded tokens: a flat array of relative 5-tuples
    /// `[deltaLine, deltaStart, length, tokenType, tokenModifiers]` per the
    /// spec's delta encoding.
    pub data: Vec<u32>,
}

/// The result of `textDocument/semanticTokens/full/delta`: either a full
/// re-scan (if the server decides a delta isn't worth computing) or an
/// incremental edit list against the previous result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SemanticTokensDeltaResult {
    /// A full token set, same shape as `semanticTokens/full`.
    Tokens(SemanticTokens),
    /// An incremental delta against the previous result.
    Delta(SemanticTokensDelta),
}

/// An incremental edit against a previous [`SemanticTokens::data`] array.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticTokensDelta {
    /// The new result's id, for further deltas to build on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_id: Option<String>,
    /// The edits to apply, in order.
    pub edits: Vec<SemanticTokensEdit>,
}

/// One splice into a previous [`SemanticTokens::data`] array: remove
/// `delete_count` `u32`s starting at `start`, then insert `data` (if any) in
/// their place.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticTokensEdit {
    /// The index into the previous data array to start editing at.
    pub start: u32,
    /// How many `u32`s to remove starting at `start`.
    pub delete_count: u32,
    /// The `u32`s to insert in their place, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Vec<u32>>,
}

/// Options describing the server's `full` semantic-tokens support: either a
/// bare `true`, or an object opting into `full/delta` as well.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SemanticTokensFullOptions {
    /// `true`/`false`: full-document tokens are (not) supported, with no
    /// delta support.
    Simple(bool),
    /// Full-document tokens are supported, with delta support as given.
    Delta {
        /// Whether `semanticTokens/full/delta` is supported.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delta: Option<bool>,
    },
}

/// Options describing the server's `textDocument/semanticTokens` support.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticTokensOptions {
    /// Whether the server reports work-done progress for this provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_done_progress: Option<bool>,
    /// The token/modifier legend every token's indices refer to.
    pub legend: SemanticTokensLegend,
    /// Whether `textDocument/semanticTokens/range` is supported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<bool>,
    /// Whether `textDocument/semanticTokens/full` (and optionally
    /// `full/delta`) is supported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full: Option<SemanticTokensFullOptions>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn semantic_tokens_delta_result_is_untagged() {
        let tokens = SemanticTokensDeltaResult::Tokens(SemanticTokens {
            result_id: Some("1".to_owned()),
            data: vec![0, 0, 3, 0, 0],
        });
        let value = serde_json::to_value(&tokens).unwrap();
        assert_eq!(value["data"], json!([0, 0, 3, 0, 0]));

        let delta = SemanticTokensDeltaResult::Delta(SemanticTokensDelta {
            result_id: Some("2".to_owned()),
            edits: vec![SemanticTokensEdit {
                start: 0,
                delete_count: 5,
                data: Some(vec![0, 0, 4, 1, 0]),
            }],
        });
        let value = serde_json::to_value(&delta).unwrap();
        assert_eq!(value["edits"][0]["deleteCount"], json!(5));
        assert!(value.get("data").is_none());
    }

    #[test]
    fn semantic_tokens_legend_uses_camel_case() {
        let legend = SemanticTokensLegend {
            token_types: vec!["variable".to_owned()],
            token_modifiers: vec!["readonly".to_owned()],
        };
        let value = serde_json::to_value(&legend).unwrap();
        assert_eq!(
            value,
            json!({"tokenTypes": ["variable"], "tokenModifiers": ["readonly"]})
        );
    }

    #[test]
    fn semantic_tokens_full_options_is_untagged() {
        assert_eq!(
            serde_json::to_value(SemanticTokensFullOptions::Simple(true)).unwrap(),
            json!(true)
        );
        assert_eq!(
            serde_json::to_value(SemanticTokensFullOptions::Delta { delta: Some(true) }).unwrap(),
            json!({"delta": true})
        );
    }

    #[test]
    fn semantic_tokens_options_advertise_work_done_progress() {
        let options = SemanticTokensOptions {
            work_done_progress: Some(true),
            legend: SemanticTokensLegend {
                token_types: vec!["keyword".to_owned()],
                token_modifiers: vec![],
            },
            range: None,
            full: None,
        };
        let value = serde_json::to_value(&options).unwrap();
        assert_eq!(value["workDoneProgress"], json!(true));
        assert!(value.get("range").is_none());
        assert!(value.get("full").is_none());
    }
}

/// Builds the spec's relative-encoded token array from absolute token
/// positions, so servers never hand-compute `deltaLine`/`deltaStart` (the
/// most error-prone encoding in LSP).
///
/// Push tokens in any order with [`push`](Self::push) (names resolved
/// against the legend) or [`push_range`](Self::push_range) (a document
/// [`crate::lsp::Range`], split across lines automatically);
/// [`build`](Self::build) sorts them into document order and delta-encodes:
///
/// ```rust
/// use rusty_lsp::lsp::{SemanticTokensBuilder, SemanticTokensLegend};
///
/// let legend = SemanticTokensLegend {
///     token_types: vec!["keyword".into(), "variable".into()],
///     token_modifiers: vec!["declaration".into()],
/// };
/// let mut builder = SemanticTokensBuilder::new(&legend);
/// builder.push(0, 4, 1, "variable", &["declaration"]).unwrap();
/// builder.push(0, 0, 3, "keyword", &[]).unwrap(); // out of order is fine
/// let tokens = builder.build(None);
/// assert_eq!(tokens.data, [0, 0, 3, 0, 0, 0, 4, 1, 1, 1]);
/// ```
#[derive(Debug, Clone)]
pub struct SemanticTokensBuilder {
    token_types: Vec<String>,
    token_modifiers: Vec<String>,
    /// Absolute tokens: (line, start, length, type index, modifier bits).
    tokens: Vec<(u32, u32, u32, u32, u32)>,
}

impl SemanticTokensBuilder {
    /// Build against the legend the server advertised in
    /// [`SemanticTokensOptions::legend`]; token names passed to
    /// [`push`](Self::push) resolve to indices in it.
    pub fn new(legend: &SemanticTokensLegend) -> Self {
        SemanticTokensBuilder {
            token_types: legend.token_types.clone(),
            token_modifiers: legend.token_modifiers.clone(),
            tokens: Vec::new(),
        }
    }

    /// Add a single-line token at an absolute position, resolving
    /// `token_type` and `modifiers` names against the legend. Fails if a
    /// name is not in the legend (a mismatch would silently mis-colour
    /// every later token on the client).
    pub fn push(
        &mut self,
        line: u32,
        start: u32,
        length: u32,
        token_type: &str,
        modifiers: &[&str],
    ) -> crate::error::Result<()> {
        let type_index = self
            .token_types
            .iter()
            .position(|t| t == token_type)
            .ok_or_else(|| {
                crate::error::Error::internal(format!(
                    "token type {token_type:?} is not in the legend"
                ))
            })? as u32;
        let mut bits = 0u32;
        for modifier in modifiers {
            let index = self
                .token_modifiers
                .iter()
                .position(|m| m == modifier)
                .ok_or_else(|| {
                    crate::error::Error::internal(format!(
                        "token modifier {modifier:?} is not in the legend"
                    ))
                })?;
            bits |= 1 << index;
        }
        self.push_raw(line, start, length, type_index, bits);
        Ok(())
    }

    /// Add a token by pre-resolved legend index and modifier bitmask.
    pub fn push_raw(
        &mut self,
        line: u32,
        start: u32,
        length: u32,
        type_index: u32,
        modifier_bits: u32,
    ) {
        self.tokens
            .push((line, start, length, type_index, modifier_bits));
    }

    /// Add a token covering `range` in `text`, splitting a multi-line range
    /// into one token per line (semantic tokens are single-line by
    /// definition). Columns and lengths are measured in `encoding` units.
    pub fn push_range(
        &mut self,
        text: &str,
        index: &crate::text::LineIndex,
        range: crate::lsp::Range,
        encoding: crate::lsp::PositionEncodingKind,
        token_type: &str,
        modifiers: &[&str],
    ) -> crate::error::Result<()> {
        use crate::text::byte_to_column;
        for line in range.start.line..=range.end.line {
            let Some(line_start) = index.line_start(line) else {
                break;
            };
            let line_end = index.line_start(line + 1).map_or(text.len(), |n| n - 1);
            let line_text = &text[line_start..line_end];
            let line_units = byte_to_column(line_text, line_text.len(), encoding);
            let start = if line == range.start.line {
                range.start.character.min(line_units)
            } else {
                0
            };
            let end = if line == range.end.line {
                range.end.character.min(line_units)
            } else {
                line_units
            };
            if end > start {
                self.push(line, start, end - start, token_type, modifiers)?;
            }
        }
        Ok(())
    }

    /// Sort the tokens into document order and emit the delta-encoded
    /// [`SemanticTokens`].
    pub fn build(mut self, result_id: Option<String>) -> SemanticTokens {
        self.tokens.sort_unstable();
        let mut data = Vec::with_capacity(self.tokens.len() * 5);
        let (mut previous_line, mut previous_start) = (0u32, 0u32);
        for (line, start, length, type_index, modifier_bits) in self.tokens {
            let delta_line = line - previous_line;
            let delta_start = if delta_line == 0 {
                start - previous_start
            } else {
                start
            };
            data.extend_from_slice(&[delta_line, delta_start, length, type_index, modifier_bits]);
            (previous_line, previous_start) = (line, start);
        }
        SemanticTokens { result_id, data }
    }
}

#[cfg(test)]
mod builder_tests {
    use super::*;
    use crate::lsp::{Position, PositionEncodingKind, Range};
    use crate::text::LineIndex;

    fn legend() -> SemanticTokensLegend {
        SemanticTokensLegend {
            token_types: vec!["keyword".into(), "variable".into()],
            token_modifiers: vec!["declaration".into(), "readonly".into()],
        }
    }

    #[test]
    fn delta_encodes_across_lines_and_within_a_line() {
        let mut builder = SemanticTokensBuilder::new(&legend());
        builder.push(2, 10, 4, "variable", &["readonly"]).unwrap();
        builder.push(0, 0, 3, "keyword", &[]).unwrap();
        builder.push(2, 4, 2, "keyword", &[]).unwrap();
        let tokens = builder.build(Some("r1".into()));
        assert_eq!(tokens.result_id.as_deref(), Some("r1"));
        assert_eq!(
            tokens.data,
            [
                0, 0, 3, 0, 0, // line 0 col 0: keyword
                2, 4, 2, 0, 0, // +2 lines, col 4: keyword
                0, 6, 4, 1, 2, // same line, +6 cols: variable, readonly bit
            ]
        );
    }

    #[test]
    fn unknown_names_are_rejected() {
        let mut builder = SemanticTokensBuilder::new(&legend());
        assert!(builder.push(0, 0, 1, "nope", &[]).is_err());
        assert!(builder.push(0, 0, 1, "keyword", &["nope"]).is_err());
    }

    #[test]
    fn push_range_splits_multi_line_tokens() {
        // A "comment" spanning three lines, with a multi-byte char on line 1.
        let legend = SemanticTokensLegend {
            token_types: vec!["comment".into()],
            token_modifiers: vec![],
        };
        let text = "abc\ndé\nxyz";
        let index = LineIndex::new(text);
        let mut builder = SemanticTokensBuilder::new(&legend);
        builder
            .push_range(
                text,
                &index,
                Range::new(Position::new(0, 1), Position::new(2, 2)),
                PositionEncodingKind::Utf16,
                "comment",
                &[],
            )
            .unwrap();
        let tokens = builder.build(None);
        assert_eq!(
            tokens.data,
            [
                0, 1, 2, 0, 0, // line 0: cols 1..3
                1, 0, 2, 0, 0, // line 1: cols 0..2 ("dé" is 2 UTF-16 units)
                1, 0, 2, 0, 0, // line 2: cols 0..2
            ]
        );
    }
}
