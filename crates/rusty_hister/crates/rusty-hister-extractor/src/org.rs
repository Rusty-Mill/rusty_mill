//! `OrgMode` — a Rust port of `server/extractor/extractors/org/org.go`
//! (capability inventory §4.5.2). Preview-only for locally indexed Org
//! files, structurally identical to [`crate::MarkdownExtractor`] and for
//! the same reason: `Indexer.AddOrg` renders Org source to HTML and
//! stores it in `document.html` at index time, so this extractor's only
//! job is to sanitize and return whatever HTML is already there — no
//! Org-mode-parsing dependency of its own, since that's
//! `rusty-hister-indexer`'s concern (capability inventory §5.7).

use crate::sanitizer::sanitize_html;
use rusty_hister_core::{
    Capabilities, Document, ExtractOutcome, Extractor, ExtractorConfig, HisterError,
    PreviewOutcome, PreviewResponse,
};

#[derive(Debug, Default)]
pub struct OrgModeExtractor {
    config: ExtractorConfig,
}

impl Extractor for OrgModeExtractor {
    fn name(&self) -> &str {
        "OrgMode"
    }

    fn description(&self) -> &str {
        "Renders locally indexed Org files (.org) as HTML for preview."
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            enrich: false,
            extract: false,
            preview: true,
        }
    }

    /// Accepts `file://` URLs ending in `.org` (case insensitive),
    /// matching Go's plain prefix/suffix check exactly.
    fn matches(&self, document: &Document) -> bool {
        if !document.url.starts_with("file://") {
            return false;
        }
        document.url.to_ascii_lowercase().ends_with(".org")
    }

    fn extract(&self, _document: &Document) -> ExtractOutcome {
        ExtractOutcome::Fallback(HisterError::Extraction(
            "OrgMode has no extract phase, preview-only".to_string(),
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
    fn matches_file_urls_ending_in_org_case_insensitively() {
        let ext = OrgModeExtractor::default();
        let cases: &[(&str, bool)] = &[
            ("file:///home/user/notes.org", true),
            ("file:///home/user/notes.ORG", true),
            ("file:///home/user/notes.txt", false),
            ("https://example.com/notes.org", false),
            ("file:///home/user/organization", false),
        ];
        for (url, want) in cases {
            assert_eq!(ext.matches(&doc(url, None)), *want, "url = {url}");
        }
    }

    #[test]
    fn extract_always_falls_back() {
        let ext = OrgModeExtractor::default();
        let d = doc("file:///home/user/notes.org", Some("<p>hi</p>"));
        assert!(matches!(ext.extract(&d), ExtractOutcome::Fallback(_)));
    }

    #[test]
    fn preview_sanitizes_the_already_rendered_html() {
        let ext = OrgModeExtractor::default();
        let d = doc(
            "file:///home/user/notes.org",
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
        let ext = OrgModeExtractor::default();
        assert!(matches!(
            ext.preview(&doc("file:///home/user/notes.org", None)),
            PreviewOutcome::Fallback(_)
        ));
        assert!(matches!(
            ext.preview(&doc("file:///home/user/notes.org", Some(""))),
            PreviewOutcome::Fallback(_)
        ));
    }
}
