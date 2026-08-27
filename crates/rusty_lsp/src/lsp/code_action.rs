//! `textDocument/codeAction` types.

use super::base::{Range, TextDocumentIdentifier};
use super::diagnostics::Diagnostic;
use super::enums::CodeActionTriggerKind;
use super::progress::{PartialResultParams, WorkDoneProgressParams};
use super::workspace::WorkspaceEdit;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Well-known [`CodeActionKind`] values from the spec. `CodeActionKind` is an
/// open string enum — servers may return any dotted value — so these are
/// plain constants rather than a closed Rust enum.
pub mod code_action_kind {
    /// The empty kind, matching every `only` filter.
    pub const EMPTY: &str = "";
    /// A fix for a problem, typically tied to a diagnostic.
    pub const QUICKFIX: &str = "quickfix";
    /// A general refactor not covered by a more specific kind below.
    pub const REFACTOR: &str = "refactor";
    /// Extract code into a new function/variable/etc.
    pub const REFACTOR_EXTRACT: &str = "refactor.extract";
    /// Inline a function/variable/etc.
    pub const REFACTOR_INLINE: &str = "refactor.inline";
    /// Rewrite code in place (e.g. convert a loop to an iterator chain).
    pub const REFACTOR_REWRITE: &str = "refactor.rewrite";
    /// A source-wide action not tied to a specific diagnostic.
    pub const SOURCE: &str = "source";
    /// Organize imports.
    pub const SOURCE_ORGANIZE_IMPORTS: &str = "source.organizeImports";
    /// Fix all auto-fixable problems in the document.
    pub const SOURCE_FIX_ALL: &str = "source.fixAll";
}

/// A [`CodeActionKind`] value, e.g. `"quickfix"` or `"refactor.extract"`. An
/// open string enum per the spec (see the [`code_action_kind`] module for
/// well-known values), not a closed Rust enum.
pub type CodeActionKind = String;

/// Parameters of `textDocument/codeAction`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeActionParams {
    /// The document to compute code actions for.
    pub text_document: TextDocumentIdentifier,
    /// The range the actions apply to (e.g. the current selection).
    pub range: Range,
    /// Additional context, primarily the diagnostics in range.
    pub context: CodeActionContext,
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result: PartialResultParams,
}

/// Additional context for a `textDocument/codeAction` request.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeActionContext {
    /// Diagnostics currently known for the requested range.
    pub diagnostics: Vec<Diagnostic>,
    /// Restrict results to these kinds, if given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub only: Option<Vec<CodeActionKind>>,
    /// What triggered this request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_kind: Option<CodeActionTriggerKind>,
}

/// One element of a `textDocument/codeAction` response: either a full
/// [`CodeAction`] or a bare [`Command`] (an older, simpler form).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CodeActionOrCommand {
    /// A code action, optionally carrying its own edit and/or command.
    Action(Box<CodeAction>),
    /// A bare command with no inline edit.
    Command(Command),
}

impl From<CodeAction> for CodeActionOrCommand {
    fn from(action: CodeAction) -> Self {
        CodeActionOrCommand::Action(Box::new(action))
    }
}

impl From<Command> for CodeActionOrCommand {
    fn from(command: Command) -> Self {
        CodeActionOrCommand::Command(command)
    }
}

/// A code action: a title plus how to carry it out (an inline
/// [`WorkspaceEdit`], a [`Command`] to execute, or both).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeAction {
    /// A short, user-facing title.
    pub title: String,
    /// The action's kind (see [`code_action_kind`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<CodeActionKind>,
    /// Diagnostics this action resolves, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<Vec<Diagnostic>>,
    /// Whether this is the preferred action among several for the same
    /// diagnostic/range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_preferred: Option<bool>,
    /// If set, explains why this action is shown disabled rather than
    /// omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<CodeActionDisabled>,
    /// The edit to apply, if this action carries one inline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit: Option<WorkspaceEdit>,
    /// A command to execute (before or instead of `edit`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<Command>,
    /// Opaque data round-tripped through `codeAction/resolve`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl CodeAction {
    /// Build a code action from just a title.
    pub fn new(title: impl Into<String>) -> Self {
        CodeAction {
            title: title.into(),
            ..Default::default()
        }
    }

    /// Set the action's kind.
    #[must_use]
    pub fn with_kind(mut self, kind: impl Into<CodeActionKind>) -> Self {
        self.kind = Some(kind.into());
        self
    }

    /// Attach an inline edit.
    #[must_use]
    pub fn with_edit(mut self, edit: WorkspaceEdit) -> Self {
        self.edit = Some(edit);
        self
    }
}

/// Explains why a [`CodeAction`] is shown but disabled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeActionDisabled {
    /// A human-readable reason, shown to the user.
    pub reason: String,
}

/// A reference to a command the client can invoke, either standalone or
/// attached to a [`CodeAction`]/`CodeLens`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Command {
    /// A short, user-facing title.
    pub title: String,
    /// The command identifier, dispatched back to the server via
    /// `workspace/executeCommand`.
    pub command: String,
    /// Arguments for the command, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Vec<Value>>,
}

/// Options describing the server's `textDocument/codeAction` support.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeActionOptions {
    /// Whether the server reports work-done progress for this provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_done_progress: Option<bool>,
    /// The kinds of code action the server may return, if it wants to
    /// advertise them upfront.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub code_action_kinds: Vec<CodeActionKind>,
    /// Whether the server supports `codeAction/resolve` for lazily filling
    /// in `edit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolve_provider: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn code_action_or_command_is_untagged() {
        let action: CodeActionOrCommand = CodeAction::new("Fix it")
            .with_kind(code_action_kind::QUICKFIX)
            .into();
        let value = serde_json::to_value(&action).unwrap();
        assert_eq!(value["title"], json!("Fix it"));
        assert_eq!(value["kind"], json!("quickfix"));
        assert!(value.get("command").is_none());

        let command: CodeActionOrCommand = Command {
            title: "Run".to_owned(),
            command: "my.command".to_owned(),
            arguments: None,
        }
        .into();
        let value = serde_json::to_value(&command).unwrap();
        assert_eq!(value["command"], json!("my.command"));
        assert!(value.get("edit").is_none());
    }

    #[test]
    fn code_action_options_advertise_work_done_progress() {
        let options = CodeActionOptions {
            work_done_progress: Some(true),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(&options).unwrap(),
            json!({"workDoneProgress": true})
        );
        assert_eq!(
            serde_json::to_value(CodeActionOptions::default()).unwrap(),
            json!({})
        );
    }
}
