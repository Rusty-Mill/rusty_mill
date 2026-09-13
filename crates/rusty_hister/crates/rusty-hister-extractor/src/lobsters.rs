//! `Lobsters` — a Rust port of
//! `server/extractor/extractors/lobsters/lobsters.go` (capability inventory
//! §4.5.10). Extract and preview: pulls the submission metadata, story
//! body, and the full nested comment tree from a lobste.rs story page.
//!
//! The comment tree is genuinely recursive (`li.comments_subtree` nests
//! `ol.comments > li.comments_subtree` arbitrarily deep), reproduced here
//! with the same shape as Go's own recursive helpers
//! (`writeCommentText`/`writeCommentHTML`), walking `scraper`'s DOM instead
//! of `goquery`'s. `ElementRef::child_elements()` — direct children only —
//! is used to descend into exactly one nesting level per recursive call,
//! matching Go's `.Children().Filter(...)`; a broader descendant selector
//! would double-visit deeper subtrees.
//!
//! `writeCommentHTML`'s comment author/score/timestamp are interpolated
//! into the accumulated HTML *unescaped* in Go (unlike the story
//! header/byline, which does escape). Reproduced as-is rather than
//! "corrected", since the entire accumulated string still passes through
//! [`crate::sanitizer::sanitize_html`] before being returned — the same
//! defense Go's own `sanitizer.SanitizeHTML` provides at the same point —
//! so this asymmetry has no observable security effect either way.

use crate::sanitizer::sanitize_html;
use crate::stackexchange::{element_text, html_escape, selector};
use rusty_hister_core::{
    Capabilities, Document, ExtractOutcome, Extractor, ExtractorConfig, HisterError,
    PreviewOutcome, PreviewResponse,
};
use scraper::{CaseSensitivity, ElementRef, Html};

const MATCH_URL_PREFIX: &str = "https://lobste.rs/s/";

#[derive(Debug, Default)]
pub struct LobstersExtractor {
    config: ExtractorConfig,
}

impl Extractor for LobstersExtractor {
    fn name(&self) -> &str {
        "Lobsters"
    }

    fn description(&self) -> &str {
        "Extracts the submission metadata, story body and full nested comment tree from lobste.rs story pages."
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            enrich: false,
            extract: true,
            preview: true,
        }
    }

    fn matches(&self, document: &Document) -> bool {
        document.url.starts_with(MATCH_URL_PREFIX) && document.url.len() > MATCH_URL_PREFIX.len()
    }

    fn extract(&self, document: &Document) -> ExtractOutcome {
        let Some(html_str) = document.html.as_deref() else {
            return ExtractOutcome::Fallback(HisterError::Extraction(
                "document has no HTML".to_string(),
            ));
        };
        let html = Html::parse_document(html_str);

        let story = html.select(&selector("li.story")).next();
        let title = story
            .as_ref()
            .and_then(|s| s.select(&selector(".link .u-url")).next())
            .map(|el| element_text(&el).trim().to_string())
            .unwrap_or_default();

        let mut text = String::new();
        if let Some(story) = &story {
            write_story_text(&mut text, story);
        }
        let body = html
            .select(&selector(".story_content"))
            .next()
            .map(|el| element_text(&el).trim().to_string())
            .unwrap_or_default();
        if !body.is_empty() {
            text.push_str("\n\n");
            text.push_str(&body);
        }
        for subtree in html.select(&selector(
            "#story_comments > ol.comments > li.comments_subtree",
        )) {
            write_comment_text(&mut text, &subtree, 0);
        }
        let text = text.trim().to_string();

        if text.is_empty() && title.is_empty() {
            return ExtractOutcome::Fallback(HisterError::Extraction(
                "no content found".to_string(),
            ));
        }
        let mut extracted = document.clone();
        extracted.title = Some(title);
        extracted.text = Some(text);
        ExtractOutcome::Extracted(extracted)
    }

    fn preview(&self, document: &Document) -> PreviewOutcome {
        let Some(html_str) = document.html.as_deref() else {
            return PreviewOutcome::Fallback(HisterError::Extraction(
                "document has no HTML".to_string(),
            ));
        };
        let html = Html::parse_document(html_str);

        let story = html.select(&selector("li.story")).next();
        let story_link = story
            .as_ref()
            .and_then(|s| s.select(&selector(".link .u-url")).next());
        let title = story_link
            .as_ref()
            .map(|el| element_text(el).trim().to_string())
            .unwrap_or_default();
        let link = story_link
            .as_ref()
            .and_then(|el| el.value().attr("href"))
            .unwrap_or("");
        let author = story
            .as_ref()
            .and_then(|s| s.select(&selector(".byline .u-author")).next())
            .map(|el| element_text(&el).trim().to_string())
            .unwrap_or_default();
        let submitted = story
            .as_ref()
            .and_then(|s| s.select(&selector(".byline time")).next())
            .and_then(|el| el.value().attr("title"))
            .unwrap_or("")
            .trim()
            .to_string();
        let tags: Vec<String> = story
            .as_ref()
            .map(|s| {
                s.select(&selector(".tags .tag"))
                    .map(|el| element_text(&el).trim().to_string())
                    .collect()
            })
            .unwrap_or_default();

        let mut out = String::new();
        if !title.is_empty() || !link.is_empty() {
            out.push_str("<h2>");
            if !link.is_empty() {
                out.push_str(&format!(
                    r#"<a href="{}">{}</a>"#,
                    html_escape(link),
                    html_escape(&title)
                ));
            } else {
                out.push_str(&html_escape(&title));
            }
            out.push_str("</h2>");
        }

        let mut byline_parts = Vec::new();
        if !author.is_empty() {
            byline_parts.push(format!(
                "submitted by <strong>{}</strong>",
                html_escape(&author)
            ));
        }
        if !submitted.is_empty() {
            byline_parts.push(format!("on {}", html_escape(&submitted)));
        }
        if !tags.is_empty() {
            let escaped: Vec<String> = tags.iter().map(|t| html_escape(t)).collect();
            byline_parts.push(format!("tags: {}", escaped.join(", ")));
        }
        if !byline_parts.is_empty() {
            out.push_str(&format!("<p>{}</p>", byline_parts.join(" \u{b7} ")));
        }

        if let Some(content) = html.select(&selector(".story_content")).next() {
            let body = content.inner_html();
            if !body.trim().is_empty() {
                out.push_str(&body);
            }
        }

        let comments: Vec<_> = html
            .select(&selector("ol.comments > li.comments_subtree"))
            .collect();
        if !comments.is_empty() {
            out.push_str("<h2>Comments</h2>");
            out.push_str(r#"<ol class="comments">"#);
            for comment in &comments {
                write_comment_html(&mut out, comment);
            }
            out.push_str("</ol>");
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

/// The byline contains two `/~user` anchors: an `aria-hidden` avatar anchor
/// (empty text) and the real author anchor, so this picks the first one
/// that carries visible text.
fn comment_author(comment: &ElementRef) -> String {
    comment
        .select(&selector(".byline a[href^='/~']"))
        .map(|el| element_text(&el).trim().to_string())
        .find(|text| !text.is_empty())
        .unwrap_or_default()
}

fn is_comments_subtree(element: &ElementRef) -> bool {
    element.value().name() == "li"
        && element
            .value()
            .has_class("comments_subtree", CaseSensitivity::CaseSensitive)
}

/// Writes a short, searchable summary of the submission header.
fn write_story_text(out: &mut String, story: &ElementRef) {
    let title = story
        .select(&selector(".link .u-url"))
        .next()
        .map(|el| element_text(&el).trim().to_string())
        .unwrap_or_default();
    if !title.is_empty() {
        out.push_str(&title);
    }
    let author = story
        .select(&selector(".byline .u-author"))
        .next()
        .map(|el| element_text(&el).trim().to_string())
        .unwrap_or_default();
    if !author.is_empty() {
        out.push_str("\nsubmitted by ");
        out.push_str(&author);
    }
    let tags: Vec<String> = story
        .select(&selector(".tags .tag"))
        .map(|el| element_text(&el).trim().to_string())
        .collect();
    if !tags.is_empty() {
        out.push_str("\ntags: ");
        out.push_str(&tags.join(", "));
    }
}

/// Walks the nested comment subtree and writes each comment as an indented
/// block of plain text so parent/child relationships stay visible in the
/// indexed text.
fn write_comment_text(out: &mut String, subtree: &ElementRef, depth: usize) {
    if let Some(comment) = subtree.select(&selector("div.comment")).next() {
        if comment
            .value()
            .attr("data-shortid")
            .is_some_and(|id| !id.is_empty())
        {
            let indent = "  ".repeat(depth);
            let author = comment_author(&comment);
            let score = comment
                .select(&selector(".voters .upvoter"))
                .next()
                .map(|el| element_text(&el).trim().to_string())
                .unwrap_or_default();
            let body = comment
                .select(&selector(".comment_text"))
                .next()
                .map(|el| element_text(&el).trim().to_string())
                .unwrap_or_default();

            out.push_str("\n\n");
            out.push_str(&indent);
            if !author.is_empty() {
                out.push_str(&author);
            }
            if !score.is_empty() {
                out.push_str(&format!(" [{score}]"));
            }
            if !body.is_empty() {
                for line in body.split('\n') {
                    out.push('\n');
                    out.push_str(&indent);
                    out.push_str(line);
                }
            }
        }
    }
    if let Some(ol_comments) = subtree.select(&selector("ol.comments")).next() {
        for child in ol_comments.child_elements() {
            if is_comments_subtree(&child) {
                write_comment_text(out, &child, depth + 1);
            }
        }
    }
}

/// Renders a single comment subtree as nested `<li>`/`<ol>`, preserving the
/// original reply hierarchy.
fn write_comment_html(out: &mut String, subtree: &ElementRef) {
    let Some(comment) = subtree.select(&selector("div.comment")).next() else {
        return;
    };
    if comment
        .value()
        .attr("data-shortid")
        .is_none_or(|id| id.is_empty())
    {
        return;
    }
    let author = comment_author(&comment);
    let score = comment
        .select(&selector(".voters .upvoter"))
        .next()
        .map(|el| element_text(&el).trim().to_string())
        .unwrap_or_default();
    let when = comment
        .select(&selector(".byline time"))
        .next()
        .and_then(|el| el.value().attr("title"))
        .unwrap_or("")
        .trim()
        .to_string();
    let body = comment
        .select(&selector(".comment_text"))
        .next()
        .map(|el| el.inner_html())
        .unwrap_or_default();

    out.push_str("<li><p>");
    if !author.is_empty() {
        out.push_str(&format!("<strong>{author}</strong>"));
    }
    if !score.is_empty() {
        out.push_str(&format!(" [{score}]"));
    }
    if !when.is_empty() {
        out.push_str(&format!(" &middot; {when}"));
    }
    out.push_str("</p>");
    out.push_str(&body);

    if let Some(ol_comments) = subtree.select(&selector("ol.comments")).next() {
        let children: Vec<_> = ol_comments
            .child_elements()
            .filter(is_comments_subtree)
            .collect();
        if !children.is_empty() {
            out.push_str(r#"<ol class="comments">"#);
            for child in &children {
                write_comment_html(out, child);
            }
            out.push_str("</ol>");
        }
    }
    out.push_str("</li>");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(url: &str, html: &str) -> Document {
        let mut d = Document::new(url);
        d.html = Some(html.to_string());
        d
    }

    const STORY_PAGE: &str = r##"
        <html><body>
        <li class="story">
            <div class="link"><a class="u-url" href="https://example.com/article">An Interesting Article</a></div>
            <div class="byline">
                submitted by <a class="u-author">alice</a>
                <time title="2024-03-01T10:00:00Z">3 days ago</time>
            </div>
            <div class="tags"><a class="tag">rust</a><a class="tag">programming</a></div>
        </li>
        <div class="story_content"><p>This is the story body.</p></div>
        <div id="story_comments">
        <ol class="comments">
            <li class="comments_subtree">
                <div class="comment" data-shortid="abc1">
                    <div class="byline">
                        <a href="/~bob" aria-hidden="true"></a>
                        <a href="/~bob">bob</a>
                        <time title="2024-03-02T11:00:00Z">2 days ago</time>
                    </div>
                    <div class="voters"><span class="upvoter">5</span></div>
                    <div class="comment_text"><p>Great find!</p></div>
                </div>
                <ol class="comments">
                    <li class="comments_subtree">
                        <div class="comment" data-shortid="abc2">
                            <div class="byline"><a href="/~carol">carol</a></div>
                            <div class="voters"><span class="upvoter">2</span></div>
                            <div class="comment_text"><p>Agreed.</p></div>
                        </div>
                    </li>
                </ol>
            </li>
        </ol>
        </div>
        </body></html>
    "##;

    #[test]
    fn matches_lobsters_story_urls_with_a_nonempty_slug() {
        let ext = LobstersExtractor::default();
        assert!(ext.matches(&doc("https://lobste.rs/s/abc123/some_title", "")));
        assert!(!ext.matches(&doc("https://lobste.rs/s/", "")));
        assert!(!ext.matches(&doc("https://lobste.rs/", "")));
        assert!(!ext.matches(&doc("https://example.com/s/abc123/x", "")));
    }

    #[test]
    fn extract_collects_title_body_and_nested_comments() {
        let ext = LobstersExtractor::default();
        match ext.extract(&doc(
            "https://lobste.rs/s/abc123/an_interesting_article",
            STORY_PAGE,
        )) {
            ExtractOutcome::Extracted(result) => {
                assert_eq!(result.title.as_deref(), Some("An Interesting Article"));
                let text = result.text.unwrap();
                assert!(text.contains("An Interesting Article"));
                assert!(text.contains("submitted by alice"));
                assert!(text.contains("tags: rust, programming"));
                assert!(text.contains("This is the story body."));
                assert!(text.contains("bob"));
                assert!(text.contains("[5]"));
                assert!(text.contains("Great find!"));
                assert!(text.contains("carol"));
                assert!(text.contains("Agreed."));
                let bob_pos = text.find("bob").unwrap();
                let carol_pos = text.find("carol").unwrap();
                assert!(
                    bob_pos < carol_pos,
                    "parent comment should precede its nested reply"
                );
            }
            other => panic!("expected Extracted, got {other:?}"),
        }
    }

    #[test]
    fn extract_falls_back_when_nothing_is_found() {
        let ext = LobstersExtractor::default();
        let result = ext.extract(&doc(
            "https://lobste.rs/s/abc123/x",
            "<html><body>nothing here</body></html>",
        ));
        assert!(matches!(result, ExtractOutcome::Fallback(_)));
    }

    #[test]
    fn preview_renders_header_byline_body_and_nested_comments() {
        let ext = LobstersExtractor::default();
        match ext.preview(&doc(
            "https://lobste.rs/s/abc123/an_interesting_article",
            STORY_PAGE,
        )) {
            PreviewOutcome::Previewed(response) => {
                let out = response.html.unwrap();
                assert!(out.contains("An Interesting Article"));
                assert!(out.contains(r#"href="https://example.com/article""#));
                assert!(out.contains("submitted by"));
                assert!(out.contains("alice"));
                assert!(out.contains("rust, programming"));
                assert!(out.contains("This is the story body."));
                assert!(out.contains("<h2>Comments</h2>"));
                assert!(out.contains("Great find!"));
                assert!(out.contains("Agreed."));
            }
            other => panic!("expected Previewed, got {other:?}"),
        }
    }

    #[test]
    fn preview_falls_back_when_document_has_no_html() {
        let ext = LobstersExtractor::default();
        let mut d = Document::new("https://lobste.rs/s/abc123/x");
        d.html = None;
        assert!(matches!(ext.preview(&d), PreviewOutcome::Fallback(_)));
    }

    #[test]
    fn comment_without_data_shortid_is_skipped() {
        let html = r##"
            <html><body>
            <li class="story"><div class="link"><a class="u-url" href="#">T</a></div></li>
            <div id="story_comments"><ol class="comments">
                <li class="comments_subtree">
                    <div class="comment">
                        <div class="comment_text">should not appear</div>
                    </div>
                </li>
            </ol></div>
            </body></html>
        "##;
        let ext = LobstersExtractor::default();
        match ext.extract(&doc("https://lobste.rs/s/abc123/x", html)) {
            ExtractOutcome::Extracted(result) => {
                assert!(!result.text.unwrap().contains("should not appear"));
            }
            other => panic!("expected Extracted, got {other:?}"),
        }
    }
}
