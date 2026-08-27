//! Typed views over sub-trees of [`crate::lsp::ClientCapabilities`].
//!
//! [`ClientCapabilities`](crate::lsp::ClientCapabilities) itself stays a raw
//! JSON tree by design (see its module doc comment in `lifecycle.rs`) — a
//! server author can always fall back to
//! [`get`](crate::lsp::ClientCapabilities::get)/[`supports`](crate::lsp::ClientCapabilities::supports)
//! for anything not modelled here. The types in this module are an additive,
//! typed convenience layer on top of that same data: methods like
//! [`ClientCapabilities::workspace`](crate::lsp::ClientCapabilities::workspace)
//! parse the relevant sub-tree into one of these structs, tolerating
//! malformed or absent data by falling back to the type's `Default` rather
//! than erroring — client-supplied data is untrusted, and a slightly-off
//! capability announcement shouldn't be able to crash a server.

use super::code_action::CodeActionKind;
use super::enums::{
    CompletionItemKind, CompletionItemTag, DiagnosticTag, InsertTextMode, MarkupKind,
    PositionEncodingKind, PrepareSupportDefaultBehavior, SymbolKind, SymbolTag,
};
use super::ranges::FoldingRangeKind;
use serde::{Deserialize, Serialize};

/// The common `{ dynamicRegistration?: boolean }` shape shared by many
/// capability groups that support nothing beyond opting into dynamic
/// registration via `client/registerCapability`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DynamicRegistrationCapability {
    /// Whether the client supports dynamic registration for this method.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_registration: Option<bool>,
}

/// The common `{ refreshSupport?: boolean }` shape shared by capability
/// groups that support a server asking the client to re-pull results (e.g.
/// `workspace/semanticTokens/refresh`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshSupportCapability {
    /// Whether the client honors the corresponding `workspace/*/refresh` request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_support: Option<bool>,
}

/// The common `{ properties: string[] }` shape shared by capability groups
/// that support lazily resolving additional item properties (e.g.
/// `workspaceSymbol/resolve`, `codeAction/resolve`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveSupportCapability {
    /// The property names the client can resolve lazily.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<String>,
}

/// The common `{ valueSet: T[] }` shape shared by capability groups that
/// enumerate which values of a tag-like enum the client understands (e.g.
/// `workspace.symbol.tagSupport`, `textDocument.documentSymbol.tagSupport`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
// serde's automatic bound inference would otherwise require `T: Default`
// because of the `#[serde(default)]` field below, even though `Vec<T>`
// implements `Default` unconditionally — spelled out explicitly instead.
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
pub struct TagSupportCapability<T> {
    /// The tag values the client can render.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub value_set: Vec<T>,
}

// Manual `Default` instead of `#[derive(Default)]`: the derive would require
// `T: Default`, but the tag enums this is used with (e.g. `SymbolTag`) don't
// implement it — an empty `Vec<T>` needs no bound on `T` at all.
impl<T> Default for TagSupportCapability<T> {
    fn default() -> Self {
        TagSupportCapability {
            value_set: Vec::new(),
        }
    }
}

/// `ClientCapabilities.workspace`: capabilities that apply to the workspace
/// as a whole rather than to a single document.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceClientCapabilities {
    /// The client supports applying batch edits via `workspace/applyEdit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apply_edit: Option<bool>,
    /// Capabilities specific to `WorkspaceEdit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_edit: Option<WorkspaceEditClientCapabilities>,
    /// `workspace/didChangeConfiguration` capabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub did_change_configuration: Option<DynamicRegistrationCapability>,
    /// `workspace/didChangeWatchedFiles` capabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub did_change_watched_files: Option<DidChangeWatchedFilesClientCapabilities>,
    /// `workspace/symbol` capabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<WorkspaceSymbolClientCapabilities>,
    /// `workspace/executeCommand` capabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execute_command: Option<DynamicRegistrationCapability>,
    /// The client supports `workspace/workspaceFolders` (LSP 3.6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_folders: Option<bool>,
    /// The client supports `workspace/configuration` (LSP 3.6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration: Option<bool>,
    /// The client honors `workspace/semanticTokens/refresh` (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_tokens: Option<RefreshSupportCapability>,
    /// The client honors `workspace/codeLens/refresh` (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_lens: Option<RefreshSupportCapability>,
    /// File-operation notification/request support (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_operations: Option<FileOperationClientCapabilities>,
    /// The client honors `workspace/inlineValue/refresh` (LSP 3.17).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline_value: Option<RefreshSupportCapability>,
    /// The client honors `workspace/inlayHint/refresh` (LSP 3.17).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inlay_hint: Option<RefreshSupportCapability>,
    /// The client honors `workspace/diagnostic/refresh` (LSP 3.17).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<RefreshSupportCapability>,
}

/// `ClientCapabilities.workspace.workspaceEdit`: capabilities specific to
/// `WorkspaceEdit` construction and application.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceEditClientCapabilities {
    /// The client supports versioned document changes
    /// (`WorkspaceEdit::documentChanges`) over the plain `changes` map.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_changes: Option<bool>,
    /// Which resource operations (create/rename/delete) the client supports
    /// in a `WorkspaceEdit` (LSP 3.13).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resource_operations: Vec<ResourceOperationKind>,
    /// How the client behaves if a resource operation in a `WorkspaceEdit`
    /// fails partway through (LSP 3.13).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_handling: Option<FailureHandlingKind>,
    /// The client normalizes line endings when applying edits (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalizes_line_endings: Option<bool>,
    /// The client's support for `ChangeAnnotation`-grouped edits (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_annotation_support: Option<ChangeAnnotationSupportCapability>,
}

/// One kind of resource operation a `WorkspaceEdit` can contain (LSP 3.13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResourceOperationKind {
    /// A file/folder creation.
    Create,
    /// A file/folder rename.
    Rename,
    /// A file/folder deletion.
    Delete,
}

/// How the client behaves if a resource operation in a `WorkspaceEdit` fails
/// partway through (LSP 3.13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FailureHandlingKind {
    /// Applying stops; earlier changes are left as-is.
    Abort,
    /// All changes are applied transactionally: all-or-nothing.
    Transactional,
    /// The client applies changes then undoes them all if any operation
    /// fails.
    Undo,
    /// Text edits are applied transactionally; resource operations are
    /// best-effort.
    TextOnlyTransactional,
}

/// `ClientCapabilities.workspace.workspaceEdit.changeAnnotationSupport`
/// (LSP 3.16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeAnnotationSupportCapability {
    /// The client groups edits by `ChangeAnnotation::label` in its UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub groups_on_label: Option<bool>,
}

/// `ClientCapabilities.workspace.didChangeWatchedFiles`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DidChangeWatchedFilesClientCapabilities {
    /// Whether the client supports dynamic registration for this method.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_registration: Option<bool>,
    /// The client supports [`super::registration::RelativePattern`] globs
    /// (LSP 3.17).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relative_pattern_support: Option<bool>,
}

/// `ClientCapabilities.workspace.symbol`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSymbolClientCapabilities {
    /// Whether the client supports dynamic registration for this method.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_registration: Option<bool>,
    /// Which `SymbolKind`s the client understands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol_kind: Option<SymbolKindCapability>,
    /// Which `SymbolTag`s the client understands (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag_support: Option<TagSupportCapability<SymbolTag>>,
    /// Which `WorkspaceSymbol` properties the client can resolve lazily via
    /// `workspaceSymbol/resolve` (LSP 3.17).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolve_support: Option<ResolveSupportCapability>,
}

/// `ClientCapabilities.workspace.symbol.symbolKind`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolKindCapability {
    /// The `SymbolKind`s the client understands. Absent means the client
    /// only understands the original LSP 1.0 range (`File` through
    /// `Array`); a server should degrade unknown kinds outside the set the
    /// client reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_set: Option<Vec<SymbolKind>>,
}

/// `ClientCapabilities.workspace.fileOperations` (LSP 3.16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileOperationClientCapabilities {
    /// Whether the client supports dynamic registration for these methods.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_registration: Option<bool>,
    /// The client sends `workspace/didCreateFiles`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub did_create: Option<bool>,
    /// The client sends `workspace/willCreateFiles`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub will_create: Option<bool>,
    /// The client sends `workspace/didRenameFiles`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub did_rename: Option<bool>,
    /// The client sends `workspace/willRenameFiles`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub will_rename: Option<bool>,
    /// The client sends `workspace/didDeleteFiles`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub did_delete: Option<bool>,
    /// The client sends `workspace/willDeleteFiles`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub will_delete: Option<bool>,
}

/// `ClientCapabilities.window`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowClientCapabilities {
    /// The client supports `window/workDoneProgress/create` (LSP 3.15).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_done_progress: Option<bool>,
    /// `window/showMessageRequest` capabilities (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_message: Option<ShowMessageClientCapabilities>,
    /// `window/showDocument` capabilities (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_document: Option<ShowDocumentClientCapabilities>,
}

/// `ClientCapabilities.window.showMessage` (LSP 3.16).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShowMessageClientCapabilities {
    /// Properties the client supports on `MessageActionItem`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_action_item: Option<MessageActionItemCapability>,
}

/// `ClientCapabilities.window.showMessage.messageActionItem` (LSP 3.16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageActionItemCapability {
    /// The client honors extra properties on `MessageActionItem` beyond
    /// `title`, round-tripping them back in `window/showMessageRequest`'s
    /// response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additional_properties_support: Option<bool>,
}

/// `ClientCapabilities.window.showDocument` (LSP 3.16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShowDocumentClientCapabilities {
    /// The client supports `window/showDocument`. Unlike sibling capability
    /// flags, the spec marks this required (not optional) once the
    /// `showDocument` object itself is present.
    pub support: bool,
}

/// `ClientCapabilities.general` (LSP 3.16).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneralClientCapabilities {
    /// The client's support for reviving a request whose response arrived
    /// after a `ContentModified` error (LSP 3.17).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale_request_support: Option<StaleRequestSupportCapability>,
    /// The client's regular-expression engine, so a server can avoid
    /// constructs it doesn't support (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regular_expressions: Option<RegularExpressionsClientCapabilities>,
    /// The client's Markdown parser, for the same reason (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub markdown: Option<MarkdownClientCapabilities>,
    /// The position encodings the client supports, in decreasing preference
    /// order (LSP 3.17). See also
    /// [`ClientCapabilities::position_encodings`](super::lifecycle::ClientCapabilities::position_encodings),
    /// which reads this same data with the spec's "absent means `[utf-16]`"
    /// default already applied.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub position_encodings: Vec<PositionEncodingKind>,
}

/// `ClientCapabilities.general.staleRequestSupport` (LSP 3.17).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StaleRequestSupportCapability {
    /// The client retries a cancelled request that the server reports as
    /// `ContentModified`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel: Option<bool>,
    /// The method names the client retries on `ContentModified`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub retry_on_content_modified: Vec<String>,
}

/// `ClientCapabilities.general.regularExpressions` (LSP 3.16).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegularExpressionsClientCapabilities {
    /// The regex engine's name (e.g. `"ECMAScript"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    /// The engine's version string, if relevant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// `ClientCapabilities.general.markdown` (LSP 3.16).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkdownClientCapabilities {
    /// The Markdown parser's name (e.g. `"marked"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parser: Option<String>,
    /// The parser's version string, if relevant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// HTML tags the parser allows through unescaped (LSP 3.17).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tags: Vec<String>,
}

/// `ClientCapabilities.notebookDocument` (LSP 3.17).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookDocumentClientCapabilities {
    /// Notebook document synchronization capabilities — the spec's only
    /// member of `notebookDocument` today.
    #[serde(default)]
    pub synchronization: NotebookDocumentSyncClientCapabilities,
}

/// `ClientCapabilities.notebookDocument.synchronization` (LSP 3.17).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookDocumentSyncClientCapabilities {
    /// Whether the client supports dynamic registration for notebook
    /// document sync.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_registration: Option<bool>,
    /// The client shows a cell's `ExecutionSummary` in its notebook UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_summary_support: Option<bool>,
}

/// The common `{ dynamicRegistration?: boolean; linkSupport?: boolean }`
/// shape shared by the four "go to" capability groups (`declaration`,
/// `definition`, `typeDefinition`, `implementation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GotoClientCapabilities {
    /// Whether the client supports dynamic registration for this method.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_registration: Option<bool>,
    /// The client can render `LocationLink` results (a link with an
    /// `originSelectionRange`/`targetSelectionRange`) instead of a plain
    /// `Location` (LSP 3.14).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_support: Option<bool>,
}

/// `ClientCapabilities.textDocument`: capabilities specific to a single
/// document, as opposed to the workspace as a whole. Models the full spec
/// surface: document sync, completion, hover, signature help, the "go to"
/// family, references, document highlight, document symbol, code action,
/// code lens, document link, color provider, the formatting family, rename,
/// folding range, selection range, publish-diagnostics, call hierarchy,
/// semantic tokens, linked-editing range, moniker, type hierarchy, inline
/// value, inlay hint, and diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextDocumentClientCapabilities {
    /// `textDocument/didOpen`/`didChange`/`didClose`/`willSave`/
    /// `willSaveWaitUntil`/`didSave` capabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synchronization: Option<TextDocumentSyncClientCapabilities>,
    /// `textDocument/completion` capabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion: Option<CompletionClientCapabilities>,
    /// `textDocument/hover` capabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hover: Option<HoverClientCapabilities>,
    /// `textDocument/signatureHelp` capabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature_help: Option<SignatureHelpClientCapabilities>,
    /// `textDocument/declaration` capabilities (LSP 3.14).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declaration: Option<GotoClientCapabilities>,
    /// `textDocument/definition` capabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<GotoClientCapabilities>,
    /// `textDocument/typeDefinition` capabilities (LSP 3.6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_definition: Option<GotoClientCapabilities>,
    /// `textDocument/implementation` capabilities (LSP 3.6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implementation: Option<GotoClientCapabilities>,
    /// `textDocument/references` capabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub references: Option<DynamicRegistrationCapability>,
    /// `textDocument/documentHighlight` capabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_highlight: Option<DynamicRegistrationCapability>,
    /// `textDocument/documentSymbol` capabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_symbol: Option<DocumentSymbolClientCapabilities>,
    /// `textDocument/codeAction` capabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_action: Option<CodeActionClientCapabilities>,
    /// `textDocument/codeLens` capabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_lens: Option<DynamicRegistrationCapability>,
    /// `textDocument/documentLink` capabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_link: Option<DocumentLinkClientCapabilities>,
    /// `textDocument/documentColor`/`colorPresentation` capabilities
    /// (LSP 3.6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_provider: Option<DynamicRegistrationCapability>,
    /// `textDocument/formatting` capabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formatting: Option<DynamicRegistrationCapability>,
    /// `textDocument/rangeFormatting` capabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range_formatting: Option<DynamicRegistrationCapability>,
    /// `textDocument/onTypeFormatting` capabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_type_formatting: Option<DynamicRegistrationCapability>,
    /// `textDocument/rename` capabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rename: Option<RenameClientCapabilities>,
    /// `textDocument/foldingRange` capabilities (LSP 3.10).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folding_range: Option<FoldingRangeClientCapabilities>,
    /// `textDocument/selectionRange` capabilities (LSP 3.15).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_range: Option<DynamicRegistrationCapability>,
    /// `textDocument/publishDiagnostics` capabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publish_diagnostics: Option<PublishDiagnosticsClientCapabilities>,
    /// Call-hierarchy capabilities (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_hierarchy: Option<DynamicRegistrationCapability>,
    /// `textDocument/semanticTokens` capabilities (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_tokens: Option<SemanticTokensClientCapabilities>,
    /// `textDocument/linkedEditingRange` capabilities (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_editing_range: Option<DynamicRegistrationCapability>,
    /// `textDocument/moniker` capabilities (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moniker: Option<DynamicRegistrationCapability>,
    /// Type-hierarchy capabilities (LSP 3.17).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_hierarchy: Option<DynamicRegistrationCapability>,
    /// `textDocument/inlineValue` capabilities (LSP 3.17).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline_value: Option<DynamicRegistrationCapability>,
    /// `textDocument/inlayHint` capabilities (LSP 3.17).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inlay_hint: Option<InlayHintClientCapabilities>,
    /// `textDocument/diagnostic`/`workspace/diagnostic` capabilities
    /// (LSP 3.17).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<DiagnosticClientCapabilities>,
}

/// `ClientCapabilities.textDocument.synchronization`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextDocumentSyncClientCapabilities {
    /// Whether the client supports dynamic registration for document sync.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_registration: Option<bool>,
    /// The client sends `textDocument/willSave`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub will_save: Option<bool>,
    /// The client sends `textDocument/willSaveWaitUntil` and applies the
    /// returned edits before saving.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub will_save_wait_until: Option<bool>,
    /// The client sends `textDocument/didSave`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub did_save: Option<bool>,
}

/// `ClientCapabilities.textDocument.completion`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionClientCapabilities {
    /// Whether the client supports dynamic registration for completion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_registration: Option<bool>,
    /// Capabilities specific to an individual `CompletionItem`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_item: Option<CompletionItemCapability>,
    /// Which `CompletionItemKind`s the client understands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_item_kind: Option<CompletionItemKindCapability>,
    /// The insert-text mode the client uses when the server doesn't specify
    /// one on an item (LSP 3.17).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub insert_text_mode: Option<InsertTextMode>,
    /// The client sends `CompletionContext` (how completion was triggered)
    /// with its requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_support: Option<bool>,
    /// Capabilities specific to `CompletionList` (LSP 3.17).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_list: Option<CompletionListCapability>,
}

/// `ClientCapabilities.textDocument.completion.completionItem`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionItemCapability {
    /// The client interprets `CompletionItem::insert_text`/`text_edit` as an
    /// LSP snippet when `insert_text_format` says so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet_support: Option<bool>,
    /// The client supports `CompletionItem::commit_characters`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_characters_support: Option<bool>,
    /// Markup kinds the client accepts for `CompletionItem::documentation`,
    /// in decreasing preference order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub documentation_format: Vec<MarkupKind>,
    /// The client supports `CompletionItem::deprecated`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecated_support: Option<bool>,
    /// The client supports `CompletionItem::preselect`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preselect_support: Option<bool>,
    /// Which `CompletionItemTag`s the client understands (LSP 3.15).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag_support: Option<TagSupportCapability<CompletionItemTag>>,
    /// The client supports `CompletionItem::text_edit` being an
    /// `InsertReplaceEdit` (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub insert_replace_support: Option<bool>,
    /// Which `CompletionItem` properties the client can resolve lazily via
    /// `completionItem/resolve` (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolve_support: Option<ResolveSupportCapability>,
    /// Which `InsertTextMode`s the client honors on a per-item basis
    /// (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub insert_text_mode_support: Option<InsertTextModeSupportCapability>,
    /// The client renders `CompletionItem::label_details` (LSP 3.17).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_details_support: Option<bool>,
}

/// `ClientCapabilities.textDocument.completion.completionItem.insertTextModeSupport`
/// (LSP 3.16).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InsertTextModeSupportCapability {
    /// The `InsertTextMode`s the client honors.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub value_set: Vec<InsertTextMode>,
}

/// `ClientCapabilities.textDocument.completion.completionItemKind`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionItemKindCapability {
    /// The `CompletionItemKind`s the client understands. Absent means the
    /// client only understands the original LSP 1.0 range (`Text` through
    /// `Reference`); a server should degrade unknown kinds outside the set
    /// the client reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_set: Option<Vec<CompletionItemKind>>,
}

/// `ClientCapabilities.textDocument.completion.completionList` (LSP 3.17).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionListCapability {
    /// Which `CompletionList::item_defaults` keys the client understands.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub item_defaults: Vec<String>,
}

/// `ClientCapabilities.textDocument.hover`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HoverClientCapabilities {
    /// Whether the client supports dynamic registration for hover.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_registration: Option<bool>,
    /// Markup kinds the client accepts for `Hover::contents`, in decreasing
    /// preference order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content_format: Vec<MarkupKind>,
}

/// `ClientCapabilities.textDocument.signatureHelp`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureHelpClientCapabilities {
    /// Whether the client supports dynamic registration for signature help.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_registration: Option<bool>,
    /// Capabilities specific to `SignatureInformation`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature_information: Option<SignatureInformationCapability>,
    /// The client sends `SignatureHelpContext` with its requests (LSP 3.15).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_support: Option<bool>,
}

/// `ClientCapabilities.textDocument.signatureHelp.signatureInformation`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureInformationCapability {
    /// Markup kinds the client accepts for
    /// `SignatureInformation::documentation`, in decreasing preference
    /// order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub documentation_format: Vec<MarkupKind>,
    /// Capabilities specific to `ParameterInformation`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameter_information: Option<ParameterInformationCapability>,
    /// The client highlights `SignatureHelp::active_parameter` even when a
    /// signature has no `parameters` of its own (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_parameter_support: Option<bool>,
}

/// `ClientCapabilities.textDocument.signatureHelp.signatureInformation.parameterInformation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParameterInformationCapability {
    /// The client supports `ParameterInformation::label` as a
    /// `[start, end)` UTF-16 offset pair into the signature's label, instead
    /// of a plain substring.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_offset_support: Option<bool>,
}

/// `ClientCapabilities.textDocument.documentSymbol`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSymbolClientCapabilities {
    /// Whether the client supports dynamic registration for document
    /// symbols.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_registration: Option<bool>,
    /// Which `SymbolKind`s the client understands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol_kind: Option<SymbolKindCapability>,
    /// The client renders `DocumentSymbol`'s nested `children` as a tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hierarchical_document_symbol_support: Option<bool>,
    /// Which `SymbolTag`s the client understands (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag_support: Option<TagSupportCapability<SymbolTag>>,
    /// The client renders `DocumentSymbol::detail` as a label (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_support: Option<bool>,
}

/// `ClientCapabilities.textDocument.codeAction`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeActionClientCapabilities {
    /// Whether the client supports dynamic registration for code actions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_registration: Option<bool>,
    /// The client can restrict a request to specific `CodeActionKind`s and
    /// group results by kind (LSP 3.8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_action_literal_support: Option<CodeActionLiteralSupportCapability>,
    /// The client renders `CodeAction::is_preferred` (LSP 3.15).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_preferred_support: Option<bool>,
    /// The client renders `CodeAction::disabled` (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_support: Option<bool>,
    /// The client round-trips `CodeAction::data` through
    /// `codeAction/resolve` (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_support: Option<bool>,
    /// Which `CodeAction` properties the client can resolve lazily via
    /// `codeAction/resolve` (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolve_support: Option<ResolveSupportCapability>,
    /// The client applies `WorkspaceEdit::change_annotations` from a
    /// resolved code action (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub honors_change_annotations: Option<bool>,
}

/// `ClientCapabilities.textDocument.codeAction.codeActionLiteralSupport`
/// (LSP 3.8).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeActionLiteralSupportCapability {
    /// Which `CodeActionKind`s the client understands.
    #[serde(default)]
    pub code_action_kind: CodeActionKindCapability,
}

/// `ClientCapabilities.textDocument.codeAction.codeActionLiteralSupport.codeActionKind`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeActionKindCapability {
    /// The `CodeActionKind`s the client understands.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub value_set: Vec<CodeActionKind>,
}

/// `ClientCapabilities.textDocument.documentLink`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentLinkClientCapabilities {
    /// Whether the client supports dynamic registration for document links.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_registration: Option<bool>,
    /// The client shows `DocumentLink::tooltip` (LSP 3.15).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tooltip_support: Option<bool>,
}

/// `ClientCapabilities.textDocument.rename`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameClientCapabilities {
    /// Whether the client supports dynamic registration for rename.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_registration: Option<bool>,
    /// The client sends `textDocument/prepareRename` before renaming
    /// (LSP 3.12).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prepare_support: Option<bool>,
    /// Which `prepareRename` "default behavior" variants the client
    /// understands (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prepare_support_default_behavior: Option<PrepareSupportDefaultBehavior>,
    /// The client applies `WorkspaceEdit::change_annotations` from a rename
    /// edit (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub honors_change_annotations: Option<bool>,
}

/// `ClientCapabilities.textDocument.foldingRange` (LSP 3.10).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoldingRangeClientCapabilities {
    /// Whether the client supports dynamic registration for folding ranges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_registration: Option<bool>,
    /// The maximum number of folding ranges the client renders per
    /// document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range_limit: Option<u32>,
    /// The client only folds whole lines, ignoring `FoldingRange`'s
    /// character offsets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_folding_only: Option<bool>,
    /// Which `FoldingRangeKind`s the client understands (LSP 3.17).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folding_range_kind: Option<FoldingRangeKindCapability>,
    /// The client's support for `FoldingRange::collapsed_text` (LSP 3.17).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folding_range: Option<FoldingRangeCollapsedTextCapability>,
}

/// `ClientCapabilities.textDocument.foldingRange.foldingRangeKind`
/// (LSP 3.17).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoldingRangeKindCapability {
    /// The `FoldingRangeKind`s the client understands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_set: Option<Vec<FoldingRangeKind>>,
}

/// `ClientCapabilities.textDocument.foldingRange.foldingRange` (LSP 3.17) —
/// yes, the spec nests a member with the same name as its parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoldingRangeCollapsedTextCapability {
    /// The client renders `FoldingRange::collapsed_text`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collapsed_text: Option<bool>,
}

/// `ClientCapabilities.textDocument.publishDiagnostics`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishDiagnosticsClientCapabilities {
    /// The client renders `Diagnostic::related_information`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub related_information: Option<bool>,
    /// Which `DiagnosticTag`s the client understands (LSP 3.15).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag_support: Option<TagSupportCapability<DiagnosticTag>>,
    /// The client resets diagnostics when a document's version moves past
    /// the version a diagnostic was published for (LSP 3.15).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_support: Option<bool>,
    /// The client renders `Diagnostic::code_description` (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_description_support: Option<bool>,
    /// The client round-trips `Diagnostic::data` back to the server
    /// (LSP 3.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_support: Option<bool>,
}

/// Well-known [`TokenFormat`] values from the spec (currently just
/// `"relative"`). Like [`crate::lsp::CodeActionKind`], this is an open
/// string enum, so these are plain constants rather than a closed Rust enum.
pub mod token_format {
    /// Tokens are encoded relative to the previous token (the only format
    /// the spec currently defines).
    pub const RELATIVE: &str = "relative";
}

/// A [`TokenFormat`] value, e.g. `"relative"`. An open string enum per the
/// spec (see the [`token_format`] module for well-known values), not a
/// closed Rust enum.
pub type TokenFormat = String;

/// `ClientCapabilities.textDocument.semanticTokens` (LSP 3.16).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticTokensClientCapabilities {
    /// Whether the client supports dynamic registration for semantic
    /// tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_registration: Option<bool>,
    /// Which of `textDocument/semanticTokens/range` and `/full` (optionally
    /// with `/full/delta`) the client will request.
    #[serde(default)]
    pub requests: SemanticTokensRequestsCapability,
    /// The token types the client understands, beyond the spec's standard
    /// set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub token_types: Vec<String>,
    /// The token modifiers the client understands, beyond the spec's
    /// standard set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub token_modifiers: Vec<String>,
    /// The token-array encodings the client understands.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub formats: Vec<TokenFormat>,
    /// The client supports overlapping tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlapping_token_support: Option<bool>,
    /// The client supports tokens spanning multiple lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multiline_token_support: Option<bool>,
    /// The client can cancel a semantic-tokens request mid-computation and
    /// have the server terminate normally rather than send a full response
    /// (LSP 3.17).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_cancel_support: Option<bool>,
    /// The client merges this server's tokens with another semantic-tokens
    /// provider's instead of the two conflicting (LSP 3.17).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub augments_syntax_tokens: Option<bool>,
}

/// `ClientCapabilities.textDocument.semanticTokens.requests`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticTokensRequestsCapability {
    /// The client's support for `textDocument/semanticTokens/range`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<SemanticTokensRangeClientCapability>,
    /// The client's support for `textDocument/semanticTokens/full`
    /// (optionally `full/delta`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full: Option<SemanticTokensFullClientCapability>,
}

/// The `range` member of [`SemanticTokensRequestsCapability`]: a plain
/// boolean, or an empty object (semantically equivalent — the spec allows
/// both, reserving the object form for future extension).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SemanticTokensRangeClientCapability {
    /// `true`/`false`: range requests are (not) supported.
    Simple(bool),
    /// Range requests are supported; reserved for future options.
    Options(EmptyCapability),
}

/// The `full` member of [`SemanticTokensRequestsCapability`]: a plain
/// boolean, or an object opting into `full/delta` as well.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SemanticTokensFullClientCapability {
    /// `true`/`false`: full-document tokens are (not) supported, with no
    /// opinion on delta support.
    Simple(bool),
    /// Full-document tokens are supported, with delta support as given.
    Options(SemanticTokensFullClientCapabilityOptions),
}

/// The options form of [`SemanticTokensFullClientCapability`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticTokensFullClientCapabilityOptions {
    /// The client also requests `textDocument/semanticTokens/full/delta`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta: Option<bool>,
}

/// An empty JSON object (`{}`), used where the spec allows an object form
/// with no members today, reserved for future extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct EmptyCapability {}

/// `ClientCapabilities.textDocument.inlayHint` (LSP 3.17).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlayHintClientCapabilities {
    /// Whether the client supports dynamic registration for inlay hints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_registration: Option<bool>,
    /// Which `InlayHint` properties the client can resolve lazily via
    /// `inlayHint/resolve`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolve_support: Option<ResolveSupportCapability>,
}

/// `ClientCapabilities.textDocument.diagnostic` (LSP 3.17).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticClientCapabilities {
    /// Whether the client supports dynamic registration for pull
    /// diagnostics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_registration: Option<bool>,
    /// The client supports `DocumentDiagnosticReportKind::Full`'s
    /// `related_documents` (diagnostics for documents other than the one
    /// requested, e.g. a header file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub related_document_support: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn workspace_client_capabilities_parses_a_realistic_payload() {
        let value = json!({
            "applyEdit": true,
            "workspaceEdit": {
                "documentChanges": true,
                "resourceOperations": ["create", "rename", "delete"],
                "failureHandling": "textOnlyTransactional",
                "changeAnnotationSupport": {"groupsOnLabel": true},
            },
            "didChangeWatchedFiles": {"dynamicRegistration": true, "relativePatternSupport": true},
            "symbol": {
                "symbolKind": {"valueSet": [1, 2, 3]},
                "tagSupport": {"valueSet": [1]},
                "resolveSupport": {"properties": ["location.range"]},
            },
            "workspaceFolders": true,
            "configuration": true,
            "semanticTokens": {"refreshSupport": true},
            "fileOperations": {"didCreate": true, "willRename": true},
        });
        let caps: WorkspaceClientCapabilities = serde_json::from_value(value).unwrap();

        assert_eq!(caps.apply_edit, Some(true));
        let edit = caps.workspace_edit.unwrap();
        assert_eq!(edit.document_changes, Some(true));
        assert_eq!(
            edit.resource_operations,
            vec![
                ResourceOperationKind::Create,
                ResourceOperationKind::Rename,
                ResourceOperationKind::Delete,
            ]
        );
        assert_eq!(
            edit.failure_handling,
            Some(FailureHandlingKind::TextOnlyTransactional)
        );
        assert_eq!(
            edit.change_annotation_support.unwrap().groups_on_label,
            Some(true)
        );
        let watched_files = caps.did_change_watched_files.unwrap();
        assert_eq!(watched_files.dynamic_registration, Some(true));
        assert_eq!(watched_files.relative_pattern_support, Some(true));
        let symbol = caps.symbol.unwrap();
        assert_eq!(
            symbol.symbol_kind.unwrap().value_set,
            Some(vec![
                SymbolKind::File,
                SymbolKind::Module,
                SymbolKind::Namespace
            ])
        );
        assert_eq!(
            symbol.tag_support.unwrap().value_set,
            vec![SymbolTag::Deprecated]
        );
        assert_eq!(
            symbol.resolve_support.unwrap().properties,
            vec!["location.range".to_owned()]
        );
        assert_eq!(caps.workspace_folders, Some(true));
        assert_eq!(caps.configuration, Some(true));
        assert_eq!(caps.semantic_tokens.unwrap().refresh_support, Some(true));
        let file_ops = caps.file_operations.unwrap();
        assert_eq!(file_ops.did_create, Some(true));
        assert_eq!(file_ops.will_rename, Some(true));
        assert_eq!(file_ops.did_rename, None);
    }

    #[test]
    fn workspace_client_capabilities_defaults_on_empty_object() {
        let caps: WorkspaceClientCapabilities = serde_json::from_value(json!({})).unwrap();
        assert_eq!(caps, WorkspaceClientCapabilities::default());
    }

    #[test]
    fn resource_operation_kind_uses_lowercase_strings() {
        assert_eq!(
            serde_json::to_value(ResourceOperationKind::Rename).unwrap(),
            json!("rename")
        );
    }

    #[test]
    fn failure_handling_kind_uses_camel_case_strings() {
        assert_eq!(
            serde_json::to_value(FailureHandlingKind::TextOnlyTransactional).unwrap(),
            json!("textOnlyTransactional")
        );
    }

    #[test]
    fn window_client_capabilities_parses_a_realistic_payload() {
        let value = json!({
            "workDoneProgress": true,
            "showMessage": {"messageActionItem": {"additionalPropertiesSupport": true}},
            "showDocument": {"support": true},
        });
        let caps: WindowClientCapabilities = serde_json::from_value(value).unwrap();
        assert_eq!(caps.work_done_progress, Some(true));
        assert_eq!(
            caps.show_message
                .unwrap()
                .message_action_item
                .unwrap()
                .additional_properties_support,
            Some(true)
        );
        assert!(caps.show_document.unwrap().support);
    }

    #[test]
    fn window_client_capabilities_defaults_on_empty_object() {
        let caps: WindowClientCapabilities = serde_json::from_value(json!({})).unwrap();
        assert_eq!(caps, WindowClientCapabilities::default());
    }

    #[test]
    fn general_client_capabilities_parses_a_realistic_payload() {
        let value = json!({
            "staleRequestSupport": {
                "cancel": true,
                "retryOnContentModified": ["textDocument/rangeFormatting"],
            },
            "regularExpressions": {"engine": "ECMAScript", "version": "ES2020"},
            "markdown": {"parser": "marked", "version": "1.1.0", "allowedTags": ["b", "i"]},
            "positionEncodings": ["utf-8", "utf-16"],
        });
        let caps: GeneralClientCapabilities = serde_json::from_value(value).unwrap();

        let stale = caps.stale_request_support.unwrap();
        assert_eq!(stale.cancel, Some(true));
        assert_eq!(
            stale.retry_on_content_modified,
            vec!["textDocument/rangeFormatting".to_owned()]
        );
        let regex = caps.regular_expressions.unwrap();
        assert_eq!(regex.engine, Some("ECMAScript".to_owned()));
        assert_eq!(regex.version, Some("ES2020".to_owned()));
        let markdown = caps.markdown.unwrap();
        assert_eq!(markdown.parser, Some("marked".to_owned()));
        assert_eq!(markdown.allowed_tags, vec!["b".to_owned(), "i".to_owned()]);
        assert_eq!(
            caps.position_encodings,
            vec![PositionEncodingKind::Utf8, PositionEncodingKind::Utf16]
        );
    }

    #[test]
    fn general_client_capabilities_defaults_on_empty_object() {
        let caps: GeneralClientCapabilities = serde_json::from_value(json!({})).unwrap();
        assert_eq!(caps, GeneralClientCapabilities::default());
    }

    #[test]
    fn notebook_document_client_capabilities_parses_synchronization() {
        let value = json!({
            "synchronization": {"dynamicRegistration": true, "executionSummarySupport": true},
        });
        let caps: NotebookDocumentClientCapabilities = serde_json::from_value(value).unwrap();
        assert_eq!(caps.synchronization.dynamic_registration, Some(true));
        assert_eq!(caps.synchronization.execution_summary_support, Some(true));
    }

    #[test]
    fn notebook_document_client_capabilities_defaults_on_empty_object() {
        let caps: NotebookDocumentClientCapabilities = serde_json::from_value(json!({})).unwrap();
        assert_eq!(caps, NotebookDocumentClientCapabilities::default());
    }

    #[test]
    fn text_document_client_capabilities_parses_a_realistic_payload() {
        let value = json!({
            "synchronization": {"dynamicRegistration": true, "didSave": true},
            "completion": {
                "completionItem": {
                    "snippetSupport": true,
                    "documentationFormat": ["markdown", "plaintext"],
                    "tagSupport": {"valueSet": [1]},
                    "resolveSupport": {"properties": ["documentation"]},
                    "insertTextModeSupport": {"valueSet": [1, 2]},
                    "labelDetailsSupport": true,
                },
                "completionItemKind": {"valueSet": [1, 2, 3]},
                "insertTextMode": 2,
                "contextSupport": true,
                "completionList": {"itemDefaults": ["editRange", "insertTextFormat"]},
            },
            "hover": {"contentFormat": ["markdown"]},
            "signatureHelp": {
                "signatureInformation": {
                    "documentationFormat": ["markdown"],
                    "parameterInformation": {"labelOffsetSupport": true},
                    "activeParameterSupport": true,
                },
                "contextSupport": true,
            },
            "declaration": {"dynamicRegistration": true, "linkSupport": true},
            "definition": {"linkSupport": true},
            "references": {"dynamicRegistration": true},
            "documentHighlight": {"dynamicRegistration": true},
            "documentSymbol": {
                "hierarchicalDocumentSymbolSupport": true,
                "tagSupport": {"valueSet": [1]},
                "labelSupport": true,
            },
        });
        let caps: TextDocumentClientCapabilities = serde_json::from_value(value).unwrap();

        let sync = caps.synchronization.unwrap();
        assert_eq!(sync.dynamic_registration, Some(true));
        assert_eq!(sync.did_save, Some(true));

        let completion = caps.completion.unwrap();
        let item = completion.completion_item.unwrap();
        assert_eq!(item.snippet_support, Some(true));
        assert_eq!(
            item.documentation_format,
            vec![MarkupKind::Markdown, MarkupKind::PlainText]
        );
        assert_eq!(
            item.tag_support.unwrap().value_set,
            vec![CompletionItemTag::Deprecated]
        );
        assert_eq!(
            item.resolve_support.unwrap().properties,
            vec!["documentation".to_owned()]
        );
        assert_eq!(
            item.insert_text_mode_support.unwrap().value_set,
            vec![InsertTextMode::AsIs, InsertTextMode::AdjustIndentation]
        );
        assert_eq!(item.label_details_support, Some(true));
        assert_eq!(
            completion.completion_item_kind.unwrap().value_set,
            Some(vec![
                CompletionItemKind::Text,
                CompletionItemKind::Method,
                CompletionItemKind::Function
            ])
        );
        assert_eq!(
            completion.insert_text_mode,
            Some(InsertTextMode::AdjustIndentation)
        );
        assert_eq!(completion.context_support, Some(true));
        assert_eq!(
            completion.completion_list.unwrap().item_defaults,
            vec!["editRange".to_owned(), "insertTextFormat".to_owned()]
        );

        let hover = caps.hover.unwrap();
        assert_eq!(hover.content_format, vec![MarkupKind::Markdown]);

        let sig_help = caps.signature_help.unwrap();
        let sig_info = sig_help.signature_information.unwrap();
        assert_eq!(sig_info.documentation_format, vec![MarkupKind::Markdown]);
        assert_eq!(
            sig_info.parameter_information.unwrap().label_offset_support,
            Some(true)
        );
        assert_eq!(sig_info.active_parameter_support, Some(true));
        assert_eq!(sig_help.context_support, Some(true));

        assert_eq!(caps.declaration.unwrap().link_support, Some(true));
        assert_eq!(caps.definition.unwrap().link_support, Some(true));
        assert_eq!(caps.type_definition, None);
        assert_eq!(caps.references.unwrap().dynamic_registration, Some(true));
        assert_eq!(
            caps.document_highlight.unwrap().dynamic_registration,
            Some(true)
        );

        let doc_symbol = caps.document_symbol.unwrap();
        assert_eq!(doc_symbol.hierarchical_document_symbol_support, Some(true));
        assert_eq!(
            doc_symbol.tag_support.unwrap().value_set,
            vec![SymbolTag::Deprecated]
        );
        assert_eq!(doc_symbol.label_support, Some(true));
    }

    #[test]
    fn text_document_client_capabilities_defaults_on_empty_object() {
        let caps: TextDocumentClientCapabilities = serde_json::from_value(json!({})).unwrap();
        assert_eq!(caps, TextDocumentClientCapabilities::default());
    }

    #[test]
    fn text_document_client_capabilities_parses_advanced_group_a() {
        let value = json!({
            "codeAction": {
                "codeActionLiteralSupport": {"codeActionKind": {"valueSet": ["quickfix", "refactor"]}},
                "isPreferredSupport": true,
                "disabledSupport": true,
                "dataSupport": true,
                "resolveSupport": {"properties": ["edit"]},
                "honorsChangeAnnotations": true,
            },
            "codeLens": {"dynamicRegistration": true},
            "documentLink": {"tooltipSupport": true},
            "colorProvider": {"dynamicRegistration": true},
            "formatting": {"dynamicRegistration": true},
            "rangeFormatting": {"dynamicRegistration": true},
            "onTypeFormatting": {"dynamicRegistration": true},
            "rename": {
                "prepareSupport": true,
                "prepareSupportDefaultBehavior": 1,
                "honorsChangeAnnotations": true,
            },
            "foldingRange": {
                "rangeLimit": 5000,
                "lineFoldingOnly": true,
                "foldingRangeKind": {"valueSet": ["comment", "region"]},
                "foldingRange": {"collapsedText": true},
            },
            "selectionRange": {"dynamicRegistration": true},
            "publishDiagnostics": {
                "relatedInformation": true,
                "tagSupport": {"valueSet": [1, 2]},
                "versionSupport": true,
                "codeDescriptionSupport": true,
                "dataSupport": true,
            },
        });
        let caps: TextDocumentClientCapabilities = serde_json::from_value(value).unwrap();

        let code_action = caps.code_action.unwrap();
        assert_eq!(
            code_action
                .code_action_literal_support
                .unwrap()
                .code_action_kind
                .value_set,
            vec!["quickfix".to_owned(), "refactor".to_owned()]
        );
        assert_eq!(code_action.is_preferred_support, Some(true));
        assert_eq!(code_action.disabled_support, Some(true));
        assert_eq!(code_action.data_support, Some(true));
        assert_eq!(
            code_action.resolve_support.unwrap().properties,
            vec!["edit".to_owned()]
        );
        assert_eq!(code_action.honors_change_annotations, Some(true));

        assert_eq!(caps.code_lens.unwrap().dynamic_registration, Some(true));
        assert_eq!(caps.document_link.unwrap().tooltip_support, Some(true));
        assert_eq!(
            caps.color_provider.unwrap().dynamic_registration,
            Some(true)
        );
        assert_eq!(caps.formatting.unwrap().dynamic_registration, Some(true));
        assert_eq!(
            caps.range_formatting.unwrap().dynamic_registration,
            Some(true)
        );
        assert_eq!(
            caps.on_type_formatting.unwrap().dynamic_registration,
            Some(true)
        );

        let rename = caps.rename.unwrap();
        assert_eq!(rename.prepare_support, Some(true));
        assert_eq!(
            rename.prepare_support_default_behavior,
            Some(PrepareSupportDefaultBehavior::Identifier)
        );
        assert_eq!(rename.honors_change_annotations, Some(true));

        let folding = caps.folding_range.unwrap();
        assert_eq!(folding.range_limit, Some(5000));
        assert_eq!(folding.line_folding_only, Some(true));
        assert_eq!(
            folding.folding_range_kind.unwrap().value_set,
            Some(vec!["comment".to_owned(), "region".to_owned()])
        );
        assert_eq!(folding.folding_range.unwrap().collapsed_text, Some(true));

        assert_eq!(
            caps.selection_range.unwrap().dynamic_registration,
            Some(true)
        );

        let diagnostics = caps.publish_diagnostics.unwrap();
        assert_eq!(diagnostics.related_information, Some(true));
        assert_eq!(
            diagnostics.tag_support.unwrap().value_set,
            vec![DiagnosticTag::Unnecessary, DiagnosticTag::Deprecated]
        );
        assert_eq!(diagnostics.version_support, Some(true));
        assert_eq!(diagnostics.code_description_support, Some(true));
        assert_eq!(diagnostics.data_support, Some(true));
    }

    #[test]
    fn text_document_client_capabilities_parses_advanced_group_b() {
        let value = json!({
            "callHierarchy": {"dynamicRegistration": true},
            "semanticTokens": {
                "dynamicRegistration": true,
                "requests": {"range": true, "full": {"delta": true}},
                "tokenTypes": ["keyword"],
                "tokenModifiers": ["readonly"],
                "formats": ["relative"],
                "overlappingTokenSupport": true,
                "multilineTokenSupport": true,
                "serverCancelSupport": true,
                "augmentsSyntaxTokens": true,
            },
            "linkedEditingRange": {"dynamicRegistration": true},
            "moniker": {"dynamicRegistration": true},
            "typeHierarchy": {"dynamicRegistration": true},
            "inlineValue": {"dynamicRegistration": true},
            "inlayHint": {"resolveSupport": {"properties": ["tooltip"]}},
            "diagnostic": {"dynamicRegistration": true, "relatedDocumentSupport": true},
        });
        let caps: TextDocumentClientCapabilities = serde_json::from_value(value).unwrap();

        assert_eq!(
            caps.call_hierarchy.unwrap().dynamic_registration,
            Some(true)
        );

        let tokens = caps.semantic_tokens.unwrap();
        assert_eq!(tokens.dynamic_registration, Some(true));
        assert_eq!(
            tokens.requests.range,
            Some(SemanticTokensRangeClientCapability::Simple(true))
        );
        assert_eq!(
            tokens.requests.full,
            Some(SemanticTokensFullClientCapability::Options(
                SemanticTokensFullClientCapabilityOptions { delta: Some(true) }
            ))
        );
        assert_eq!(tokens.token_types, vec!["keyword".to_owned()]);
        assert_eq!(tokens.token_modifiers, vec!["readonly".to_owned()]);
        assert_eq!(tokens.formats, vec![token_format::RELATIVE.to_owned()]);
        assert_eq!(tokens.overlapping_token_support, Some(true));
        assert_eq!(tokens.multiline_token_support, Some(true));
        assert_eq!(tokens.server_cancel_support, Some(true));
        assert_eq!(tokens.augments_syntax_tokens, Some(true));

        assert_eq!(
            caps.linked_editing_range.unwrap().dynamic_registration,
            Some(true)
        );
        assert_eq!(caps.moniker.unwrap().dynamic_registration, Some(true));
        assert_eq!(
            caps.type_hierarchy.unwrap().dynamic_registration,
            Some(true)
        );
        assert_eq!(caps.inline_value.unwrap().dynamic_registration, Some(true));
        assert_eq!(
            caps.inlay_hint.unwrap().resolve_support.unwrap().properties,
            vec!["tooltip".to_owned()]
        );

        let diagnostic = caps.diagnostic.unwrap();
        assert_eq!(diagnostic.dynamic_registration, Some(true));
        assert_eq!(diagnostic.related_document_support, Some(true));
    }

    #[test]
    fn semantic_tokens_range_and_full_accept_the_object_form() {
        let range: SemanticTokensRangeClientCapability = serde_json::from_value(json!({})).unwrap();
        assert_eq!(
            range,
            SemanticTokensRangeClientCapability::Options(EmptyCapability {})
        );

        let full: SemanticTokensFullClientCapability =
            serde_json::from_value(json!(false)).unwrap();
        assert_eq!(full, SemanticTokensFullClientCapability::Simple(false));
    }

    #[test]
    fn semantic_tokens_client_capabilities_defaults_on_empty_object() {
        let caps: SemanticTokensClientCapabilities = serde_json::from_value(json!({})).unwrap();
        assert_eq!(caps, SemanticTokensClientCapabilities::default());
    }
}
