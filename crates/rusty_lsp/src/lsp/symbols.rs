//! `textDocument/documentSymbol` and `workspace/symbol` types.

use super::base::{Location, Range, TextDocumentIdentifier, Uri};
use super::enums::{SymbolKind, SymbolTag};
use super::progress::{PartialResultParams, WorkDoneProgressParams};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Parameters of `textDocument/documentSymbol`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSymbolParams {
    /// The document to list symbols for.
    pub text_document: TextDocumentIdentifier,
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result: PartialResultParams,
}

/// Options describing the server's `textDocument/documentSymbol` support.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSymbolOptions {
    /// Whether the server reports work-done progress for this provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_done_progress: Option<bool>,
    /// A human-readable label for this symbol source, shown when a client
    /// merges results from multiple document-symbol providers (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// The result of a `textDocument/documentSymbol` request: either the modern
/// hierarchical form ([`DocumentSymbol`], preferred — nests methods inside
/// classes, etc.) or the older flat form ([`SymbolInformation`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DocumentSymbolResponse {
    /// Hierarchical symbols, nested via [`DocumentSymbol::children`].
    Nested(Vec<DocumentSymbol>),
    /// A flat list of symbols, each with its own [`Location`].
    Flat(Vec<SymbolInformation>),
}

impl From<Vec<DocumentSymbol>> for DocumentSymbolResponse {
    fn from(symbols: Vec<DocumentSymbol>) -> Self {
        DocumentSymbolResponse::Nested(symbols)
    }
}

impl From<Vec<SymbolInformation>> for DocumentSymbolResponse {
    fn from(symbols: Vec<SymbolInformation>) -> Self {
        DocumentSymbolResponse::Flat(symbols)
    }
}

/// A symbol within a document, optionally nesting child symbols (e.g. a
/// class's methods and fields).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSymbol {
    /// The symbol's name.
    pub name: String,
    /// Additional detail, e.g. a function's signature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The symbol's kind.
    pub kind: SymbolKind,
    /// Tags qualifying the symbol (e.g. deprecated) — the modern form
    /// clients render (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<SymbolTag>>,
    /// Whether the symbol is deprecated. Prefer `tags` where the client
    /// supports it; kept for older clients.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<bool>,
    /// The symbol's full range (e.g. a function's entire body), used for
    /// operations like "expand selection".
    pub range: Range,
    /// The range that should be selected/highlighted when navigating to this
    /// symbol (typically just its name), a sub-range of [`range`](Self::range).
    pub selection_range: Range,
    /// Nested child symbols.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<DocumentSymbol>>,
}

impl DocumentSymbol {
    /// Build a leaf symbol (no children) from its name, kind, and ranges.
    pub fn new(
        name: impl Into<String>,
        kind: SymbolKind,
        range: Range,
        selection_range: Range,
    ) -> Self {
        DocumentSymbol {
            name: name.into(),
            detail: None,
            kind,
            tags: None,
            deprecated: None,
            range,
            selection_range,
            children: None,
        }
    }
}

/// A symbol anchored to a specific document location — the flat,
/// non-hierarchical symbol representation used by both
/// `textDocument/documentSymbol` (older clients) and `workspace/symbol`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolInformation {
    /// The symbol's name.
    pub name: String,
    /// The symbol's kind.
    pub kind: SymbolKind,
    /// Tags qualifying the symbol (e.g. deprecated), preferred over
    /// `deprecated` by modern clients (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<SymbolTag>>,
    /// Whether the symbol is deprecated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<bool>,
    /// The symbol's location.
    pub location: Location,
    /// The name of the symbol containing this one (e.g. a method's class),
    /// if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_name: Option<String>,
}

impl SymbolInformation {
    /// Build a symbol from its name, kind, and location.
    pub fn new(name: impl Into<String>, kind: SymbolKind, location: Location) -> Self {
        SymbolInformation {
            name: name.into(),
            kind,
            tags: None,
            deprecated: None,
            location,
            container_name: None,
        }
    }
}

/// Parameters of `workspace/symbol`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceSymbolParams {
    /// The user's search query. An empty string requests all symbols the
    /// server is willing to return.
    pub query: String,
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result: PartialResultParams,
}

/// Result of `workspace/symbol`: either the pre-3.17 flat
/// [`SymbolInformation`] list or the 3.17 [`WorkspaceSymbol`] list (which
/// supports lazy range resolution via `workspaceSymbol/resolve`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WorkspaceSymbolResponse {
    /// The pre-3.17 form.
    Flat(Vec<SymbolInformation>),
    /// The 3.17 form, resolvable via `workspaceSymbol/resolve`.
    Full(Vec<WorkspaceSymbol>),
}

impl From<Vec<SymbolInformation>> for WorkspaceSymbolResponse {
    fn from(symbols: Vec<SymbolInformation>) -> Self {
        WorkspaceSymbolResponse::Flat(symbols)
    }
}

impl From<Vec<WorkspaceSymbol>> for WorkspaceSymbolResponse {
    fn from(symbols: Vec<WorkspaceSymbol>) -> Self {
        WorkspaceSymbolResponse::Full(symbols)
    }
}

/// A workspace symbol (LSP 3.17). Unlike [`SymbolInformation`], its location
/// may initially carry only a URI, with the precise range filled in later by
/// `workspaceSymbol/resolve`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSymbol {
    /// The symbol's name.
    pub name: String,
    /// The symbol's kind.
    pub kind: SymbolKind,
    /// Tags qualifying the symbol (e.g. deprecated).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<SymbolTag>>,
    /// The name of the symbol containing this one, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_name: Option<String>,
    /// Where the symbol lives; may be URI-only until resolved.
    pub location: WorkspaceSymbolLocation,
    /// Opaque data round-tripped through `workspaceSymbol/resolve`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl WorkspaceSymbol {
    /// Build a symbol from its name, kind, and (possibly URI-only) location.
    pub fn new(
        name: impl Into<String>,
        kind: SymbolKind,
        location: impl Into<WorkspaceSymbolLocation>,
    ) -> Self {
        WorkspaceSymbol {
            name: name.into(),
            kind,
            tags: None,
            container_name: None,
            location: location.into(),
            data: None,
        }
    }
}

/// A [`WorkspaceSymbol`]'s location: a full [`Location`], or just a document
/// URI for servers that resolve the range lazily.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WorkspaceSymbolLocation {
    /// A full location (URI + range).
    Full(Location),
    /// A URI-only location; the range arrives via `workspaceSymbol/resolve`.
    UriOnly {
        /// The document URI.
        uri: Uri,
    },
}

impl From<Location> for WorkspaceSymbolLocation {
    fn from(location: Location) -> Self {
        WorkspaceSymbolLocation::Full(location)
    }
}

impl From<Uri> for WorkspaceSymbolLocation {
    fn from(uri: Uri) -> Self {
        WorkspaceSymbolLocation::UriOnly { uri }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn range() -> Range {
        Range::new(
            super::super::base::Position::new(0, 0),
            super::super::base::Position::new(0, 1),
        )
    }

    #[test]
    fn document_symbol_response_is_untagged() {
        let nested = DocumentSymbolResponse::Nested(vec![DocumentSymbol::new(
            "foo",
            SymbolKind::Function,
            range(),
            range(),
        )]);
        let value = serde_json::to_value(&nested).unwrap();
        assert_eq!(value[0]["name"], json!("foo"));
        assert_eq!(value[0]["kind"], json!(12));
        assert!(value[0].get("location").is_none());

        let flat = DocumentSymbolResponse::Flat(vec![SymbolInformation::new(
            "bar",
            SymbolKind::Variable,
            Location {
                uri: "file:///a".into(),
                range: range(),
            },
        )]);
        let value = serde_json::to_value(&flat).unwrap();
        assert_eq!(value[0]["name"], json!("bar"));
        assert_eq!(value[0]["location"]["uri"], json!("file:///a"));
    }

    #[test]
    fn document_symbol_nests_children() {
        let class = DocumentSymbol {
            children: Some(vec![DocumentSymbol::new(
                "method",
                SymbolKind::Method,
                range(),
                range(),
            )]),
            ..DocumentSymbol::new("Class", SymbolKind::Class, range(), range())
        };
        let value = serde_json::to_value(&class).unwrap();
        assert_eq!(value["children"][0]["name"], json!("method"));
    }

    #[test]
    fn document_symbol_params_uses_camel_case() {
        let params = DocumentSymbolParams {
            text_document: TextDocumentIdentifier {
                uri: "file:///a".into(),
            },
            work_done: Default::default(),
            partial_result: Default::default(),
        };
        let value = serde_json::to_value(&params).unwrap();
        assert_eq!(value, json!({"textDocument": {"uri": "file:///a"}}));
    }

    #[test]
    fn document_symbol_options_advertise_label_and_work_done_progress() {
        let options = DocumentSymbolOptions {
            work_done_progress: Some(true),
            label: Some("Rust Analyzer".to_owned()),
        };
        assert_eq!(
            serde_json::to_value(&options).unwrap(),
            json!({"workDoneProgress": true, "label": "Rust Analyzer"})
        );
        assert_eq!(
            serde_json::to_value(DocumentSymbolOptions::default()).unwrap(),
            json!({})
        );
    }
}

/// Options describing the server's `workspace/symbol` support, advertised in
/// [`crate::lsp::ServerCapabilities::workspace_symbol_provider`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSymbolOptions {
    /// Whether the server reports work-done progress for this provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_done_progress: Option<bool>,
    /// Whether the server resolves ranges lazily via
    /// `workspaceSymbol/resolve`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolve_provider: Option<bool>,
}

#[cfg(test)]
mod workspace_symbol_options_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn advertises_work_done_progress() {
        let options = WorkspaceSymbolOptions {
            work_done_progress: Some(true),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(options).unwrap(),
            json!({"workDoneProgress": true})
        );
        assert_eq!(
            serde_json::to_value(WorkspaceSymbolOptions::default()).unwrap(),
            json!({})
        );
    }
}
