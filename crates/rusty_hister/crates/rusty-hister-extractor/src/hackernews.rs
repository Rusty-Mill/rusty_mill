//! `HackerNews` — a Rust port of
//! `server/extractor/extractors/hackernews/hackernews.go` (capability
//! inventory §4.5.11). Extract and preview: the submission metadata, self
//! text, and full comment tree from a news.ycombinator.com item page.
//!
//! Unlike Lobsters' nested markup, Hacker News does not nest its comments:
//! every comment is a sibling row in one flat table, and its depth is
//! carried by the `indent` attribute on the leading `td.ind` cell.
//! Reconstructing the tree therefore means tracking that number across the
//! row sequence (see [`write_comment_tree`]) rather than recursing, which
//! is what [`comment_rows`] and this module's Go original both do.

use crate::sanitizer::sanitize_html;
use crate::stackexchange::{element_text, html_escape, selector};
use crate::textutil::selection_text;
use crate::urlutil::rewrite_urls;
use ammonia::Url;
use rusty_hister_core::{
    Capabilities, Document, ExtractOutcome, Extractor, ExtractorConfig, HisterError,
    PreviewOutcome, PreviewResponse,
};
use scraper::{ElementRef, Html};

#[derive(Debug, Default)]
pub struct HackerNewsExtractor {
    config: ExtractorConfig,
}

impl Extractor for HackerNewsExtractor {
    fn name(&self) -> &str {
        "HackerNews"
    }

    fn description(&self) -> &str {
        "Extracts the submission metadata, self text and full comment tree from Hacker News item pages."
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            enrich: false,
            extract: true,
            preview: true,
        }
    }

    /// Accepts item pages on news.ycombinator.com. The URL is parsed
    /// rather than prefix-matched so that pagination and tracking
    /// parameters, which show up on long threads, don't stop a page from
    /// being recognized.
    fn matches(&self, document: &Document) -> bool {
        let Ok(url) = Url::parse(&document.url) else {
            return false;
        };
        let host = url.host_str().unwrap_or("").to_ascii_lowercase();
        if host != "news.ycombinator.com" && host != "www.news.ycombinator.com" {
            return false;
        }
        if url.path().trim_end_matches('/') != "/item" {
            return false;
        }
        let id = url
            .query_pairs()
            .find(|(key, _)| key == "id")
            .map(|(_, value)| value.into_owned());
        !id.unwrap_or_default().is_empty()
    }

    fn extract(&self, document: &Document) -> ExtractOutcome {
        let Some(html_str) = document.html.as_deref() else {
            return ExtractOutcome::Fallback(HisterError::Extraction(
                "document has no HTML".to_string(),
            ));
        };
        let html = Html::parse_document(html_str);
        let story = parse_story(&html);

        let mut text = String::new();
        if !story.title.is_empty() {
            text.push_str(&story.title);
        }
        if !story.site.is_empty() {
            text.push('\n');
            text.push_str(&story.site);
        }
        let mut byline = Vec::new();
        if !story.score.is_empty() {
            byline.push(story.score.clone());
        }
        if !story.author.is_empty() {
            byline.push(format!("by {}", story.author));
        }
        if !story.age.is_empty() {
            byline.push(format!("on {}", story.age));
        }
        if !byline.is_empty() {
            text.push('\n');
            text.push_str(&byline.join(" "));
        }
        if let Some(self_text) = &story.self_text {
            let body = selection_text(self_text);
            if !body.is_empty() {
                text.push_str("\n\n");
                text.push_str(&body);
            }
        }

        for comment in comment_rows(&html) {
            let indent = "  ".repeat(comment.depth);
            text.push_str("\n\n");
            text.push_str(&indent);
            if !comment.author.is_empty() {
                text.push_str(&comment.author);
            }
            if !comment.age.is_empty() {
                text.push_str(&format!(" [{}]", comment.age));
            }
            for line in comment.body.split('\n') {
                text.push('\n');
                if line.is_empty() {
                    continue;
                }
                text.push_str(&indent);
                text.push_str(line);
            }
        }

        let text = text.trim().to_string();
        if text.is_empty() && story.title.is_empty() {
            return ExtractOutcome::Fallback(HisterError::Extraction(
                "no content found".to_string(),
            ));
        }
        let mut extracted = document.clone();
        extracted.title = Some(story.title);
        extracted.text = Some(text);
        ExtractOutcome::Extracted(extracted)
    }

    /// Renders the submission and its comment tree as sanitized HTML. The
    /// flat indent sequence is turned back into nested lists so the
    /// preview shows the reply structure rather than a wall of
    /// equal-weight comments.
    fn preview(&self, document: &Document) -> PreviewOutcome {
        let Some(html_str) = document.html.as_deref() else {
            return PreviewOutcome::Fallback(HisterError::Extraction(
                "document has no HTML".to_string(),
            ));
        };
        let mut html = Html::parse_document(html_str);

        // Hacker News links internally with relative URLs (item?id=123 on
        // Ask HN titles, polls, and reply links) which the sanitizer
        // strips since it only keeps absolute http(s) URLs. Resolving
        // everything against the page URL up front keeps those links in
        // the preview.
        if let Ok(base) = Url::parse(&document.url) {
            rewrite_urls(&mut html, &base);
        }

        let story = parse_story(&html);

        let mut out = String::new();
        if !story.title.is_empty() || !story.link.is_empty() {
            out.push_str("<h2>");
            if !story.link.is_empty() {
                out.push_str(&format!(
                    r#"<a href="{}">{}</a>"#,
                    html_escape(&story.link),
                    html_escape(&story.title)
                ));
            } else {
                out.push_str(&html_escape(&story.title));
            }
            out.push_str("</h2>");
        }

        let mut byline_parts = Vec::new();
        if !story.score.is_empty() {
            byline_parts.push(html_escape(&story.score));
        }
        if !story.author.is_empty() {
            byline_parts.push(format!(
                "submitted by <strong>{}</strong>",
                html_escape(&story.author)
            ));
        }
        if !story.age.is_empty() {
            byline_parts.push(format!("on {}", html_escape(&story.age)));
        }
        if !story.site.is_empty() {
            byline_parts.push(html_escape(&story.site));
        }
        if !byline_parts.is_empty() {
            out.push_str(&format!("<p>{}</p>", byline_parts.join(" \u{b7} ")));
        }

        if let Some(self_text) = &story.self_text {
            let inner = self_text.inner_html();
            if !inner.trim().is_empty() {
                out.push_str(&inner);
            }
        }

        let comments = comment_rows(&html);
        if !comments.is_empty() {
            out.push_str("<h2>Comments</h2>");
            write_comment_tree(&mut out, &comments);
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

/// The header fields shared by `extract` and `preview`.
struct Story<'a> {
    title: String,
    link: String,
    site: String,
    score: String,
    author: String,
    age: String,
    /// The body of an Ask HN or text submission, absent for link
    /// submissions.
    self_text: Option<ElementRef<'a>>,
}

fn parse_story(html: &Html) -> Story<'_> {
    let title_link = html.select(&selector(".titleline > a")).next();
    let subline = html.select(&selector(".subline")).next();
    Story {
        title: title_link
            .as_ref()
            .map(|el| element_text(el).trim().to_string())
            .unwrap_or_default(),
        link: title_link
            .as_ref()
            .and_then(|el| el.value().attr("href"))
            .unwrap_or("")
            .trim()
            .to_string(),
        site: html
            .select(&selector(".titleline .sitestr"))
            .next()
            .map(|el| element_text(&el).trim().to_string())
            .unwrap_or_default(),
        score: subline
            .as_ref()
            .and_then(|s| s.select(&selector(".score")).next())
            .map(|el| element_text(&el).trim().to_string())
            .unwrap_or_default(),
        author: subline
            .as_ref()
            .and_then(|s| s.select(&selector(".hnuser")).next())
            .map(|el| element_text(&el).trim().to_string())
            .unwrap_or_default(),
        age: subline
            .as_ref()
            .and_then(|s| s.select(&selector(".age")).next())
            .and_then(|el| el.value().attr("title"))
            .unwrap_or("")
            .trim()
            .to_string(),
        self_text: html.select(&selector(".fatitem .toptext")).next(),
    }
}

/// Maximum comment nesting depth accepted from a crawled page's `indent`
/// attribute. Hacker News threads rarely nest beyond a few dozen levels;
/// the attribute comes straight from untrusted crawled markup, so an
/// attacker-controlled `indent="999999999"` is clamped here before it can
/// reach `"  ".repeat(depth)` or the `<ul>`/`</li></ul>` loops in
/// [`write_comment_tree`] — both of which would otherwise attempt an
/// allocation/iteration count proportional to the attacker-chosen value.
const MAX_COMMENT_DEPTH: usize = 100;

/// One row of the flat comment table.
struct Comment {
    depth: usize,
    author: String,
    age: String,
    body: String,
    body_html: String,
}

fn comment_rows(html: &Html) -> Vec<Comment> {
    let mut comments = Vec::new();
    for row in html.select(&selector("tr.athing.comtr")) {
        let depth = row
            .select(&selector("td.ind"))
            .next()
            .and_then(|el| el.value().attr("indent"))
            .and_then(|s| s.parse::<i64>().ok())
            .filter(|&d| d >= 0)
            .unwrap_or(0)
            .min(MAX_COMMENT_DEPTH as i64) as usize;
        let text = row.select(&selector(".commtext")).next();
        // Collapsed and flagged comments keep their row but carry no
        // body. They are still worth a line so the thread shape
        // survives, but an entirely empty one adds nothing to the index.
        let body = text.as_ref().map(selection_text).unwrap_or_default();
        let author = row
            .select(&selector(".comhead .hnuser"))
            .next()
            .map(|el| element_text(&el).trim().to_string())
            .unwrap_or_default();
        if body.is_empty() && author.is_empty() {
            continue;
        }
        let body_html = text.as_ref().map(|el| el.inner_html()).unwrap_or_default();
        let age = row
            .select(&selector(".comhead .age"))
            .next()
            .and_then(|el| el.value().attr("title"))
            .unwrap_or("")
            .trim()
            .to_string();
        comments.push(Comment {
            depth,
            author,
            age,
            body,
            body_html,
        });
    }
    comments
}

/// Converts the flat depth-tagged sequence into nested lists. Depth can
/// jump by more than one only downward in malformed markup, so the close
/// path loops while the open path steps once, which keeps the tag stack
/// balanced whatever the input.
fn write_comment_tree(out: &mut String, comments: &[Comment]) {
    let mut depth: i64 = -1;
    for comment in comments {
        let comment_depth = comment.depth as i64;
        while depth > comment_depth {
            out.push_str("</li></ul>");
            depth -= 1;
        }
        if depth < comment_depth {
            while depth < comment_depth {
                out.push_str("<ul>");
                depth += 1;
            }
        } else {
            out.push_str("</li>");
        }
        out.push_str("<li>");
        let mut head = Vec::new();
        if !comment.author.is_empty() {
            head.push(format!("<strong>{}</strong>", html_escape(&comment.author)));
        }
        if !comment.age.is_empty() {
            head.push(html_escape(&comment.age));
        }
        if !head.is_empty() {
            out.push_str(&format!("<p>{}</p>", head.join(" \u{b7} ")));
        }
        if !comment.body_html.is_empty() {
            out.push_str(&comment.body_html);
        } else if !comment.body.is_empty() {
            out.push_str(&format!("<p>{}</p>", html_escape(&comment.body)));
        }
    }
    while depth >= 0 {
        out.push_str("</li></ul>");
        depth -= 1;
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

    const ITEM_PAGE: &str = r##"
        <html><body>
        <span class="titleline"><a href="http://example.com/post">Example Post</a>
            <span class="sitestr">example.com</span>
        </span>
        <span class="subline">
            <span class="score">57 points</span>
            <a class="hnuser">alice</a>
            <span class="age" title="2 hours ago"></span>
        </span>
        <div class="fatitem"><div class="toptext"><p>Body of the submission.</p></div></div>
        <table><tbody>
            <tr class="athing comtr">
                <td class="ind" indent="0"></td>
                <td>
                    <div class="comhead"><a class="hnuser">bob</a><span class="age" title="1 hour ago"></span></div>
                    <div class="commtext">Top level remark.<p>Second paragraph, see <a href="item?id=8863">this thread</a>.</p></div>
                </td>
            </tr>
            <tr class="athing comtr">
                <td class="ind" indent="1"></td>
                <td>
                    <div class="comhead"><a class="hnuser">carol</a><span class="age" title="30 minutes ago"></span></div>
                    <div class="commtext">Nested reply.</div>
                </td>
            </tr>
            <tr class="athing comtr">
                <td class="ind" indent="0"></td>
                <td>
                    <div class="comhead"><a class="hnuser">dan</a><span class="age" title="10 minutes ago"></span></div>
                    <div class="commtext">Back to top level.</div>
                </td>
            </tr>
        </tbody></table>
        </body></html>
    "##;

    #[test]
    fn matches_item_pages_with_a_nonempty_id_only() {
        let ext = HackerNewsExtractor::default();
        assert!(ext.matches(&doc("https://news.ycombinator.com/item?id=1", "")));
        assert!(ext.matches(&doc("https://news.ycombinator.com/item?id=1&p=2", "")));
        assert!(ext.matches(&doc("https://news.ycombinator.com/item/?id=9", "")));
        assert!(ext.matches(&doc("https://www.news.ycombinator.com/item?id=9", "")));
        assert!(!ext.matches(&doc("https://news.ycombinator.com/", "")));
        assert!(!ext.matches(&doc("https://news.ycombinator.com/newest", "")));
        assert!(!ext.matches(&doc("https://news.ycombinator.com/item", "")));
        assert!(!ext.matches(&doc("https://news.ycombinator.com/user?id=alice", "")));
        assert!(!ext.matches(&doc("https://news.ycombinator.com.example/item?id=1", "")));
        assert!(!ext.matches(&doc("https://example.com/item?id=1", "")));
    }

    #[test]
    fn extract_collects_submission_and_comments() {
        let ext = HackerNewsExtractor::default();
        match ext.extract(&doc("https://news.ycombinator.com/item?id=1", ITEM_PAGE)) {
            ExtractOutcome::Extracted(result) => {
                assert_eq!(result.title.as_deref(), Some("Example Post"));
                let text = result.text.unwrap();
                for expected in [
                    "example.com",
                    "57 points",
                    "by alice",
                    "Body of the submission.",
                    "Top level remark.",
                    "Nested reply.",
                    "Back to top level.",
                    "bob",
                    "carol",
                ] {
                    assert!(
                        text.contains(expected),
                        "text missing {expected:?}:\n{text}"
                    );
                }
            }
            other => panic!("expected Extracted, got {other:?}"),
        }
    }

    #[test]
    fn extract_indents_replies_by_depth() {
        let ext = HackerNewsExtractor::default();
        match ext.extract(&doc("https://news.ycombinator.com/item?id=1", ITEM_PAGE)) {
            ExtractOutcome::Extracted(result) => {
                let text = result.text.unwrap();
                for line in text.lines() {
                    let trimmed = line.trim_start();
                    let indent = line.len() - trimmed.len();
                    match trimmed {
                        "Top level remark." | "Back to top level." => {
                            assert_eq!(indent, 0, "{trimmed:?} should not be indented")
                        }
                        "Nested reply." => assert!(indent > 0, "{trimmed:?} should be indented"),
                        _ => {}
                    }
                }
            }
            other => panic!("expected Extracted, got {other:?}"),
        }
    }

    #[test]
    fn extract_keeps_paragraph_boundaries() {
        let ext = HackerNewsExtractor::default();
        match ext.extract(&doc("https://news.ycombinator.com/item?id=1", ITEM_PAGE)) {
            ExtractOutcome::Extracted(result) => {
                let text = result.text.unwrap();
                assert!(
                    !text.contains("remark.Second"),
                    "paragraphs ran together:\n{text}"
                );
                assert!(
                    text.contains("Top level remark.\n"),
                    "first paragraph does not end its line:\n{text}"
                );
                assert!(
                    text.contains("Second paragraph, see this thread."),
                    "second paragraph missing:\n{text}"
                );
            }
            other => panic!("expected Extracted, got {other:?}"),
        }
    }

    #[test]
    fn extract_falls_back_without_content() {
        let ext = HackerNewsExtractor::default();
        let result = ext.extract(&doc(
            "https://news.ycombinator.com/item?id=1",
            "<html><body></body></html>",
        ));
        assert!(matches!(result, ExtractOutcome::Fallback(_)));
    }

    #[test]
    fn preview_nests_comments_and_balances_tags() {
        let ext = HackerNewsExtractor::default();
        match ext.preview(&doc("https://news.ycombinator.com/item?id=1", ITEM_PAGE)) {
            PreviewOutcome::Previewed(response) => {
                let html = response.html.unwrap();
                assert_eq!(html.matches("<ul>").count(), html.matches("</ul>").count());
                assert_eq!(html.matches("<li>").count(), html.matches("</li>").count());
                assert_eq!(
                    html.matches("<ul>").count(),
                    2,
                    "two depths in the fixture should produce two levels of nesting"
                );
                assert!(html.contains("Example Post"));
            }
            other => panic!("expected Previewed, got {other:?}"),
        }
    }

    #[test]
    fn preview_resolves_relative_comment_links() {
        let ext = HackerNewsExtractor::default();
        match ext.preview(&doc("https://news.ycombinator.com/item?id=1", ITEM_PAGE)) {
            PreviewOutcome::Previewed(response) => {
                let html = response.html.unwrap();
                assert!(
                    html.contains(r#"href="https://news.ycombinator.com/item?id=8863""#),
                    "relative comment link was not resolved:\n{html}"
                );
            }
            other => panic!("expected Previewed, got {other:?}"),
        }
    }

    #[test]
    fn preview_resolves_relative_title_link() {
        let html_src =
            ITEM_PAGE.replace(r#"href="http://example.com/post""#, r#"href="item?id=1""#);
        let ext = HackerNewsExtractor::default();
        match ext.preview(&doc("https://news.ycombinator.com/item?id=1", &html_src)) {
            PreviewOutcome::Previewed(response) => {
                let html = response.html.unwrap();
                assert!(
                    html.contains(r#"href="https://news.ycombinator.com/item?id=1""#),
                    "relative title link was not resolved:\n{html}"
                );
            }
            other => panic!("expected Previewed, got {other:?}"),
        }
    }

    #[test]
    fn comment_rows_clamps_a_maliciously_large_indent_value() {
        let html_src = r##"
            <html><body>
            <table><tbody>
                <tr class="athing comtr">
                    <td class="ind" indent="999999999"></td>
                    <td>
                        <div class="comhead"><a class="hnuser">eve</a></div>
                        <div class="commtext">Malicious depth.</div>
                    </td>
                </tr>
            </tbody></table>
            </body></html>
        "##;
        let html = Html::parse_document(html_src);
        let comments = comment_rows(&html);
        assert_eq!(comments.len(), 1);
        assert_eq!(
            comments[0].depth, MAX_COMMENT_DEPTH,
            "an attacker-controlled indent must be clamped rather than used as-is"
        );
    }

    #[test]
    fn extract_completes_with_a_maliciously_large_indent_value() {
        let html_src = ITEM_PAGE.replace(r#"indent="0""#, r#"indent="999999999""#);
        let ext = HackerNewsExtractor::default();
        let result = ext.extract(&doc("https://news.ycombinator.com/item?id=1", &html_src));
        assert!(
            matches!(result, ExtractOutcome::Extracted(_)),
            "extraction must complete instead of attempting an allocation \
             proportional to the attacker-chosen indent"
        );
    }
}
