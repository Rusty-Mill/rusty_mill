/// A document's dynamic, arbitrarily-keyed metadata blob (author,
/// description, site name, image, custom extractor-set keys, ...).
///
/// A plain alias over [`rusty_json::Map`] rather than a wrapper type:
/// capability inventory §5.3 documents `metadata.KEY:value` as a dynamic
/// pass-through query field over exactly this JSON blob, so keeping the
/// real `Map` API (not a narrower façade) is what that query layer will
/// need to build against later.
pub type Metadata = rusty_json::Map;

/// What kind of document this is — capability inventory §5.3's
/// `document_types` value set (`web` / `local` / `remote_file`; Hister's
/// `file` alias for "local or remote_file" is a query-layer concern, not a
/// document-shape one, so it isn't a variant here).
///
/// The wire-format integer Hister's HTTP API encodes this as (row 1.6 only
/// confirms "2 = remote-file snapshot") is **not yet assigned** here — v1's
/// byte-compatibility requirement means those integers must match the real
/// Hister source exactly, which capability-inventory review didn't extract
/// precisely enough to guess safely. Assign `TryFrom<i64>`/`Into<i64>` once
/// `rusty-hister-server` needs them and the exact values are confirmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DocumentType {
    /// A normal crawled web page.
    Web,
    /// A local file (`file://...`) indexed from a watched directory.
    Local,
    /// A locally-stored snapshot of a remote file (e.g. a downloaded PDF).
    RemoteFile,
}

/// The extractor-pipeline's working representation of a document: the
/// input every [`crate::Extractor`] matches against and enriches, and the
/// output a content extractor produces. Distinct from `rusty-hister-model`'s
/// persisted `History`/`Link` rows (capability inventory §7.2) — this type
/// is the in-flight processing shape; the model crate owns what's actually
/// stored.
#[derive(Debug, Clone)]
pub struct Document {
    /// The document's URL. Required and non-empty for any document that
    /// reached an extractor (enforced by [`Document::new`], not by the
    /// field's type, since extractors and tests alike need direct field
    /// access to the rest of the struct).
    pub url: String,
    /// The page title, if known.
    pub title: Option<String>,
    /// Plain-text content, if extracted.
    pub text: Option<String>,
    /// Raw or rendered HTML content, if extracted.
    pub html: Option<String>,
    /// A base64-encoded favicon data URI, if fetched.
    pub favicon: Option<String>,
    /// A user-defined label (capability inventory §1 row 1.9).
    pub label: Option<String>,
    /// What kind of document this is, if classified.
    pub document_type: Option<DocumentType>,
    /// The document's detected/declared language code, if known.
    pub language: Option<String>,
    /// Arbitrary extractor- and enricher-set metadata.
    pub metadata: Metadata,
}

impl Document {
    /// Creates a new document for the given URL, with every other field
    /// empty. `url` is the one field every extractor and the crawler
    /// itself depend on being present before matching/extraction starts.
    pub fn new(url: impl Into<String>) -> Self {
        Document {
            url: url.into(),
            title: None,
            text: None,
            html: None,
            favicon: None,
            label: None,
            document_type: None,
            language: None,
            metadata: Metadata::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_sets_url_and_defaults_everything_else() {
        let doc = Document::new("https://example.com/page");
        assert_eq!(doc.url, "https://example.com/page");
        assert_eq!(doc.title, None);
        assert_eq!(doc.text, None);
        assert_eq!(doc.html, None);
        assert_eq!(doc.favicon, None);
        assert_eq!(doc.label, None);
        assert_eq!(doc.document_type, None);
        assert_eq!(doc.language, None);
        assert!(doc.metadata.is_empty());
    }

    #[test]
    fn new_accepts_owned_and_borrowed_url() {
        let owned = Document::new(String::from("https://example.com/a"));
        let borrowed = Document::new("https://example.com/b");
        assert_eq!(owned.url, "https://example.com/a");
        assert_eq!(borrowed.url, "https://example.com/b");
    }

    #[test]
    fn metadata_holds_arbitrary_keys() {
        let mut doc = Document::new("https://example.com");
        doc.metadata.insert(
            "author".to_string(),
            rusty_json::Value::String("Jane".to_string()),
        );
        assert_eq!(
            doc.metadata.get("author"),
            Some(&rusty_json::Value::String("Jane".to_string()))
        );
    }

    #[test]
    fn document_type_variants_are_distinct() {
        assert_ne!(DocumentType::Web, DocumentType::Local);
        assert_ne!(DocumentType::Local, DocumentType::RemoteFile);
        assert_ne!(DocumentType::Web, DocumentType::RemoteFile);
    }
}
