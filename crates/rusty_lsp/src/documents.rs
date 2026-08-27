//! [`Documents`]: a managed, URI-keyed store of open document text, applying
//! `textDocument/didOpen`/`didChange`/`didClose` edits for you so a backend
//! doesn't have to hand-roll a `HashMap<Uri, String>` and patch incremental
//! edits itself.
//!
//! `Documents` is entirely optional — the framework works the same with or
//! without it. Wire it up by calling [`Documents::did_open`]/
//! [`did_change`](Documents::did_change)/[`did_close`](Documents::did_close)
//! from your [`crate::LanguageServer`] implementation's matching methods.
//!
//! Incremental edits are patched position-encoding-aware: the store defaults
//! to UTF-16 (the base-spec default) and [`Documents::with_encoding`] adapts
//! it to whatever [`crate::lsp::PositionEncodingKind`] the server negotiated
//! via [`crate::lsp::ServerCapabilities::position_encoding`].

use crate::lsp::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams, Position,
    PositionEncodingKind, Uri,
};
use crate::text::{LineIndex, position_to_offset_with};
use std::collections::HashMap;
use std::sync::OnceLock;
use tokio::sync::RwLock;

/// A single open document's current text and metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// The document's language identifier (e.g. `"rust"`), as sent on open.
    pub language_id: String,
    /// The document's current version, updated on every change.
    pub version: i32,
    /// The document's current full text.
    pub text: String,
}

/// A managed, concurrency-safe store of open documents, keyed by URI.
///
/// Shares the same "shared via `Arc`, mutated through `&self`" pattern as
/// [`crate::LanguageServer`] backends: hold a `Documents` as a plain field
/// on your backend struct (not wrapped in an `Arc` of its own) and call its
/// methods through `&self`.
#[derive(Debug)]
pub struct Documents {
    inner: RwLock<HashMap<Uri, Entry>>,
    encoding: PositionEncodingKind,
}

/// A stored document plus its lazily built, edit-invalidated [`LineIndex`].
#[derive(Debug)]
struct Entry {
    document: Document,
    index: OnceLock<LineIndex>,
}

impl Entry {
    fn new(document: Document) -> Self {
        Entry {
            document,
            index: OnceLock::new(),
        }
    }

    /// The line index for the current text, built on first use after each
    /// edit.
    fn index(&self) -> &LineIndex {
        self.index
            .get_or_init(|| LineIndex::new(&self.document.text))
    }
}

/// Look up `uri` in `map`, falling back to its normalized spelling when
/// the raw string misses — so a caller passing e.g. `"FILE:///a"` still
/// finds the document stored under the normalized [`Uri`] key. The
/// fallback allocates only when normalization would actually change the
/// string.
fn lookup<'a>(map: &'a HashMap<Uri, Entry>, uri: &str) -> Option<&'a Entry> {
    if let Some(entry) = map.get(uri) {
        return Some(entry);
    }
    let normalized = Uri::new(uri);
    (normalized != uri)
        .then(|| map.get(normalized.as_str()))
        .flatten()
}

impl Default for Documents {
    fn default() -> Self {
        Documents::new()
    }
}

impl Documents {
    /// Build an empty document store that patches incremental edits using
    /// UTF-16 positions (the base-spec default).
    pub fn new() -> Self {
        Documents::with_encoding(PositionEncodingKind::Utf16)
    }

    /// Build an empty document store for a server that negotiated a
    /// different position encoding via
    /// [`crate::lsp::ServerCapabilities::position_encoding`]. Incremental
    /// edits are patched interpreting `Position::character` in `encoding`.
    pub fn with_encoding(encoding: PositionEncodingKind) -> Self {
        Documents {
            inner: RwLock::new(HashMap::new()),
            encoding,
        }
    }

    /// The position encoding this store patches incremental edits with.
    pub fn encoding(&self) -> PositionEncodingKind {
        self.encoding
    }

    /// Record a newly opened document (`textDocument/didOpen`). Returns the
    /// previously stored document if the URI was somehow already open —
    /// letting a backend detect a client re-opening without closing.
    pub async fn did_open(&self, params: &DidOpenTextDocumentParams) -> Option<Document> {
        let item = &params.text_document;
        self.inner
            .write()
            .await
            .insert(
                item.uri.clone(),
                Entry::new(Document {
                    language_id: item.language_id.clone(),
                    version: item.version,
                    text: item.text.clone(),
                }),
            )
            .map(|entry| entry.document)
    }

    /// Apply a document change (`textDocument/didChange`), patching in the
    /// full-document or incremental edits in order. Returns `true` if the
    /// change was applied.
    ///
    /// Two kinds of change are ignored (returning `false`): a change for a
    /// document that isn't open (matching how the rest of the framework
    /// treats messages referencing unknown state), and a change whose
    /// version is **older** than the stored version, which guards against
    /// replayed or reordered edits.
    pub async fn did_change(&self, params: &DidChangeTextDocumentParams) -> bool {
        let mut documents = self.inner.write().await;
        let Some(entry) = documents.get_mut(&params.text_document.uri) else {
            return false;
        };
        if params.text_document.version < entry.document.version {
            return false;
        }
        let document = &mut entry.document;
        for change in &params.content_changes {
            match change.range {
                Some(range) => {
                    let start = position_to_offset_with(&document.text, range.start, self.encoding);
                    let end = position_to_offset_with(&document.text, range.end, self.encoding);
                    document.text.replace_range(start..end, &change.text);
                }
                None => document.text.clone_from(&change.text),
            }
        }
        document.version = params.text_document.version;
        // The text changed; the cached line index no longer matches it.
        entry.index = OnceLock::new();
        true
    }

    /// Forget a closed document (`textDocument/didClose`), returning it.
    pub async fn did_close(&self, params: &DidCloseTextDocumentParams) -> Option<Document> {
        self.inner
            .write()
            .await
            .remove(&params.text_document.uri)
            .map(|entry| entry.document)
    }

    /// Get a clone of a document's current state, if it's open.
    ///
    /// This clones the full text; for read-only access on a hot path (e.g.
    /// hover over a large file), prefer [`with`](Self::with), which borrows
    /// instead.
    pub async fn get(&self, uri: impl AsRef<str>) -> Option<Document> {
        lookup(&*self.inner.read().await, uri.as_ref()).map(|entry| entry.document.clone())
    }

    /// Get a clone of a document's current text, if it's open.
    ///
    /// Like [`get`](Self::get), this clones; prefer [`with`](Self::with) on
    /// hot paths.
    pub async fn text(&self, uri: impl AsRef<str>) -> Option<String> {
        lookup(&*self.inner.read().await, uri.as_ref()).map(|entry| entry.document.text.clone())
    }

    /// Run `f` against a document's current state without cloning it,
    /// returning `None` if the document isn't open.
    ///
    /// The store's read lock is held for the duration of `f`, so keep the
    /// closure short and non-blocking.
    ///
    /// ```rust,ignore
    /// let word_count = documents
    ///     .with(&uri, |doc| doc.text.split_whitespace().count())
    ///     .await;
    /// ```
    pub async fn with<T>(&self, uri: impl AsRef<str>, f: impl FnOnce(&Document) -> T) -> Option<T> {
        lookup(&*self.inner.read().await, uri.as_ref()).map(|entry| f(&entry.document))
    }

    /// The URIs of every open document, in no particular order.
    pub async fn uris(&self) -> Vec<Uri> {
        self.inner.read().await.keys().cloned().collect()
    }

    /// How many documents are open.
    pub async fn len(&self) -> usize {
        self.inner.read().await.len()
    }

    /// Whether no documents are open.
    pub async fn is_empty(&self) -> bool {
        self.inner.read().await.is_empty()
    }

    /// Run `f` over every open document under a single read lock, in no
    /// particular order — e.g. to compute `workspace/diagnostic` results or
    /// republish diagnostics after a configuration change. Keep `f` short
    /// and non-blocking, as with [`with`](Self::with).
    pub async fn for_each(&self, mut f: impl FnMut(&Uri, &Document)) {
        for (uri, entry) in self.inner.read().await.iter() {
            f(uri, &entry.document);
        }
    }

    /// Like [`with`](Self::with), also handing `f` the document's cached
    /// [`LineIndex`] (built lazily and invalidated on every edit), for
    /// batches of position math over one snapshot.
    pub async fn with_index<T>(
        &self,
        uri: impl AsRef<str>,
        f: impl FnOnce(&Document, &LineIndex) -> T,
    ) -> Option<T> {
        lookup(&*self.inner.read().await, uri.as_ref())
            .map(|entry| f(&entry.document, entry.index()))
    }

    /// Convert a request [`Position`] in a stored document to a byte offset,
    /// using the store's negotiated encoding and the document's cached line
    /// index. Returns `None` if the document isn't open; out-of-range
    /// positions clamp.
    pub async fn offset_at(&self, uri: impl AsRef<str>, position: Position) -> Option<usize> {
        self.with_index(uri, |doc, index| {
            index.position_to_offset(&doc.text, position, self.encoding)
        })
        .await
    }

    /// Convert a byte offset in a stored document to a [`Position`], using
    /// the store's negotiated encoding and the document's cached line index.
    /// Returns `None` if the document isn't open; out-of-range offsets clamp.
    pub async fn position_at(&self, uri: impl AsRef<str>, offset: usize) -> Option<Position> {
        self.with_index(uri, |doc, index| {
            index.offset_to_position(&doc.text, offset, self.encoding)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::{
        Position, Range, TextDocumentContentChangeEvent, TextDocumentIdentifier, TextDocumentItem,
        VersionedTextDocumentIdentifier,
    };

    pub fn open(uri: &str, text: &str) -> DidOpenTextDocumentParams {
        DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.into(),
                language_id: "plaintext".to_owned(),
                version: 1,
                text: text.to_owned(),
            },
        }
    }

    #[tokio::test]
    async fn open_then_get_round_trips() {
        let documents = Documents::new();
        documents.did_open(&open("file:///a", "hello")).await;

        let doc = documents.get("file:///a").await.expect("open");
        assert_eq!(doc.text, "hello");
        assert_eq!(doc.version, 1);
        assert_eq!(doc.language_id, "plaintext");
    }

    #[tokio::test]
    async fn full_sync_replaces_whole_text() {
        let documents = Documents::new();
        documents.did_open(&open("file:///a", "hello")).await;

        documents
            .did_change(&DidChangeTextDocumentParams {
                text_document: VersionedTextDocumentIdentifier {
                    uri: "file:///a".into(),
                    version: 2,
                },
                content_changes: vec![TextDocumentContentChangeEvent {
                    range: None,
                    text: "goodbye".to_owned(),
                }],
            })
            .await;

        assert_eq!(
            documents.text("file:///a").await.as_deref(),
            Some("goodbye")
        );
    }

    #[tokio::test]
    async fn incremental_edits_apply_in_order_using_utf16_positions() {
        let documents = Documents::new();
        documents.did_open(&open("file:///a", "héllo world")).await;

        // "héllo world" -- replace "llo" (UTF-16 columns 2..5, since é is 1
        // unit) with "y", then replace "world" with "there" using the
        // position that only makes sense *after* the first edit landed.
        documents
            .did_change(&DidChangeTextDocumentParams {
                text_document: VersionedTextDocumentIdentifier {
                    uri: "file:///a".into(),
                    version: 2,
                },
                content_changes: vec![
                    TextDocumentContentChangeEvent {
                        range: Some(Range::new(Position::new(0, 2), Position::new(0, 5))),
                        text: "y".to_owned(),
                    },
                    TextDocumentContentChangeEvent {
                        range: Some(Range::new(Position::new(0, 4), Position::new(0, 9))),
                        text: "there".to_owned(),
                    },
                ],
            })
            .await;

        assert_eq!(
            documents.text("file:///a").await.as_deref(),
            Some("héy there")
        );
    }

    #[tokio::test]
    async fn close_removes_the_document() {
        let documents = Documents::new();
        documents.did_open(&open("file:///a", "hello")).await;
        documents
            .did_close(&DidCloseTextDocumentParams {
                text_document: TextDocumentIdentifier {
                    uri: "file:///a".into(),
                },
            })
            .await;

        assert!(documents.get("file:///a").await.is_none());
    }

    #[tokio::test]
    async fn stale_version_change_is_ignored() {
        let documents = Documents::new();
        documents.did_open(&open("file:///a", "v1 text")).await;
        documents
            .did_change(&DidChangeTextDocumentParams {
                text_document: VersionedTextDocumentIdentifier {
                    uri: "file:///a".into(),
                    version: 5,
                },
                content_changes: vec![TextDocumentContentChangeEvent {
                    range: None,
                    text: "v5 text".to_owned(),
                }],
            })
            .await;

        // A replayed older change must not clobber the newer state.
        let applied = documents
            .did_change(&DidChangeTextDocumentParams {
                text_document: VersionedTextDocumentIdentifier {
                    uri: "file:///a".into(),
                    version: 3,
                },
                content_changes: vec![TextDocumentContentChangeEvent {
                    range: None,
                    text: "stale".to_owned(),
                }],
            })
            .await;
        assert!(!applied);
        assert_eq!(
            documents.text("file:///a").await.as_deref(),
            Some("v5 text")
        );
    }

    #[tokio::test]
    async fn reopen_returns_the_previous_document() {
        let documents = Documents::new();
        assert!(
            documents
                .did_open(&open("file:///a", "first"))
                .await
                .is_none()
        );
        let previous = documents
            .did_open(&open("file:///a", "second"))
            .await
            .expect("previously open");
        assert_eq!(previous.text, "first");
        assert_eq!(documents.text("file:///a").await.as_deref(), Some("second"));
    }

    #[tokio::test]
    async fn with_borrows_instead_of_cloning() {
        let documents = Documents::new();
        documents
            .did_open(&open("file:///a", "one two three"))
            .await;

        let words = documents
            .with("file:///a", |doc| doc.text.split_whitespace().count())
            .await;
        assert_eq!(words, Some(3));
        assert_eq!(documents.with("file:///missing", |_| ()).await, None);
    }

    #[tokio::test]
    async fn with_encoding_patches_edits_in_that_encoding() {
        use crate::lsp::PositionEncodingKind;

        // "é😀x": in UTF-8 columns, "x" starts at byte 6.
        let documents = Documents::with_encoding(PositionEncodingKind::Utf8);
        assert_eq!(documents.encoding(), PositionEncodingKind::Utf8);
        documents.did_open(&open("file:///a", "é😀x")).await;

        documents
            .did_change(&DidChangeTextDocumentParams {
                text_document: VersionedTextDocumentIdentifier {
                    uri: "file:///a".into(),
                    version: 2,
                },
                content_changes: vec![TextDocumentContentChangeEvent {
                    range: Some(Range::new(Position::new(0, 6), Position::new(0, 7))),
                    text: "y".to_owned(),
                }],
            })
            .await;
        assert_eq!(documents.text("file:///a").await.as_deref(), Some("é😀y"));
    }

    #[tokio::test]
    async fn offset_and_position_helpers_use_the_negotiated_encoding() {
        let documents = Documents::new(); // UTF-16
        documents.did_open(&open("file:///a", "é😀x\nsecond")).await;

        // Column 3 on line 0 (UTF-16: é=1, 😀=2) is the byte offset of "x".
        let offset = documents
            .offset_at("file:///a", Position::new(0, 3))
            .await
            .expect("open");
        assert_eq!(offset, 6);
        assert_eq!(
            documents.position_at("file:///a", offset).await,
            Some(Position::new(0, 3))
        );
        // Second line resolves through the cached index too.
        assert_eq!(
            documents.offset_at("file:///a", Position::new(1, 0)).await,
            Some(8)
        );
        assert!(
            documents
                .offset_at("file:///missing", Position::new(0, 0))
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn cached_line_index_is_invalidated_by_edits() {
        let documents = Documents::new();
        documents.did_open(&open("file:///a", "one\ntwo")).await;
        // Prime the cache.
        assert_eq!(
            documents.offset_at("file:///a", Position::new(1, 0)).await,
            Some(4)
        );

        // Grow line 0; line 1 must shift.
        documents
            .did_change(&DidChangeTextDocumentParams {
                text_document: VersionedTextDocumentIdentifier {
                    uri: "file:///a".into(),
                    version: 2,
                },
                content_changes: vec![TextDocumentContentChangeEvent {
                    range: Some(Range::new(Position::new(0, 0), Position::new(0, 0))),
                    text: "zero ".to_owned(),
                }],
            })
            .await;
        assert_eq!(
            documents.offset_at("file:///a", Position::new(1, 0)).await,
            Some(9)
        );

        // with_index hands out the same snapshot-consistent pair.
        let line_count = documents
            .with_index("file:///a", |_doc, index| index.line_count())
            .await;
        assert_eq!(line_count, Some(2));
    }

    #[tokio::test]
    async fn change_to_unopened_document_is_ignored() {
        let documents = Documents::new();
        documents
            .did_change(&DidChangeTextDocumentParams {
                text_document: VersionedTextDocumentIdentifier {
                    uri: "file:///never-opened".into(),
                    version: 2,
                },
                content_changes: vec![TextDocumentContentChangeEvent {
                    range: None,
                    text: "x".to_owned(),
                }],
            })
            .await;

        assert!(documents.get("file:///never-opened").await.is_none());
    }
}

#[cfg(test)]
mod lookup_tests {
    use super::tests::open;
    use super::*;

    #[tokio::test]
    async fn raw_string_lookups_fall_back_to_the_normalized_spelling() {
        let documents = Documents::new();
        documents.did_open(&open("file:///a", "hello")).await;

        // A raw string that only matches after normalization still hits.
        assert!(documents.get("FILE:///a").await.is_some());
        assert!(documents.text("file:///a").await.is_some());
        assert_eq!(
            documents.with("FILE:///a", |doc| doc.text.len()).await,
            Some(5)
        );
        assert!(documents.get("file:///other").await.is_none());
    }
}
