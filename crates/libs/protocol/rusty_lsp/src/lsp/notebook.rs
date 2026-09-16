//! `notebookDocument/didOpen`/`didChange`/`didSave`/`didClose` types (LSP
//! 3.17): synchronising a notebook (an ordered list of cells, each backed by
//! its own text document) rather than a single flat document.
//!
//! A notebook's cells are also opened/closed as ordinary text documents
//! (their URIs appear in [`DidOpenNotebookDocumentParams::cell_text_documents`]
//! and [`DidCloseNotebookDocumentParams::cell_text_documents`]), so
//! `textDocument/*` requests against a cell work unchanged; these types only
//! cover the notebook-level structure layered on top.

use super::base::{TextDocumentIdentifier, TextDocumentItem, Uri, VersionedTextDocumentIdentifier};
use super::document::TextDocumentContentChangeEvent;
use super::enums::NotebookCellKind;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A single cell within a [`NotebookDocument`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookCell {
    /// Whether this is a markup or code cell.
    pub kind: NotebookCellKind,
    /// The URI of the text document backing this cell's content.
    pub document: Uri,
    /// Cell-specific metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Map<String, Value>>,
    /// The result of the most recent execution of this cell, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_summary: Option<ExecutionSummary>,
}

/// The result of executing a [`NotebookCell`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionSummary {
    /// The cell's execution order among all cells executed so far.
    pub execution_order: u32,
    /// Whether the execution succeeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
}

/// A notebook document: an ordered list of cells plus notebook-level
/// metadata, identified by its own URI distinct from any cell's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookDocument {
    /// The notebook document's URI.
    pub uri: Uri,
    /// The notebook type, e.g. `"jupyter-notebook"`.
    pub notebook_type: String,
    /// The notebook's version, incremented on every change.
    pub version: i32,
    /// Notebook-level metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Map<String, Value>>,
    /// The notebook's cells, in display order.
    pub cells: Vec<NotebookCell>,
}

/// Identifies a notebook document by URI alone, without its version or
/// cells — used where the full [`NotebookDocument`] isn't needed (e.g.
/// `didSave`/`didClose`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotebookDocumentIdentifier {
    /// The notebook document's URI.
    pub uri: Uri,
}

/// Identifies a notebook document by URI and version, without its cells —
/// used by `didChange`, which carries the cell delta separately.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionedNotebookDocumentIdentifier {
    /// The notebook document's version, after the change it accompanies.
    pub version: i32,
    /// The notebook document's URI.
    pub uri: Uri,
}

/// Parameters of `notebookDocument/didOpen`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DidOpenNotebookDocumentParams {
    /// The notebook that was opened.
    pub notebook_document: NotebookDocument,
    /// The text documents backing each of the notebook's cells, opened
    /// alongside the notebook itself.
    pub cell_text_documents: Vec<TextDocumentItem>,
}

/// Parameters of `notebookDocument/didChange`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DidChangeNotebookDocumentParams {
    /// The notebook that changed, and its new version.
    pub notebook_document: VersionedNotebookDocumentIdentifier,
    /// The change itself.
    pub change: NotebookDocumentChangeEvent,
}

/// Parameters of `notebookDocument/didSave`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DidSaveNotebookDocumentParams {
    /// The notebook that was saved.
    pub notebook_document: NotebookDocumentIdentifier,
}

/// Parameters of `notebookDocument/didClose`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DidCloseNotebookDocumentParams {
    /// The notebook that was closed.
    pub notebook_document: NotebookDocumentIdentifier,
    /// The text documents backing each of the notebook's cells, closed
    /// alongside the notebook itself.
    pub cell_text_documents: Vec<TextDocumentIdentifier>,
}

/// The change carried by a `notebookDocument/didChange` notification: any
/// combination of notebook-level metadata, cell structure (added/removed/
/// reordered cells), cell-level metadata, and cell text content.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookDocumentChangeEvent {
    /// The notebook's new metadata, if it changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Map<String, Value>>,
    /// Changes to the notebook's cells, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cells: Option<NotebookDocumentCellChanges>,
}

/// The cell-related portion of a [`NotebookDocumentChangeEvent`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookDocumentCellChanges {
    /// Cells added, removed, or reordered, and the text documents opened or
    /// closed as a result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structure: Option<NotebookDocumentCellStructureChange>,
    /// Cells whose own data (e.g. metadata) changed, without a structural
    /// change to the notebook.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Vec<NotebookCell>>,
    /// Text content changes within cells, addressed by each cell's own
    /// document URI and version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_content: Option<Vec<NotebookCellTextContentChange>>,
}

/// A structural change to a notebook's cell list: `array` describes the
/// splice (matching [`Vec::splice`]'s `start`/`delete_count`/insert shape),
/// while `did_open`/`did_close` carry the text documents that were opened or
/// closed as a result — cells inserted here are also opened as text
/// documents, and cells removed here are also closed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookDocumentCellStructureChange {
    /// The splice into the notebook's cell array.
    pub array: NotebookCellArrayChange,
    /// Text documents opened for any newly inserted cells.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub did_open: Option<Vec<TextDocumentItem>>,
    /// Text documents closed for any removed cells.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub did_close: Option<Vec<TextDocumentIdentifier>>,
}

/// A splice into a notebook's cell array: remove `delete_count` cells
/// starting at `start`, then insert `cells` (if any) in their place.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookCellArrayChange {
    /// The index of the first removed cell.
    pub start: u32,
    /// How many cells starting at `start` were removed.
    pub delete_count: u32,
    /// Cells inserted at `start` in their place, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cells: Option<Vec<NotebookCell>>,
}

/// A text content change within one cell's backing document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookCellTextContentChange {
    /// The cell's text document and its new version.
    pub document: VersionedTextDocumentIdentifier,
    /// The edits to apply, in the same shape as `textDocument/didChange`.
    pub changes: Vec<TextDocumentContentChangeEvent>,
}

/// Options describing the server's notebook-document sync support.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookDocumentSyncOptions {
    /// Which notebooks/cells this registration applies to. Kept as raw JSON
    /// — the spec's `NotebookDocumentFilterWithNotebook |
    /// NotebookDocumentFilterWithCells` union is deep and rarely branched on
    /// by servers; build entries with `serde_json::json!`, e.g.
    /// `json!({"notebookType": "jupyter-notebook"})`.
    pub notebook_selector: Vec<Value>,
    /// Whether the server wants `notebookDocument/didSave` notifications.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn notebook_document_uses_camel_case() {
        let doc = NotebookDocument {
            uri: "file:///a.ipynb".into(),
            notebook_type: "jupyter-notebook".to_owned(),
            version: 1,
            metadata: None,
            cells: vec![NotebookCell {
                kind: NotebookCellKind::Code,
                document: "file:///a.ipynb#cell1".into(),
                metadata: None,
                execution_summary: None,
            }],
        };
        let value = serde_json::to_value(&doc).unwrap();
        assert_eq!(value["notebookType"], json!("jupyter-notebook"));
        assert_eq!(value["cells"][0]["kind"], json!(2));
        assert_eq!(
            value["cells"][0]["document"],
            json!("file:///a.ipynb#cell1")
        );
        assert!(value.get("metadata").is_none());
    }

    #[test]
    fn versioned_notebook_document_identifier_round_trips() {
        let id = VersionedNotebookDocumentIdentifier {
            version: 2,
            uri: "file:///a.ipynb".into(),
        };
        let value = serde_json::to_value(id).unwrap();
        assert_eq!(value, json!({"version": 2, "uri": "file:///a.ipynb"}));
    }

    #[test]
    fn did_open_params_use_camel_case() {
        let params = DidOpenNotebookDocumentParams {
            notebook_document: NotebookDocument {
                uri: "file:///a.ipynb".into(),
                notebook_type: "jupyter-notebook".to_owned(),
                version: 1,
                metadata: None,
                cells: vec![],
            },
            cell_text_documents: vec![],
        };
        let value = serde_json::to_value(&params).unwrap();
        assert!(value.get("notebookDocument").is_some());
        assert!(value.get("cellTextDocuments").is_some());
    }

    #[test]
    fn cell_structure_change_round_trips() {
        let change = NotebookDocumentChangeEvent {
            metadata: None,
            cells: Some(NotebookDocumentCellChanges {
                structure: Some(NotebookDocumentCellStructureChange {
                    array: NotebookCellArrayChange {
                        start: 1,
                        delete_count: 0,
                        cells: Some(vec![NotebookCell {
                            kind: NotebookCellKind::Markup,
                            document: "file:///a.ipynb#cell2".into(),
                            metadata: None,
                            execution_summary: None,
                        }]),
                    },
                    did_open: Some(vec![]),
                    did_close: None,
                }),
                data: None,
                text_content: None,
            }),
        };
        let value = serde_json::to_value(&change).unwrap();
        assert_eq!(
            value["cells"]["structure"]["array"]["deleteCount"],
            json!(0)
        );
        assert_eq!(
            value["cells"]["structure"]["array"]["cells"][0]["kind"],
            json!(1)
        );
        assert!(value["cells"]["structure"].get("didClose").is_none());
    }

    #[test]
    fn notebook_document_sync_options_round_trips() {
        let options = NotebookDocumentSyncOptions {
            notebook_selector: vec![json!({"notebookType": "jupyter-notebook"})],
            save: Some(true),
        };
        let value = serde_json::to_value(&options).unwrap();
        assert_eq!(
            value["notebookSelector"][0]["notebookType"],
            json!("jupyter-notebook")
        );
        assert_eq!(value["save"], json!(true));
    }
}
