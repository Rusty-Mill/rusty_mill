//! `textDocument/moniker` types (LSP 3.16): stable symbol identifiers that
//! let tools correlate symbols across index formats (e.g. LSIF/SCIP) and
//! across projects.

use super::base::TextDocumentPositionParams;
use super::progress::{PartialResultParams, WorkDoneProgressParams};
use serde::{Deserialize, Serialize};

/// Parameters of `textDocument/moniker`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonikerParams {
    #[serde(flatten)]
    pub text_document_position: TextDocumentPositionParams,
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result: PartialResultParams,
}

/// How unique a moniker's identifier is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UniquenessLevel {
    /// Unique within the containing document only.
    Document,
    /// Unique within the containing project.
    Project,
    /// Unique within the publishing group (e.g. all projects of an org).
    Group,
    /// Unique within the moniker's scheme.
    Scheme,
    /// Globally unique.
    Global,
}

/// Whether the moniker names a symbol imported into, exported from, or local
/// to the project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MonikerKind {
    /// The moniker names a symbol imported into the project.
    Import,
    /// The moniker names a symbol exported from the project.
    Export,
    /// The moniker names a symbol local to the project (e.g. a local
    /// variable, made available for cross-document navigation only).
    Local,
}

/// A moniker: a stable, scheme-qualified identifier for a symbol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Moniker {
    /// The scheme the identifier is drawn from (e.g. `"tsc"`, `".NET"`).
    pub scheme: String,
    /// The identifier itself; opaque, interpreted within `scheme`.
    pub identifier: String,
    /// How unique the identifier is.
    pub unique: UniquenessLevel,
    /// Whether the symbol is imported, exported, or local.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<MonikerKind>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn moniker_serializes_string_enums_camel_case() {
        let moniker = Moniker {
            scheme: "tsc".to_owned(),
            identifier: "pkg:mod:sym".to_owned(),
            unique: UniquenessLevel::Global,
            kind: Some(MonikerKind::Export),
        };
        assert_eq!(
            serde_json::to_value(&moniker).unwrap(),
            json!({
                "scheme": "tsc",
                "identifier": "pkg:mod:sym",
                "unique": "global",
                "kind": "export",
            })
        );
    }

    #[test]
    fn moniker_params_flatten_position() {
        let params: MonikerParams = serde_json::from_value(json!({
            "textDocument": {"uri": "file:///a"},
            "position": {"line": 1, "character": 2},
        }))
        .unwrap();
        assert_eq!(params.text_document_position.position.line, 1);
    }
}
