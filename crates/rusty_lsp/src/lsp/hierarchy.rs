//! `textDocument/prepareCallHierarchy`, `callHierarchy/incomingCalls`,
//! `callHierarchy/outgoingCalls`, `textDocument/prepareTypeHierarchy`,
//! `typeHierarchy/supertypes`, and `typeHierarchy/subtypes` types (LSP 3.16+
//! for call hierarchy, 3.17+ for type hierarchy).
//!
//! Both hierarchies follow the same two-step shape: a `prepare*` request
//! anchored at a text position returns the item(s) under the cursor, then
//! `incomingCalls`/`outgoingCalls` (or `supertypes`/`subtypes`) walk the
//! hierarchy from a previously returned item.

use super::base::{Range, TextDocumentPositionParams, Uri};
use super::enums::{SymbolKind, SymbolTag};
use super::progress::{PartialResultParams, WorkDoneProgressParams};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Parameters of `textDocument/prepareCallHierarchy`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallHierarchyPrepareParams {
    #[serde(flatten)]
    pub text_document_position: TextDocumentPositionParams,
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
}

/// One symbol reachable through the call hierarchy: returned by
/// `prepareCallHierarchy` and carried in [`CallHierarchyIncomingCall::from`]
/// / [`CallHierarchyOutgoingCall::to`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallHierarchyItem {
    /// The symbol's name.
    pub name: String,
    /// The symbol's kind.
    pub kind: SymbolKind,
    /// Extra tags for this symbol (e.g. deprecated).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<SymbolTag>,
    /// Additional detail, e.g. the symbol's signature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The document containing the symbol.
    pub uri: Uri,
    /// The symbol's full range (e.g. a function's entire body).
    pub range: Range,
    /// The range that should be selected/highlighted when navigating to this
    /// symbol, a sub-range of [`range`](Self::range).
    pub selection_range: Range,
    /// Opaque data round-tripped back on `incomingCalls`/`outgoingCalls`
    /// requests for this item, useful for avoiding a second lookup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Parameters of `callHierarchy/incomingCalls`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallHierarchyIncomingCallsParams {
    /// The item to find callers of.
    pub item: CallHierarchyItem,
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result: PartialResultParams,
}

/// One caller of the item requested via [`CallHierarchyIncomingCallsParams`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallHierarchyIncomingCall {
    /// The calling symbol.
    pub from: CallHierarchyItem,
    /// The ranges at which `from` calls the requested item; usually one, but
    /// a symbol may call it more than once.
    pub from_ranges: Vec<Range>,
}

/// Parameters of `callHierarchy/outgoingCalls`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallHierarchyOutgoingCallsParams {
    /// The item to find callees of.
    pub item: CallHierarchyItem,
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result: PartialResultParams,
}

/// One callee of the item requested via [`CallHierarchyOutgoingCallsParams`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallHierarchyOutgoingCall {
    /// The called symbol.
    pub to: CallHierarchyItem,
    /// The ranges at which the requested item calls `to`.
    pub from_ranges: Vec<Range>,
}

/// Parameters of `textDocument/prepareTypeHierarchy`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeHierarchyPrepareParams {
    #[serde(flatten)]
    pub text_document_position: TextDocumentPositionParams,
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
}

/// One symbol reachable through the type hierarchy: returned by
/// `prepareTypeHierarchy` and carried in `supertypes`/`subtypes` requests
/// and results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeHierarchyItem {
    /// The type's name.
    pub name: String,
    /// The type's kind.
    pub kind: SymbolKind,
    /// Extra tags for this type (e.g. deprecated).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<SymbolTag>,
    /// Additional detail, e.g. the type's fully qualified name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The document containing the type.
    pub uri: Uri,
    /// The type's full range (e.g. the whole class body).
    pub range: Range,
    /// The range that should be selected/highlighted when navigating to this
    /// type, a sub-range of [`range`](Self::range).
    pub selection_range: Range,
    /// Opaque data round-tripped back on `supertypes`/`subtypes` requests for
    /// this item, useful for avoiding a second lookup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Parameters of `typeHierarchy/supertypes`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeHierarchySupertypesParams {
    /// The item to find supertypes of.
    pub item: TypeHierarchyItem,
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result: PartialResultParams,
}

/// Parameters of `typeHierarchy/subtypes`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeHierarchySubtypesParams {
    /// The item to find subtypes of.
    pub item: TypeHierarchyItem,
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result: PartialResultParams,
}

/// Options describing the server's call-hierarchy support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallHierarchyOptions {
    /// Whether the server reports work-done progress for this provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_done_progress: Option<bool>,
}

/// Options describing the server's type-hierarchy support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeHierarchyOptions {
    /// Whether the server reports work-done progress for this provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_done_progress: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::base::{Position, TextDocumentIdentifier};
    use serde_json::json;

    fn range() -> Range {
        Range::new(Position::new(0, 0), Position::new(0, 1))
    }

    #[test]
    fn call_hierarchy_prepare_params_flattens_position() {
        let params = CallHierarchyPrepareParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: "file:///a".into(),
                },
                position: Position::new(1, 2),
            },
            work_done: Default::default(),
        };
        let value = serde_json::to_value(&params).unwrap();
        assert_eq!(
            value,
            json!({"textDocument": {"uri": "file:///a"}, "position": {"line": 1, "character": 2}})
        );
    }

    #[test]
    fn call_hierarchy_item_uses_camel_case() {
        let item = CallHierarchyItem {
            name: "foo".to_owned(),
            kind: SymbolKind::Function,
            tags: vec![],
            detail: None,
            uri: "file:///a".into(),
            range: range(),
            selection_range: range(),
            data: None,
        };
        let value = serde_json::to_value(&item).unwrap();
        assert_eq!(value["selectionRange"], value["range"]);
        assert!(value.get("tags").is_none());
        assert!(value.get("detail").is_none());
    }

    #[test]
    fn call_hierarchy_incoming_call_round_trips() {
        let call = CallHierarchyIncomingCall {
            from: CallHierarchyItem {
                name: "caller".to_owned(),
                kind: SymbolKind::Function,
                tags: vec![],
                detail: None,
                uri: "file:///a".into(),
                range: range(),
                selection_range: range(),
                data: None,
            },
            from_ranges: vec![range()],
        };
        let value = serde_json::to_value(&call).unwrap();
        assert_eq!(value["from"]["name"], json!("caller"));
        assert_eq!(
            value["fromRanges"][0],
            serde_json::to_value(range()).unwrap()
        );
    }

    #[test]
    fn type_hierarchy_item_uses_camel_case() {
        let item = TypeHierarchyItem {
            name: "Foo".to_owned(),
            kind: SymbolKind::Class,
            tags: vec![],
            detail: Some("mod::Foo".to_owned()),
            uri: "file:///a".into(),
            range: range(),
            selection_range: range(),
            data: None,
        };
        let value = serde_json::to_value(&item).unwrap();
        assert_eq!(value["selectionRange"], value["range"]);
        assert_eq!(value["detail"], json!("mod::Foo"));
    }
}
