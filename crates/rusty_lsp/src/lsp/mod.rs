//! LSP protocol data types.
//!
//! These are plain serde structs/enums modelling the subset of the Language
//! Server Protocol that the framework dispatches to typed handlers, plus the
//! notifications a server commonly sends back to the client (diagnostics, log
//! and show-message). Anything outside this subset can still be handled through
//! the untyped escape hatches on [`crate::LanguageServer`] and advertised via
//! [`ServerCapabilities::extra`].
//!
//! All types follow the LSP JSON conventions: `camelCase` field names and
//! integer discriminants for the enums in [`enums`].

pub mod base;
pub mod client_capabilities;
pub mod code_action;
pub mod code_lens;
pub mod diagnostics;
pub mod document;
pub mod enums;
pub mod features;
pub mod file_operations;
pub mod formatting;
pub mod hierarchy;
pub mod inlay_hint;
pub mod inline_completion;
pub mod inline_value;
pub mod lifecycle;
pub mod links;
pub mod moniker;
pub mod notebook;
pub mod progress;
pub mod ranges;
pub mod registration;
pub mod rename;
pub mod semantic_tokens;
pub mod signature;
pub mod symbols;
pub mod trace;
pub mod window;
pub mod workspace;

pub use base::{
    Location, Position, Range, TextDocumentIdentifier, TextDocumentItem,
    TextDocumentPositionParams, Uri, VersionedTextDocumentIdentifier,
};
pub use client_capabilities::{
    ChangeAnnotationSupportCapability, CodeActionClientCapabilities, CodeActionKindCapability,
    CodeActionLiteralSupportCapability, CompletionClientCapabilities, CompletionItemCapability,
    CompletionItemKindCapability, CompletionListCapability, DiagnosticClientCapabilities,
    DidChangeWatchedFilesClientCapabilities, DocumentLinkClientCapabilities,
    DocumentSymbolClientCapabilities, DynamicRegistrationCapability, EmptyCapability,
    FailureHandlingKind, FileOperationClientCapabilities, FoldingRangeClientCapabilities,
    FoldingRangeCollapsedTextCapability, FoldingRangeKindCapability, GeneralClientCapabilities,
    GotoClientCapabilities, HoverClientCapabilities, InlayHintClientCapabilities,
    InsertTextModeSupportCapability, MarkdownClientCapabilities, MessageActionItemCapability,
    NotebookDocumentClientCapabilities, NotebookDocumentSyncClientCapabilities,
    ParameterInformationCapability, PublishDiagnosticsClientCapabilities, RefreshSupportCapability,
    RegularExpressionsClientCapabilities, RenameClientCapabilities, ResolveSupportCapability,
    ResourceOperationKind, SemanticTokensClientCapabilities, SemanticTokensFullClientCapability,
    SemanticTokensFullClientCapabilityOptions, SemanticTokensRangeClientCapability,
    SemanticTokensRequestsCapability, ShowDocumentClientCapabilities,
    ShowMessageClientCapabilities, SignatureHelpClientCapabilities, SignatureInformationCapability,
    StaleRequestSupportCapability, SymbolKindCapability, TagSupportCapability,
    TextDocumentClientCapabilities, TextDocumentSyncClientCapabilities, TokenFormat,
    WindowClientCapabilities, WorkspaceClientCapabilities, WorkspaceEditClientCapabilities,
    WorkspaceSymbolClientCapabilities, token_format,
};
pub use code_action::{
    CodeAction, CodeActionContext, CodeActionDisabled, CodeActionKind, CodeActionOptions,
    CodeActionOrCommand, CodeActionParams, Command, code_action_kind,
};
pub use code_lens::{CodeLens, CodeLensOptions, CodeLensParams};
pub use diagnostics::{
    CodeDescription, Diagnostic, DiagnosticOptions, DiagnosticRelatedInformation,
    DocumentDiagnosticParams, DocumentDiagnosticReport, FullDocumentDiagnosticReport,
    PreviousResultId, PublishDiagnosticsParams, UnchangedDocumentDiagnosticReport,
    WorkspaceDiagnosticParams, WorkspaceDiagnosticReport, WorkspaceDocumentDiagnosticReport,
    WorkspaceFullDocumentDiagnosticReport, WorkspaceUnchangedDocumentDiagnosticReport,
};
pub use document::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DidSaveTextDocumentParams, SaveOptions, SaveOptionsCapability, TextDocumentContentChangeEvent,
    TextDocumentSyncCapability, TextDocumentSyncOptions, WillSaveTextDocumentParams,
};
pub use enums::{
    CodeActionTriggerKind, CompletionItemKind, CompletionItemTag, CompletionTriggerKind,
    DiagnosticSeverity, DiagnosticTag, DocumentHighlightKind, FileChangeType, InlayHintKind,
    InlineCompletionTriggerKind, InsertTextFormat, InsertTextMode, MarkupKind, MessageType,
    NotebookCellKind, PositionEncodingKind, PrepareSupportDefaultBehavior,
    SignatureHelpTriggerKind, SymbolKind, SymbolTag, TextDocumentSaveReason, TextDocumentSyncKind,
};
pub use features::{
    CompletionContext, CompletionEditRange, CompletionItem, CompletionItemDefaults,
    CompletionItemLabelDetails, CompletionList, CompletionParams, CompletionResponse,
    CompletionTextEdit, DeclarationParams, DefinitionParams, DocumentHighlight,
    DocumentHighlightParams, GotoDefinitionResponse, Hover, HoverParams, ImplementationParams,
    InsertReplaceEdit, LocationLink, MarkupContent, ReferenceContext, ReferenceParams,
    TypeDefinitionParams,
};
pub use file_operations::{
    CreateFilesParams, DeleteFilesParams, FileCreate, FileDelete, FileOperationFilter,
    FileOperationPattern, FileOperationRegistrationOptions, FileOperationsServerCapabilities,
    FileRename, RenameFilesParams,
};
pub use formatting::{
    DocumentFormattingParams, DocumentOnTypeFormattingOptions, DocumentOnTypeFormattingParams,
    DocumentRangeFormattingParams, FormattingOptions,
};
pub use hierarchy::{
    CallHierarchyIncomingCall, CallHierarchyIncomingCallsParams, CallHierarchyItem,
    CallHierarchyOptions, CallHierarchyOutgoingCall, CallHierarchyOutgoingCallsParams,
    CallHierarchyPrepareParams, TypeHierarchyItem, TypeHierarchyOptions,
    TypeHierarchyPrepareParams, TypeHierarchySubtypesParams, TypeHierarchySupertypesParams,
};
pub use inlay_hint::{
    InlayHint, InlayHintLabel, InlayHintLabelPart, InlayHintOptions, InlayHintParams,
};
pub use inline_completion::{
    InlineCompletionContext, InlineCompletionInsertText, InlineCompletionItem,
    InlineCompletionList, InlineCompletionParams, InlineCompletionResponse, SelectedCompletionInfo,
    StringValue,
};
pub use inline_value::{
    InlineValue, InlineValueContext, InlineValueEvaluatableExpression, InlineValueParams,
    InlineValueText, InlineValueVariableLookup,
};
pub use lifecycle::{
    CallHierarchyProviderCapability, ClientCapabilities, ClientInfo, CodeActionProviderCapability,
    ColorProviderCapability, CompletionOptions, CompletionOptionsCompletionItem,
    DeclarationProviderCapability, DefinitionProviderCapability,
    DocumentFormattingProviderCapability, DocumentHighlightProviderCapability,
    DocumentRangeFormattingProviderCapability, DocumentSymbolProviderCapability,
    FoldingRangeProviderCapability, HoverProviderCapability, ImplementationProviderCapability,
    InitializeParams, InitializeResult, InlineValueProviderCapability,
    LinkedEditingRangeProviderCapability, MonikerProviderCapability, ReferencesProviderCapability,
    RenameProviderCapability, SelectionRangeProviderCapability, ServerCapabilities, ServerInfo,
    SignatureHelpOptions, TypeDefinitionProviderCapability, TypeHierarchyProviderCapability,
    WorkspaceServerCapabilities, WorkspaceSymbolProviderCapability,
};
pub use links::{
    Color, ColorInformation, ColorPresentation, ColorPresentationParams, DocumentColorParams,
    DocumentLink, DocumentLinkOptions, DocumentLinkParams,
};
pub use moniker::{Moniker, MonikerKind, MonikerParams, UniquenessLevel};
pub use notebook::{
    DidChangeNotebookDocumentParams, DidCloseNotebookDocumentParams, DidOpenNotebookDocumentParams,
    DidSaveNotebookDocumentParams, ExecutionSummary, NotebookCell, NotebookCellArrayChange,
    NotebookCellTextContentChange, NotebookDocument, NotebookDocumentCellChanges,
    NotebookDocumentCellStructureChange, NotebookDocumentChangeEvent, NotebookDocumentIdentifier,
    NotebookDocumentSyncOptions, VersionedNotebookDocumentIdentifier,
};
pub use progress::{
    PartialResultParams, ProgressParams, ProgressToken, WorkDoneProgress, WorkDoneProgressBegin,
    WorkDoneProgressCancelParams, WorkDoneProgressCreateParams, WorkDoneProgressEnd,
    WorkDoneProgressOptions, WorkDoneProgressParams, WorkDoneProgressReport,
};
pub use ranges::{
    FoldingRange, FoldingRangeKind, FoldingRangeParams, LinkedEditingRangeParams,
    LinkedEditingRanges, SelectionRange, SelectionRangeParams, folding_range_kind,
};
pub use registration::{
    CallHierarchyRegistrationOptions, DeclarationRegistrationOptions,
    DiagnosticRegistrationOptions, DidChangeWatchedFilesRegistrationOptions,
    DocumentColorRegistrationOptions, DocumentFilter, DocumentSelector, FileSystemWatcher,
    FoldingRangeRegistrationOptions, GlobPattern, ImplementationRegistrationOptions,
    InlineValueRegistrationOptions, LinkedEditingRangeRegistrationOptions,
    MonikerRegistrationOptions, Registration, RegistrationParams, RelativePattern,
    SelectionRangeRegistrationOptions, SemanticTokensRegistrationOptions,
    SimpleRegistrationOptions, StaticRegistrationOptions, TextDocumentRegistrationOptions,
    TypeDefinitionRegistrationOptions, TypeHierarchyRegistrationOptions, Unregistration,
    UnregistrationParams, watch_kind,
};
pub use rename::{PrepareRenameResponse, RenameOptions, RenameParams};
pub use semantic_tokens::{
    SemanticTokens, SemanticTokensBuilder, SemanticTokensDelta, SemanticTokensDeltaParams,
    SemanticTokensDeltaResult, SemanticTokensEdit, SemanticTokensFullOptions, SemanticTokensLegend,
    SemanticTokensOptions, SemanticTokensParams, SemanticTokensRangeParams,
};
pub use signature::{
    Documentation, ParameterInformation, ParameterLabel, SignatureHelp, SignatureHelpContext,
    SignatureHelpParams, SignatureInformation,
};
pub use symbols::{
    DocumentSymbol, DocumentSymbolOptions, DocumentSymbolParams, DocumentSymbolResponse,
    SymbolInformation, WorkspaceSymbol, WorkspaceSymbolLocation, WorkspaceSymbolOptions,
    WorkspaceSymbolParams, WorkspaceSymbolResponse,
};
pub use trace::{LogTraceParams, SetTraceParams, TraceValue};
pub use window::{
    LogMessageParams, MessageActionItem, ShowDocumentParams, ShowDocumentResult, ShowMessageParams,
    ShowMessageRequestParams,
};
pub use workspace::{
    AnnotatedTextEdit, ApplyWorkspaceEditParams, ApplyWorkspaceEditResult, ChangeAnnotation,
    ConfigurationItem, ConfigurationParams, CreateFile, CreateFileOptions, DeleteFile,
    DeleteFileOptions, DidChangeConfigurationParams, DidChangeWatchedFilesParams,
    DidChangeWorkspaceFoldersParams, DocumentChange, ExecuteCommandOptions, ExecuteCommandParams,
    FileEvent, OptionalVersionedTextDocumentIdentifier, RenameFile, ResourceOperation,
    TextDocumentEdit, TextEdit, WorkspaceEdit, WorkspaceFolder, WorkspaceFoldersChangeEvent,
};
