//! `Basic` — a Rust port of the `basicExtractor` in
//! `server/extractor/extractor.go` (capability inventory §4.4). The
//! universal last-resort fallback: strips markup from any HTML document
//! and keeps whatever plain text and `<title>` remain, so a page nothing
//! else recognizes still becomes searchable rather than dropping out of
//! the chain entirely. [`matches`](Extractor::matches) always returns
//! `true` — this only works because every real chain places it last, after
//! every extractor that can do better.
//!
//! Go walks the raw byte stream with its own HTML tokenizer, tracking
//! "am I inside `<body>`" and "am I inside `<script>`/`<style>`/
//! `<noscript>`" by hand. This port instead selects the parsed `<body>`
//! element (via `scraper`, already this crate's dependency) and walks its
//! subtree, which is simpler but not perfectly equivalent in one respect:
//! Go's version only starts collecting text after it has actually seen a
//! `<body>` *token* in the byte stream, so a fragment with no `<body>` tag
//! at all yields no text, whereas `html5ever` (via `scraper`) always
//! synthesizes a `<body>` around whatever content it parses, so this port
//! would still find it. This has no practical effect on real crawled
//! pages, which always have a body; it isn't reproduced because doing so
//! faithfully would mean abandoning the DOM-based approach entirely for a
//! fallback extractor that rarely matters this precisely.
//!
//! Text nodes are concatenated with no added separators (not even between
//! block elements) — this is a bare "strip the tags" dump, deliberately
//! cruder than [`crate::textutil::selection_text`]'s block-aware
//! flattening, matching Go's own token-by-token concatenation exactly.
//!
//! [`BasicExtractor::preview`] doesn't re-derive anything from
//! `document.html` — like Go, it just HTML-escapes whatever
//! `document.text` already holds (typically populated by an earlier
//! `extract` call), succeeding with empty content when there is none
//! rather than falling back, another faithfully-preserved Go quirk (see
//! `GoDocExtractor`'s own module doc for the precedent).

use crate::stackexchange::{html_escape, selector};
use rusty_hister_core::{
    Capabilities, Document, ExtractOutcome, Extractor, ExtractorConfig, HisterError,
    PreviewOutcome, PreviewResponse,
};
use scraper::{ElementRef, Html, Node};

const SKIPPED_TAGS: &[&str] = &["script", "style", "noscript"];

#[derive(Debug, Default)]
pub struct BasicExtractor {
    config: ExtractorConfig,
}

impl Extractor for BasicExtractor {
    fn name(&self) -> &str {
        "Basic"
    }

    fn description(&self) -> &str {
        "Fallback extractor that strips HTML tags and extracts plain text from any web page."
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            enrich: false,
            extract: true,
            preview: true,
        }
    }

    /// Always matches — this extractor only makes sense as the last entry
    /// in a chain, tried after everything more specific has already had
    /// its chance.
    fn matches(&self, _document: &Document) -> bool {
        true
    }

    fn extract(&self, document: &Document) -> ExtractOutcome {
        let html_str = document.html.as_deref().unwrap_or("");
        let html = Html::parse_document(html_str);

        let text = html
            .select(&selector("body"))
            .next()
            .map(|body| body_text(&body))
            .unwrap_or_default();
        let text = text.trim();

        let title = html
            .select(&selector("title"))
            .next()
            .map(|el| el.text().collect::<String>())
            .unwrap_or_default();
        let title = title.trim();

        if text.is_empty() && title.is_empty() {
            return ExtractOutcome::Fallback(HisterError::Extraction(
                "no content found".to_string(),
            ));
        }

        let mut extracted = document.clone();
        if !text.is_empty() {
            extracted.text = Some(text.to_string());
        }
        if !title.is_empty() {
            extracted.title = Some(title.to_string());
        }
        ExtractOutcome::Extracted(extracted)
    }

    /// Escapes whatever `document.text` already holds — it does not
    /// derive anything from `document.html` itself, matching Go's own
    /// `basicExtractor.Preview`.
    fn preview(&self, document: &Document) -> PreviewOutcome {
        let text = document.text.as_deref().unwrap_or("");
        PreviewOutcome::Previewed(PreviewResponse {
            html: Some(html_escape(text)),
            text: None,
            metadata: Default::default(),
        })
    }

    fn config(&self) -> &ExtractorConfig {
        &self.config
    }

    fn set_config(&mut self, config: ExtractorConfig) -> Result<(), HisterError> {
        self.config = config;
        Ok(())
    }
}

fn body_text(body: &ElementRef) -> String {
    let mut out = String::new();
    write_text(&mut out, **body);
    out
}

fn write_text(out: &mut String, node: ego_tree::NodeRef<Node>) {
    match node.value() {
        Node::Text(text) => out.push_str(text),
        Node::Element(element) => {
            if SKIPPED_TAGS.contains(&element.name()) {
                return;
            }
            for child in node.children() {
                write_text(out, child);
            }
        }
        _ => {
            for child in node.children() {
                write_text(out, child);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(url: &str, html: &str) -> Document {
        let mut d = Document::new(url);
        d.html = Some(html.to_string());
        d
    }

    #[test]
    fn matches_any_document_including_an_empty_url() {
        let ext = BasicExtractor::default();
        assert!(ext.matches(&Document::new("")));
        assert!(ext.matches(&Document::new("not even a url")));
        assert!(ext.matches(&doc("https://example.com/whatever", "<html></html>")));
    }

    #[test]
    fn extract_strips_tags_and_captures_the_title() {
        let ext = BasicExtractor::default();
        let d = doc(
            "https://example.com/page",
            "<html><head><title>  A Page  </title></head><body><p>Hello</p> <p>World</p></body></html>",
        );
        let ExtractOutcome::Extracted(extracted) = ext.extract(&d) else {
            panic!("expected Extracted");
        };
        assert_eq!(extracted.title.as_deref(), Some("A Page"));
        assert_eq!(extracted.text.as_deref(), Some("Hello World"));
    }

    #[test]
    fn extract_skips_script_style_and_noscript_content() {
        let ext = BasicExtractor::default();
        let d = doc(
            "https://example.com/page",
            "<html><body>keep<script>drop()</script><style>.c{}</style><noscript>drop</noscript></body></html>",
        );
        let ExtractOutcome::Extracted(extracted) = ext.extract(&d) else {
            panic!("expected Extracted");
        };
        assert_eq!(extracted.text.as_deref(), Some("keep"));
    }

    #[test]
    fn extract_does_not_insert_breaks_between_block_elements() {
        let ext = BasicExtractor::default();
        let d = doc(
            "https://example.com/page",
            "<html><body><p>First</p><p>Second</p></body></html>",
        );
        let ExtractOutcome::Extracted(extracted) = ext.extract(&d) else {
            panic!("expected Extracted");
        };
        assert_eq!(extracted.text.as_deref(), Some("FirstSecond"));
    }

    #[test]
    fn extract_falls_back_when_there_is_neither_text_nor_title() {
        let ext = BasicExtractor::default();
        let d = doc(
            "https://example.com/page",
            "<html><head><script>drop()</script></head><body>   </body></html>",
        );
        assert!(matches!(ext.extract(&d), ExtractOutcome::Fallback(_)));
    }

    #[test]
    fn extract_treats_missing_html_as_empty() {
        let ext = BasicExtractor::default();
        let d = Document::new("https://example.com/page");
        assert!(matches!(ext.extract(&d), ExtractOutcome::Fallback(_)));
    }

    #[test]
    fn preview_escapes_the_documents_existing_text() {
        let ext = BasicExtractor::default();
        let mut d = Document::new("https://example.com/page");
        d.text = Some("<script>alert(1)</script> & friends".to_string());
        let PreviewOutcome::Previewed(response) = ext.preview(&d) else {
            panic!("expected Previewed");
        };
        assert_eq!(
            response.html.as_deref(),
            Some("&lt;script&gt;alert(1)&lt;/script&gt; &amp; friends")
        );
    }

    #[test]
    fn preview_succeeds_with_empty_content_when_there_is_no_text() {
        let ext = BasicExtractor::default();
        let d = Document::new("https://example.com/page");
        let PreviewOutcome::Previewed(response) = ext.preview(&d) else {
            panic!("expected Previewed");
        };
        assert_eq!(response.html.as_deref(), Some(""));
    }
}
