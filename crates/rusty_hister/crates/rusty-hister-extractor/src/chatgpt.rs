//! `ChatGPT` — a Rust port of
//! `server/extractor/extractors/chatgpt/extractor.go` (capability inventory
//! §4.5.18). Extracts one visible ChatGPT conversation (authenticated,
//! public-shared, or custom-GPT) into one searchable document. Works only
//! with the rendered HTML already on the document; never fetches
//! conversation data itself.
//!
//! Conversation turns are found one of two ways, in order: `<article
//! data-testid="conversation-turn-...">` wrappers (the common case), or —
//! when none match — any `[data-message-author-role]` element that isn't
//! nested under another one (public-share and custom-GPT pages that skip
//! the article wrapper). Both a turn's own hiddenness and any ancestor's
//! (`hidden` attribute, or `aria-hidden="true"`) exclude it; nested
//! elements carrying a *different* role than the turn's own, and a fixed
//! list of non-content tags (`script`, `img`, form controls, ...), are
//! stripped from a turn's content wherever they appear inside it.
//!
//! Two things this module can't just delegate to existing helpers:
//! - The content-cleaning rules above apply identically whether producing
//!   plain text ([`conversation_text_of_children`]) or rendered preview
//!   HTML ([`build_turn_fragment`]) — but `scraper`'s `ElementRef` is
//!   read-only, so there's no way to hand back "the same subtree with some
//!   descendants missing" without actually building a new, smaller tree.
//!   [`build_turn_fragment`] does that directly with `ego_tree::NodeMut`
//!   (copying kept `Node` values across, skipping the rest), rather than
//!   serializing HTML by hand.
//! - The plain-text conversion needs list/table-aware formatting
//!   (`<li>` items get a leading `- `, table cells get ` | ` separators)
//!   that [`crate::textutil`] doesn't do — Go's own version doesn't reuse
//!   `textutil.SelectionText` for this either, hand-rolling a similar but
//!   extended writer instead, which this module mirrors rather than
//!   generalizing `textutil` speculatively for a shape only this one
//!   extractor needs. The final whitespace/blank-line normalization pass
//!   *is* identical to `textutil`'s, so [`crate::textutil::normalize_text`]
//!   (promoted to `pub(crate)`) is reused rather than copied a third time.
//!
//! When a conversation URL genuinely has no visible turns, `extract`/
//! `preview` report [`ExtractOutcome::Abort`]/[`PreviewOutcome::Abort`]
//! rather than `Fallback` — matching Go's own `AbortExtraction`, since a
//! matched ChatGPT URL with nothing extractable is a dead end, not a case
//! for the next extractor in the chain to try.

use crate::sanitizer::{sanitize_html, sanitize_text};
use crate::stackexchange::{html_escape, selector};
use crate::textutil::normalize_text;
use crate::urlutil::rewrite_urls;
use ammonia::Url;
use ego_tree::NodeMut;
use percent_encoding::percent_decode_str;
use rusty_hister_core::{
    Capabilities, Document, ExtractOutcome, Extractor, ExtractorConfig, HisterError,
    PreviewOutcome, PreviewResponse,
};
use rusty_json::Value;
use scraper::{ElementRef, Html, Node};

const FORBIDDEN_TAGS: &[&str] = &[
    "script", "style", "noscript", "template", "button", "svg", "img", "picture", "video", "audio",
    "iframe", "embed", "object", "canvas", "source", "form", "input", "textarea", "select",
    "option",
];

const CONVERSATION_BLOCK_ELEMENTS: &[&str] = &[
    "address",
    "article",
    "blockquote",
    "dd",
    "div",
    "dl",
    "dt",
    "figcaption",
    "figure",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "main",
    "p",
    "section",
];

#[derive(Debug, Default)]
pub struct ChatGptExtractor {
    config: ExtractorConfig,
}

impl Extractor for ChatGptExtractor {
    fn name(&self) -> &str {
        "ChatGPT"
    }

    fn description(&self) -> &str {
        "Extracts the visible user and assistant turns from ChatGPT conversations as one searchable document."
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            enrich: false,
            extract: true,
            preview: true,
        }
    }

    /// Accepts authenticated, custom-GPT, and public-shared conversation
    /// URLs on chatgpt.com. Other ChatGPT pages are left to later
    /// extractors.
    fn matches(&self, document: &Document) -> bool {
        let Ok(url) = Url::parse(&document.url) else {
            return false;
        };
        if !url.scheme().eq_ignore_ascii_case("https") {
            return false;
        }
        // Approximates Go's `u.User != nil` (any userinfo present at
        // all) — `url` doesn't distinguish "no userinfo" from "present
        // but empty username", but that distinction has no practical
        // ChatGPT URL to matter for.
        if !url.username().is_empty() || url.password().is_some() {
            return false;
        }
        let host = url.host_str().unwrap_or("").to_ascii_lowercase();
        let host = host.strip_suffix('.').unwrap_or(&host);
        if host != "chatgpt.com" && host != "www.chatgpt.com" {
            return false;
        }

        let path = url.path();
        if !path.starts_with('/') {
            return false;
        }
        let path = path.trim_start_matches('/');
        let path = path.strip_suffix('/').unwrap_or(path);
        let parts: Vec<&str> = path.split('/').collect();
        let id_parts: Vec<&str> = match parts.as_slice() {
            [seg, id] if *seg == "c" || *seg == "share" => vec![id],
            [g, gid, c, id] if *g == "g" && *c == "c" => vec![gid, id],
            _ => return false,
        };
        id_parts.into_iter().all(valid_conversation_path_segment)
    }

    fn extract(&self, document: &Document) -> ExtractOutcome {
        if !self.matches(document) {
            return ExtractOutcome::Fallback(HisterError::Extraction(
                "not a ChatGPT conversation URL".to_string(),
            ));
        }
        let Some(html_str) = document.html.as_deref() else {
            return ExtractOutcome::Fallback(HisterError::Extraction(
                "document has no HTML".to_string(),
            ));
        };
        let html = Html::parse_document(html_str);
        let turns = find_conversation_turns(&html);
        if turns.is_empty() {
            return ExtractOutcome::Abort(HisterError::Extraction(
                "no visible user or assistant turns found".to_string(),
            ));
        }

        let mut extracted = document.clone();
        let title = document_title(document, &html);
        if !title.is_empty() {
            extracted.title = Some(title);
        }
        extracted.text = Some(conversation_text(&turns));
        extracted
            .metadata
            .insert("type".to_string(), Value::String("chatgpt".to_string()));
        ExtractOutcome::Extracted(extracted)
    }

    fn preview(&self, document: &Document) -> PreviewOutcome {
        if !self.matches(document) {
            return PreviewOutcome::Fallback(HisterError::Extraction(
                "not a ChatGPT conversation URL".to_string(),
            ));
        }
        let Some(html_str) = document.html.as_deref() else {
            return PreviewOutcome::Fallback(HisterError::Extraction(
                "document has no HTML".to_string(),
            ));
        };
        let html = Html::parse_document(html_str);
        let turns = find_conversation_turns(&html);
        if turns.is_empty() {
            return PreviewOutcome::Fallback(HisterError::Extraction(
                "no visible user or assistant turns found".to_string(),
            ));
        }
        let Ok(base) = Url::parse(&document.url) else {
            return PreviewOutcome::Fallback(HisterError::Extraction(
                "invalid document URL".to_string(),
            ));
        };

        let mut out = String::new();
        let title = document_title(document, &html);
        if !title.is_empty() {
            out.push_str(&format!("<h1>{}</h1>", html_escape(&title)));
        }
        for (role, role_node) in &turns {
            out.push_str(&format!("<div><h2>{}</h2>", html_escape(role_label(role))));
            let mut fragment = build_turn_fragment(*role_node, role);
            rewrite_urls(&mut fragment, &base);
            out.push_str(&fragment.html());
            out.push_str("</div>");
        }

        let content = sanitize_html(&out);
        if content.trim().is_empty() {
            return PreviewOutcome::Fallback(HisterError::Extraction(
                "no preview content".to_string(),
            ));
        }
        PreviewOutcome::Previewed(PreviewResponse {
            html: Some(content),
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

fn valid_conversation_path_segment(raw_id: &str) -> bool {
    if raw_id.is_empty() {
        return false;
    }
    let Ok(decoded) = percent_decode_str(raw_id).decode_utf8() else {
        return false;
    };
    if decoded == "." || decoded == ".." || decoded.contains('/') {
        return false;
    }
    !decoded.chars().any(|c| c.is_whitespace() || c.is_control())
}

fn normalize_role(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "user" => Some("user"),
        "assistant" => Some("assistant"),
        _ => None,
    }
}

fn role_label(role: &str) -> &'static str {
    if role == "assistant" {
        "Assistant"
    } else {
        "User"
    }
}

fn has_hidden_marker(element: &scraper::node::Element) -> bool {
    if element.attr("hidden").is_some() {
        return true;
    }
    element
        .attr("aria-hidden")
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("true"))
}

/// Checks the element itself and every ancestor up to the document root.
fn is_hidden_element(element: &ElementRef) -> bool {
    has_hidden_marker(element.value())
        || element
            .ancestors()
            .any(|node| node.value().as_element().is_some_and(has_hidden_marker))
}

/// Checks every ancestor up to the document root for the mere *presence*
/// of `data-message-author-role`, regardless of whether its value is a
/// recognized role — matching Go's `ParentsFiltered` selector match.
fn has_role_ancestor(element: &ElementRef) -> bool {
    element.ancestors().any(|node| {
        node.value()
            .as_element()
            .is_some_and(|el| el.attr("data-message-author-role").is_some())
    })
}

fn find_role_node<'a>(article: &ElementRef<'a>) -> Option<(ElementRef<'a>, &'static str)> {
    if let Some(raw_role) = article.value().attr("data-message-author-role") {
        return normalize_role(raw_role).map(|role| (*article, role));
    }
    for candidate in article.select(&selector("[data-message-author-role]")) {
        let Some(raw) = candidate.value().attr("data-message-author-role") else {
            continue;
        };
        let Some(role) = normalize_role(raw) else {
            continue;
        };
        if has_role_ancestor(&candidate) {
            continue;
        }
        return Some((candidate, role));
    }
    None
}

fn find_article_conversation_turns(html: &Html) -> Vec<(&'static str, ElementRef<'_>)> {
    let mut turns = Vec::new();
    for article in html.select(&selector(r#"article[data-testid^="conversation-turn-"]"#)) {
        if is_hidden_element(&article) {
            continue;
        }
        let Some((role_node, role)) = find_role_node(&article) else {
            continue;
        };
        if is_hidden_element(&role_node) {
            continue;
        }
        if conversation_text_of_children(role_node, role).is_empty() {
            continue;
        }
        turns.push((role, role_node));
    }
    turns
}

fn find_role_conversation_turns(html: &Html) -> Vec<(&'static str, ElementRef<'_>)> {
    let mut turns = Vec::new();
    for role_node in html.select(&selector("[data-message-author-role]")) {
        let Some(raw) = role_node.value().attr("data-message-author-role") else {
            continue;
        };
        let Some(role) = normalize_role(raw) else {
            continue;
        };
        if has_role_ancestor(&role_node) || is_hidden_element(&role_node) {
            continue;
        }
        if conversation_text_of_children(role_node, role).is_empty() {
            continue;
        }
        turns.push((role, role_node));
    }
    turns
}

fn find_conversation_turns(html: &Html) -> Vec<(&'static str, ElementRef<'_>)> {
    let article_turns = find_article_conversation_turns(html);
    if !article_turns.is_empty() {
        return article_turns;
    }
    find_role_conversation_turns(html)
}

fn document_title(document: &Document, html: &Html) -> String {
    if let Some(title) = document.title.as_deref() {
        let sanitized = sanitize_text(title);
        if !sanitized.is_empty() {
            return sanitized;
        }
    }
    html.select(&selector("title"))
        .next()
        .map(|el| sanitize_text(&el.text().collect::<String>()))
        .unwrap_or_default()
}

fn conversation_text(turns: &[(&'static str, ElementRef)]) -> String {
    let mut out = String::new();
    for (role, role_node) in turns {
        let text = conversation_text_of_children(*role_node, role);
        if text.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(role_label(role));
        out.push_str(":\n");
        out.push_str(&text);
    }
    out.trim().to_string()
}

/// True if `element` itself carries a *different* role than `role` —
/// meaning it (and its subtree) should be dropped wherever it appears
/// inside a turn's content, whether producing text or preview HTML.
fn is_foreign_role_node(element: &scraper::node::Element, role: &str) -> bool {
    element
        .attr("data-message-author-role")
        .is_some_and(|node_role| normalize_role(node_role) != Some(role))
}

fn conversation_text_of_children(role_node: ElementRef, role: &str) -> String {
    let mut writer = ConversationTextWriter::default();
    for child in role_node.children() {
        writer.write_node(child, role, false);
    }
    normalize_text(&writer.buf)
}

#[derive(Default)]
struct ConversationTextWriter {
    buf: String,
    row_cells: Vec<usize>,
}

impl ConversationTextWriter {
    fn write_break(&mut self) {
        if !self.buf.is_empty() && !self.buf.ends_with('\n') {
            self.buf.push('\n');
        }
    }

    fn write_children(&mut self, node: ego_tree::NodeRef<Node>, role: &str, hidden_ancestor: bool) {
        for child in node.children() {
            self.write_node(child, role, hidden_ancestor);
        }
    }

    fn write_node(&mut self, node: ego_tree::NodeRef<Node>, role: &str, hidden_ancestor: bool) {
        match node.value() {
            Node::Text(text) => self.buf.push_str(text),
            Node::Element(element) => {
                if is_foreign_role_node(element, role) {
                    return;
                }
                let name = element.name();
                if FORBIDDEN_TAGS.contains(&name) {
                    return;
                }
                let is_hidden = hidden_ancestor || has_hidden_marker(element);
                if is_hidden {
                    return;
                }
                match name {
                    "br" => self.write_break(),
                    "pre" => {
                        self.write_break();
                        self.write_children(node, role, is_hidden);
                        self.write_break();
                    }
                    "li" => {
                        self.write_break();
                        self.buf.push_str("- ");
                        self.write_children(node, role, is_hidden);
                        self.write_break();
                    }
                    "ul" | "ol" | "table" => {
                        self.write_break();
                        self.write_children(node, role, is_hidden);
                        self.write_break();
                    }
                    "tr" => {
                        self.write_break();
                        self.row_cells.push(0);
                        self.write_children(node, role, is_hidden);
                        self.row_cells.pop();
                        self.write_break();
                    }
                    "td" | "th" => {
                        if let Some(last) = self.row_cells.last_mut() {
                            if *last > 0 {
                                self.buf.push_str(" | ");
                            }
                            *last += 1;
                        }
                        self.write_children(node, role, is_hidden);
                    }
                    _ => {
                        let is_block = CONVERSATION_BLOCK_ELEMENTS.contains(&name);
                        if is_block {
                            self.write_break();
                        }
                        self.write_children(node, role, is_hidden);
                        if is_block {
                            self.write_break();
                        }
                    }
                }
            }
            _ => self.write_children(node, role, hidden_ancestor),
        }
    }
}

/// Builds a standalone HTML fragment containing `role_node`'s children,
/// with foreign-role nodes, forbidden tags, and hidden elements dropped —
/// the preview-HTML counterpart to [`conversation_text_of_children`].
fn build_turn_fragment(role_node: ElementRef, role: &str) -> Html {
    let mut html = Html::new_fragment();
    let mut root_mut = html.tree.root_mut();
    for child in role_node.children() {
        copy_filtered(&mut root_mut, child, role, false);
    }
    html
}

fn copy_filtered(
    parent: &mut NodeMut<Node>,
    source: ego_tree::NodeRef<Node>,
    role: &str,
    hidden_ancestor: bool,
) {
    match source.value() {
        Node::Text(text) => {
            parent.append(Node::Text(text.clone()));
        }
        Node::Element(element) => {
            if is_foreign_role_node(element, role) {
                return;
            }
            if FORBIDDEN_TAGS.contains(&element.name()) {
                return;
            }
            let is_hidden = hidden_ancestor || has_hidden_marker(element);
            if is_hidden {
                return;
            }
            let mut child_mut = parent.append(Node::Element(element.clone()));
            for grandchild in source.children() {
                copy_filtered(&mut child_mut, grandchild, role, is_hidden);
            }
        }
        _ => {
            for child in source.children() {
                copy_filtered(parent, child, role, hidden_ancestor);
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
    fn matches_authenticated_shared_and_custom_gpt_conversation_urls() {
        let ext = ChatGptExtractor::default();
        let cases: &[(&str, bool)] = &[
            ("https://chatgpt.com/c/conv-123", true),
            ("https://chatgpt.com/share/conv-123", true),
            ("https://chatgpt.com/g/gpt-123/c/conv-123", true),
            ("https://www.chatgpt.com/g/gpt-123/c/conv-123", true),
            ("https://chatgpt.com/g/gpt-123/c/conv-123/", true),
            ("https://chatgpt.com/c/conv-123/", true),
            ("https://chatgpt.com/", false),
            ("https://chatgpt.com/c/", false),
            ("https://chatgpt.com/c/conv-123/extra", false),
            ("https://chatgpt.com/g/gpt-123", false),
            ("https://chatgpt.com/g//c/conv-123", false),
            ("https://chatgpt.com/g/gpt-123/c/", false),
            ("https://chatgpt.com/g/gpt-123/c/conv-123/extra", false),
            ("https://chatgpt.com/g/gpt%2F123/c/conv-123", false),
            ("https://chatgpt.com/g/gpt-123/c/conv%2F123", false),
            ("https://chatgpt.com/g/./c/conv-123", false),
            ("https://chatgpt.com/g/gpt-123/c/..", false),
            ("https://chatgpt.com/g/%2E/c/conv-123", false),
            ("https://chatgpt.com/g/gpt-123/c/%2E%2E", false),
            ("https://chatgpt.com/g/gpt%20123/c/conv-123", false),
            ("https://chatgpt.com/g/gpt-123/c/conv%00-123", false),
            ("https://notchatgpt.com/c/conv-123", false),
            ("http://chatgpt.com/c/conv-123", false),
        ];
        for (url, want) in cases {
            assert_eq!(ext.matches(&doc(url, "")), *want, "matches({url:?})");
        }
    }

    const CONVERSATION_PAGE: &str = r#"
        <html><head><title>My Conversation</title></head><body>
        <article data-testid="conversation-turn-1">
            <div data-message-author-role="user"><p>What is Rust?</p></div>
        </article>
        <article data-testid="conversation-turn-2">
            <div data-message-author-role="assistant">
                <p>Rust is a systems language.</p>
                <ul><li>Fast</li><li>Safe</li></ul>
            </div>
        </article>
        </body></html>
    "#;

    #[test]
    fn extract_collects_turns_as_one_document_with_role_labels() {
        let ext = ChatGptExtractor::default();
        match ext.extract(&doc("https://chatgpt.com/c/conv-123", CONVERSATION_PAGE)) {
            ExtractOutcome::Extracted(result) => {
                assert_eq!(result.title.as_deref(), Some("My Conversation"));
                assert_eq!(
                    result.metadata.get("type"),
                    Some(&Value::String("chatgpt".to_string()))
                );
                let text = result.text.unwrap();
                assert_eq!(text, "User:\nWhat is Rust?\n\nAssistant:\nRust is a systems language.\n\n- Fast\n- Safe");
            }
            other => panic!("expected Extracted, got {other:?}"),
        }
    }

    #[test]
    fn extract_falls_back_for_a_non_conversation_url() {
        let ext = ChatGptExtractor::default();
        let result = ext.extract(&doc("https://chatgpt.com/", CONVERSATION_PAGE));
        assert!(matches!(result, ExtractOutcome::Fallback(_)));
    }

    #[test]
    fn extract_aborts_without_visible_turns() {
        let ext = ChatGptExtractor::default();
        let result = ext.extract(&doc(
            "https://chatgpt.com/c/empty-123",
            "<html><body>nothing here</body></html>",
        ));
        assert!(
            matches!(result, ExtractOutcome::Abort(_)),
            "expected Abort, got {result:?}"
        );
    }

    const VISIBILITY_PAGE: &str = r#"
        <html><body>
        <div hidden><article data-testid="conversation-turn-a"><div data-message-author-role="user">Hidden ancestor</div></article></div>
        <article data-testid="conversation-turn-b" aria-hidden="true"><div data-message-author-role="assistant">Hidden root</div></article>
        <article data-testid="conversation-turn-c"><div data-message-author-role="assistant" hidden>Hidden role</div></article>
        <article data-testid="conversation-turn-d">
            <div data-message-author-role="system"><div data-message-author-role="user">Misnested user</div></div>
        </article>
        <article data-testid="conversation-turn-e">
            <div data-message-author-role="assistant">
                <p>Visible answer.</p>
                <div data-message-author-role="system">Nested system content.</div>
                <div data-message-author-role="assistant"><p>Nested duplicate answer.</p></div>
            </div>
        </article>
        </body></html>
    "#;

    #[test]
    fn skips_hidden_and_misnested_content_but_keeps_same_role_nesting() {
        let ext = ChatGptExtractor::default();
        match ext.extract(&doc(
            "https://chatgpt.com/c/visibility-123",
            VISIBILITY_PAGE,
        )) {
            ExtractOutcome::Extracted(result) => {
                let text = result.text.unwrap();
                assert_eq!(
                    text,
                    "Assistant:\nVisible answer.\n\nNested duplicate answer."
                );
                for unwanted in [
                    "Hidden ancestor",
                    "Hidden root",
                    "Hidden role",
                    "Misnested user",
                    "Nested system content",
                ] {
                    assert!(
                        !text.contains(unwanted),
                        "text should not contain {unwanted:?}:\n{text}"
                    );
                }
            }
            other => panic!("expected Extracted, got {other:?}"),
        }

        match ext.preview(&doc(
            "https://chatgpt.com/c/visibility-123",
            VISIBILITY_PAGE,
        )) {
            PreviewOutcome::Previewed(response) => {
                let html = response.html.unwrap();
                for unwanted in [
                    "Hidden ancestor",
                    "Hidden root",
                    "Hidden role",
                    "Misnested user",
                    "Nested system content",
                ] {
                    assert!(
                        !html.contains(unwanted),
                        "preview should not contain {unwanted:?}:\n{html}"
                    );
                }
                assert!(html.contains("Visible answer."));
                assert!(html.contains("Nested duplicate answer."));
            }
            other => panic!("expected Previewed, got {other:?}"),
        }
    }

    const PUBLIC_SHARE_PAGE: &str = r#"
        <html><head><title>Shared chat</title></head><body>
        <div data-message-author-role="user"><p>Hello there.</p></div>
        <div data-message-author-role="assistant"><p>General Kenobi.</p></div>
        </body></html>
    "#;

    #[test]
    fn extracts_role_nodes_without_article_wrappers() {
        let ext = ChatGptExtractor::default();
        match ext.extract(&doc(
            "https://chatgpt.com/share/sleep-123",
            PUBLIC_SHARE_PAGE,
        )) {
            ExtractOutcome::Extracted(result) => {
                let text = result.text.unwrap();
                assert_eq!(text, "User:\nHello there.\n\nAssistant:\nGeneral Kenobi.");
            }
            other => panic!("expected Extracted, got {other:?}"),
        }
    }

    #[test]
    fn preview_rewrites_relative_links_and_removes_forbidden_elements() {
        let html = r#"<html><body>
            <article data-testid="conversation-turn-1">
                <div data-message-author-role="assistant">
                    <p>See <a href="/docs">the docs</a>.</p>
                    <img src="pic.png">
                    <script>alert(1)</script>
                </div>
            </article>
        </body></html>"#;
        let ext = ChatGptExtractor::default();
        match ext.preview(&doc("https://chatgpt.com/c/conv-123", html)) {
            PreviewOutcome::Previewed(response) => {
                let out = response.html.unwrap();
                assert!(out.contains(r#"href="https://chatgpt.com/docs""#));
                assert!(!out.contains("<img"));
                assert!(!out.contains("<script"));
                assert!(out.contains("the docs"));
            }
            other => panic!("expected Previewed, got {other:?}"),
        }
    }
}
