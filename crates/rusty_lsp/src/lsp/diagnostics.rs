//! Diagnostic types: the `textDocument/publishDiagnostics` notification
//! (push model), and `textDocument/diagnostic` / `workspace/diagnostic`
//! (pull model, LSP 3.17 — the client asks for diagnostics instead of
//! waiting for the server to push them, which scales better for
//! expensive-to-compute or huge-workspace diagnostics).

use super::base::{Location, Range, TextDocumentIdentifier, Uri};
use super::enums::{DiagnosticSeverity, DiagnosticTag};
use super::progress::{PartialResultParams, WorkDoneProgressParams};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A diagnostic — an error, warning, or hint anchored to a document range.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    /// The range the diagnostic applies to.
    pub range: Range,
    /// The diagnostic's severity. Clients may render differently when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<DiagnosticSeverity>,
    /// A machine-readable code (number or string).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<Value>,
    /// A link explaining the code (LSP 3.16), e.g. a lint's documentation
    /// page; clients render the code as clickable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_description: Option<CodeDescription>,
    /// A human-readable origin, e.g. the linter name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// The diagnostic message.
    pub message: String,
    /// Tags qualifying the diagnostic (unnecessary/deprecated), letting
    /// clients fade or strike the range instead of squiggling it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<DiagnosticTag>>,
    /// Secondary locations that explain the diagnostic, e.g. "first
    /// declared here" for a duplicate-definition error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub related_information: Option<Vec<DiagnosticRelatedInformation>>,
    /// Opaque payload preserved by the client and handed back on the
    /// `textDocument/codeAction` request for this diagnostic — the standard
    /// way to carry quick-fix context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl Diagnostic {
    /// Build a diagnostic with a range, severity, and message.
    pub fn new(range: Range, severity: DiagnosticSeverity, message: impl Into<String>) -> Self {
        Diagnostic {
            range,
            severity: Some(severity),
            code: None,
            code_description: None,
            source: None,
            message: message.into(),
            tags: None,
            related_information: None,
            data: None,
        }
    }

    /// Set the diagnostic's source label.
    #[must_use]
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Set the diagnostic's machine-readable code, optionally with a link
    /// explaining it.
    #[must_use]
    pub fn with_code(mut self, code: impl Into<Value>, href: Option<Uri>) -> Self {
        self.code = Some(code.into());
        self.code_description = href.map(|href| CodeDescription { href });
        self
    }

    /// Add a tag (unnecessary/deprecated) to the diagnostic.
    #[must_use]
    pub fn with_tag(mut self, tag: DiagnosticTag) -> Self {
        self.tags.get_or_insert_with(Vec::new).push(tag);
        self
    }

    /// Add a secondary location that explains the diagnostic.
    #[must_use]
    pub fn with_related(mut self, location: Location, message: impl Into<String>) -> Self {
        self.related_information
            .get_or_insert_with(Vec::new)
            .push(DiagnosticRelatedInformation {
                location,
                message: message.into(),
            });
        self
    }

    /// Attach the opaque payload handed back on `textDocument/codeAction`.
    #[must_use]
    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }
}

/// A link explaining a diagnostic's [`code`](Diagnostic::code) (LSP 3.16).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeDescription {
    /// The URI to open, typically the lint's documentation page.
    pub href: Uri,
}

/// A secondary location related to a [`Diagnostic`], with its own message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticRelatedInformation {
    /// The related location.
    pub location: Location,
    /// What that location contributes, e.g. `"first declared here"`.
    pub message: String,
}

/// Parameters of `textDocument/publishDiagnostics`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishDiagnosticsParams {
    /// The document the diagnostics belong to.
    pub uri: Uri,
    /// The document version the diagnostics were computed against, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<i32>,
    /// The full set of diagnostics for the document (replaces any previous set).
    pub diagnostics: Vec<Diagnostic>,
}

/// Parameters of `textDocument/diagnostic` (the pull-model request).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentDiagnosticParams {
    /// The document to compute diagnostics for.
    pub text_document: TextDocumentIdentifier,
    /// Distinguishes this diagnostic source when a document has more than
    /// one (matches [`DiagnosticOptions::identifier`](crate::lsp::DiagnosticOptions::identifier)).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    /// The [`FullDocumentDiagnosticReport::result_id`] of the previous
    /// result, so the server can reply
    /// [`Unchanged`](DocumentDiagnosticReport::Unchanged) if nothing changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_result_id: Option<String>,
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result: PartialResultParams,
}

/// The result of a `textDocument/diagnostic` request: either a fresh
/// [`Full`](Self::Full) report, or confirmation that nothing changed since
/// [`DocumentDiagnosticParams::previous_result_id`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum DocumentDiagnosticReport {
    /// A complete, fresh set of diagnostics.
    Full(FullDocumentDiagnosticReport),
    /// Diagnostics are unchanged since `previous_result_id`; the client
    /// should keep using what it already has.
    Unchanged(UnchangedDocumentDiagnosticReport),
}

/// A complete, fresh set of diagnostics for one document.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FullDocumentDiagnosticReport {
    /// An opaque id for this result, to pass back as a later request's
    /// `previous_result_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_id: Option<String>,
    /// The diagnostics.
    pub items: Vec<Diagnostic>,
}

/// Confirms a document's diagnostics are unchanged since a previous result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnchangedDocumentDiagnosticReport {
    /// The unchanged result's id (echoes the request's `previous_result_id`).
    pub result_id: String,
}

/// Parameters of `workspace/diagnostic` (the workspace-wide pull-model
/// request).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDiagnosticParams {
    /// Distinguishes this diagnostic source when a document has more than
    /// one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    /// Result ids from a previous `workspace/diagnostic` response, one per
    /// document the client already has diagnostics for.
    pub previous_result_ids: Vec<PreviousResultId>,
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result: PartialResultParams,
}

/// One entry in [`WorkspaceDiagnosticParams::previous_result_ids`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviousResultId {
    /// The document the result id belongs to.
    pub uri: Uri,
    /// The previous result's id.
    pub value: String,
}

/// The result of a `workspace/diagnostic` request.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WorkspaceDiagnosticReport {
    /// Diagnostics per document, in server-chosen order.
    pub items: Vec<WorkspaceDocumentDiagnosticReport>,
}

/// One document's entry in a [`WorkspaceDiagnosticReport`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum WorkspaceDocumentDiagnosticReport {
    /// A complete, fresh set of diagnostics for this document.
    Full(WorkspaceFullDocumentDiagnosticReport),
    /// This document's diagnostics are unchanged since the matching
    /// `previous_result_ids` entry.
    Unchanged(WorkspaceUnchangedDocumentDiagnosticReport),
}

/// A complete, fresh set of diagnostics for one document, as part of a
/// workspace-wide report.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFullDocumentDiagnosticReport {
    /// The document these diagnostics belong to.
    pub uri: Uri,
    /// The document's version, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<i32>,
    /// An opaque id for this result, to pass back in a later
    /// `previous_result_ids` entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_id: Option<String>,
    /// The diagnostics.
    pub items: Vec<Diagnostic>,
}

/// Confirms one document's diagnostics are unchanged, as part of a
/// workspace-wide report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceUnchangedDocumentDiagnosticReport {
    /// The document these diagnostics belong to.
    pub uri: Uri,
    /// The document's version, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<i32>,
    /// The unchanged result's id.
    pub result_id: String,
}

/// Options describing the server's diagnostic-pull-model support.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticOptions {
    /// Whether the server reports work-done progress for this provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_done_progress: Option<bool>,
    /// Distinguishes this diagnostic source from others the server may also
    /// provide for the same document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    /// Whether one document's diagnostics can change because of edits to a
    /// *different* document (e.g. a header file). If `true`, clients should
    /// re-pull dependent documents more conservatively.
    pub inter_file_dependencies: bool,
    /// Whether the server supports `workspace/diagnostic`.
    pub workspace_diagnostics: bool,
}

#[cfg(test)]
mod tests {
    use super::super::base::Position;
    use super::*;
    use serde_json::json;

    #[test]
    fn document_diagnostic_report_tags_by_kind() {
        let full = DocumentDiagnosticReport::Full(FullDocumentDiagnosticReport {
            result_id: Some("1".to_owned()),
            items: vec![Diagnostic::new(
                Range::new(Position::new(0, 0), Position::new(0, 1)),
                DiagnosticSeverity::Warning,
                "oops",
            )],
        });
        let value = serde_json::to_value(&full).unwrap();
        assert_eq!(value["kind"], json!("full"));
        assert_eq!(value["items"][0]["message"], json!("oops"));

        let unchanged = DocumentDiagnosticReport::Unchanged(UnchangedDocumentDiagnosticReport {
            result_id: "1".to_owned(),
        });
        let value = serde_json::to_value(&unchanged).unwrap();
        assert_eq!(value, json!({"kind": "unchanged", "resultId": "1"}));
    }

    #[test]
    fn document_diagnostic_params_flattens_progress_mixins() {
        let value = json!({
            "textDocument": {"uri": "file:///a"},
            "workDoneToken": "t1",
            "partialResultToken": "t2",
        });
        let params: DocumentDiagnosticParams = serde_json::from_value(value).unwrap();
        assert_eq!(
            params.work_done.work_done_token,
            Some(super::super::progress::ProgressToken::from("t1"))
        );
        assert_eq!(
            params.partial_result.partial_result_token,
            Some(super::super::progress::ProgressToken::from("t2"))
        );
    }
}

#[cfg(test)]
mod diagnostic_field_tests {
    use super::*;
    use crate::lsp::{DiagnosticSeverity, Position};
    use serde_json::json;

    #[test]
    fn diagnostic_rich_fields_round_trip() {
        let range = Range::new(Position::new(1, 0), Position::new(1, 4));
        let diagnostic = Diagnostic::new(range, DiagnosticSeverity::Warning, "unused variable")
            .with_source("lint")
            .with_code(
                "unused_variables",
                Some("https://doc.rust-lang.org/error_codes".into()),
            )
            .with_tag(DiagnosticTag::Unnecessary)
            .with_related(
                Location {
                    uri: "file:///lib.rs".into(),
                    range,
                },
                "declared here",
            )
            .with_data(json!({"fix": "remove"}));

        let value = serde_json::to_value(&diagnostic).unwrap();
        assert_eq!(value["tags"], json!([1]));
        assert_eq!(
            value["codeDescription"]["href"],
            json!("https://doc.rust-lang.org/error_codes")
        );
        assert_eq!(
            value["relatedInformation"][0]["message"],
            json!("declared here")
        );
        assert_eq!(value["data"], json!({"fix": "remove"}));

        let parsed: Diagnostic = serde_json::from_value(value).unwrap();
        assert_eq!(parsed, diagnostic);
    }

    #[test]
    fn minimal_diagnostic_omits_rich_fields() {
        let range = Range::new(Position::new(0, 0), Position::new(0, 1));
        let value = serde_json::to_value(Diagnostic::new(range, DiagnosticSeverity::Error, "boom"))
            .unwrap();
        assert!(value.get("tags").is_none());
        assert!(value.get("relatedInformation").is_none());
        assert!(value.get("codeDescription").is_none());
        assert!(value.get("data").is_none());
    }

    #[test]
    fn diagnostic_options_advertise_work_done_progress() {
        let options = DiagnosticOptions {
            work_done_progress: Some(true),
            identifier: None,
            inter_file_dependencies: false,
            workspace_diagnostics: true,
        };
        let value = serde_json::to_value(&options).unwrap();
        assert_eq!(value["workDoneProgress"], json!(true));
        assert!(value.get("identifier").is_none());
        assert_eq!(value["workspaceDiagnostics"], json!(true));
    }
}
