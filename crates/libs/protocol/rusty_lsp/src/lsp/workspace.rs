//! `workspace/*` types: multi-root workspace folders, pulling client
//! configuration, applying edits back to the client's buffers, watched-file
//! change notifications, and command execution.

use super::base::{Range, Uri};
use super::enums::FileChangeType;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;

/// A single root folder in a (possibly multi-root) workspace.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkspaceFolder {
    /// The folder's URI.
    pub uri: Uri,
    /// The folder's display name.
    pub name: String,
}

/// Parameters of the `workspace/didChangeWorkspaceFolders` notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DidChangeWorkspaceFoldersParams {
    /// The added/removed folders.
    pub event: WorkspaceFoldersChangeEvent,
}

/// The folders added and removed since the last known state.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WorkspaceFoldersChangeEvent {
    /// Folders added to the workspace.
    pub added: Vec<WorkspaceFolder>,
    /// Folders removed from the workspace.
    pub removed: Vec<WorkspaceFolder>,
}

/// Parameters of a server-to-client `workspace/configuration` request: asks
/// for the value of one or more configuration sections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigurationParams {
    /// The sections being requested.
    pub items: Vec<ConfigurationItem>,
}

/// One configuration section to look up, optionally scoped to a resource.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurationItem {
    /// Restrict the lookup to this resource, if given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_uri: Option<Uri>,
    /// The dotted configuration section to read (e.g. `"editor.tabSize"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
}

/// A single textual edit: replace [`range`](Self::range) with
/// [`new_text`](Self::new_text).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextEdit {
    /// The range to replace.
    pub range: Range,
    /// The replacement text (empty string to delete the range).
    pub new_text: String,
}

impl TextEdit {
    /// Build a text edit from a range and its replacement text.
    pub fn new(range: Range, new_text: impl Into<String>) -> Self {
        TextEdit {
            range,
            new_text: new_text.into(),
        }
    }
}

/// A set of edits across one or more documents, sent to
/// `workspace/applyEdit` (and returned from `rename`, `willRenameFiles`,
/// code-action edits, …).
///
/// Two forms exist: the simple [`changes`](Self::changes) map, and the
/// richer [`document_changes`](Self::document_changes) list, which supports
/// versioned edits, file creates/renames/deletes, and change annotations.
/// Servers should prefer `document_changes` when the client advertises
/// `workspace.workspaceEdit.documentChanges`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceEdit {
    /// Per-document lists of edits to apply (the simple form).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changes: Option<HashMap<Uri, Vec<TextEdit>>>,
    /// Ordered document edits and file operations (the rich form). Applied
    /// in order; a versioned edit lets the client refuse edits computed
    /// against stale text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_changes: Option<Vec<DocumentChange>>,
    /// Descriptions for the annotation ids referenced by annotated edits
    /// and file operations, keyed by id (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_annotations: Option<HashMap<String, ChangeAnnotation>>,
    /// Any additional fields not modelled above.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl WorkspaceEdit {
    /// Build a `changes`-form edit from a single document's edit list.
    pub fn for_document(uri: Uri, edits: Vec<TextEdit>) -> Self {
        let mut changes = HashMap::new();
        changes.insert(uri, edits);
        WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        }
    }

    /// Build a `documentChanges`-form edit from an ordered list of document
    /// edits and file operations.
    pub fn with_document_changes(document_changes: Vec<DocumentChange>) -> Self {
        WorkspaceEdit {
            document_changes: Some(document_changes),
            ..Default::default()
        }
    }
}

/// One entry of [`WorkspaceEdit::document_changes`]: a textual edit to an
/// (optionally versioned) document, or a file create/rename/delete.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DocumentChange {
    /// Textual edits to one document.
    Edit(TextDocumentEdit),
    /// A file create/rename/delete operation.
    Operation(ResourceOperation),
}

impl From<TextDocumentEdit> for DocumentChange {
    fn from(edit: TextDocumentEdit) -> Self {
        DocumentChange::Edit(edit)
    }
}

impl From<ResourceOperation> for DocumentChange {
    fn from(op: ResourceOperation) -> Self {
        DocumentChange::Operation(op)
    }
}

/// Textual edits scoped to one document, optionally pinned to the version
/// they were computed against.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextDocumentEdit {
    /// The document (and expected version) the edits apply to.
    pub text_document: OptionalVersionedTextDocumentIdentifier,
    /// The edits, possibly annotated.
    pub edits: Vec<AnnotatedTextEdit>,
}

impl TextDocumentEdit {
    /// Build a document edit. Pass the version the edits were computed
    /// against (letting the client refuse stale edits), or `None` to apply
    /// regardless.
    pub fn new(uri: Uri, version: Option<i32>, edits: Vec<TextEdit>) -> Self {
        TextDocumentEdit {
            text_document: OptionalVersionedTextDocumentIdentifier { uri, version },
            edits: edits.into_iter().map(AnnotatedTextEdit::from).collect(),
        }
    }
}

/// Identifies a document plus the version an edit expects it to be at;
/// `version: None` (JSON `null`) means "apply regardless of version".
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OptionalVersionedTextDocumentIdentifier {
    /// The document URI.
    pub uri: Uri,
    /// The expected version, or `None` to skip the check. Required on the
    /// wire (as `null` when absent), per the spec.
    pub version: Option<i32>,
}

/// A [`TextEdit`] optionally tagged with a change-annotation id (LSP 3.16).
/// With no id it is wire-identical to a plain `TextEdit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnnotatedTextEdit {
    /// The range to replace.
    pub range: Range,
    /// The replacement text.
    pub new_text: String,
    /// The id of a [`ChangeAnnotation`] in
    /// [`WorkspaceEdit::change_annotations`] describing this edit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotation_id: Option<String>,
}

impl From<TextEdit> for AnnotatedTextEdit {
    fn from(edit: TextEdit) -> Self {
        AnnotatedTextEdit {
            range: edit.range,
            new_text: edit.new_text,
            annotation_id: None,
        }
    }
}

/// A file create/rename/delete inside [`WorkspaceEdit::document_changes`],
/// discriminated on the wire by its `kind` field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ResourceOperation {
    /// Create a new file.
    Create(CreateFile),
    /// Rename a file.
    Rename(RenameFile),
    /// Delete a file.
    Delete(DeleteFile),
}

/// Create a file as part of a [`WorkspaceEdit`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFile {
    /// The file to create.
    pub uri: Uri,
    /// Behaviour when the file already exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<CreateFileOptions>,
    /// The id of a [`ChangeAnnotation`] describing this operation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotation_id: Option<String>,
}

/// Options of a [`CreateFile`] operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFileOptions {
    /// Overwrite an existing file. Wins over `ignore_if_exists`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overwrite: Option<bool>,
    /// Do nothing if the file already exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ignore_if_exists: Option<bool>,
}

/// Rename a file as part of a [`WorkspaceEdit`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameFile {
    /// The file to rename.
    pub old_uri: Uri,
    /// The new location.
    pub new_uri: Uri,
    /// Behaviour when the target already exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<CreateFileOptions>,
    /// The id of a [`ChangeAnnotation`] describing this operation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotation_id: Option<String>,
}

/// Delete a file as part of a [`WorkspaceEdit`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteFile {
    /// The file to delete.
    pub uri: Uri,
    /// Behaviour for folders and missing files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<DeleteFileOptions>,
    /// The id of a [`ChangeAnnotation`] describing this operation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotation_id: Option<String>,
}

/// Options of a [`DeleteFile`] operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteFileOptions {
    /// Delete folder contents recursively.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recursive: Option<bool>,
    /// Do nothing if the file does not exist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ignore_if_not_exists: Option<bool>,
}

/// A description of a group of changes within a [`WorkspaceEdit`]
/// (LSP 3.16), shown by clients that let the user confirm edits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeAnnotation {
    /// A human-readable label for the change group.
    pub label: String,
    /// Whether the client should ask the user to confirm before applying.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs_confirmation: Option<bool>,
    /// A longer description of the change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Parameters of the server-to-client `workspace/applyEdit` request.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ApplyWorkspaceEditParams {
    /// An optional label describing the edit, for undo history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The edit to apply.
    pub edit: WorkspaceEdit,
}

/// Result of a `workspace/applyEdit` request.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyWorkspaceEditResult {
    /// Whether the edit was applied. May be `false` if the client rejected
    /// it or a document's content did not match expectations.
    pub applied: bool,
    /// A human-readable explanation when `applied` is `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    /// The index of the change that failed to apply, if identifiable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_change: Option<u32>,
}

/// Parameters of the `workspace/didChangeConfiguration` notification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DidChangeConfigurationParams {
    /// The new configuration settings. The shape is entirely server-defined,
    /// so it round-trips as raw JSON.
    pub settings: Value,
}

/// Parameters of the `workspace/didChangeWatchedFiles` notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DidChangeWatchedFilesParams {
    /// The files that changed.
    pub changes: Vec<FileEvent>,
}

/// One watched file's change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEvent {
    /// The file's URI.
    pub uri: Uri,
    /// How the file changed.
    #[serde(rename = "type")]
    pub typ: FileChangeType,
}

/// Parameters of the `workspace/executeCommand` request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecuteCommandParams {
    /// The command identifier, one of the strings the server advertised in
    /// [`ExecuteCommandOptions::commands`].
    pub command: String,
    /// Arguments for the command, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Vec<Value>>,
}

/// Options describing the server's `workspace/executeCommand` support,
/// advertised in [`crate::lsp::ServerCapabilities::execute_command_provider`].
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteCommandOptions {
    /// Whether the server reports work-done progress for this provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_done_progress: Option<bool>,
    /// The command identifiers the server accepts.
    pub commands: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::super::base::Position;
    use super::*;
    use serde_json::json;

    #[test]
    fn workspace_edit_document_changes_are_typed() {
        let value = json!({
            "documentChanges": [
                {"textDocument": {"uri": "file:///a", "version": 1}, "edits": [
                    {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}}, "newText": "x"},
                ]},
                {"kind": "create", "uri": "file:///new.rs"},
                {"kind": "rename", "oldUri": "file:///a", "newUri": "file:///b"},
                {"kind": "delete", "uri": "file:///gone.rs", "options": {"recursive": true}},
            ],
        });
        let edit: WorkspaceEdit = serde_json::from_value(value.clone()).unwrap();
        assert!(edit.changes.is_none());
        let changes = edit.document_changes.as_ref().expect("typed");
        assert!(matches!(&changes[0], DocumentChange::Edit(e)
            if e.text_document.version == Some(1) && e.edits[0].new_text == "x"));
        assert!(matches!(
            &changes[1],
            DocumentChange::Operation(ResourceOperation::Create(c)) if c.uri == "file:///new.rs"
        ));
        assert!(matches!(
            &changes[2],
            DocumentChange::Operation(ResourceOperation::Rename(r)) if r.new_uri == "file:///b"
        ));
        assert!(matches!(
            &changes[3],
            DocumentChange::Operation(ResourceOperation::Delete(d))
                if d.options.and_then(|o| o.recursive) == Some(true)
        ));

        let round_tripped = serde_json::to_value(&edit).unwrap();
        assert_eq!(round_tripped["documentChanges"], value["documentChanges"]);
    }

    #[test]
    fn workspace_edit_with_document_changes_builder() {
        let edit = WorkspaceEdit::with_document_changes(vec![
            ResourceOperation::Create(CreateFile {
                uri: "file:///new.rs".into(),
                options: None,
                annotation_id: None,
            })
            .into(),
            TextDocumentEdit::new(
                "file:///new.rs".into(),
                None,
                vec![TextEdit::new(
                    Range::new(Position::new(0, 0), Position::new(0, 0)),
                    "// hello\n",
                )],
            )
            .into(),
        ]);
        let value = serde_json::to_value(&edit).unwrap();
        assert_eq!(value["documentChanges"][0]["kind"], json!("create"));
        // A versionless document edit still carries an explicit null version.
        assert_eq!(
            value["documentChanges"][1]["textDocument"]["version"],
            json!(null)
        );
        assert!(
            value["documentChanges"][1]["edits"][0]
                .get("annotationId")
                .is_none()
        );
    }

    #[test]
    fn workspace_edit_for_document_round_trips() {
        let edit = WorkspaceEdit::for_document(
            "file:///a".into(),
            vec![TextEdit::new(
                Range::new(Position::new(0, 0), Position::new(0, 1)),
                "x",
            )],
        );
        let value = serde_json::to_value(&edit).unwrap();
        assert_eq!(value["changes"]["file:///a"][0]["newText"], json!("x"));
    }

    #[test]
    fn file_event_type_uses_type_keyword() {
        let event = FileEvent {
            uri: "file:///a".into(),
            typ: FileChangeType::Changed,
        };
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value, json!({"uri": "file:///a", "type": 2}));
    }

    #[test]
    fn execute_command_options_advertise_work_done_progress() {
        let options = ExecuteCommandOptions {
            work_done_progress: Some(true),
            commands: vec!["my.command".to_owned()],
        };
        assert_eq!(
            serde_json::to_value(&options).unwrap(),
            json!({"workDoneProgress": true, "commands": ["my.command"]})
        );
        let options = ExecuteCommandOptions {
            work_done_progress: None,
            commands: vec![],
        };
        assert_eq!(
            serde_json::to_value(&options).unwrap(),
            json!({"commands": []})
        );
    }
}
