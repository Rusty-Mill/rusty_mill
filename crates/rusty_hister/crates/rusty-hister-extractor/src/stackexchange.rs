//! `StackExchange` — a Rust port of
//! `server/extractor/extractors/stackexchange/stackexchange.go`
//! (capability inventory §4.5.7). Extract and preview: pulls the question
//! and every answer already rendered on a Stack Exchange network question
//! page (Stack Overflow, Server Fault, Super User, Ask Ubuntu,
//! `*.stackexchange.com`, and a handful of others — [`SE_DOMAINS`]).
//!
//! The first extractor in this crate with real `preview()` output (rather
//! than an enrich-only fallback), so it's also the first to exercise
//! [`crate::sanitizer::sanitize_html`] (untrusted third-party HTML) and
//! [`crate::urlutil::rewrite_urls`] (relative links/images resolved to
//! absolute before rendering).

use crate::sanitizer::sanitize_html;
use crate::urlutil::rewrite_urls;
use ammonia::Url;
use chrono::{DateTime, NaiveDateTime, Utc};
use rusty_hister_core::{
    Capabilities, Document, ExtractOutcome, Extractor, ExtractorConfig, HisterError,
    PreviewOutcome, PreviewResponse,
};
use rusty_json::{Number, Value};
use scraper::{CaseSensitivity, ElementRef, Html, Selector};

/// Apex domains for the Stack Exchange network (Go: `seDomains`).
const SE_DOMAINS: &[&str] = &[
    "stackexchange.com",
    "stackoverflow.com",
    "serverfault.com",
    "superuser.com",
    "askubuntu.com",
    "mathoverflow.net",
    "stackapps.com",
];

#[derive(Debug, Default)]
pub struct StackExchangeExtractor {
    config: ExtractorConfig,
}

impl Extractor for StackExchangeExtractor {
    fn name(&self) -> &str {
        "StackExchange"
    }

    fn description(&self) -> &str {
        "Extracts the question and all answers from Stack Exchange network question pages (Stack Overflow, Server Fault, Super User, Ask Ubuntu, *.stackexchange.com, and more)."
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            enrich: false,
            extract: true,
            preview: true,
        }
    }

    fn matches(&self, document: &Document) -> bool {
        let Ok(url) = Url::parse(&document.url) else {
            return false;
        };
        if !is_question_path(url.path()) {
            return false;
        }
        let Some(host) = url.host_str() else {
            return false;
        };
        matches_se_domain(&host.to_ascii_lowercase())
    }

    fn extract(&self, document: &Document) -> ExtractOutcome {
        let Some(html_str) = document.html.as_deref() else {
            return ExtractOutcome::Fallback(HisterError::Extraction(
                "document has no HTML".to_string(),
            ));
        };
        let html = Html::parse_document(html_str);

        let question_sel = selector(".question .js-post-body");
        let post_body_sel = selector(".js-post-body");
        let Some(question) = html
            .select(&question_sel)
            .next()
            .or_else(|| html.select(&post_body_sel).next())
        else {
            return ExtractOutcome::Fallback(HisterError::Extraction(
                "no question body found".to_string(),
            ));
        };

        let mut extracted = document.clone();
        extracted.title = Some(question_title(&html));

        let mut text = element_text(&question).trim().to_string();
        let answer_sel = selector(".answer");
        for answer in html.select(&answer_sel) {
            let body = answer_body_text(&answer);
            if body.is_empty() {
                continue;
            }
            text.push_str("\n\n");
            if is_accepted_answer(&answer) {
                text.push_str("[Accepted Answer]\n");
            }
            text.push_str(&body);
        }
        extracted.text = Some(text.trim().to_string());

        set_metadata(&mut extracted, &html);

        ExtractOutcome::Extracted(extracted)
    }

    fn preview(&self, document: &Document) -> PreviewOutcome {
        let Some(html_str) = document.html.as_deref() else {
            return PreviewOutcome::Fallback(HisterError::Extraction(
                "document has no HTML".to_string(),
            ));
        };
        let mut html = Html::parse_document(html_str);

        // Removes the "copy" button injected into code blocks.
        remove_elements(&mut html, &selector("pre div"));

        if let Ok(base) = Url::parse(&document.url) {
            rewrite_urls(&mut html, &base);
        }

        let question_sel = selector(".question .js-post-body");
        let post_body_sel = selector(".js-post-body");
        let Some(question) = html
            .select(&question_sel)
            .next()
            .or_else(|| html.select(&post_body_sel).next())
        else {
            return PreviewOutcome::Fallback(HisterError::Extraction(
                "no question body found".to_string(),
            ));
        };
        let question_html = question.html();

        let date_sel = selector("[itemprop=dateCreated]");
        let question_post_sel = selector(".question");
        let question_post = html.select(&question_post_sel).next();

        let mut out = String::from("<h2>Question</h2>");
        let q_date = html
            .select(&date_sel)
            .next()
            .and_then(|el| el.value().attr("datetime"))
            .unwrap_or("");
        let q_author = question_post.as_ref().map(post_author).unwrap_or_default();
        let q_score = question_post.as_ref().map(post_score).unwrap_or_default();
        out.push_str(&post_meta_line("asked", q_date, &q_author, &q_score));
        out.push_str(&question_html);

        let answer_sel = selector(".answer");
        let inner_post_body_sel = selector(".js-post-body");
        let prose_sel = selector(".s-prose");
        let mut n = 0;
        for answer in html.select(&answer_sel) {
            let body = answer
                .select(&inner_post_body_sel)
                .next()
                .or_else(|| answer.select(&prose_sel).next());
            let Some(body) = body else { continue };
            n += 1;
            let mut heading = format!("Answer #{n}");
            if is_accepted_answer(&answer) {
                heading.push_str(" (accepted)");
            }
            let a_date = answer
                .select(&date_sel)
                .next()
                .and_then(|el| el.value().attr("datetime"))
                .unwrap_or("");
            out.push_str(&format!("<hr /><h2>{heading}</h2>"));
            out.push_str(&post_meta_line(
                "answered",
                a_date,
                &post_author(&answer),
                &post_score(&answer),
            ));
            out.push_str(&body.html());
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

pub(crate) fn selector(css: &str) -> Selector {
    Selector::parse(css).expect("static selector is valid")
}

pub(crate) fn element_text(element: &ElementRef) -> String {
    element.text().collect::<String>()
}

fn is_question_path(path: &str) -> bool {
    const PREFIX: &str = "/questions/";
    match path.strip_prefix(PREFIX) {
        Some(rest) => rest.as_bytes().first().is_some_and(u8::is_ascii_digit),
        None => false,
    }
}

fn matches_se_domain(host: &str) -> bool {
    SE_DOMAINS.iter().any(|&domain| {
        host == domain
            || host
                .strip_suffix(domain)
                .is_some_and(|prefix| prefix.ends_with('.'))
    })
}

fn question_title(html: &Html) -> String {
    if let Some(el) = html.select(&selector("#question-header h1")).next() {
        let text = element_text(&el).trim().to_string();
        if !text.is_empty() {
            return text;
        }
    }
    html.select(&selector("title"))
        .next()
        .map(|el| element_text(&el).trim().to_string())
        .unwrap_or_default()
}

fn answer_body_text(answer: &ElementRef) -> String {
    let body = answer
        .select(&selector(".js-post-body"))
        .next()
        .or_else(|| answer.select(&selector(".s-prose")).next());
    body.map(|b| element_text(&b).trim().to_string())
        .unwrap_or_default()
}

fn is_accepted_answer(answer: &ElementRef) -> bool {
    answer
        .value()
        .has_class("accepted-answer", CaseSensitivity::CaseSensitive)
}

fn post_author(post: &ElementRef) -> String {
    post.select(&selector(".post-signature .user-details a"))
        .last()
        .map(|el| element_text(&el).trim().to_string())
        .unwrap_or_default()
}

fn post_score(post: &ElementRef) -> String {
    post.select(&selector(".js-vote-count"))
        .next()
        .map(|el| element_text(&el).trim().to_string())
        .unwrap_or_default()
}

/// Mirrors Go's two-attempt `parseSEDate`: a space-separated
/// `"2006-01-02 15:04:05Z07:00"`-shaped timestamp (either a literal `Z`
/// suffix or a numeric offset), falling back to RFC 3339.
fn parse_se_date(dt: &str) -> Option<DateTime<Utc>> {
    if let Some(prefix) = dt.strip_suffix('Z') {
        if let Ok(naive) = NaiveDateTime::parse_from_str(prefix, "%Y-%m-%d %H:%M:%S") {
            return Some(naive.and_utc());
        }
    }
    if let Ok(parsed) = DateTime::parse_from_str(dt, "%Y-%m-%d %H:%M:%S%:z") {
        return Some(parsed.with_timezone(&Utc));
    }
    DateTime::parse_from_rfc3339(dt)
        .ok()
        .map(|parsed| parsed.with_timezone(&Utc))
}

fn post_meta_line(verb: &str, date: &str, author: &str, score: &str) -> String {
    let mut parts = Vec::new();
    if let Some(parsed) = parse_se_date(date) {
        parts.push(format!("{verb} {}", parsed.format("%Y-%m-%d")));
    } else if !date.is_empty() {
        parts.push(format!("{verb} {date}"));
    }
    if !author.is_empty() {
        parts.push(format!("by {author}"));
    }
    if !score.is_empty() {
        parts.push(format!("{score} votes"));
    }
    if parts.is_empty() {
        return String::new();
    }
    format!(
        r#"<p style="font-size:0.9em;color:#888"><em>{}</em></p>"#,
        html_escape(&parts.join(" \u{b7} "))
    )
}

pub(crate) fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn set_metadata(document: &mut Document, html: &Html) {
    let question = html.select(&selector(".question")).next();

    if let Some(author) = question
        .as_ref()
        .and_then(|q| q.select(&selector(".user-details a")).next())
        .map(|el| element_text(&el).trim().to_string())
        .filter(|s| !s.is_empty())
    {
        document
            .metadata
            .insert("author".to_string(), Value::String(author));
    }

    let date_sel = selector("[itemprop=dateCreated]");
    let date_created = question
        .as_ref()
        .and_then(|q| q.select(&date_sel).next())
        .or_else(|| html.select(&date_sel).next());
    if let Some(dt) = date_created
        .and_then(|el| el.value().attr("datetime"))
        .filter(|s| !s.is_empty())
    {
        let published = parse_se_date(dt)
            .map(|parsed| parsed.to_rfc3339())
            .unwrap_or_else(|| dt.to_string());
        document
            .metadata
            .insert("published".to_string(), Value::String(published));
    }

    let tags: Vec<String> = question
        .as_ref()
        .map(|q| {
            q.select(&selector(".post-tag"))
                .filter_map(|el| {
                    let text = element_text(&el).trim().to_string();
                    (!text.is_empty()).then_some(text)
                })
                .collect()
        })
        .unwrap_or_default();
    if !tags.is_empty() {
        document
            .metadata
            .insert("tags".to_string(), Value::String(tags.join(", ")));
    }

    if let Some(score) = question
        .as_ref()
        .and_then(|q| q.select(&selector(".js-vote-count")).next())
        .map(|el| element_text(&el).trim().to_string())
        .filter(|s| !s.is_empty())
    {
        document
            .metadata
            .insert("score".to_string(), Value::String(score));
    }

    let answer_count = html.select(&selector(".answer")).count();
    if answer_count > 0 {
        document.metadata.insert(
            "answers".to_string(),
            Value::Number(Number::from(answer_count)),
        );
    }
}

fn remove_elements(html: &mut Html, selector: &Selector) {
    let ids: Vec<_> = html.select(selector).map(|element| element.id()).collect();
    for id in ids {
        if let Some(mut node_mut) = html.tree.get_mut(id) {
            node_mut.detach();
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

    const QUESTION_PAGE: &str = r#"
        <html><body>
        <div id="question-header"><h1>How do I write a test?</h1></div>
        <div class="question" itemscope>
            <div class="post-signature"><div class="user-details"><a>Alice</a></div></div>
            <div class="js-vote-count">7</div>
            <span itemprop="dateCreated" datetime="2024-03-01T10:00:00Z"></span>
            <a class="post-tag">rust</a><a class="post-tag">testing</a>
            <div class="js-post-body">What is the best way to write a unit test?</div>
        </div>
        <div class="answer accepted-answer">
            <div class="post-signature"><div class="user-details"><a>Bob</a></div></div>
            <div class="js-vote-count">12</div>
            <span itemprop="dateCreated" datetime="2024-03-02T11:30:00Z"></span>
            <div class="js-post-body">Use the built-in test framework.</div>
        </div>
        <div class="answer">
            <div class="js-post-body">Or use a third-party crate.</div>
        </div>
        </body></html>
    "#;

    #[test]
    fn matches_question_urls_on_known_se_domains() {
        let ext = StackExchangeExtractor::default();
        assert!(ext.matches(&doc("https://stackoverflow.com/questions/123/title", "")));
        assert!(ext.matches(&doc("https://rust.stackexchange.com/questions/9/x", "")));
        assert!(ext.matches(&doc("https://superuser.com/questions/1/y", "")));
    }

    #[test]
    fn does_not_match_non_question_paths_or_non_se_domains() {
        let ext = StackExchangeExtractor::default();
        assert!(!ext.matches(&doc("https://stackoverflow.com/users/1/bob", "")));
        assert!(!ext.matches(&doc("https://stackoverflow.com/questions/", "")));
        assert!(!ext.matches(&doc("https://example.com/questions/1/x", "")));
        assert!(!ext.matches(&doc("https://notstackoverflow.com/questions/1/x", "")));
    }

    #[test]
    fn extract_concatenates_question_and_answers_marking_the_accepted_one() {
        let ext = StackExchangeExtractor::default();
        match ext.extract(&doc(
            "https://stackoverflow.com/questions/123/x",
            QUESTION_PAGE,
        )) {
            ExtractOutcome::Extracted(result) => {
                assert_eq!(result.title.as_deref(), Some("How do I write a test?"));
                let text = result.text.unwrap();
                assert!(text.starts_with("What is the best way to write a unit test?"));
                assert!(text.contains("[Accepted Answer]\nUse the built-in test framework."));
                assert!(text.contains("Or use a third-party crate."));
            }
            other => panic!("expected Extracted, got {other:?}"),
        }
    }

    #[test]
    fn extract_populates_metadata() {
        let ext = StackExchangeExtractor::default();
        match ext.extract(&doc(
            "https://stackoverflow.com/questions/123/x",
            QUESTION_PAGE,
        )) {
            ExtractOutcome::Extracted(result) => {
                assert_eq!(
                    result.metadata.get("author"),
                    Some(&Value::String("Alice".to_string()))
                );
                assert_eq!(
                    result.metadata.get("score"),
                    Some(&Value::String("7".to_string()))
                );
                assert_eq!(
                    result.metadata.get("tags"),
                    Some(&Value::String("rust, testing".to_string()))
                );
                assert_eq!(
                    result.metadata.get("published"),
                    Some(&Value::String("2024-03-01T10:00:00+00:00".to_string()))
                );
                assert_eq!(
                    result.metadata.get("answers"),
                    Some(&Value::Number(Number::from(2u64)))
                );
            }
            other => panic!("expected Extracted, got {other:?}"),
        }
    }

    #[test]
    fn extract_falls_back_when_no_question_body_is_present() {
        let ext = StackExchangeExtractor::default();
        let result = ext.extract(&doc(
            "https://stackoverflow.com/questions/123/x",
            "<html><body>nothing here</body></html>",
        ));
        assert!(matches!(result, ExtractOutcome::Fallback(_)));
    }

    #[test]
    fn preview_renders_question_and_numbered_answers_with_accepted_marker() {
        let ext = StackExchangeExtractor::default();
        match ext.preview(&doc(
            "https://stackoverflow.com/questions/123/x",
            QUESTION_PAGE,
        )) {
            PreviewOutcome::Previewed(response) => {
                let html = response.html.unwrap();
                assert!(html.contains("<h2>Question</h2>"));
                assert!(html.contains("What is the best way to write a unit test?"));
                assert!(html.contains("Answer #1 (accepted)"));
                assert!(html.contains("Answer #2"));
                assert!(!html.contains("Answer #2 (accepted)"));
                assert!(html.contains("by Alice"));
                assert!(html.contains("by Bob"));
            }
            other => panic!("expected Previewed, got {other:?}"),
        }
    }

    #[test]
    fn preview_rewrites_relative_urls_to_absolute() {
        let html_src = r#"<html><body><div class="question"><div class="js-post-body"><a href="/other">rel</a><img src="pic.png"></div></div></body></html>"#;
        let ext = StackExchangeExtractor::default();
        match ext.preview(&doc("https://stackoverflow.com/questions/123/x", html_src)) {
            PreviewOutcome::Previewed(response) => {
                let out = response.html.unwrap();
                assert!(out.contains(r#"href="https://stackoverflow.com/other""#));
                assert!(out.contains(r#"src="https://stackoverflow.com/questions/123/pic.png""#));
            }
            other => panic!("expected Previewed, got {other:?}"),
        }
    }

    #[test]
    fn preview_removes_codeblock_copy_buttons_and_sanitizes_scripts() {
        let html_src = r#"<html><body><div class="question"><div class="js-post-body"><pre><div class="copy-btn">Copy</div>code here</pre><script>alert(1)</script></div></div></body></html>"#;
        let ext = StackExchangeExtractor::default();
        match ext.preview(&doc("https://stackoverflow.com/questions/123/x", html_src)) {
            PreviewOutcome::Previewed(response) => {
                let out = response.html.unwrap();
                assert!(!out.contains("copy-btn"));
                assert!(out.contains("code here"));
                assert!(!out.contains("<script"));
            }
            other => panic!("expected Previewed, got {other:?}"),
        }
    }

    #[test]
    fn preview_falls_back_when_no_question_body_is_present() {
        let ext = StackExchangeExtractor::default();
        let result = ext.preview(&doc(
            "https://stackoverflow.com/questions/123/x",
            "<html><body>nothing here</body></html>",
        ));
        assert!(matches!(result, PreviewOutcome::Fallback(_)));
    }

    #[test]
    fn question_title_falls_back_to_the_title_tag() {
        let html_src = r#"<html><head><title>Fallback Title</title></head><body><div class="question"><div class="js-post-body">q</div></div></body></html>"#;
        let ext = StackExchangeExtractor::default();
        match ext.extract(&doc("https://stackoverflow.com/questions/123/x", html_src)) {
            ExtractOutcome::Extracted(result) => {
                assert_eq!(result.title.as_deref(), Some("Fallback Title"))
            }
            other => panic!("expected Extracted, got {other:?}"),
        }
    }
}
