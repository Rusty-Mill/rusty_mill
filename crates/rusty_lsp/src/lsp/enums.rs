//! Integer- and string-coded LSP enumerations.
//!
//! Several LSP enums are transmitted as JSON integers (`1`, `2`, …) rather than
//! strings. serde derives string discriminants for fieldless enums, so these
//! types get hand-written codecs via the `int_enum!` macro, which keeps the
//! numeric mapping explicit and reversible.

use serde::{Deserialize, Serialize};

/// Generate a fieldless enum that (de)serializes as a JSON integer.
macro_rules! int_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $(
                $(#[$vmeta:meta])*
                $variant:ident = $value:literal
            ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[repr(u8)]
        $vis enum $name {
            $(
                $(#[$vmeta])*
                $variant = $value,
            )+
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_u8(*self as u8)
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(
                deserializer: D,
            ) -> Result<Self, D::Error> {
                let value = u8::deserialize(deserializer)?;
                match value {
                    $($value => Ok($name::$variant),)+
                    other => Err(serde::de::Error::custom(format!(
                        concat!("invalid ", stringify!($name), " discriminant: {}"),
                        other
                    ))),
                }
            }
        }
    };
}

int_enum! {
    /// How the client should synchronise document content to the server.
    pub enum TextDocumentSyncKind {
        /// Documents are not synced at all.
        None = 0,
        /// The full document text is sent on every change.
        Full = 1,
        /// Only incremental ranges are sent on each change.
        Incremental = 2,
    }
}

int_enum! {
    /// Severity used by `window/showMessage` and `window/logMessage`.
    pub enum MessageType {
        /// An error message.
        Error = 1,
        /// A warning message.
        Warning = 2,
        /// An informational message.
        Info = 3,
        /// A log message.
        Log = 4,
        /// A debug message (LSP 3.18).
        Debug = 5,
    }
}

int_enum! {
    /// Severity of a [`crate::lsp::Diagnostic`].
    pub enum DiagnosticSeverity {
        /// Reports an error.
        Error = 1,
        /// Reports a warning.
        Warning = 2,
        /// Reports an informational hint.
        Information = 3,
        /// Reports a low-priority hint.
        Hint = 4,
    }
}

int_enum! {
    /// What triggered a completion request.
    pub enum CompletionTriggerKind {
        /// Completion was invoked manually (e.g. Ctrl+Space) or via the API.
        Invoked = 1,
        /// Completion was triggered by a trigger character.
        TriggerCharacter = 2,
        /// Completion was re-triggered because the current list is incomplete.
        TriggerForIncompleteCompletions = 3,
    }
}

int_enum! {
    /// The semantic kind of a [`crate::lsp::CompletionItem`], used by clients to
    /// pick an icon. Values match the LSP `CompletionItemKind` table.
    pub enum CompletionItemKind {
        /// A free-text completion.
        Text = 1,
        /// A method.
        Method = 2,
        /// A function.
        Function = 3,
        /// A constructor.
        Constructor = 4,
        /// A field.
        Field = 5,
        /// A variable.
        Variable = 6,
        /// A class.
        Class = 7,
        /// An interface.
        Interface = 8,
        /// A module.
        Module = 9,
        /// A property.
        Property = 10,
        /// A unit.
        Unit = 11,
        /// A literal value.
        Value = 12,
        /// An enum.
        Enum = 13,
        /// A keyword.
        Keyword = 14,
        /// A code snippet.
        Snippet = 15,
        /// A color.
        Color = 16,
        /// A file.
        File = 17,
        /// A reference.
        Reference = 18,
        /// A folder.
        Folder = 19,
        /// An enum member.
        EnumMember = 20,
        /// A constant.
        Constant = 21,
        /// A struct.
        Struct = 22,
        /// An event.
        Event = 23,
        /// An operator.
        Operator = 24,
        /// A type parameter.
        TypeParameter = 25,
    }
}

int_enum! {
    /// How a watched file changed, reported in a [`crate::lsp::FileEvent`].
    pub enum FileChangeType {
        /// The file was created.
        Created = 1,
        /// The file's content changed.
        Changed = 2,
        /// The file was deleted.
        Deleted = 3,
    }
}

int_enum! {
    /// The semantic kind of a symbol, used by
    /// [`crate::lsp::DocumentSymbol`] and [`crate::lsp::SymbolInformation`].
    pub enum SymbolKind {
        /// A file.
        File = 1,
        /// A module.
        Module = 2,
        /// A namespace.
        Namespace = 3,
        /// A package.
        Package = 4,
        /// A class.
        Class = 5,
        /// A method.
        Method = 6,
        /// A property.
        Property = 7,
        /// A field.
        Field = 8,
        /// A constructor.
        Constructor = 9,
        /// An enum.
        Enum = 10,
        /// An interface.
        Interface = 11,
        /// A function.
        Function = 12,
        /// A variable.
        Variable = 13,
        /// A constant.
        Constant = 14,
        /// A string literal.
        String = 15,
        /// A numeric literal.
        Number = 16,
        /// A boolean literal.
        Boolean = 17,
        /// An array.
        Array = 18,
        /// An object.
        Object = 19,
        /// A key in a map/object.
        Key = 20,
        /// A null literal.
        Null = 21,
        /// An enum member.
        EnumMember = 22,
        /// A struct.
        Struct = 23,
        /// An event.
        Event = 24,
        /// An operator.
        Operator = 25,
        /// A type parameter.
        TypeParameter = 26,
    }
}

int_enum! {
    /// A tag modifying how a symbol is rendered, e.g. struck through for
    /// [`Deprecated`](SymbolTag::Deprecated).
    pub enum SymbolTag {
        /// Render the symbol as deprecated.
        Deprecated = 1,
    }
}

int_enum! {
    /// What triggered a `textDocument/codeAction` request.
    pub enum CodeActionTriggerKind {
        /// Explicitly invoked (e.g. the editor's code action menu).
        Invoked = 1,
        /// Triggered automatically, e.g. due to cursor movement over a
        /// diagnostic's range.
        Automatic = 2,
    }
}

int_enum! {
    /// What triggered a `textDocument/signatureHelp` request.
    pub enum SignatureHelpTriggerKind {
        /// Explicitly invoked (e.g. via a keybinding or the API).
        Invoked = 1,
        /// Triggered by a trigger character.
        TriggerCharacter = 2,
        /// Triggered because the cursor moved into a new parameter of an
        /// already-showing signature.
        ContentChange = 3,
    }
}

int_enum! {
    /// The kind of an [`crate::lsp::InlayHint`], used by clients to style it.
    pub enum InlayHintKind {
        /// A type annotation, e.g. `: i32` after an inferred `let` binding.
        Type = 1,
        /// A parameter name annotation, e.g. `x:` before a positional argument.
        Parameter = 2,
    }
}

int_enum! {
    /// Why `textDocument/willSave` (or `willSaveWaitUntil`) fired.
    pub enum TextDocumentSaveReason {
        /// The user manually triggered the save (e.g. Ctrl+S).
        Manual = 1,
        /// Saved automatically after a delay.
        AfterDelay = 2,
        /// Saved automatically because the editor lost focus.
        FocusOut = 3,
    }
}

int_enum! {
    /// The kind of a [`crate::lsp::NotebookCell`].
    pub enum NotebookCellKind {
        /// A markup (prose/documentation) cell.
        Markup = 1,
        /// A code cell.
        Code = 2,
    }
}

int_enum! {
    /// How a `textDocument/inlineCompletion` request was triggered
    /// (LSP 3.18, proposed).
    pub enum InlineCompletionTriggerKind {
        /// Explicitly invoked by a user gesture.
        Invoked = 1,
        /// Triggered automatically while the user types.
        Automatic = 2,
    }
}

int_enum! {
    /// The kind of a [`crate::lsp::DocumentHighlight`], used by clients to
    /// style read vs. write occurrences differently.
    pub enum DocumentHighlightKind {
        /// A textual occurrence.
        Text = 1,
        /// A read access of a symbol (e.g. reading a variable).
        Read = 2,
        /// A write access of a symbol (e.g. assigning to a variable).
        Write = 3,
    }
}

int_enum! {
    /// A tag qualifying a [`crate::lsp::CompletionItem`].
    pub enum CompletionItemTag {
        /// Render the item as obsolete, usually struck through.
        Deprecated = 1,
    }
}

int_enum! {
    /// How a [`crate::lsp::CompletionItem`]'s insert text should be
    /// interpreted.
    pub enum InsertTextFormat {
        /// Insert the text verbatim.
        PlainText = 1,
        /// Interpret the text as an LSP snippet (`${1:placeholder}`, `$0`).
        Snippet = 2,
    }
}

int_enum! {
    /// How whitespace/indentation in a [`crate::lsp::CompletionItem`]'s
    /// insert text should be adjusted to the surrounding context (LSP 3.16).
    pub enum InsertTextMode {
        /// Insert the text and indentation exactly as given.
        AsIs = 1,
        /// Adjust leading whitespace/indentation to match the current line.
        AdjustIndentation = 2,
    }
}

int_enum! {
    /// What a client falls back to when `textDocument/prepareRename` returns
    /// its "default behavior" response variant instead of an explicit range
    /// (LSP 3.16), advertised via
    /// `ClientCapabilities.textDocument.rename.prepareSupportDefaultBehavior`.
    pub enum PrepareSupportDefaultBehavior {
        /// The client selects the identifier under the cursor by its own
        /// word-boundary rules.
        Identifier = 1,
    }
}

int_enum! {
    /// A tag qualifying a [`crate::lsp::Diagnostic`], letting clients render
    /// it specially (faded, struck through) instead of squiggling it.
    pub enum DiagnosticTag {
        /// Unused or unnecessary code; clients typically fade it.
        Unnecessary = 1,
        /// Deprecated or obsolete code; clients typically strike it through.
        Deprecated = 2,
    }
}

/// The format of a [`crate::lsp::MarkupContent`] value. Unlike the enums above
/// this is encoded as a JSON string, so it uses a plain serde derive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MarkupKind {
    /// Plain text.
    PlainText,
    /// Markdown.
    Markdown,
}

/// The character encoding a [`crate::lsp::Position`]'s `character` field is
/// measured in, negotiated via `capabilities.general.positionEncodings` /
/// [`crate::lsp::ServerCapabilities::position_encoding`] (LSP 3.17). Encoded
/// as the JSON strings the spec defines, not integers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PositionEncodingKind {
    /// Positions are measured in UTF-8 code units (bytes).
    #[serde(rename = "utf-8")]
    Utf8,
    /// Positions are measured in UTF-16 code units. The default per the base
    /// LSP spec, and what every function in [`crate::text`] assumes unless
    /// told otherwise.
    #[serde(rename = "utf-16")]
    Utf16,
    /// Positions are measured in UTF-32 code units (Unicode scalar values).
    #[serde(rename = "utf-32")]
    Utf32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn integer_enums_encode_as_numbers() {
        assert_eq!(
            serde_json::to_value(TextDocumentSyncKind::Full).unwrap(),
            json!(1)
        );
        assert_eq!(
            serde_json::to_value(MessageType::Warning).unwrap(),
            json!(2)
        );
        assert_eq!(
            serde_json::to_value(DiagnosticSeverity::Hint).unwrap(),
            json!(4)
        );
        assert_eq!(
            serde_json::to_value(CompletionItemKind::TypeParameter).unwrap(),
            json!(25)
        );
    }

    #[test]
    fn integer_enums_decode_from_numbers() {
        let kind: DiagnosticSeverity = serde_json::from_value(json!(1)).unwrap();
        assert_eq!(kind, DiagnosticSeverity::Error);
        let trigger: CompletionTriggerKind = serde_json::from_value(json!(2)).unwrap();
        assert_eq!(trigger, CompletionTriggerKind::TriggerCharacter);
    }

    #[test]
    fn invalid_discriminant_is_rejected() {
        assert!(serde_json::from_value::<DiagnosticSeverity>(json!(99)).is_err());
    }

    #[test]
    fn markup_kind_encodes_as_lowercase_string() {
        assert_eq!(
            serde_json::to_value(MarkupKind::PlainText).unwrap(),
            json!("plaintext")
        );
        assert_eq!(
            serde_json::to_value(MarkupKind::Markdown).unwrap(),
            json!("markdown")
        );
    }
}
