//! `Readability` — a Rust port of the `readabilityExtractor` in
//! `server/extractor/extractor.go` (capability inventory §4.4). Extract
//! *and* preview for any web page, using the [`readabilityrs`] crate (a
//! Rust port of Mozilla's Readability.js, the same algorithm family Go's
//! own dependency — `codeberg.org/readeck/go-readability/v2` — belongs
//! to) to strip navigation, ads, and other boilerplate down to the main
//! article content. `matches` always returns `true`, like
//! [`crate::BasicExtractor`]; the real chain places this extractor right
//! before `Basic` (a better-quality attempt that runs first, with `Basic`
//! as the true last resort when this one can't find an article at all).
//!
//! Go's `readability.FromReader(reader, url)` folds URL validation and
//! article extraction into one error path; `readabilityrs` splits them
//! instead — [`Readability::new`] validates the URL up front and
//! [`Readability::parse`] returns `None` (not an error) when it can't
//! find an article. This port maps the two accordingly: a URL that fails
//! to parse reports [`ExtractOutcome::Abort`]/[`PreviewOutcome::Abort`]
//! (matching Go's own separate `url.Parse` failure, which is also an
//! abort), while a malformed-HTML parse error or no-article-found result
//! reports `Fallback` (matching Go's `FromReader` error path).
//!
//! Two Go fields have no `readabilityrs` equivalent and are deliberately
//! not reproduced rather than silently ignored: a favicon URL (Go's
//! `a.Favicon()`, written to `Document.favicon`) and a `modified`
//! timestamp (Go's `a.ModifiedTime()`) — `readabilityrs`'s [`Article`]
//! exposes neither. Every other field Go copies onto `Document.Metadata`
//! (`author`/`description`/`site_name`/`image`/`language`/`published`)
//! has a direct `Article` counterpart and is reproduced faithfully.
//!
//! `Article::content` is explicitly documented upstream as unsanitized —
//! every attribute of every surviving element is written back out,
//! `onerror`/`onclick`/`javascript:` included — so, like every other
//! preview-capable extractor here, [`preview`](Extractor::preview) always
//! passes it through [`sanitize_html`] before returning it.

use crate::sanitizer::sanitize_html;
use readabilityrs::{Readability, ReadabilityError};
use rusty_hister_core::{
    Capabilities, Document, ExtractOutcome, Extractor, ExtractorConfig, HisterError, Metadata,
    PreviewOutcome, PreviewResponse,
};

#[derive(Debug, Default)]
pub struct ReadabilityExtractor {
    config: ExtractorConfig,
}

impl Extractor for ReadabilityExtractor {
    fn name(&self) -> &str {
        "Readability"
    }

    fn description(&self) -> &str {
        "Extracts the main article content from any web page using the go-readability library, filtering out navigation, ads, and other boilerplate."
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            enrich: false,
            extract: true,
            preview: true,
        }
    }

    fn matches(&self, _document: &Document) -> bool {
        true
    }

    fn extract(&self, document: &Document) -> ExtractOutcome {
        let html = document.html.as_deref().unwrap_or("");
        let readability = match Readability::new(html, Some(&document.url), None) {
            Ok(r) => r,
            Err(ReadabilityError::InvalidUrl(url)) => {
                return ExtractOutcome::Abort(HisterError::Extraction(format!(
                    "invalid document URL: {url}"
                )));
            }
            Err(e) => {
                return ExtractOutcome::Fallback(HisterError::Extraction(e.to_string()));
            }
        };
        let Some(article) = readability.parse() else {
            return ExtractOutcome::Fallback(HisterError::Extraction(
                "no article content found".to_string(),
            ));
        };

        let mut extracted = document.clone();
        write_metadata(&mut extracted.metadata, &article);
        extracted.text = article.text_content;
        if let Some(title) = article.title.filter(|t| !t.is_empty()) {
            extracted.title = Some(title);
        }
        ExtractOutcome::Extracted(extracted)
    }

    fn preview(&self, document: &Document) -> PreviewOutcome {
        let html = document.html.as_deref().unwrap_or("");
        let readability = match Readability::new(html, Some(&document.url), None) {
            Ok(r) => r,
            Err(ReadabilityError::InvalidUrl(url)) => {
                return PreviewOutcome::Abort(HisterError::Extraction(format!(
                    "invalid document URL: {url}"
                )));
            }
            Err(e) => {
                return PreviewOutcome::Fallback(HisterError::Extraction(e.to_string()));
            }
        };
        let Some(article) = readability.parse() else {
            return PreviewOutcome::Fallback(HisterError::Extraction(
                "no article content found".to_string(),
            ));
        };

        let content = article.content.unwrap_or_default();
        PreviewOutcome::Previewed(PreviewResponse {
            html: Some(sanitize_html(&content)),
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

/// Copies the rich fields `readabilityrs` already parsed (internally from
/// JSON-LD, OpenGraph, and meta tags) onto `metadata`, matching Go's own
/// `writeReadabilityMeta` — same keys, so the JSON-LD extractor's own
/// narrower `type`/`headline` fields never collide with these.
fn write_metadata(metadata: &mut Metadata, article: &readabilityrs::Article) {
    let mut set = |key: &str, value: &Option<String>| {
        if let Some(v) = value.as_deref().filter(|v| !v.is_empty()) {
            metadata.insert(key.to_string(), v.into());
        }
    };
    set("author", &article.byline);
    set("description", &article.excerpt);
    set("site_name", &article.site_name);
    set("image", &article.image);
    set("language", &article.lang);
    set("published", &article.published_time);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(url: &str, html: &str) -> Document {
        let mut d = Document::new(url);
        d.html = Some(html.to_string());
        d
    }

    const ARTICLE_HTML: &str = r#"<html>
        <head>
            <title>My Great Article</title>
            <meta property="og:site_name" content="Example News">
            <meta name="author" content="Jane Doe">
        </head>
        <body>
            <nav><a href="/">Home</a><a href="/about">About</a></nav>
            <article>
                <h1>My Great Article</h1>
                <p>This is the first paragraph of a genuinely long article, with enough
                text in it that Mozilla's Readability heuristics recognize it as the
                main content block on the page rather than boilerplate navigation.</p>
                <p>A second paragraph continues the thought with even more substantive
                prose, again long enough to weigh in the algorithm's favor as real
                article content worth extracting and presenting to the reader.</p>
                <p>And a third paragraph, for good measure, so the total text length
                comfortably clears whatever minimum character threshold the
                readability algorithm applies before it commits to this candidate.</p>
            </article>
            <aside>Related links go here, not part of the article.</aside>
        </body>
    </html>"#;

    #[test]
    fn matches_always_returns_true() {
        let ext = ReadabilityExtractor::default();
        assert!(ext.matches(&doc("https://example.com/article", "")));
        assert!(ext.matches(&doc("not a url at all", "")));
    }

    #[test]
    fn extract_collects_title_text_and_metadata() {
        let ext = ReadabilityExtractor::default();
        let d = doc("https://example.com/article", ARTICLE_HTML);
        let ExtractOutcome::Extracted(extracted) = ext.extract(&d) else {
            panic!("expected Extracted");
        };
        assert!(extracted.title.is_some());
        let text = extracted.text.expect("text_content");
        assert!(text.contains("first paragraph"));
        assert!(!text.contains("Related links"));
        assert_eq!(
            extracted.metadata.get("site_name").and_then(|v| v.as_str()),
            Some("Example News")
        );
    }

    #[test]
    fn extract_aborts_on_an_unparseable_url() {
        let ext = ReadabilityExtractor::default();
        let d = doc("not a url at all", ARTICLE_HTML);
        assert!(matches!(ext.extract(&d), ExtractOutcome::Abort(_)));
    }

    #[test]
    fn extract_falls_back_when_there_is_no_article_content() {
        let ext = ReadabilityExtractor::default();
        let d = doc(
            "https://example.com/empty",
            "<html><body><p>too short</p></body></html>",
        );
        assert!(matches!(ext.extract(&d), ExtractOutcome::Fallback(_)));
    }

    #[test]
    fn preview_renders_sanitized_article_html() {
        let ext = ReadabilityExtractor::default();
        let d = doc("https://example.com/article", ARTICLE_HTML);
        let PreviewOutcome::Previewed(response) = ext.preview(&d) else {
            panic!("expected Previewed");
        };
        let content = response.html.unwrap();
        assert!(content.contains("first paragraph"));
        assert!(!content.contains("<nav"));
    }

    #[test]
    fn preview_aborts_on_an_unparseable_url() {
        let ext = ReadabilityExtractor::default();
        let d = doc("not a url at all", ARTICLE_HTML);
        assert!(matches!(ext.preview(&d), PreviewOutcome::Abort(_)));
    }

    #[test]
    fn preview_falls_back_when_there_is_no_article_content() {
        let ext = ReadabilityExtractor::default();
        let d = doc(
            "https://example.com/empty",
            "<html><body><p>too short</p></body></html>",
        );
        assert!(matches!(ext.preview(&d), PreviewOutcome::Fallback(_)));
    }
}
