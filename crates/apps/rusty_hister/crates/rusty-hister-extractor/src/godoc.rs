//! `GoDoc` — a Rust port of `server/extractor/extractors/godoc/godoc.go`
//! (capability inventory §4.5.8). Preview-only: never contributes extracted
//! text (Go's own `Extract` always falls back too — `sdk.ExtractFallback(nil)`,
//! unconditionally), and renders the `div.Documentation-content` element
//! from a pkg.go.dev package page, with every relative `href`/`src`
//! rewritten to an absolute URL.
//!
//! Go finds this element with a hand-rolled tokenizer that reconstructs
//! HTML byte-by-byte while tracking tag-nesting depth (`golang.org/x/net/html`
//! has no CSS-selector API). `scraper` already builds a full DOM, so the
//! Rust equivalent is a single class selector (`div.Documentation-content`)
//! plus [`scraper::ElementRef::html`] to serialize the matched subtree —
//! no manual depth-tracking needed.
//!
//! When no such element is present, Go's tokenizer loop simply reaches
//! end-of-input having never entered the "in article" state, and returns an
//! empty string with a `nil` error — i.e. `Preview` *succeeds* with empty
//! content rather than falling back. Reproduced faithfully here rather than
//! "corrected" to a `Fallback`, since that's this extractor's real,
//! observable behavior in Go.

use crate::sanitizer::sanitize_html;
use crate::urlutil::rewrite_urls;
use ammonia::Url;
use rusty_hister_core::{
    Capabilities, Document, ExtractOutcome, Extractor, ExtractorConfig, HisterError,
    PreviewOutcome, PreviewResponse,
};
use scraper::{Html, Selector};

const PKG_GO_DEV_PREFIX: &str = "https://pkg.go.dev/";

#[derive(Debug, Default)]
pub struct GoDocExtractor {
    config: ExtractorConfig,
}

impl Extractor for GoDocExtractor {
    fn name(&self) -> &str {
        "GoDoc"
    }

    fn description(&self) -> &str {
        "Extracts and renders Go package documentation from pkg.go.dev pages."
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            enrich: false,
            extract: false,
            preview: true,
        }
    }

    fn matches(&self, document: &Document) -> bool {
        document.url.starts_with(PKG_GO_DEV_PREFIX) && document.url.len() > PKG_GO_DEV_PREFIX.len()
    }

    fn extract(&self, _document: &Document) -> ExtractOutcome {
        ExtractOutcome::Fallback(HisterError::Extraction(
            "GoDoc has no extract phase, preview-only".to_string(),
        ))
    }

    fn preview(&self, document: &Document) -> PreviewOutcome {
        let Some(html_str) = document.html.as_deref() else {
            return PreviewOutcome::Fallback(HisterError::Extraction(
                "document has no HTML".to_string(),
            ));
        };
        let Ok(base) = Url::parse(&document.url) else {
            return PreviewOutcome::Fallback(HisterError::Extraction(
                "invalid document URL".to_string(),
            ));
        };

        let mut html = Html::parse_document(html_str);
        rewrite_urls(&mut html, &base);

        let content_sel =
            Selector::parse("div.Documentation-content").expect("static selector is valid");
        let rendered = html
            .select(&content_sel)
            .next()
            .map(|el| el.html())
            .unwrap_or_default();

        PreviewOutcome::Previewed(PreviewResponse {
            html: Some(sanitize_html(&rendered)),
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

    fn doc(url: &str, html: &str) -> Document {
        let mut d = Document::new(url);
        d.html = Some(html.to_string());
        d
    }

    #[test]
    fn matches_pkg_go_dev_urls_with_a_nonempty_path() {
        let ext = GoDocExtractor::default();
        assert!(ext.matches(&doc("https://pkg.go.dev/fmt", "")));
        assert!(ext.matches(&doc("https://pkg.go.dev/github.com/foo/bar", "")));
    }

    #[test]
    fn does_not_match_bare_host_or_other_domains() {
        let ext = GoDocExtractor::default();
        assert!(!ext.matches(&doc("https://pkg.go.dev/", "")));
        assert!(!ext.matches(&doc("https://pkg.go.dev", "")));
        assert!(!ext.matches(&doc("https://example.com/fmt", "")));
        assert!(!ext.matches(&doc("http://pkg.go.dev/fmt", "")));
    }

    #[test]
    fn extract_always_falls_back() {
        let ext = GoDocExtractor::default();
        let result = ext.extract(&doc("https://pkg.go.dev/fmt", "<html></html>"));
        assert!(matches!(result, ExtractOutcome::Fallback(_)));
    }

    #[test]
    fn preview_renders_the_documentation_content_div() {
        let html = r#"<html><body><div class="Documentation-content"><p>Package fmt implements formatted I/O.</p></div><div class="Other">skip</div></body></html>"#;
        let ext = GoDocExtractor::default();
        match ext.preview(&doc("https://pkg.go.dev/fmt", html)) {
            PreviewOutcome::Previewed(response) => {
                let out = response.html.unwrap();
                assert!(out.contains("Package fmt implements formatted I/O."));
                assert!(!out.contains("skip"));
            }
            other => panic!("expected Previewed, got {other:?}"),
        }
    }

    #[test]
    fn preview_rewrites_relative_urls_to_absolute() {
        let html = r#"<html><body><div class="Documentation-content"><a href="/pkg/errors">errors</a><img src="icon.png"></div></body></html>"#;
        let ext = GoDocExtractor::default();
        match ext.preview(&doc("https://pkg.go.dev/fmt", html)) {
            PreviewOutcome::Previewed(response) => {
                let out = response.html.unwrap();
                assert!(out.contains(r#"href="https://pkg.go.dev/pkg/errors""#));
                assert!(out.contains(r#"src="https://pkg.go.dev/icon.png""#));
            }
            other => panic!("expected Previewed, got {other:?}"),
        }
    }

    #[test]
    fn preview_succeeds_with_empty_content_when_no_documentation_div_is_present() {
        let ext = GoDocExtractor::default();
        match ext.preview(&doc(
            "https://pkg.go.dev/fmt",
            "<html><body><p>no matching div here</p></body></html>",
        )) {
            PreviewOutcome::Previewed(response) => assert_eq!(response.html.as_deref(), Some("")),
            other => panic!("expected Previewed with empty content, got {other:?}"),
        }
    }

    #[test]
    fn preview_sanitizes_script_tags_inside_the_content() {
        let html = r#"<html><body><div class="Documentation-content"><p>safe</p><script>alert(1)</script></div></body></html>"#;
        let ext = GoDocExtractor::default();
        match ext.preview(&doc("https://pkg.go.dev/fmt", html)) {
            PreviewOutcome::Previewed(response) => {
                let out = response.html.unwrap();
                assert!(out.contains("safe"));
                assert!(!out.contains("<script"));
            }
            other => panic!("expected Previewed, got {other:?}"),
        }
    }

    #[test]
    fn preview_falls_back_when_document_has_no_html() {
        let ext = GoDocExtractor::default();
        let mut d = Document::new("https://pkg.go.dev/fmt");
        d.html = None;
        assert!(matches!(ext.preview(&d), PreviewOutcome::Fallback(_)));
    }
}
