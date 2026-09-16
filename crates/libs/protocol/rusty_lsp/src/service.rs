//! The [`LanguageServer`] trait — the single extension point applications
//! implement to provide language behaviour.
//!
//! The framework owns transport, framing, JSON-RPC dispatch, lifecycle, and
//! cancellation; an implementor only fills in the handlers it cares about.
//! Every method except [`initialize`](LanguageServer::initialize) has a
//! sensible default (no-op for notifications, "unsupported" for requests), so a
//! minimal server is just an `initialize` returning its capabilities.
//!
//! ## Why `-> impl Future + Send` instead of `async fn`
//!
//! The trait declares async methods in their desugared form,
//! `fn m(&self, ..) -> impl Future<Output = T> + Send`, rather than
//! `async fn m(&self, ..) -> T`. The explicit `+ Send` bound lets the
//! [`crate::Server`] spawn request handlers onto a multi-threaded runtime
//! without an `async-trait`-style boxing layer. Implementors may still write
//! the bodies as ordinary `async fn` — an `async fn` whose body is `Send`
//! satisfies the desugared signature.

use crate::error::{Error, Result};
use crate::lsp::{
    CallHierarchyIncomingCall, CallHierarchyIncomingCallsParams, CallHierarchyItem,
    CallHierarchyOutgoingCall, CallHierarchyOutgoingCallsParams, CallHierarchyPrepareParams,
    CodeAction, CodeActionOrCommand, CodeActionParams, CodeLens, CodeLensParams, ColorInformation,
    ColorPresentation, ColorPresentationParams, CompletionItem, CompletionParams,
    CompletionResponse, CreateFilesParams, DeclarationParams, DefinitionParams, DeleteFilesParams,
    DidChangeConfigurationParams, DidChangeNotebookDocumentParams, DidChangeTextDocumentParams,
    DidChangeWatchedFilesParams, DidChangeWorkspaceFoldersParams, DidCloseNotebookDocumentParams,
    DidCloseTextDocumentParams, DidOpenNotebookDocumentParams, DidOpenTextDocumentParams,
    DidSaveNotebookDocumentParams, DidSaveTextDocumentParams, DocumentColorParams,
    DocumentDiagnosticParams, DocumentDiagnosticReport, DocumentFormattingParams,
    DocumentHighlight, DocumentHighlightParams, DocumentLink, DocumentLinkParams,
    DocumentOnTypeFormattingParams, DocumentRangeFormattingParams, DocumentSymbolParams,
    DocumentSymbolResponse, ExecuteCommandParams, FoldingRange, FoldingRangeParams,
    FullDocumentDiagnosticReport, GotoDefinitionResponse, Hover, HoverParams, ImplementationParams,
    InitializeParams, InitializeResult, InlayHint, InlayHintParams, InlineCompletionParams,
    InlineCompletionResponse, InlineValue, InlineValueParams, LinkedEditingRangeParams,
    LinkedEditingRanges, Location, Moniker, MonikerParams, PrepareRenameResponse, ReferenceParams,
    RenameFilesParams, RenameParams, SelectionRange, SelectionRangeParams, SemanticTokens,
    SemanticTokensDeltaParams, SemanticTokensDeltaResult, SemanticTokensParams,
    SemanticTokensRangeParams, SetTraceParams, SignatureHelp, SignatureHelpParams,
    TextDocumentPositionParams, TextEdit, TypeDefinitionParams, TypeHierarchyItem,
    TypeHierarchyPrepareParams, TypeHierarchySubtypesParams, TypeHierarchySupertypesParams,
    WillSaveTextDocumentParams, WorkDoneProgressCancelParams, WorkspaceDiagnosticParams,
    WorkspaceDiagnosticReport, WorkspaceEdit, WorkspaceSymbol, WorkspaceSymbolParams,
    WorkspaceSymbolResponse,
};
use serde_json::Value;

/// Implement this trait to define a language server's behaviour.
///
/// Construct your backend from a [`crate::Client`] (handed to you by
/// [`crate::Server::serve`]) so handlers can push diagnostics and messages back
/// to the editor. The backend is shared across concurrent requests behind an
/// `Arc`, so all methods take `&self`; use interior mutability
/// (`tokio::sync::RwLock`, `Mutex`, atomics, …) for any state you mutate.
pub trait LanguageServer: Send + Sync + 'static {
    /// Handle the `initialize` request.
    ///
    /// Called exactly once, before any other request. Return the server's
    /// [`InitializeResult`], primarily its
    /// [`ServerCapabilities`](crate::lsp::ServerCapabilities). This is the only
    /// method without a default, since a server must advertise what it does.
    fn initialize(
        &self,
        params: InitializeParams,
    ) -> impl Future<Output = Result<InitializeResult>> + Send;

    /// Handle the `initialized` notification (the client is ready).
    ///
    /// A good place to register dynamic capabilities or start background work.
    fn initialized(&self) -> impl Future<Output = ()> + Send {
        async {}
    }

    /// Handle the `shutdown` request.
    ///
    /// The server should release resources but must keep running until it
    /// receives the subsequent `exit` notification.
    fn shutdown(&self) -> impl Future<Output = Result<()>> + Send {
        async { Ok(()) }
    }

    /// Handle `textDocument/didOpen`.
    fn did_open(&self, params: DidOpenTextDocumentParams) -> impl Future<Output = ()> + Send {
        let _ = params;
        async {}
    }

    /// Handle `textDocument/didChange`.
    fn did_change(&self, params: DidChangeTextDocumentParams) -> impl Future<Output = ()> + Send {
        let _ = params;
        async {}
    }

    /// Handle `textDocument/didClose`.
    fn did_close(&self, params: DidCloseTextDocumentParams) -> impl Future<Output = ()> + Send {
        let _ = params;
        async {}
    }

    /// Handle `textDocument/didSave`.
    fn did_save(&self, params: DidSaveTextDocumentParams) -> impl Future<Output = ()> + Send {
        let _ = params;
        async {}
    }

    /// Handle `textDocument/hover`. Return `Ok(None)` for "no hover here".
    fn hover(&self, params: HoverParams) -> impl Future<Output = Result<Option<Hover>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `textDocument/completion`.
    fn completion(
        &self,
        params: CompletionParams,
    ) -> impl Future<Output = Result<Option<CompletionResponse>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `textDocument/definition`.
    fn definition(
        &self,
        params: DefinitionParams,
    ) -> impl Future<Output = Result<Option<GotoDefinitionResponse>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `textDocument/declaration`.
    fn declaration(
        &self,
        params: DeclarationParams,
    ) -> impl Future<Output = Result<Option<GotoDefinitionResponse>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `textDocument/typeDefinition`.
    fn type_definition(
        &self,
        params: TypeDefinitionParams,
    ) -> impl Future<Output = Result<Option<GotoDefinitionResponse>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `textDocument/implementation`.
    fn implementation(
        &self,
        params: ImplementationParams,
    ) -> impl Future<Output = Result<Option<GotoDefinitionResponse>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `textDocument/references`.
    fn references(
        &self,
        params: ReferenceParams,
    ) -> impl Future<Output = Result<Option<Vec<Location>>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `textDocument/documentHighlight`: every occurrence of the
    /// symbol under the cursor within the document, for the client to
    /// highlight. Advertise via
    /// [`crate::lsp::ServerCapabilities::document_highlight_provider`].
    fn document_highlight(
        &self,
        params: DocumentHighlightParams,
    ) -> impl Future<Output = Result<Option<Vec<DocumentHighlight>>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `completionItem/resolve`.
    ///
    /// The default returns `item` unchanged, which is only correct if
    /// completion's `resolve_provider` is left unset/`false` in
    /// [`crate::lsp::CompletionOptions`] — override this alongside
    /// advertising resolve support.
    fn completion_resolve(
        &self,
        item: CompletionItem,
    ) -> impl Future<Output = Result<CompletionItem>> + Send {
        async { Ok(item) }
    }

    /// Handle `textDocument/documentSymbol`.
    fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> impl Future<Output = Result<Option<DocumentSymbolResponse>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `workspace/symbol`.
    ///
    /// Return either the pre-3.17 flat form or the 3.17
    /// [`WorkspaceSymbol`] form (both convert via `.into()`); the latter
    /// supports lazy range resolution through
    /// [`workspace_symbol_resolve`](Self::workspace_symbol_resolve).
    fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> impl Future<Output = Result<Option<WorkspaceSymbolResponse>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `workspaceSymbol/resolve`.
    ///
    /// The default returns `symbol` unchanged, which is only correct if
    /// [`crate::lsp::ServerCapabilities::workspace_symbol_provider`]'s
    /// resolve support is left unset/`false` — override this alongside
    /// advertising resolve support.
    fn workspace_symbol_resolve(
        &self,
        symbol: WorkspaceSymbol,
    ) -> impl Future<Output = Result<WorkspaceSymbol>> + Send {
        async { Ok(symbol) }
    }

    /// Handle `textDocument/signatureHelp`.
    fn signature_help(
        &self,
        params: SignatureHelpParams,
    ) -> impl Future<Output = Result<Option<SignatureHelp>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `textDocument/codeAction`.
    fn code_action(
        &self,
        params: CodeActionParams,
    ) -> impl Future<Output = Result<Option<Vec<CodeActionOrCommand>>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `codeAction/resolve`.
    ///
    /// The default returns `action` unchanged, which is only correct if
    /// [`crate::lsp::ServerCapabilities::code_action_provider`]'s resolve
    /// support is left unset/`false` — override this alongside advertising
    /// resolve support.
    fn code_action_resolve(
        &self,
        action: CodeAction,
    ) -> impl Future<Output = Result<CodeAction>> + Send {
        async { Ok(action) }
    }

    /// Handle `textDocument/rename`.
    fn rename(
        &self,
        params: RenameParams,
    ) -> impl Future<Output = Result<Option<WorkspaceEdit>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `textDocument/prepareRename`.
    fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> impl Future<Output = Result<Option<PrepareRenameResponse>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `workspace/didChangeConfiguration`.
    fn did_change_configuration(
        &self,
        params: DidChangeConfigurationParams,
    ) -> impl Future<Output = ()> + Send {
        let _ = params;
        async {}
    }

    /// Handle `workspace/didChangeWatchedFiles`.
    fn did_change_watched_files(
        &self,
        params: DidChangeWatchedFilesParams,
    ) -> impl Future<Output = ()> + Send {
        let _ = params;
        async {}
    }

    /// Handle `workspace/executeCommand`.
    fn execute_command(
        &self,
        params: ExecuteCommandParams,
    ) -> impl Future<Output = Result<Option<Value>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `workspace/didChangeWorkspaceFolders`, sent when the client
    /// adds or removes root folders in a multi-root workspace.
    fn did_change_workspace_folders(
        &self,
        params: DidChangeWorkspaceFoldersParams,
    ) -> impl Future<Output = ()> + Send {
        let _ = params;
        async {}
    }

    /// Handle `window/workDoneProgress/cancel`, sent when the user cancels a
    /// [`Client`](crate::Client)-reported progress sequence from the client
    /// UI. The default is a no-op; override to actually abort the work.
    fn work_done_progress_cancel(
        &self,
        params: WorkDoneProgressCancelParams,
    ) -> impl Future<Output = ()> + Send {
        let _ = params;
        async {}
    }

    /// Handle `textDocument/formatting`.
    fn formatting(
        &self,
        params: DocumentFormattingParams,
    ) -> impl Future<Output = Result<Option<Vec<TextEdit>>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `textDocument/rangeFormatting`.
    fn range_formatting(
        &self,
        params: DocumentRangeFormattingParams,
    ) -> impl Future<Output = Result<Option<Vec<TextEdit>>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `textDocument/onTypeFormatting`.
    fn on_type_formatting(
        &self,
        params: DocumentOnTypeFormattingParams,
    ) -> impl Future<Output = Result<Option<Vec<TextEdit>>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `textDocument/foldingRange`.
    fn folding_range(
        &self,
        params: FoldingRangeParams,
    ) -> impl Future<Output = Result<Option<Vec<FoldingRange>>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `textDocument/selectionRange`.
    fn selection_range(
        &self,
        params: SelectionRangeParams,
    ) -> impl Future<Output = Result<Option<Vec<SelectionRange>>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `textDocument/codeLens`.
    fn code_lens(
        &self,
        params: CodeLensParams,
    ) -> impl Future<Output = Result<Option<Vec<CodeLens>>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `codeLens/resolve`.
    ///
    /// The default returns `lens` unchanged, which is only correct if
    /// [`crate::lsp::ServerCapabilities::code_lens_provider`]'s resolve
    /// support is left unset/`false` — override this alongside advertising
    /// resolve support.
    fn code_lens_resolve(&self, lens: CodeLens) -> impl Future<Output = Result<CodeLens>> + Send {
        async { Ok(lens) }
    }

    /// Handle `textDocument/documentLink`.
    fn document_link(
        &self,
        params: DocumentLinkParams,
    ) -> impl Future<Output = Result<Option<Vec<DocumentLink>>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `documentLink/resolve`.
    ///
    /// The default returns `link` unchanged, which is only correct if
    /// [`crate::lsp::ServerCapabilities::document_link_provider`]'s resolve
    /// support is left unset/`false` — override this alongside advertising
    /// resolve support.
    fn document_link_resolve(
        &self,
        link: DocumentLink,
    ) -> impl Future<Output = Result<DocumentLink>> + Send {
        async { Ok(link) }
    }

    /// Handle `textDocument/documentColor`.
    fn document_color(
        &self,
        params: DocumentColorParams,
    ) -> impl Future<Output = Result<Vec<ColorInformation>>> + Send {
        let _ = params;
        async { Ok(Vec::new()) }
    }

    /// Handle `textDocument/colorPresentation`.
    fn color_presentation(
        &self,
        params: ColorPresentationParams,
    ) -> impl Future<Output = Result<Vec<ColorPresentation>>> + Send {
        let _ = params;
        async { Ok(Vec::new()) }
    }

    /// Handle `textDocument/semanticTokens/full`.
    fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> impl Future<Output = Result<Option<SemanticTokens>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `textDocument/semanticTokens/full/delta`.
    fn semantic_tokens_full_delta(
        &self,
        params: SemanticTokensDeltaParams,
    ) -> impl Future<Output = Result<Option<SemanticTokensDeltaResult>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `textDocument/semanticTokens/range`.
    fn semantic_tokens_range(
        &self,
        params: SemanticTokensRangeParams,
    ) -> impl Future<Output = Result<Option<SemanticTokens>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `textDocument/inlayHint`.
    fn inlay_hint(
        &self,
        params: InlayHintParams,
    ) -> impl Future<Output = Result<Option<Vec<InlayHint>>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `inlayHint/resolve`.
    ///
    /// The default returns `hint` unchanged, which is only correct if
    /// [`crate::lsp::ServerCapabilities::inlay_hint_provider`]'s resolve
    /// support is left unset/`false` — override this alongside advertising
    /// resolve support.
    fn inlay_hint_resolve(
        &self,
        hint: InlayHint,
    ) -> impl Future<Output = Result<InlayHint>> + Send {
        async { Ok(hint) }
    }

    /// Handle `textDocument/diagnostic`.
    ///
    /// The default reports an empty, fresh
    /// [`DocumentDiagnosticReport::Full`] result.
    fn diagnostic(
        &self,
        params: DocumentDiagnosticParams,
    ) -> impl Future<Output = Result<DocumentDiagnosticReport>> + Send {
        let _ = params;
        async {
            Ok(DocumentDiagnosticReport::Full(
                FullDocumentDiagnosticReport::default(),
            ))
        }
    }

    /// Handle `workspace/diagnostic`.
    ///
    /// The default reports no items; override alongside setting
    /// [`crate::lsp::ServerCapabilities::diagnostic_provider`]'s
    /// `workspace_diagnostics` to advertise real support.
    fn workspace_diagnostic(
        &self,
        params: WorkspaceDiagnosticParams,
    ) -> impl Future<Output = Result<WorkspaceDiagnosticReport>> + Send {
        let _ = params;
        async { Ok(WorkspaceDiagnosticReport::default()) }
    }

    /// Handle `textDocument/willSave`.
    fn will_save(&self, params: WillSaveTextDocumentParams) -> impl Future<Output = ()> + Send {
        let _ = params;
        async {}
    }

    /// Handle `textDocument/willSaveWaitUntil`.
    ///
    /// Unlike [`will_save`](Self::will_save), this is a request: the client
    /// waits for the response (applying any returned edits) before actually
    /// saving.
    fn will_save_wait_until(
        &self,
        params: WillSaveTextDocumentParams,
    ) -> impl Future<Output = Result<Option<Vec<TextEdit>>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `workspace/willCreateFiles`, sent before the client creates
    /// files, so the server can propose an edit (e.g. inserting boilerplate
    /// into the new file) to apply alongside the creation.
    fn will_create_files(
        &self,
        params: CreateFilesParams,
    ) -> impl Future<Output = Result<Option<WorkspaceEdit>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `workspace/didCreateFiles`, sent after the client created
    /// files.
    fn did_create_files(&self, params: CreateFilesParams) -> impl Future<Output = ()> + Send {
        let _ = params;
        async {}
    }

    /// Handle `workspace/willRenameFiles`, sent before the client renames
    /// files, so the server can propose an edit (e.g. updating imports) to
    /// apply alongside the rename.
    fn will_rename_files(
        &self,
        params: RenameFilesParams,
    ) -> impl Future<Output = Result<Option<WorkspaceEdit>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `workspace/didRenameFiles`, sent after the client renamed
    /// files.
    fn did_rename_files(&self, params: RenameFilesParams) -> impl Future<Output = ()> + Send {
        let _ = params;
        async {}
    }

    /// Handle `workspace/willDeleteFiles`, sent before the client deletes
    /// files, so the server can propose an edit (e.g. removing now-dangling
    /// imports) to apply alongside the deletion.
    fn will_delete_files(
        &self,
        params: DeleteFilesParams,
    ) -> impl Future<Output = Result<Option<WorkspaceEdit>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `workspace/didDeleteFiles`, sent after the client deleted
    /// files.
    fn did_delete_files(&self, params: DeleteFilesParams) -> impl Future<Output = ()> + Send {
        let _ = params;
        async {}
    }

    /// Handle `textDocument/prepareCallHierarchy`.
    fn prepare_call_hierarchy(
        &self,
        params: CallHierarchyPrepareParams,
    ) -> impl Future<Output = Result<Option<Vec<CallHierarchyItem>>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `callHierarchy/incomingCalls`.
    fn incoming_calls(
        &self,
        params: CallHierarchyIncomingCallsParams,
    ) -> impl Future<Output = Result<Option<Vec<CallHierarchyIncomingCall>>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `callHierarchy/outgoingCalls`.
    fn outgoing_calls(
        &self,
        params: CallHierarchyOutgoingCallsParams,
    ) -> impl Future<Output = Result<Option<Vec<CallHierarchyOutgoingCall>>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `textDocument/prepareTypeHierarchy`.
    fn prepare_type_hierarchy(
        &self,
        params: TypeHierarchyPrepareParams,
    ) -> impl Future<Output = Result<Option<Vec<TypeHierarchyItem>>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `typeHierarchy/supertypes`.
    fn supertypes(
        &self,
        params: TypeHierarchySupertypesParams,
    ) -> impl Future<Output = Result<Option<Vec<TypeHierarchyItem>>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `typeHierarchy/subtypes`.
    fn subtypes(
        &self,
        params: TypeHierarchySubtypesParams,
    ) -> impl Future<Output = Result<Option<Vec<TypeHierarchyItem>>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `textDocument/moniker` (LSP 3.16): stable symbol identifiers
    /// for cross-index correlation. Advertise via
    /// [`crate::lsp::ServerCapabilities::moniker_provider`].
    fn moniker(
        &self,
        params: MonikerParams,
    ) -> impl Future<Output = Result<Option<Vec<Moniker>>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `textDocument/linkedEditingRange` (LSP 3.16): ranges edited in
    /// lockstep (e.g. paired HTML tags). Advertise via
    /// [`crate::lsp::ServerCapabilities::linked_editing_range_provider`].
    fn linked_editing_range(
        &self,
        params: LinkedEditingRangeParams,
    ) -> impl Future<Output = Result<Option<LinkedEditingRanges>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `textDocument/inlineValue` (LSP 3.17): values shown inline
    /// while stopped in a debugger. Advertise via
    /// [`crate::lsp::ServerCapabilities::inline_value_provider`].
    fn inline_value(
        &self,
        params: InlineValueParams,
    ) -> impl Future<Output = Result<Option<Vec<InlineValue>>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `textDocument/inlineCompletion` (LSP 3.18, proposed):
    /// ghost-text completions at the cursor. Advertise via
    /// [`crate::lsp::ServerCapabilities::inline_completion_provider`].
    fn inline_completion(
        &self,
        params: InlineCompletionParams,
    ) -> impl Future<Output = Result<Option<InlineCompletionResponse>>> + Send {
        let _ = params;
        async { Ok(None) }
    }

    /// Handle `$/setTrace`, sent when the user changes the trace verbosity
    /// the client wants reported via `$/logTrace`
    /// ([`Client::log_trace`](crate::Client::log_trace)). The default is a
    /// no-op; override to actually gate what you log.
    fn set_trace(&self, params: SetTraceParams) -> impl Future<Output = ()> + Send {
        let _ = params;
        async {}
    }

    /// Handle `notebookDocument/didOpen`.
    fn did_open_notebook_document(
        &self,
        params: DidOpenNotebookDocumentParams,
    ) -> impl Future<Output = ()> + Send {
        let _ = params;
        async {}
    }

    /// Handle `notebookDocument/didChange`.
    fn did_change_notebook_document(
        &self,
        params: DidChangeNotebookDocumentParams,
    ) -> impl Future<Output = ()> + Send {
        let _ = params;
        async {}
    }

    /// Handle `notebookDocument/didSave`.
    fn did_save_notebook_document(
        &self,
        params: DidSaveNotebookDocumentParams,
    ) -> impl Future<Output = ()> + Send {
        let _ = params;
        async {}
    }

    /// Handle `notebookDocument/didClose`.
    fn did_close_notebook_document(
        &self,
        params: DidCloseNotebookDocumentParams,
    ) -> impl Future<Output = ()> + Send {
        let _ = params;
        async {}
    }

    /// Fallback for request methods the framework does not model.
    ///
    /// Override to support additional typed requests (call hierarchy,
    /// notebook sync, …). Deserialize `params` yourself and return a JSON
    /// value to be sent as the result. The default reports
    /// [`METHOD_NOT_FOUND`](crate::error::codes::METHOD_NOT_FOUND).
    fn handle_request(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> impl Future<Output = Result<Value>> + Send {
        let _ = params;
        let method = method.to_owned();
        async move {
            Err(Error::method_not_found(format!(
                "method not found: {method}"
            )))
        }
    }

    /// Fallback for notification methods the framework does not model.
    ///
    /// The default silently ignores the notification, which is the correct
    /// behaviour for unknown notifications per the LSP spec.
    fn handle_notification(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> impl Future<Output = ()> + Send {
        let _ = (method, params);
        async {}
    }
}
