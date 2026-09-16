//! `Notion` — a Rust port of
//! `server/extractor/extractors/notion/notion.go` (capability inventory
//! §4.5.16). Extract and preview for Notion pages on `notion.so` and
//! `*.notion.site`.
//!
//! Notion serves an empty SPA shell over plain HTTP and only renders page
//! content client-side, so this only produces real output when
//! `document.html` was captured by a JavaScript-rendering crawler backend
//! (chromedp or WebDriver BiDi) — a fact Go's own doc comment on
//! `NotionExtractor` calls out. That's a *production* dependency on how
//! the document was crawled, not a dependency of this module's own code
//! on the crawler: like every other extractor here, it only ever reads
//! `document.html`, whatever produced it. When the rendered block tree
//! isn't present, [`extract`](Extractor::extract)/
//! [`preview`](Extractor::preview) report [`ExtractOutcome::Abort`]/
//! [`PreviewOutcome::Abort`] rather than `Fallback` — matching Go's own
//! `AbortExtraction`/`AbortPreview`, so a matched but unrendered Notion
//! URL doesn't get indexed as a low-quality "Notion"-titled placeholder
//! that would later duplicate against a properly rendered crawl.
//!
//! Notion's rendered DOM nests presentational wrapper `<div>`s deeply;
//! [`block_selector`]'s `[class*="notion-"][class*="-block"]` substring
//! match (identical to Go's own `goquery` selector) finds every block
//! *at any depth*, so both [`block_text`] and [`write_blocks_html`] skip
//! a match whose own ancestor also matches — the outer block's own walk
//! already covers its children's text, and re-visiting them would
//! duplicate content.

use crate::sanitizer::sanitize_html;
use crate::stackexchange::{element_text, html_escape, selector};
use crate::urlutil::rewrite_urls;
use ammonia::Url;
use rusty_hister_core::{
    Capabilities, Document, ExtractOutcome, Extractor, ExtractorConfig, HisterError,
    PreviewOutcome, PreviewResponse,
};
use scraper::{CaseSensitivity, ElementRef, Html, Selector};

fn block_selector() -> Selector {
    selector(r#"[class*="notion-"][class*="-block"]"#)
}

#[derive(Debug, Default)]
pub struct NotionExtractor {
    config: ExtractorConfig,
}

impl Extractor for NotionExtractor {
    fn name(&self) -> &str {
        "Notion"
    }

    fn description(&self) -> &str {
        "Extracts the title and block content of Notion pages on notion.so and *.notion.site. Requires a JavaScript-rendering crawler backend (chromedp or bidi) because Notion renders content client-side."
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            enrich: false,
            extract: true,
            preview: true,
        }
    }

    /// Accepts `notion.so`/`www.notion.so` and any `*.notion.site`
    /// subdomain (publicly shared pages), as long as the path has at
    /// least one non-empty segment — skipping the workspace root and the
    /// login page.
    fn matches(&self, document: &Document) -> bool {
        let Ok(url) = Url::parse(&document.url) else {
            return false;
        };
        let host = url.host_str().unwrap_or("").to_ascii_lowercase();
        if host != "notion.so" && host != "www.notion.so" && !host.ends_with(".notion.site") {
            return false;
        }
        !url.path().trim_matches('/').is_empty()
    }

    fn extract(&self, document: &Document) -> ExtractOutcome {
        let html_str = document.html.as_deref().unwrap_or("");
        let html = Html::parse_document(html_str);
        let Some(content) = page_content(&html) else {
            return ExtractOutcome::Abort(HisterError::Extraction(
                "notion page content not rendered".to_string(),
            ));
        };

        let title = page_title(&html, Some(&content));
        let text = block_text(&content);
        if title.is_empty() && text.is_empty() {
            return ExtractOutcome::Fallback(HisterError::Extraction(
                "no content found".to_string(),
            ));
        }

        let mut extracted = document.clone();
        if !title.is_empty() {
            extracted.title = Some(title);
        }
        if !text.is_empty() {
            extracted.text = Some(text);
        }
        ExtractOutcome::Extracted(extracted)
    }

    fn preview(&self, document: &Document) -> PreviewOutcome {
        let html_str = document.html.as_deref().unwrap_or("");
        let html = Html::parse_document(html_str);
        let Some(content) = page_content(&html) else {
            return PreviewOutcome::Abort(HisterError::Extraction(
                "notion page content not rendered".to_string(),
            ));
        };
        let Ok(base) = Url::parse(&document.url) else {
            return PreviewOutcome::Fallback(HisterError::Extraction(
                "invalid document URL".to_string(),
            ));
        };

        // Reparse the content subtree as its own fragment before
        // rewriting URLs, rather than mutating the shared `html` document
        // in place — the same "reparse as an independent document" trick
        // `WikipediaExtractor::extract`'s noise-removal clone and
        // `RedditExtractor`'s body/comment HTML use, since
        // `scraper::ElementRef` has no in-place mutation API of its own.
        let mut fragment = Html::parse_fragment(&format!("<div>{}</div>", content.inner_html()));
        rewrite_urls(&mut fragment, &base);
        let Some(rewritten) = fragment.select(&selector("div")).next() else {
            return PreviewOutcome::Fallback(HisterError::Extraction(
                "notion page content not rendered".to_string(),
            ));
        };

        let mut out = String::new();
        let title = page_title(&html, Some(&rewritten));
        if !title.is_empty() {
            out.push_str(&format!("<h1>{}</h1>\n", html_escape(&title)));
        }
        write_blocks_html(&mut out, &rewritten);

        if out.is_empty() {
            return PreviewOutcome::Fallback(HisterError::Extraction(
                "no preview content".to_string(),
            ));
        }
        PreviewOutcome::Previewed(PreviewResponse {
            html: Some(sanitize_html(&out)),
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

/// Finds the selection wrapping a single Notion page's body: Notion
/// renders it inside `.notion-page-content` (`.notion-page-content-inner`
/// on some shared pages).
fn page_content(html: &Html) -> Option<ElementRef<'_>> {
    html.select(&selector(".notion-page-content"))
        .next()
        .or_else(|| html.select(&selector(".notion-page-content-inner")).next())
}

/// Finds the page's `<h1>` title: it lives inside the first
/// `.notion-page-block` above the content on a normal page, or as
/// `content`'s own first `<h1>` child on some shared pages.
fn page_title(html: &Html, content: Option<&ElementRef>) -> String {
    let from_block = html
        .select(&selector(".notion-page-block h1"))
        .next()
        .map(|el| element_text(&el).trim().to_string())
        .filter(|t| !t.is_empty());
    if let Some(t) = from_block {
        return t;
    }
    if let Some(content) = content {
        let from_content = content
            .select(&selector("h1"))
            .next()
            .map(|el| element_text(&el).trim().to_string())
            .filter(|t| !t.is_empty());
        if let Some(t) = from_content {
            return t;
        }
    }
    html.select(&selector("h1"))
        .next()
        .map(|el| element_text(&el).trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_default()
}

fn has_block_class(el: &ElementRef, class: &str) -> bool {
    el.value().has_class(class, CaseSensitivity::CaseSensitive)
}

fn is_nested_block(el: &ElementRef, block_sel: &Selector) -> bool {
    el.ancestors()
        .any(|a| ElementRef::wrap(a).is_some_and(|e| block_sel.matches(&e)))
}

/// Walks the top-level Notion blocks in `content` and writes a plain-text
/// representation, preserving heading prominence and list bullets so the
/// indexed text reads naturally.
fn block_text(content: &ElementRef) -> String {
    let block_sel = block_selector();
    let mut out = String::new();
    for block in content.select(&block_sel) {
        if has_block_class(&block, "notion-page-block") || is_nested_block(&block, &block_sel) {
            continue;
        }
        let text = element_text(&block).trim().to_string();
        if text.is_empty() {
            continue;
        }
        if has_block_class(&block, "notion-header-block")
            || has_block_class(&block, "notion-sub_header-block")
            || has_block_class(&block, "notion-sub_sub_header-block")
        {
            out.push_str(&format!("\n\n{text}\n"));
        } else if has_block_class(&block, "notion-bulleted_list-block")
            || has_block_class(&block, "notion-to_do-block")
        {
            out.push_str(&format!("\n* {text}"));
        } else if has_block_class(&block, "notion-numbered_list-block") {
            out.push_str(&format!("\n- {text}"));
        } else if has_block_class(&block, "notion-quote-block") {
            out.push_str(&format!("\n\n> {text}"));
        } else {
            out.push_str(&format!("\n\n{text}"));
        }
    }
    out.trim().to_string()
}

/// Renders the top-level Notion block tree in `content` as semantic HTML,
/// collapsing Notion's deeply nested presentational divs into plain
/// headings, paragraphs, lists, blockquotes, and code blocks.
fn write_blocks_html(out: &mut String, content: &ElementRef) {
    let block_sel = block_selector();
    for block in content.select(&block_sel) {
        if has_block_class(&block, "notion-page-block") || is_nested_block(&block, &block_sel) {
            continue;
        }
        if has_block_class(&block, "notion-header-block") {
            write_tag(out, "h2", &block);
        } else if has_block_class(&block, "notion-sub_header-block") {
            write_tag(out, "h3", &block);
        } else if has_block_class(&block, "notion-sub_sub_header-block") {
            write_tag(out, "h4", &block);
        } else if has_block_class(&block, "notion-bulleted_list-block")
            || has_block_class(&block, "notion-to_do-block")
        {
            let text = element_text(&block).trim().to_string();
            if !text.is_empty() {
                out.push_str(&format!("<ul><li>{}</li></ul>", html_escape(&text)));
            }
        } else if has_block_class(&block, "notion-numbered_list-block") {
            let text = element_text(&block).trim().to_string();
            if !text.is_empty() {
                out.push_str(&format!("<ol><li>{}</li></ol>", html_escape(&text)));
            }
        } else if has_block_class(&block, "notion-quote-block") {
            write_tag(out, "blockquote", &block);
        } else if has_block_class(&block, "notion-code-block") {
            // Not trimmed, unlike every other case: preserving a code
            // block's exact whitespace/indentation is the point.
            out.push_str(&format!(
                "<pre><code>{}</code></pre>",
                html_escape(&element_text(&block))
            ));
        } else if has_block_class(&block, "notion-divider-block") {
            out.push_str("<hr>");
        } else if has_block_class(&block, "notion-image-block") {
            if let Some(src) = block
                .select(&selector("img"))
                .next()
                .and_then(|img| img.value().attr("src"))
            {
                out.push_str(&format!(
                    r#"<p><img src="{}" alt=""></p>"#,
                    html_escape(src)
                ));
            }
        } else {
            write_tag(out, "p", &block);
        }
    }
}

fn write_tag(out: &mut String, tag: &str, block: &ElementRef) {
    let text = element_text(block).trim().to_string();
    if text.is_empty() {
        return;
    }
    out.push_str(&format!("<{tag}>{}</{tag}>", html_escape(&text)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(url: &str, html: &str) -> Document {
        let mut d = Document::new(url);
        d.html = Some(html.to_string());
        d
    }

    const RENDERED_PAGE: &str = r#"<html><body>
        <div class="notion-page-block"><h1>My Notion Page</h1></div>
        <div class="notion-page-content">
            <div class="notion-header-block">Section One</div>
            <div class="notion-text-block">
                <div class="notion-selectable">Intro paragraph.</div>
            </div>
            <div class="notion-bulleted_list-block">First bullet</div>
            <div class="notion-bulleted_list-block">Second bullet</div>
            <div class="notion-quote-block">A quotable line.</div>
            <div class="notion-code-block">let x = 1;</div>
        </div>
    </body></html>"#;

    #[test]
    fn matches_notion_and_notion_site_urls_with_a_nonempty_path() {
        let ext = NotionExtractor::default();
        let cases: &[(&str, bool)] = &[
            ("https://www.notion.so/My-Page-abc123", true),
            ("https://notion.so/My-Page-abc123", true),
            ("https://someone.notion.site/Shared-Page-abc123", true),
            ("https://www.notion.so/", false),
            ("https://www.notion.so", false),
            ("https://example.com/My-Page-abc123", false),
        ];
        for (url, want) in cases {
            assert_eq!(ext.matches(&Document::new(*url)), *want, "url = {url}");
        }
    }

    #[test]
    fn extract_collects_title_and_block_text_in_order() {
        let ext = NotionExtractor::default();
        let d = doc("https://www.notion.so/My-Page-abc123", RENDERED_PAGE);
        let ExtractOutcome::Extracted(extracted) = ext.extract(&d) else {
            panic!("expected Extracted");
        };
        assert_eq!(extracted.title.as_deref(), Some("My Notion Page"));
        let text = extracted.text.unwrap();
        assert!(text.contains("Section One"));
        assert!(text.contains("Intro paragraph."));
        assert!(text.contains("* First bullet"));
        assert!(text.contains("* Second bullet"));
        assert!(text.contains("> A quotable line."));
        assert!(text.contains("let x = 1;"));
    }

    #[test]
    fn extract_does_not_duplicate_nested_block_text() {
        let ext = NotionExtractor::default();
        let d = doc("https://www.notion.so/My-Page-abc123", RENDERED_PAGE);
        let ExtractOutcome::Extracted(extracted) = ext.extract(&d) else {
            panic!("expected Extracted");
        };
        let text = extracted.text.unwrap();
        assert_eq!(text.matches("Intro paragraph.").count(), 1);
    }

    #[test]
    fn extract_aborts_when_the_page_was_not_rendered() {
        let ext = NotionExtractor::default();
        let d = doc(
            "https://www.notion.so/My-Page-abc123",
            "<html><body><div id=\"notion-app\"></div></body></html>",
        );
        assert!(matches!(ext.extract(&d), ExtractOutcome::Abort(_)));
    }

    #[test]
    fn extract_falls_back_when_rendered_but_empty() {
        let ext = NotionExtractor::default();
        let d = doc(
            "https://www.notion.so/My-Page-abc123",
            r#"<html><body><div class="notion-page-content"></div></body></html>"#,
        );
        assert!(matches!(ext.extract(&d), ExtractOutcome::Fallback(_)));
    }

    #[test]
    fn preview_renders_headings_lists_and_code_and_rewrites_urls() {
        let ext = NotionExtractor::default();
        let html = r#"<html><body>
            <div class="notion-page-block"><h1>Links Page</h1></div>
            <div class="notion-page-content">
                <div class="notion-header-block">Heading</div>
                <div class="notion-bulleted_list-block">A bullet with <a href="/other-page">a link</a>.</div>
                <div class="notion-divider-block"></div>
                <div class="notion-image-block"><img src="/images/pic.png"></div>
            </div>
        </body></html>"#;
        let d = doc("https://www.notion.so/Links-Page-abc123", html);
        let PreviewOutcome::Previewed(response) = ext.preview(&d) else {
            panic!("expected Previewed");
        };
        let content = response.html.unwrap();
        assert!(content.contains("<h1>Links Page</h1>"));
        assert!(content.contains("<h2>Heading</h2>"));
        // List/heading/quote/paragraph blocks render via `element_text`
        // (plain text, HTML-escaped) exactly like Go's own `writeTag`/list
        // handling in `writeBlocksHTML` — so an inline `<a href>` inside a
        // bullet is flattened to text, not preserved as a link. Only the
        // image block's `src` attribute is read and rewritten directly, so
        // that's the one place URL-rewriting is observable in the preview.
        assert!(content.contains("<ul><li>A bullet with a link.</li></ul>"));
        assert!(content.contains("<hr>"));
        assert!(content.contains(r#"src="https://www.notion.so/images/pic.png""#));
    }

    #[test]
    fn preview_aborts_when_the_page_was_not_rendered() {
        let ext = NotionExtractor::default();
        let d = doc(
            "https://www.notion.so/My-Page-abc123",
            "<html><body><div id=\"notion-app\"></div></body></html>",
        );
        assert!(matches!(ext.preview(&d), PreviewOutcome::Abort(_)));
    }
}
