//! `Markdown` — a Rust port of
//! `server/extractor/extractors/markdown/markdown.go` (capability
//! inventory §4.5.1). Preview-only for locally indexed Markdown files.
//!
//! Go's own doc comment on `MarkdownExtractor` is the key to this
//! extractor's simplicity: during indexing, `Indexer.AddMarkdown` renders
//! the Markdown source to HTML and stores it in `doc.HTML` *before* the
//! extractor chain ever runs, so this extractor's only job is to sanitize
//! and return whatever HTML is already there. It has no markdown-parsing
//! dependency of its own — that's `rusty-hister-indexer`'s concern
//! (capability inventory §5.7, not yet started), not this crate's, and
//! adding a markdown parser here would duplicate work the indexer already
//! has to do to populate `document.html` in the first place.

use crate::sanitizer::sanitize_html;
use rusty_hister_core::{
    Capabilities, Document, ExtractOutcome, Extractor, ExtractorConfig, HisterError,
    PreviewOutcome, PreviewResponse,
};

#[derive(Debug, Default)]
pub struct MarkdownExtractor {
    config: ExtractorConfig,
}

impl Extractor for MarkdownExtractor {
    fn name(&self) -> &str {
        "Markdown"
    }

    fn description(&self) -> &str {
        "Renders locally indexed Markdown files (.md, .markdown) as HTML for preview."
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            enrich: false,
            extract: false,
            preview: true,
        }
    }

    /// Accepts `file://` URLs ending in `.md` or `.markdown` (case
    /// insensitive), matching Go's plain prefix/suffix check exactly —
    /// no URL parsing, since Go doesn't do any here either.
    fn matches(&self, document: &Document) -> bool {
        if !document.url.starts_with("file://") {
            return false;
        }
        let lower = document.url.to_ascii_lowercase();
        lower.ends_with(".md") || lower.ends_with(".markdown")
    }

    fn extract(&self, _document: &Document) -> ExtractOutcome {
        ExtractOutcome::Fallback(HisterError::Extraction(
            "Markdown has no extract phase, preview-only".to_string(),
        ))
    }

    fn preview(&self, document: &Document) -> PreviewOutcome {
        let Some(html) = document.html.as_deref().filter(|h| !h.is_empty()) else {
            return PreviewOutcome::Fallback(HisterError::Extraction(
                "document has no HTML".to_string(),
            ));
        };
        PreviewOutcome::Previewed(PreviewResponse {
            html: Some(sanitize_html(html)),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(url: &str, html: Option<&str>) -> Document {
        let mut d = Document::new(url);
        d.html = html.map(|h| h.to_string());
        d
    }

    #[test]
    fn matches_file_urls_ending_in_md_or_markdown_case_insensitively() {
        let ext = MarkdownExtractor::default();
        let cases: &[(&str, bool)] = &[
            ("file:///home/user/notes.md", true),
            ("file:///home/user/notes.MARKDOWN", true),
            ("file:///home/user/notes.Md", true),
            ("file:///home/user/notes.txt", false),
            ("https://example.com/notes.md", false),
            ("file:///home/user/markdown", false),
        ];
        for (url, want) in cases {
            assert_eq!(ext.matches(&doc(url, None)), *want, "url = {url}");
        }
    }

    #[test]
    fn extract_always_falls_back() {
        let ext = MarkdownExtractor::default();
        let d = doc("file:///home/user/notes.md", Some("<p>hi</p>"));
        assert!(matches!(ext.extract(&d), ExtractOutcome::Fallback(_)));
    }

    #[test]
    fn preview_sanitizes_the_already_rendered_html() {
        let ext = MarkdownExtractor::default();
        let d = doc(
            "file:///home/user/notes.md",
            Some(r#"<h1>Title</h1><script>alert(1)</script><p>Body text.</p>"#),
        );
        let PreviewOutcome::Previewed(response) = ext.preview(&d) else {
            panic!("expected Previewed");
        };
        let content = response.html.unwrap();
        assert!(content.contains("<h1>Title</h1>"));
        assert!(content.contains("<p>Body text.</p>"));
        assert!(!content.contains("<script>"));
    }

    #[test]
    fn preview_falls_back_when_there_is_no_html() {
        let ext = MarkdownExtractor::default();
        assert!(matches!(
            ext.preview(&doc("file:///home/user/notes.md", None)),
            PreviewOutcome::Fallback(_)
        ));
        assert!(matches!(
            ext.preview(&doc("file:///home/user/notes.md", Some(""))),
            PreviewOutcome::Fallback(_)
        ));
    }
}
