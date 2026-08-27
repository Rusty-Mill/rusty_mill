//! Language-feature request parameters and results: hover, completion,
//! goto-definition, find-references, and document highlights.

use super::base::{Location, Range, TextDocumentPositionParams};
use super::code_action::Command;
use super::enums::{
    CompletionItemKind, CompletionItemTag, CompletionTriggerKind, DocumentHighlightKind,
    InsertTextFormat, MarkupKind,
};
use super::progress::{PartialResultParams, WorkDoneProgressParams};
use super::signature::Documentation;
use super::workspace::TextEdit;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Parameters of `textDocument/hover`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoverParams {
    /// The document and position the hover was requested at.
    #[serde(flatten)]
    pub text_document_position: TextDocumentPositionParams,
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
}

/// The result of a hover request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hover {
    /// The hover's rendered content.
    pub contents: MarkupContent,
    /// The range the hover applies to, used by clients to highlight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<Range>,
}

impl Hover {
    /// Build a plain-text hover.
    pub fn plain_text(value: impl Into<String>) -> Self {
        Hover {
            contents: MarkupContent::plain_text(value),
            range: None,
        }
    }

    /// Build a Markdown hover.
    pub fn markdown(value: impl Into<String>) -> Self {
        Hover {
            contents: MarkupContent::markdown(value),
            range: None,
        }
    }

    /// Attach a highlight range to this hover.
    #[must_use]
    pub fn with_range(mut self, range: Range) -> Self {
        self.range = Some(range);
        self
    }
}

/// A string rendered by the client, either plain text or Markdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkupContent {
    /// Whether `value` is plain text or Markdown.
    pub kind: MarkupKind,
    /// The content itself.
    pub value: String,
}

impl MarkupContent {
    /// Build plain-text markup.
    pub fn plain_text(value: impl Into<String>) -> Self {
        MarkupContent {
            kind: MarkupKind::PlainText,
            value: value.into(),
        }
    }

    /// Build Markdown markup.
    pub fn markdown(value: impl Into<String>) -> Self {
        MarkupContent {
            kind: MarkupKind::Markdown,
            value: value.into(),
        }
    }
}

/// Parameters of `textDocument/completion`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionParams {
    /// The document and position completion was requested at.
    #[serde(flatten)]
    pub text_document_position: TextDocumentPositionParams,
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result: PartialResultParams,
    /// Additional information about the completion trigger.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<CompletionContext>,
}

/// Additional completion-trigger context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionContext {
    /// How completion was triggered.
    pub trigger_kind: CompletionTriggerKind,
    /// The trigger character, when `trigger_kind` is
    /// [`CompletionTriggerKind::TriggerCharacter`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_character: Option<String>,
}

/// The result of a completion request: either a bare list of items or a
/// [`CompletionList`] that can flag itself incomplete.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CompletionResponse {
    /// A simple array of items (treated as complete).
    Array(Vec<CompletionItem>),
    /// A list that may be marked incomplete to request re-querying.
    List(CompletionList),
}

impl From<Vec<CompletionItem>> for CompletionResponse {
    fn from(items: Vec<CompletionItem>) -> Self {
        CompletionResponse::Array(items)
    }
}

impl From<CompletionList> for CompletionResponse {
    fn from(list: CompletionList) -> Self {
        CompletionResponse::List(list)
    }
}

/// A list of completion items with an incompleteness flag.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionList {
    /// When `true`, the client should re-query as the user keeps typing.
    pub is_incomplete: bool,
    /// Defaults shared by all `items` (LSP 3.17), so large lists need not
    /// repeat identical `commitCharacters`/`editRange`/… on every item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_defaults: Option<CompletionItemDefaults>,
    /// The completion items.
    pub items: Vec<CompletionItem>,
}

/// Per-list default values for [`CompletionItem`] fields (LSP 3.17); an
/// item that omits the field inherits the default.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionItemDefaults {
    /// Default commit characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_characters: Option<Vec<String>>,
    /// Default edit range (shared by items that carry only text).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit_range: Option<CompletionEditRange>,
    /// Default insert-text format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub insert_text_format: Option<InsertTextFormat>,
    /// Default `data` payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// The `editRange` default: a single range, or separate insert/replace
/// ranges (mirroring [`InsertReplaceEdit`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CompletionEditRange {
    /// One range used for both insert and replace.
    Single(Range),
    /// Distinct insert and replace ranges.
    InsertReplace {
        /// The range replaced when inserting.
        insert: Range,
        /// The range replaced when replacing.
        replace: Range,
    },
}

/// A single completion proposal.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionItem {
    /// The label shown in the completion UI; also the default insert text.
    pub label: String,
    /// Extra label parts rendered less prominently (LSP 3.17), e.g. a
    /// signature after the name and the defining module at the right edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_details: Option<CompletionItemLabelDetails>,
    /// The item's semantic kind, used to choose an icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<CompletionItemKind>,
    /// Tags qualifying the item (e.g. deprecated).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<CompletionItemTag>>,
    /// A short detail string shown next to the label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Documentation rendered when the item is highlighted — a plain string
    /// or [`MarkupContent`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation: Option<Documentation>,
    /// String used when sorting items; defaults to `label`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_text: Option<String>,
    /// String matched against what the user typed; defaults to `label`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_text: Option<String>,
    /// Text inserted instead of `label`, when they differ. Prefer
    /// [`text_edit`](Self::text_edit) for precise placement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub insert_text: Option<String>,
    /// How `insert_text` (or `text_edit`'s new text) is interpreted:
    /// verbatim, or as a snippet with tab stops.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub insert_text_format: Option<InsertTextFormat>,
    /// The exact edit applied on acceptance, replacing a specific range —
    /// what most clients prefer over `insert_text`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_edit: Option<CompletionTextEdit>,
    /// Extra edits applied alongside acceptance, e.g. inserting an import
    /// at the top of the file. Must not overlap the main edit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additional_text_edits: Option<Vec<TextEdit>>,
    /// A command run after the item is inserted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<Command>,
    /// Opaque payload round-tripped through `completionItem/resolve` — use
    /// this to identify which item is being resolved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Secondary label parts of a [`CompletionItem`] (LSP 3.17).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionItemLabelDetails {
    /// Rendered directly after the label, e.g. a function signature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Rendered less prominently after `detail`, e.g. the defining module.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A [`CompletionItem`]'s main edit: a plain [`TextEdit`], or the 3.16
/// insert/replace form that lets the client pick between inserting before
/// and replacing the word under the cursor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CompletionTextEdit {
    /// A single-range edit.
    Edit(TextEdit),
    /// An edit with distinct insert and replace ranges.
    InsertReplace(InsertReplaceEdit),
}

impl From<TextEdit> for CompletionTextEdit {
    fn from(edit: TextEdit) -> Self {
        CompletionTextEdit::Edit(edit)
    }
}

/// An edit carrying separate ranges for insert vs. replace behaviour
/// (LSP 3.16).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InsertReplaceEdit {
    /// The text to insert.
    pub new_text: String,
    /// The range replaced when the client inserts.
    pub insert: Range,
    /// The range replaced when the client replaces.
    pub replace: Range,
}

impl CompletionItem {
    /// Build a completion item from just a label.
    pub fn new(label: impl Into<String>) -> Self {
        CompletionItem {
            label: label.into(),
            ..Default::default()
        }
    }

    /// Set the item's kind.
    #[must_use]
    pub fn with_kind(mut self, kind: CompletionItemKind) -> Self {
        self.kind = Some(kind);
        self
    }

    /// Set the item's detail string.
    #[must_use]
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Set the exact edit applied on acceptance.
    #[must_use]
    pub fn with_text_edit(mut self, edit: impl Into<CompletionTextEdit>) -> Self {
        self.text_edit = Some(edit.into());
        self
    }

    /// Mark the insert text as an LSP snippet (`${1:placeholder}`, `$0`).
    #[must_use]
    pub fn as_snippet(mut self) -> Self {
        self.insert_text_format = Some(InsertTextFormat::Snippet);
        self
    }

    /// Attach the opaque payload round-tripped through
    /// `completionItem/resolve`.
    #[must_use]
    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }
}

/// Parameters of `textDocument/documentHighlight`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentHighlightParams {
    /// The document and position to compute highlights for.
    #[serde(flatten)]
    pub text_document_position: TextDocumentPositionParams,
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result: PartialResultParams,
}

/// One occurrence of the symbol under the cursor, highlighted by the client
/// (e.g. every use of a variable in the current document).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentHighlight {
    /// The range to highlight.
    pub range: Range,
    /// The occurrence's kind (read/write/textual); clients may style each
    /// differently.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<DocumentHighlightKind>,
}

impl DocumentHighlight {
    /// Build a highlight from a range, with no specific kind.
    pub fn new(range: Range) -> Self {
        DocumentHighlight { range, kind: None }
    }

    /// Set the highlight's kind.
    #[must_use]
    pub fn with_kind(mut self, kind: DocumentHighlightKind) -> Self {
        self.kind = Some(kind);
        self
    }
}

/// Parameters of `textDocument/definition`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefinitionParams {
    /// The document and position definition was requested at.
    #[serde(flatten)]
    pub text_document_position: TextDocumentPositionParams,
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result: PartialResultParams,
}

/// Parameters of `textDocument/declaration`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclarationParams {
    /// The document and position declaration was requested at.
    #[serde(flatten)]
    pub text_document_position: TextDocumentPositionParams,
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result: PartialResultParams,
}

/// Parameters of `textDocument/typeDefinition`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeDefinitionParams {
    /// The document and position type-definition was requested at.
    #[serde(flatten)]
    pub text_document_position: TextDocumentPositionParams,
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result: PartialResultParams,
}

/// Parameters of `textDocument/implementation`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImplementationParams {
    /// The document and position implementation was requested at.
    #[serde(flatten)]
    pub text_document_position: TextDocumentPositionParams,
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result: PartialResultParams,
}

/// The result of a goto-definition request (also used by `declaration`,
/// `typeDefinition`, and `implementation`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GotoDefinitionResponse {
    /// A single definition location.
    Scalar(Location),
    /// Several candidate locations.
    Array(Vec<Location>),
    /// Rich links, preferred by clients advertising
    /// `textDocument.definition.linkSupport` — the editor underlines the
    /// exact origin token and lands on the symbol name instead of the
    /// item's full range. Only return this form when the client declared
    /// link support.
    Links(Vec<LocationLink>),
}

impl From<Location> for GotoDefinitionResponse {
    fn from(location: Location) -> Self {
        GotoDefinitionResponse::Scalar(location)
    }
}

impl From<Vec<Location>> for GotoDefinitionResponse {
    fn from(locations: Vec<Location>) -> Self {
        GotoDefinitionResponse::Array(locations)
    }
}

impl From<Vec<LocationLink>> for GotoDefinitionResponse {
    fn from(links: Vec<LocationLink>) -> Self {
        GotoDefinitionResponse::Links(links)
    }
}

/// A link between an origin range (the token the user invoked navigation
/// on) and a target, richer than a bare [`Location`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocationLink {
    /// The span of the origin the link applies to (e.g. the identifier
    /// under the cursor), underlined by the client; defaults to the word
    /// range at the request position.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_selection_range: Option<Range>,
    /// The target document.
    pub target_uri: super::base::Uri,
    /// The target's full range (e.g. the entire function definition),
    /// containing `target_selection_range`.
    pub target_range: Range,
    /// The precise range navigated to and highlighted (typically the
    /// symbol's name).
    pub target_selection_range: Range,
}

/// Parameters of `textDocument/references`.
///
/// Supports both progress mixins: a large search may report progress
/// against [`work_done`](Self::work_done) and stream chunks of matches on
/// [`partial_result`](Self::partial_result) via
/// [`crate::Client::send_progress`] before returning the final list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceParams {
    /// The document and position to find references from.
    #[serde(flatten)]
    pub text_document_position: TextDocumentPositionParams,
    /// Reference-search options.
    pub context: ReferenceContext,
    #[serde(flatten)]
    pub work_done: WorkDoneProgressParams,
    #[serde(flatten)]
    pub partial_result: PartialResultParams,
}

/// Options for a `textDocument/references` request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceContext {
    /// Whether to include the symbol's own declaration in the results.
    pub include_declaration: bool,
}

#[cfg(test)]
mod completion_tests {
    use super::*;
    use crate::lsp::Position;
    use serde_json::json;

    fn range() -> Range {
        Range::new(Position::new(0, 0), Position::new(0, 3))
    }

    #[test]
    fn completion_item_serializes_rich_fields() {
        let item = CompletionItem::new("println!")
            .with_kind(crate::lsp::CompletionItemKind::Snippet)
            .with_text_edit(TextEdit::new(range(), "println!(\"$1\")$0"))
            .as_snippet()
            .with_data(json!({"id": 7}));
        let value = serde_json::to_value(&item).unwrap();
        assert_eq!(value["insertTextFormat"], json!(2));
        assert_eq!(value["textEdit"]["newText"], json!("println!(\"$1\")$0"));
        assert_eq!(value["data"], json!({"id": 7}));
        // Unset optional fields stay off the wire.
        assert!(value.get("filterText").is_none());
        assert!(value.get("additionalTextEdits").is_none());
    }

    #[test]
    fn completion_item_documentation_accepts_string_or_markup() {
        let plain: CompletionItem =
            serde_json::from_value(json!({"label": "a", "documentation": "docs"})).unwrap();
        assert_eq!(
            plain.documentation,
            Some(Documentation::String("docs".to_owned()))
        );

        let markup: CompletionItem = serde_json::from_value(json!({
            "label": "a",
            "documentation": {"kind": "markdown", "value": "# docs"},
        }))
        .unwrap();
        assert!(
            matches!(markup.documentation, Some(Documentation::Markup(m)) if m.value == "# docs")
        );
    }

    #[test]
    fn insert_replace_edit_round_trips_untagged() {
        let edit = CompletionTextEdit::InsertReplace(InsertReplaceEdit {
            new_text: "x".to_owned(),
            insert: range(),
            replace: range(),
        });
        let value = serde_json::to_value(&edit).unwrap();
        assert!(value.get("insert").is_some());
        assert_eq!(
            serde_json::from_value::<CompletionTextEdit>(value).unwrap(),
            edit
        );

        let plain = CompletionTextEdit::Edit(TextEdit::new(range(), "y"));
        let value = serde_json::to_value(&plain).unwrap();
        assert!(value.get("insert").is_none());
        assert_eq!(
            serde_json::from_value::<CompletionTextEdit>(value).unwrap(),
            plain
        );
    }

    #[test]
    fn completion_list_item_defaults_round_trip() {
        let list: CompletionList = serde_json::from_value(json!({
            "isIncomplete": true,
            "itemDefaults": {
                "commitCharacters": ["."],
                "editRange": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 3}},
                "insertTextFormat": 2,
            },
            "items": [{"label": "a"}],
        }))
        .unwrap();
        let defaults = list.item_defaults.as_ref().expect("defaults");
        assert_eq!(defaults.commit_characters, Some(vec![".".to_owned()]));
        assert!(matches!(
            defaults.edit_range,
            Some(CompletionEditRange::Single(_))
        ));
        assert_eq!(defaults.insert_text_format, Some(InsertTextFormat::Snippet));

        // The insert/replace form of editRange parses too.
        let list: CompletionList = serde_json::from_value(json!({
            "isIncomplete": false,
            "itemDefaults": {"editRange": {
                "insert": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}},
                "replace": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 3}},
            }},
            "items": [],
        }))
        .unwrap();
        assert!(matches!(
            list.item_defaults.unwrap().edit_range,
            Some(CompletionEditRange::InsertReplace { .. })
        ));
    }

    #[test]
    fn document_highlight_serializes_kind_as_integer() {
        let highlight = DocumentHighlight::new(range()).with_kind(DocumentHighlightKind::Read);
        let value = serde_json::to_value(highlight).unwrap();
        assert_eq!(value["kind"], json!(2));

        let bare = DocumentHighlight::new(range());
        let value = serde_json::to_value(bare).unwrap();
        assert!(value.get("kind").is_none());
    }
}
