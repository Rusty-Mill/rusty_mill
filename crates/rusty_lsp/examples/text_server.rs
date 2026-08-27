//! A small but complete language server built on `rusty_lsp`.
//!
//! It treats every open document as plain text and demonstrates the full
//! round trip an editor exercises:
//!
//! - **Lifecycle & capabilities** — advertises full-document sync, hover, and
//!   completion in `initialize`.
//! - **Document sync** — tracks open buffers using [`rusty_lsp::Documents`],
//!   including incremental edits.
//! - **Diagnostics** (server → client) — flags `TODO` / `FIXME` / `XXX`
//!   markers, republished on every edit.
//! - **Hover** — reports the word under the cursor and how often it occurs.
//! - **Completion** — proposes the distinct words already present in the buffer.
//! - **Document highlights** — marks every occurrence of the word under the
//!   cursor, dogfooding [`rusty_lsp::Documents::offset_at`].
//!
//! Run it as an editor would (it speaks LSP over stdio):
//!
//! ```text
//! cargo run --example text_server
//! ```
//!
//! Position handling converts the protocol's UTF-16 columns to UTF-8 byte
//! offsets, so multi-byte characters on a line do not skew ranges.

use rusty_lsp::error::Result;
use rusty_lsp::lsp::{
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams, CompletionResponse,
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DocumentHighlight, DocumentHighlightKind, DocumentHighlightParams,
    Hover, HoverParams, InitializeParams, InitializeResult, MessageType, Position, Range,
    ServerCapabilities, ServerInfo, TextDocumentSyncKind, Uri,
};
use rusty_lsp::text::{byte_to_utf16_column, utf16_column_to_byte};
use rusty_lsp::{Client, Documents, LanguageServer, Server};
use std::collections::BTreeSet;

/// Markers the server reports as warnings.
const MARKERS: [&str; 3] = ["TODO", "FIXME", "XXX"];
/// Minimum length for a word to be offered as a completion.
const MIN_COMPLETION_LEN: usize = 3;

/// The language server backend.
struct TextServer {
    client: Client,
    documents: Documents,
}

impl TextServer {
    fn new(client: Client) -> Self {
        TextServer {
            client,
            documents: Documents::new(),
        }
    }

    /// Recompute and publish diagnostics for `uri` from the stored buffer.
    async fn publish_diagnostics(&self, uri: &Uri) {
        let (diagnostics, version) = match self.documents.get(uri).await {
            Some(doc) => (compute_diagnostics(&doc.text), Some(doc.version)),
            // Document was closed; clear any diagnostics for it.
            None => (Vec::new(), None),
        };
        let _ = self
            .client
            .publish_diagnostics(uri.clone(), diagnostics, version);
    }
}

impl LanguageServer for TextServer {
    async fn initialize(&self, _params: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncKind::Incremental.into()),
                hover_provider: Some(true.into()),
                document_highlight_provider: Some(true.into()),
                completion_provider: Some(CompletionOptions {
                    resolve_provider: Some(false),
                    ..Default::default()
                }),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "rusty-text-server".to_owned(),
                version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            }),
        })
    }

    async fn initialized(&self) {
        let _ = self
            .client
            .log_message(MessageType::Info, "rusty-text-server ready");
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        self.documents.did_open(&params).await;
        self.publish_diagnostics(&uri).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        self.documents.did_change(&params).await;
        self.publish_diagnostics(&uri).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        self.documents.did_close(&params).await;
        // Clear diagnostics for the now-closed document.
        let _ = self.client.publish_diagnostics(uri, Vec::new(), None);
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let position = params.text_document_position.position;
        let uri = params.text_document_position.text_document.uri;
        // `with` borrows the stored text instead of cloning the whole buffer.
        Ok(self
            .documents
            .with(&uri, |doc| hover_at(&doc.text, position))
            .await
            .flatten())
    }

    async fn document_highlight(
        &self,
        params: DocumentHighlightParams,
    ) -> Result<Option<Vec<DocumentHighlight>>> {
        let position = params.text_document_position.position;
        let uri = params.text_document_position.text_document.uri;
        // `offset_at` resolves the cursor through the store's cached line
        // index in the negotiated encoding.
        let Some(offset) = self.documents.offset_at(&uri, position).await else {
            return Ok(None);
        };
        Ok(self
            .documents
            .with(&uri, |doc| highlights_at(&doc.text, offset))
            .await
            .flatten())
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        Ok(self
            .documents
            .with(&uri, |doc| {
                CompletionResponse::Array(completion_items(&doc.text))
            })
            .await)
    }
}

/// Byte ranges of word tokens (`[A-Za-z0-9_]+`) within a single line.
fn word_tokens(line: &str) -> Vec<(usize, usize)> {
    let mut tokens = Vec::new();
    let mut start: Option<usize> = None;
    for (offset, ch) in line.char_indices() {
        let is_word = ch.is_alphanumeric() || ch == '_';
        match (is_word, start) {
            (true, None) => start = Some(offset),
            (false, Some(s)) => {
                tokens.push((s, offset));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        tokens.push((s, line.len()));
    }
    tokens
}

/// Count whole-word occurrences of `word` across the document.
fn count_word(text: &str, word: &str) -> usize {
    let mut count = 0;
    for line in text.lines() {
        for (s, e) in word_tokens(line) {
            if &line[s..e] == word {
                count += 1;
            }
        }
    }
    count
}

/// Build a hover for the word under `position`, if any.
fn hover_at(text: &str, position: Position) -> Option<Hover> {
    let line = text.lines().nth(position.line as usize)?;
    let byte = utf16_column_to_byte(line, position.character);
    let (start, end) = word_tokens(line)
        .into_iter()
        .find(|&(s, e)| byte >= s && byte <= e)?;
    let word = &line[start..end];
    if word.is_empty() {
        return None;
    }
    let occurrences = count_word(text, word);
    let range = Range::new(
        Position::new(position.line, byte_to_utf16_column(line, start)),
        Position::new(position.line, byte_to_utf16_column(line, end)),
    );
    let body = format!(
        "**{word}**\n\nWord appears {occurrences} time{} in this document.",
        if occurrences == 1 { "" } else { "s" },
    );
    Some(Hover::markdown(body).with_range(range))
}

/// Highlight every occurrence of the word at byte `offset`.
fn highlights_at(text: &str, offset: usize) -> Option<Vec<DocumentHighlight>> {
    let word = word_at_offset(text, offset)?.to_owned();

    // Mark every whole-word occurrence.
    let mut highlights = Vec::new();
    for (line_no, _line_start, line) in lines_with_offsets(text) {
        for (s, e) in word_tokens(line) {
            if line[s..e] == word {
                let range = Range::new(
                    Position::new(line_no, byte_to_utf16_column(line, s)),
                    Position::new(line_no, byte_to_utf16_column(line, e)),
                );
                highlights
                    .push(DocumentHighlight::new(range).with_kind(DocumentHighlightKind::Text));
            }
        }
    }
    Some(highlights)
}

/// The word token containing byte `offset`, if any.
fn word_at_offset(text: &str, offset: usize) -> Option<&str> {
    for (_number, line_start, line) in lines_with_offsets(text) {
        if offset < line_start {
            break;
        }
        if offset <= line_start + line.len() {
            let byte = offset - line_start;
            return word_tokens(line)
                .into_iter()
                .find(|&(s, e)| byte >= s && byte <= e)
                .map(|(s, e)| &line[s..e]);
        }
    }
    None
}

/// Each line of `text` with its zero-based number and starting byte offset.
fn lines_with_offsets(text: &str) -> impl Iterator<Item = (u32, usize, &str)> {
    let mut start = 0usize;
    text.split('\n').enumerate().map(move |(number, line)| {
        let line_start = start;
        start += line.len() + 1;
        (number as u32, line_start, line)
    })
}

/// Diagnostics for every marker (`TODO`/`FIXME`/`XXX`) in the document.
fn compute_diagnostics(text: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for (line_no, line) in text.lines().enumerate() {
        for (start, end) in word_tokens(line) {
            let word = &line[start..end];
            if MARKERS.contains(&word) {
                let range = Range::new(
                    Position::new(line_no as u32, byte_to_utf16_column(line, start)),
                    Position::new(line_no as u32, byte_to_utf16_column(line, end)),
                );
                diagnostics.push(
                    Diagnostic::new(
                        range,
                        DiagnosticSeverity::Warning,
                        format!("`{word}` marker found"),
                    )
                    .with_source("text-server"),
                );
            }
        }
    }
    diagnostics
}

/// Distinct, sorted words eligible to be offered as completions.
fn completion_items(text: &str) -> Vec<CompletionItem> {
    let mut words = BTreeSet::new();
    for line in text.lines() {
        for (start, end) in word_tokens(line) {
            let word = &line[start..end];
            let starts_with_alpha = word
                .chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_');
            if word.len() >= MIN_COMPLETION_LEN && starts_with_alpha {
                words.insert(word.to_owned());
            }
        }
    }
    words
        .into_iter()
        .map(|word| CompletionItem::new(word).with_kind(CompletionItemKind::Text))
        .collect()
}

#[tokio::main]
async fn main() -> Result<()> {
    Server::stdio().serve(TextServer::new).await
}
