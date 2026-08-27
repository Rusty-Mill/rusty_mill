//! `client/registerCapability` and `client/unregisterCapability` types —
//! lets a server register interest in a method after `initialize` (e.g.
//! scoped to a `documentSelector`) instead of only declaring capabilities
//! statically in [`crate::lsp::ServerCapabilities`].

use super::base::Uri;
use super::diagnostics::DiagnosticOptions;
use super::hierarchy::{CallHierarchyOptions, TypeHierarchyOptions};
use super::semantic_tokens::SemanticTokensOptions;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One capability registration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Registration {
    /// An id for this registration, unique among this server's active
    /// registrations, used to unregister it later.
    pub id: String,
    /// The method being registered for, e.g. `"textDocument/formatting"`.
    pub method: String,
    /// Method-specific registration options (e.g. a `documentSelector`),
    /// if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub register_options: Option<Value>,
}

impl Registration {
    /// Build a registration from its id, method, and options.
    pub fn new(
        id: impl Into<String>,
        method: impl Into<String>,
        register_options: Option<Value>,
    ) -> Self {
        Registration {
            id: id.into(),
            method: method.into(),
            register_options,
        }
    }
}

/// Parameters of `client/registerCapability`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegistrationParams {
    /// The registrations to add.
    pub registrations: Vec<Registration>,
}

/// One capability unregistration, referencing an earlier
/// [`Registration::id`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unregistration {
    /// The [`Registration::id`] to undo.
    pub id: String,
    /// The method that was registered for.
    pub method: String,
}

/// Parameters of `client/unregisterCapability`.
///
/// The `unregisterations` field name (missing an "r") is not a typo in this
/// crate — it matches the LSP specification's field name verbatim, which is
/// itself a long-standing, wire-compatibility-frozen misspelling.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnregistrationParams {
    /// The unregistrations to apply.
    pub unregisterations: Vec<Unregistration>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn registration_params_round_trips() {
        let params = RegistrationParams {
            registrations: vec![Registration::new(
                "1",
                "textDocument/formatting",
                Some(json!({"documentSelector": [{"language": "rust"}]})),
            )],
        };
        let value = serde_json::to_value(&params).unwrap();
        assert_eq!(
            value["registrations"][0]["method"],
            json!("textDocument/formatting")
        );
        assert_eq!(
            value["registrations"][0]["registerOptions"]["documentSelector"][0]["language"],
            json!("rust")
        );
    }

    #[test]
    fn unregistration_params_uses_spec_field_name() {
        let params = UnregistrationParams {
            unregisterations: vec![Unregistration {
                id: "1".to_owned(),
                method: "textDocument/formatting".to_owned(),
            }],
        };
        let value = serde_json::to_value(&params).unwrap();
        assert!(value.get("unregisterations").is_some());
        assert!(value.get("unregistrations").is_none());
    }
}

/// A filter selecting documents by language, URI scheme, and/or glob
/// pattern. At least one field should be set.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DocumentFilter {
    /// Match documents with this language id (e.g. `"rust"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Match documents with this URI scheme (e.g. `"file"`, `"untitled"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheme: Option<String>,
    /// Match documents whose path matches this glob (e.g. `"**/*.toml"`;
    /// `*`, `**`, `?`, `{a,b}`, `[...]` per the spec).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
}

impl DocumentFilter {
    /// Filter by language id alone.
    pub fn language(language: impl Into<String>) -> Self {
        DocumentFilter {
            language: Some(language.into()),
            ..Default::default()
        }
    }

    /// Filter by glob pattern alone.
    pub fn pattern(pattern: impl Into<String>) -> Self {
        DocumentFilter {
            pattern: Some(pattern.into()),
            ..Default::default()
        }
    }
}

/// The set of documents a dynamic registration applies to: a document
/// matches if any filter matches.
pub type DocumentSelector = Vec<DocumentFilter>;

/// The registration options common to all `textDocument/*` methods: which
/// documents the registration applies to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextDocumentRegistrationOptions {
    /// The documents the registration applies to. `None` falls back to the
    /// selector the client provides on the server's behalf.
    pub document_selector: Option<DocumentSelector>,
}

impl Registration {
    /// Build a registration scoped to the documents matching `selector`,
    /// the common case for dynamic `textDocument/*` registrations:
    ///
    /// ```rust
    /// use rusty_lsp::lsp::{DocumentFilter, Registration};
    ///
    /// let registration = Registration::for_documents(
    ///     "fmt-toml",
    ///     "textDocument/formatting",
    ///     vec![DocumentFilter::language("toml")],
    /// );
    /// ```
    pub fn for_documents(
        id: impl Into<String>,
        method: impl Into<String>,
        selector: DocumentSelector,
    ) -> Self {
        let options = TextDocumentRegistrationOptions {
            document_selector: Some(selector),
        };
        Registration {
            id: id.into(),
            method: method.into(),
            register_options: Some(
                serde_json::to_value(options).expect("registration options serialize"),
            ),
        }
    }
}

#[cfg(test)]
mod registration_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn for_documents_builds_a_document_selector() {
        let registration = Registration::for_documents(
            "fmt-1",
            "textDocument/formatting",
            vec![
                DocumentFilter::language("toml"),
                DocumentFilter::pattern("**/*.lock"),
            ],
        );
        assert_eq!(
            serde_json::to_value(&registration).unwrap(),
            json!({
                "id": "fmt-1",
                "method": "textDocument/formatting",
                "registerOptions": {
                    "documentSelector": [
                        {"language": "toml"},
                        {"pattern": "**/*.lock"},
                    ],
                },
            })
        );
    }
}

/// Bit flags for [`FileSystemWatcher::kind`]: which file events the server
/// wants reported. Combine with `|`; the spec default (all three) applies
/// when the field is omitted.
pub mod watch_kind {
    /// Report file creations.
    pub const CREATE: u32 = 1;
    /// Report file content changes.
    pub const CHANGE: u32 = 2;
    /// Report file deletions.
    pub const DELETE: u32 = 4;
    /// All of create, change, and delete (the spec default).
    pub const ALL: u32 = CREATE | CHANGE | DELETE;
}

/// The pattern a [`FileSystemWatcher`] watches: a bare glob (matched
/// against absolute paths) or a glob relative to a base URI (LSP 3.17).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GlobPattern {
    /// A glob such as `"**/*.toml"` (`*`, `**`, `?`, `{a,b}`, `[...]`).
    Pattern(String),
    /// A glob interpreted relative to a base folder.
    Relative(RelativePattern),
}

impl From<&str> for GlobPattern {
    fn from(pattern: &str) -> Self {
        GlobPattern::Pattern(pattern.to_owned())
    }
}

impl From<String> for GlobPattern {
    fn from(pattern: String) -> Self {
        GlobPattern::Pattern(pattern)
    }
}

/// A glob interpreted relative to a base folder (LSP 3.17).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelativePattern {
    /// The folder the pattern is relative to.
    pub base_uri: Uri,
    /// The glob itself.
    pub pattern: String,
}

/// One watch subscription inside
/// [`DidChangeWatchedFilesRegistrationOptions`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSystemWatcher {
    /// What to watch.
    pub glob_pattern: GlobPattern,
    /// Which events to report, as [`watch_kind`] flags; omitted means all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<u32>,
}

impl FileSystemWatcher {
    /// Watch `pattern` for all event kinds.
    pub fn new(pattern: impl Into<GlobPattern>) -> Self {
        FileSystemWatcher {
            glob_pattern: pattern.into(),
            kind: None,
        }
    }

    /// Restrict which events are reported (see [`watch_kind`]).
    #[must_use]
    pub fn with_kind(mut self, kind: u32) -> Self {
        self.kind = Some(kind);
        self
    }
}

/// Registration options of `workspace/didChangeWatchedFiles`: the file
/// patterns the client should watch on the server's behalf.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DidChangeWatchedFilesRegistrationOptions {
    /// The watch subscriptions.
    pub watchers: Vec<FileSystemWatcher>,
}

impl Registration {
    /// Build a `workspace/didChangeWatchedFiles` registration asking the
    /// client to watch the given patterns — the standard `initialized`-time
    /// call for servers that track files beyond the open set:
    ///
    /// ```rust
    /// use rusty_lsp::lsp::{FileSystemWatcher, Registration, watch_kind};
    ///
    /// let registration = Registration::for_watched_files(
    ///     "watch-manifests",
    ///     vec![
    ///         FileSystemWatcher::new("**/Cargo.toml"),
    ///         FileSystemWatcher::new("**/*.lock").with_kind(watch_kind::CHANGE),
    ///     ],
    /// );
    /// ```
    pub fn for_watched_files(id: impl Into<String>, watchers: Vec<FileSystemWatcher>) -> Self {
        let options = DidChangeWatchedFilesRegistrationOptions { watchers };
        Registration {
            id: id.into(),
            method: "workspace/didChangeWatchedFiles".to_owned(),
            register_options: Some(
                serde_json::to_value(options).expect("watcher options serialize"),
            ),
        }
    }
}

#[cfg(test)]
mod watcher_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn for_watched_files_builds_the_registration() {
        let registration = Registration::for_watched_files(
            "w1",
            vec![
                FileSystemWatcher::new("**/*.toml"),
                FileSystemWatcher::new(GlobPattern::Relative(RelativePattern {
                    base_uri: "file:///ws".into(),
                    pattern: "src/**".to_owned(),
                }))
                .with_kind(watch_kind::CREATE | watch_kind::DELETE),
            ],
        );
        assert_eq!(
            serde_json::to_value(&registration).unwrap(),
            json!({
                "id": "w1",
                "method": "workspace/didChangeWatchedFiles",
                "registerOptions": {
                    "watchers": [
                        {"globPattern": "**/*.toml"},
                        {
                            "globPattern": {"baseUri": "file:///ws", "pattern": "src/**"},
                            "kind": 5,
                        },
                    ],
                },
            })
        );
    }
}

/// The `{ id?: string }` shape shared by every `*RegistrationOptions` type
/// that supports being referenced later by a fixed id, e.g. so a
/// `workspace/semanticTokens/refresh`-style request can be correlated back
/// to the registration that produced the tokens being refreshed.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StaticRegistrationOptions {
    /// The registration's static id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// Registration options shared by every dynamically-registerable method
/// whose only server-side option is `workDoneProgress` support:
/// `textDocument/declaration`, `typeDefinition`, `implementation`,
/// `documentColor`/`colorPresentation`, `foldingRange`, `selectionRange`,
/// `moniker`, `linkedEditingRange`, and `inlineValue` all share this exact
/// shape in the spec, so one struct (and matching type aliases below, named
/// for discoverability) covers all nine rather than nine near-duplicates.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimpleRegistrationOptions {
    /// The documents the registration applies to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_selector: Option<DocumentSelector>,
    /// Whether the server reports work-done progress for this provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_done_progress: Option<bool>,
    /// The registration's static id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// Registration options of `textDocument/declaration` (LSP 3.14).
pub type DeclarationRegistrationOptions = SimpleRegistrationOptions;
/// Registration options of `textDocument/typeDefinition` (LSP 3.6).
pub type TypeDefinitionRegistrationOptions = SimpleRegistrationOptions;
/// Registration options of `textDocument/implementation` (LSP 3.6).
pub type ImplementationRegistrationOptions = SimpleRegistrationOptions;
/// Registration options of `textDocument/documentColor`/`colorPresentation`
/// (LSP 3.6).
pub type DocumentColorRegistrationOptions = SimpleRegistrationOptions;
/// Registration options of `textDocument/foldingRange` (LSP 3.10).
pub type FoldingRangeRegistrationOptions = SimpleRegistrationOptions;
/// Registration options of `textDocument/selectionRange` (LSP 3.15).
pub type SelectionRangeRegistrationOptions = SimpleRegistrationOptions;
/// Registration options of `textDocument/moniker` (LSP 3.16).
pub type MonikerRegistrationOptions = SimpleRegistrationOptions;
/// Registration options of `textDocument/linkedEditingRange` (LSP 3.16).
pub type LinkedEditingRangeRegistrationOptions = SimpleRegistrationOptions;
/// Registration options of `textDocument/inlineValue` (LSP 3.17).
pub type InlineValueRegistrationOptions = SimpleRegistrationOptions;

impl Registration {
    /// Build a registration for one of the [`SimpleRegistrationOptions`]
    /// methods (see its doc comment for the full list) — like
    /// [`for_documents`](Self::for_documents), but also carrying
    /// `work_done_progress`/`id` when the caller wants them set:
    ///
    /// ```rust
    /// use rusty_lsp::lsp::{DocumentFilter, Registration, SimpleRegistrationOptions};
    ///
    /// let registration = Registration::for_documents_with_options(
    ///     "decl-1",
    ///     "textDocument/declaration",
    ///     vec![DocumentFilter::language("rust")],
    ///     SimpleRegistrationOptions {
    ///         work_done_progress: Some(true),
    ///         ..Default::default()
    ///     },
    /// );
    /// ```
    pub fn for_documents_with_options(
        id: impl Into<String>,
        method: impl Into<String>,
        selector: DocumentSelector,
        options: SimpleRegistrationOptions,
    ) -> Self {
        let options = SimpleRegistrationOptions {
            document_selector: Some(selector),
            ..options
        };
        Registration::new(
            id,
            method,
            Some(serde_json::to_value(options).expect("registration options serialize")),
        )
    }
}

/// Registration options of `textDocument/semanticTokens` (LSP 3.16), covering
/// both `.../full` and `.../range`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticTokensRegistrationOptions {
    /// The documents the registration applies to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_selector: Option<DocumentSelector>,
    /// The semantic-tokens options (legend, range/full support, …).
    #[serde(flatten)]
    pub options: SemanticTokensOptions,
    /// The registration's static id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// Registration options of `textDocument/diagnostic` (LSP 3.17).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticRegistrationOptions {
    /// The documents the registration applies to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_selector: Option<DocumentSelector>,
    /// The diagnostic-pull-model options.
    #[serde(flatten)]
    pub options: DiagnosticOptions,
    /// The registration's static id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// Registration options of `textDocument/prepareCallHierarchy` (LSP 3.16).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallHierarchyRegistrationOptions {
    /// The documents the registration applies to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_selector: Option<DocumentSelector>,
    /// The call-hierarchy options.
    #[serde(flatten)]
    pub options: CallHierarchyOptions,
    /// The registration's static id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// Registration options of `textDocument/prepareTypeHierarchy` (LSP 3.17).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeHierarchyRegistrationOptions {
    /// The documents the registration applies to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_selector: Option<DocumentSelector>,
    /// The type-hierarchy options.
    #[serde(flatten)]
    pub options: TypeHierarchyOptions,
    /// The registration's static id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

impl Registration {
    /// Build a `textDocument/semanticTokens` registration.
    pub fn for_semantic_tokens(
        id: impl Into<String>,
        selector: DocumentSelector,
        options: SemanticTokensOptions,
    ) -> Self {
        let options = SemanticTokensRegistrationOptions {
            document_selector: Some(selector),
            options,
            id: None,
        };
        Registration::new(
            id,
            "textDocument/semanticTokens",
            Some(serde_json::to_value(options).expect("registration options serialize")),
        )
    }

    /// Build a `textDocument/diagnostic` registration.
    pub fn for_diagnostic(
        id: impl Into<String>,
        selector: DocumentSelector,
        options: DiagnosticOptions,
    ) -> Self {
        let options = DiagnosticRegistrationOptions {
            document_selector: Some(selector),
            options,
            id: None,
        };
        Registration::new(
            id,
            "textDocument/diagnostic",
            Some(serde_json::to_value(options).expect("registration options serialize")),
        )
    }

    /// Build a `textDocument/prepareCallHierarchy` registration.
    pub fn for_call_hierarchy(
        id: impl Into<String>,
        selector: DocumentSelector,
        options: CallHierarchyOptions,
    ) -> Self {
        let options = CallHierarchyRegistrationOptions {
            document_selector: Some(selector),
            options,
            id: None,
        };
        Registration::new(
            id,
            "textDocument/prepareCallHierarchy",
            Some(serde_json::to_value(options).expect("registration options serialize")),
        )
    }

    /// Build a `textDocument/prepareTypeHierarchy` registration.
    pub fn for_type_hierarchy(
        id: impl Into<String>,
        selector: DocumentSelector,
        options: TypeHierarchyOptions,
    ) -> Self {
        let options = TypeHierarchyRegistrationOptions {
            document_selector: Some(selector),
            options,
            id: None,
        };
        Registration::new(
            id,
            "textDocument/prepareTypeHierarchy",
            Some(serde_json::to_value(options).expect("registration options serialize")),
        )
    }
}

#[cfg(test)]
mod typed_registration_options_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn for_documents_with_options_sets_work_done_progress_and_id() {
        let registration = Registration::for_documents_with_options(
            "decl-1",
            "textDocument/declaration",
            vec![DocumentFilter::language("rust")],
            SimpleRegistrationOptions {
                work_done_progress: Some(true),
                id: Some("static-1".to_owned()),
                ..Default::default()
            },
        );
        assert_eq!(
            serde_json::to_value(&registration).unwrap(),
            json!({
                "id": "decl-1",
                "method": "textDocument/declaration",
                "registerOptions": {
                    "documentSelector": [{"language": "rust"}],
                    "workDoneProgress": true,
                    "id": "static-1",
                },
            })
        );
    }

    #[test]
    fn for_semantic_tokens_flattens_the_options() {
        let registration = Registration::for_semantic_tokens(
            "tok-1",
            vec![DocumentFilter::language("rust")],
            SemanticTokensOptions {
                work_done_progress: None,
                legend: super::super::semantic_tokens::SemanticTokensLegend {
                    token_types: vec!["keyword".to_owned()],
                    token_modifiers: vec![],
                },
                range: Some(true),
                full: None,
            },
        );
        let value = serde_json::to_value(&registration).unwrap();
        assert_eq!(value["method"], json!("textDocument/semanticTokens"));
        assert_eq!(
            value["registerOptions"]["documentSelector"][0]["language"],
            json!("rust")
        );
        assert_eq!(
            value["registerOptions"]["legend"]["tokenTypes"][0],
            json!("keyword")
        );
        assert_eq!(value["registerOptions"]["range"], json!(true));
        assert!(value["registerOptions"].get("id").is_none());
    }

    #[test]
    fn for_diagnostic_flattens_the_options() {
        let registration = Registration::for_diagnostic(
            "diag-1",
            vec![DocumentFilter::language("rust")],
            DiagnosticOptions {
                identifier: Some("clippy".to_owned()),
                inter_file_dependencies: true,
                workspace_diagnostics: true,
                ..Default::default()
            },
        );
        let value = serde_json::to_value(&registration).unwrap();
        assert_eq!(value["registerOptions"]["identifier"], json!("clippy"));
        assert_eq!(
            value["registerOptions"]["interFileDependencies"],
            json!(true)
        );
        assert_eq!(
            value["registerOptions"]["workspaceDiagnostics"],
            json!(true)
        );
    }

    #[test]
    fn for_call_hierarchy_and_type_hierarchy_use_the_prepare_methods() {
        let selector = vec![DocumentFilter::language("rust")];
        let call = Registration::for_call_hierarchy(
            "ch-1",
            selector.clone(),
            CallHierarchyOptions::default(),
        );
        assert_eq!(call.method, "textDocument/prepareCallHierarchy");

        let ty =
            Registration::for_type_hierarchy("th-1", selector, TypeHierarchyOptions::default());
        assert_eq!(ty.method, "textDocument/prepareTypeHierarchy");
    }
}
