//! `Bluesky` — a Rust port of
//! `server/extractor/extractors/bluesky/extractor.go` (capability
//! inventory §4.5.14). Decomposes a Bluesky profile/feed/thread page
//! into one [`Document`] per visible post, reusing the
//! `extra_documents`/`skip_indexing` mechanism [`crate::MastodonExtractor`]
//! established.
//!
//! Bluesky pages can carry the same post data in up to three independent
//! forms, and — like Go — this port tries all three, in priority order,
//! merging results by canonical post URL rather than picking just one:
//! - a `schema.org` (`DiscussionForumPosting`/`SocialMediaPosting`/
//!   `Comment`) JSON-LD block, walked recursively through `@graph`/
//!   `mainEntity`/`hasPart`/`itemListElement`/`item`/`comment` wrapper
//!   keys ([`schema_post_nodes`]/[`collect_schema_posts`]) — Bluesky
//!   embeds this on most rendered pages;
//! - the already-rendered post DOM ([`rendered_post_selections`]), found
//!   via a handful of known selectors (`[data-testid^="feedItem-by-"]`,
//!   `article`, `[role="article"]`, ...) plus a fallback heuristic that
//!   walks up from any anchor linking to a post URL
//!   ([`semantic_post_container`]);
//! - as a last resort, when *neither* of the above finds anything, the
//!   page's own Open Graph/Twitter Card meta tags become a single
//!   [`page_post_document`] for the page's own URL — the only path that
//!   fires when the page is itself a single post with no JSON-LD and an
//!   unrecognized rendered markup.
//!
//! [`canonical_post_url`] is the load-bearing helper nearly everything
//! else calls through: it validates a URL is a Bluesky post permalink
//! shape (`/profile/<actor>/post/<id>` on a known Bluesky host) and
//! always returns the same canonical `https://bsky.app/...` form
//! regardless of which host/subdomain the input used — the same key
//! [`append_post`]/[`merge_post`] dedupe candidates by, so the same post
//! discovered via JSON-LD *and* the rendered DOM merges into one
//! document (preferring the rendered DOM's own text/HTML when present,
//! matching Go's own field-by-field merge precedence) instead of
//! appearing twice.

use crate::sanitizer::{sanitize_html, sanitize_text};
use crate::stackexchange::{element_text, html_escape, selector};
use crate::urlutil::{resolve_url, rewrite_urls};
use ammonia::Url;
use rusty_hister_core::{
    Capabilities, Document, ExtractOutcome, Extractor, ExtractorConfig, HisterError, Metadata,
    PreviewOutcome, PreviewResponse,
};
use rusty_json::Value;
use scraper::{ElementRef, Html};
use std::collections::{HashMap, HashSet};

const POST_TYPE: &str = "bluesky";
const BLUESKY_HOSTS: [&str; 3] = ["bsky.app", "www.bsky.app", "embed.bsky.app"];

#[derive(Debug, Default)]
pub struct BlueskyExtractor {
    config: ExtractorConfig,
}

impl Extractor for BlueskyExtractor {
    fn name(&self) -> &str {
        "Bluesky"
    }

    fn description(&self) -> &str {
        "Extracts Bluesky posts as individual documents from profiles, feeds, and post pages."
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            enrich: false,
            extract: true,
            preview: true,
        }
    }

    fn matches(&self, document: &Document) -> bool {
        if metadata_type(document) == POST_TYPE {
            return true;
        }
        let Ok(url) = Url::parse(&document.url) else {
            return false;
        };
        is_bluesky_host(url.host_str().unwrap_or(""))
    }

    fn extract(&self, document: &Document) -> ExtractOutcome {
        if metadata_type(document) == POST_TYPE {
            return ExtractOutcome::Extracted(document.clone());
        }

        let mut extracted = document.clone();
        extracted.skip_indexing = true;

        let html_str = document.html.as_deref().unwrap_or("");
        let html = Html::parse_document(html_str);
        let Ok(base) = Url::parse(&document.url) else {
            return ExtractOutcome::Fallback(HisterError::Extraction(
                "invalid document URL".to_string(),
            ));
        };

        let mut posts: Vec<Document> = Vec::new();
        let mut by_url: HashMap<String, usize> = HashMap::new();

        for node in schema_post_nodes(&html) {
            if let Some(post) = schema_post_document(&node, &base) {
                append_post(&mut posts, &mut by_url, post);
            }
        }
        for post_el in rendered_post_selections(&html) {
            if let Some(post) = rendered_post_document(&post_el, &base) {
                append_post(&mut posts, &mut by_url, post);
            }
        }
        if posts.is_empty() {
            if let Some(post) = page_post_document(&html, &base) {
                append_post(&mut posts, &mut by_url, post);
            }
        }

        extracted.extra_documents.extend(posts);
        ExtractOutcome::Extracted(extracted)
    }

    fn preview(&self, document: &Document) -> PreviewOutcome {
        let mut out = String::new();
        if let Some(title) = document
            .title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            out.push_str(&format!("<h2>{}</h2>", html_escape(title)));
        }
        let html_trimmed = document.html.as_deref().unwrap_or("").trim();
        let text_trimmed = document.text.as_deref().unwrap_or("").trim();
        if !html_trimmed.is_empty() {
            out.push_str(html_trimmed);
        } else if !text_trimmed.is_empty() {
            out.push_str(&paragraph_html(text_trimmed));
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

fn metadata_type(document: &Document) -> &str {
    document
        .metadata
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
}

fn is_bluesky_host(host: &str) -> bool {
    BLUESKY_HOSTS.iter().any(|h| h.eq_ignore_ascii_case(host))
}

fn append_post(
    posts: &mut Vec<Document>,
    by_url: &mut HashMap<String, usize>,
    candidate: Document,
) {
    if let Some(&index) = by_url.get(&candidate.url) {
        merge_post(&mut posts[index], candidate);
        return;
    }
    by_url.insert(candidate.url.clone(), posts.len());
    posts.push(candidate);
}

fn merge_post(existing: &mut Document, candidate: Document) {
    if candidate.text.as_deref().is_some_and(|t| !t.is_empty()) {
        existing.text = candidate.text;
    }
    if candidate.html.as_deref().is_some_and(|h| !h.is_empty()) {
        existing.html = candidate.html;
    }
    if existing.title.as_deref() == Some("Bluesky post") {
        if let Some(t) = candidate.title.filter(|t| !t.is_empty()) {
            existing.title = Some(t);
        }
    }
    for (key, value) in candidate.metadata.iter() {
        let should_set = match existing.metadata.get(key) {
            None => true,
            Some(v) => v.as_str() == Some(""),
        };
        if should_set {
            existing.metadata.insert(key.clone(), value.clone());
        }
    }
}

// --- schema.org JSON-LD extraction ------------------------------------

fn schema_post_nodes(html: &Html) -> Vec<Value> {
    let mut posts = Vec::new();
    for script in html.select(&selector("script")) {
        let type_attr = script.value().attr("type").unwrap_or("").trim();
        if !type_attr.eq_ignore_ascii_case("application/ld+json") {
            continue;
        }
        let Ok(value) = Value::parse(&element_text(&script)) else {
            continue;
        };
        collect_schema_posts(&value, &mut posts, 0);
    }
    posts
}

fn collect_schema_posts(value: &Value, posts: &mut Vec<Value>, depth: u32) {
    if depth > 20 {
        return;
    }
    if let Some(array) = value.as_array() {
        for item in array {
            collect_schema_posts(item, posts, depth + 1);
        }
        return;
    }
    let Some(node) = value.as_object() else {
        return;
    };
    if is_schema_post(value) {
        posts.push(value.clone());
        if let Some(comment) = node.get("comment") {
            collect_schema_posts(comment, posts, depth + 1);
        }
        return;
    }
    for key in [
        "@graph",
        "mainEntity",
        "hasPart",
        "itemListElement",
        "item",
        "comment",
    ] {
        if let Some(child) = node.get(key) {
            collect_schema_posts(child, posts, depth + 1);
        }
    }
}

fn is_schema_post(node: &Value) -> bool {
    let Some(object) = node.as_object() else {
        return false;
    };
    let Some(type_value) = object.get("@type") else {
        return false;
    };
    for raw in schema_types(type_value) {
        let name = schema_type_name(&raw);
        if name == "DiscussionForumPosting" || name == "SocialMediaPosting" || name == "Comment" {
            return true;
        }
    }
    false
}

fn schema_types(value: &Value) -> Vec<String> {
    if let Some(s) = value.as_str() {
        return vec![s.to_string()];
    }
    if let Some(array) = value.as_array() {
        return array
            .iter()
            .filter_map(|item| item.as_str().map(|s| s.to_string()))
            .collect();
    }
    Vec::new()
}

fn schema_type_name(value: &str) -> String {
    let value = value.trim();
    match value.rfind(['/', '#']) {
        Some(index) => value[index + 1..].to_string(),
        None => value.to_string(),
    }
}

fn schema_post_document(node: &Value, base: &Url) -> Option<Document> {
    let object = node.as_object()?;
    let raw_url = first_schema_string(node, &["url", "mainEntityOfPage"]);
    let (post_url, _, _) = canonical_post_url(&raw_url, base)?;

    let text = sanitize_text(&first_schema_string(
        node,
        &["text", "articleBody", "description"],
    ));
    let images = schema_images(node, base);
    if text.is_empty() && images.is_empty() {
        return None;
    }

    let (name, mut handle, did) = schema_author(object.get("author"));
    if handle.is_empty() {
        if let Some((_, actor, _)) = canonical_post_url(&post_url, base) {
            if !actor.to_ascii_lowercase().starts_with("did:") {
                handle = actor;
            }
        }
    }
    let author = format_author(&name, &handle);
    let mut metadata = post_metadata(
        &author,
        &handle,
        &first_schema_string(node, &["datePublished", "dateCreated"]),
    );
    if !did.is_empty() {
        metadata.insert("did".to_string(), did.into());
    }
    let identifier = first_schema_string(node, &["identifier"]);
    if identifier.starts_with("at://") {
        metadata.insert("at_uri".to_string(), sanitize_text(&identifier).into());
    }
    let language = first_schema_string(node, &["inLanguage"]);
    if !language.is_empty() {
        metadata.insert("language".to_string(), sanitize_text(&language).into());
    }
    if let Some(first_image) = images.first() {
        metadata.insert("image".to_string(), first_image.clone().into());
    }

    let mut html = schema_post_html(&text, &images);
    let quoted = object.get("isBasedOn").and_then(|v| schema_url(v, base));
    if let Some(quoted) = quoted {
        metadata.insert("quoted_post".to_string(), quoted.clone().into());
        html.push_str(&format!(
            r#"<blockquote><p><a href="{}">Quoted Bluesky post</a></p></blockquote>"#,
            html_escape(&quoted)
        ));
    }
    if let Some(shared) = object.get("sharedContent") {
        let (card_html, card_url) = schema_card_html(shared, base);
        if !card_html.is_empty() {
            html.push_str(&card_html);
            metadata.insert("external_url".to_string(), card_url.into());
        }
    }

    let mut doc = Document::new(post_url);
    doc.title = Some(post_title(&author));
    doc.text = Some(text);
    doc.html = Some(html);
    doc.metadata = metadata;
    Some(doc)
}

/// Walks `node[key]` for each `key` in order, returning the first
/// non-empty string found — a plain string value directly, or (for an
/// object value) its own `url`/`@id` field. Matches Go's
/// `firstSchemaString`, including the one-level object-unwrap.
fn first_schema_string(node: &Value, keys: &[&str]) -> String {
    let Some(object) = node.as_object() else {
        return String::new();
    };
    for key in keys {
        let Some(value) = object.get(*key) else {
            continue;
        };
        if let Some(s) = value.as_str() {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
            continue;
        }
        if value.as_object().is_some() {
            let nested = first_schema_string(value, &["url", "@id"]);
            if !nested.is_empty() {
                return nested;
            }
        }
    }
    String::new()
}

fn schema_author(value: Option<&Value>) -> (String, String, String) {
    let author = value.and_then(|v| {
        if v.as_object().is_some() {
            Some(v)
        } else {
            v.as_array()?.iter().find(|item| item.as_object().is_some())
        }
    });
    let Some(author) = author else {
        return (String::new(), String::new(), String::new());
    };
    let name = sanitize_text(&first_schema_string(author, &["name"]));
    let mut handle = sanitize_text(&first_schema_string(author, &["alternateName"]));
    handle = handle.strip_prefix('@').unwrap_or(&handle).to_string();
    if !valid_handle(&handle) {
        handle.clear();
    }
    let mut did = sanitize_text(&first_schema_string(author, &["identifier"]));
    if !did.to_ascii_lowercase().starts_with("did:") {
        did.clear();
    }
    (name, handle, did)
}

fn schema_images(node: &Value, base: &Url) -> Vec<String> {
    let mut images = Vec::new();
    let mut seen = HashSet::new();
    let Some(object) = node.as_object() else {
        return images;
    };
    for key in ["image", "thumbnailUrl"] {
        if let Some(value) = object.get(key) {
            collect_schema_images(value, base, &mut images, &mut seen);
        }
    }
    images
}

fn collect_schema_images(
    value: &Value,
    base: &Url,
    images: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    if let Some(s) = value.as_str() {
        append_image(images, seen, s, base);
        return;
    }
    if let Some(array) = value.as_array() {
        for item in array {
            collect_schema_images(item, base, images, seen);
        }
        return;
    }
    if let Some(object) = value.as_object() {
        let raw = first_schema_string(value, &["contentUrl", "url", "thumbnailUrl"]);
        let _ = object; // node already consulted via first_schema_string
        append_image(images, seen, &raw, base);
    }
}

fn append_image(images: &mut Vec<String>, seen: &mut HashSet<String>, raw: &str, base: &Url) {
    let image = safe_http_url(raw, base);
    if image.is_empty() || seen.contains(&image) {
        return;
    }
    seen.insert(image.clone());
    images.push(image);
}

fn schema_post_html(text: &str, images: &[String]) -> String {
    let mut out = String::new();
    if !text.is_empty() {
        out.push_str(&paragraph_html(text));
    }
    if !images.is_empty() {
        out.push_str("<figure>");
        for image in images {
            out.push_str(&format!(r#"<img src="{}" alt="">"#, html_escape(image)));
        }
        out.push_str("</figure>");
    }
    out
}

fn schema_card_html(value: &Value, base: &Url) -> (String, String) {
    let Some(_card) = value.as_object() else {
        return (String::new(), String::new());
    };
    let card_url = safe_http_url(&first_schema_string(value, &["url"]), base);
    if card_url.is_empty() {
        return (String::new(), String::new());
    }
    let mut title = sanitize_text(&first_schema_string(value, &["headline", "name"]));
    let description = sanitize_text(&first_schema_string(value, &["description"]));
    if title.is_empty() {
        title = card_url.clone();
    }

    let mut out = String::from("<aside><p>");
    out.push_str(&format!(
        r#"<a href="{}">{}</a>"#,
        html_escape(&card_url),
        html_escape(&title)
    ));
    out.push_str("</p>");
    if !description.is_empty() {
        out.push_str(&paragraph_html(&description));
    }
    let images = schema_images(value, base);
    if let Some(first_image) = images.first() {
        out.push_str(&format!(
            r#"<img src="{}" alt="">"#,
            html_escape(first_image)
        ));
    }
    out.push_str("</aside>");
    (out, card_url)
}

fn schema_url(value: &Value, base: &Url) -> Option<String> {
    let raw = if let Some(s) = value.as_str() {
        s.to_string()
    } else if value.as_object().is_some() {
        first_schema_string(value, &["url", "@id"])
    } else {
        String::new()
    };
    canonical_post_url(&raw, base).map(|(url, _, _)| url)
}

// --- rendered-DOM extraction -------------------------------------------

fn rendered_post_selections<'a>(html: &'a Html) -> Vec<ElementRef<'a>> {
    let mut posts = Vec::new();
    let mut seen = HashSet::new();

    let container_sel = selector(
        r#"[data-testid^="feedItem-by-"], [data-testid^="postThreadItem-by-"], article, [role="article"]"#,
    );
    for post in html.select(&container_sel) {
        if seen.insert(post.id()) {
            posts.push(post);
        }
    }

    let anchor_sel = selector(r#"a[href*="/profile/"][href*="/post/"]"#);
    let time_sel = selector("time");
    for anchor in html.select(&anchor_sel) {
        let has_time = anchor.select(&time_sel).next().is_some();
        let has_tooltip = anchor
            .value()
            .attr("data-tooltip")
            .is_some_and(|v| !v.is_empty());
        let has_aria_label = anchor
            .value()
            .attr("aria-label")
            .is_some_and(|v| !v.is_empty());
        if !has_time && !has_tooltip && !has_aria_label {
            continue;
        }
        if let Some(container) = semantic_post_container(anchor) {
            if seen.insert(container.id()) {
                posts.push(container);
            }
        }
    }
    posts
}

fn semantic_post_container<'a>(anchor: ElementRef<'a>) -> Option<ElementRef<'a>> {
    let container_sel = selector(
        r#"article, [role="article"], [data-testid^="feedItem-by-"], [data-testid^="postThreadItem-by-"]"#,
    );
    let post_text_sel = selector(r#"[data-testid="postText"]"#);
    let mut current_node = *anchor;
    for _ in 0..14 {
        current_node = current_node.parent()?;
        let current = ElementRef::wrap(current_node)?;
        if container_sel.matches(&current) || current.select(&post_text_sel).next().is_some() {
            return Some(current);
        }
    }
    None
}

fn rendered_post_document(post: &ElementRef, base: &Url) -> Option<Document> {
    let (post_url, url_actor) = rendered_post_url(post, base)?;
    let (name, author_handle) = rendered_author(post, &url_actor);
    let handle = if !author_handle.is_empty() {
        author_handle
    } else {
        url_actor
    };
    let author = format_author(&name, &handle);

    let content = first_selection(
        post,
        &[
            r#"[data-testid="contentHider-post"]"#,
            r#"[itemprop="articleBody"]:not(meta)"#,
            r#"[data-testid="postText"]"#,
        ],
    );
    let text_selection = first_selection(
        post,
        &[
            r#"[data-testid="postText"]"#,
            r#"[itemprop="articleBody"]:not(meta)"#,
        ],
    );
    let text = if let Some(text_el) = &text_selection {
        sanitize_text(&element_text(text_el))
    } else if let Some(content_el) = &content {
        sanitize_text(&element_text(content_el))
    } else {
        String::new()
    };
    let has_media = content
        .as_ref()
        .is_some_and(|c| c.select(&selector("img, video")).next().is_some());
    if text.is_empty() && !has_media {
        return None;
    }

    let mut html = String::new();
    if let Some(content_el) = &content {
        let mut fragment = Html::parse_fragment(&format!("<div>{}</div>", content_el.inner_html()));
        rewrite_urls(&mut fragment, base);
        if let Some(rewritten) = fragment.select(&selector("div")).next() {
            html = rewritten.inner_html();
        }
    }
    if html.trim().is_empty() && !text.is_empty() {
        html = paragraph_html(&text);
    }

    let metadata = post_metadata(&author, &handle, &rendered_published(post, &post_url, base));
    let mut doc = Document::new(post_url);
    doc.title = Some(post_title(&author));
    doc.text = Some(text);
    doc.html = Some(html);
    doc.metadata = metadata;
    Some(doc)
}

fn first_selection<'a>(root: &ElementRef<'a>, selectors: &[&str]) -> Option<ElementRef<'a>> {
    for css in selectors {
        if let Some(found) = root.select(&selector(css)).next() {
            return Some(found);
        }
    }
    None
}

fn rendered_post_url(post: &ElementRef, base: &Url) -> Option<(String, String)> {
    struct Candidate {
        raw: String,
        score: i32,
    }
    let mut candidates = Vec::new();
    let expected_handle = handle_from_test_id(post.value().attr("data-testid").unwrap_or(""));

    let time_sel = selector("time");
    for anchor in post.select(&selector("a[href]")) {
        let raw = anchor.value().attr("href").unwrap_or("").trim().to_string();
        let Some((post_url, actor, _)) = canonical_post_url(&raw, base) else {
            continue;
        };
        let mut score = 1;
        if anchor.select(&time_sel).next().is_some() || time_sel.matches(&anchor) {
            score += 5;
        }
        if anchor
            .value()
            .attr("data-tooltip")
            .is_some_and(|v| !v.is_empty())
            || anchor
                .value()
                .attr("aria-label")
                .is_some_and(|v| !v.is_empty())
        {
            score += 4;
        }
        if !expected_handle.is_empty() && actor.eq_ignore_ascii_case(&expected_handle) {
            score += 3;
        }
        candidates.push(Candidate {
            raw: post_url,
            score,
        });
    }
    for item in post.select(&selector(r#"meta[itemprop="url"]"#)) {
        candidates.push(Candidate {
            raw: item.value().attr("content").unwrap_or("").to_string(),
            score: 8,
        });
    }
    for item in post.select(&selector(r#"link[itemprop="url"]"#)) {
        candidates.push(Candidate {
            raw: item.value().attr("href").unwrap_or("").to_string(),
            score: 8,
        });
    }
    for attr in ["href", "itemid"] {
        if let Some(raw) = post.value().attr(attr) {
            if !raw.is_empty() {
                candidates.push(Candidate {
                    raw: raw.to_string(),
                    score: 2,
                });
            }
        }
    }

    let mut best: Option<(i32, String, String)> = None;
    for candidate in &candidates {
        let Some((post_url, actor, _)) = canonical_post_url(&candidate.raw, base) else {
            continue;
        };
        if best
            .as_ref()
            .is_none_or(|(score, _, _)| candidate.score > *score)
        {
            best = Some((candidate.score, post_url, actor));
        }
    }
    best.map(|(_, url, actor)| (url, actor))
}

fn handle_from_test_id(test_id: &str) -> String {
    for marker in ["feedItem-by-", "postThreadItem-by-"] {
        if let Some(handle) = test_id.split_once(marker).map(|(_, rest)| rest) {
            if valid_handle(handle) {
                return handle.to_string();
            }
        }
    }
    String::new()
}

fn rendered_author(post: &ElementRef, url_actor: &str) -> (String, String) {
    let mut handle = handle_from_test_id(post.value().attr("data-testid").unwrap_or(""));
    if handle.is_empty()
        && !url_actor.to_ascii_lowercase().starts_with("did:")
        && valid_handle(url_actor)
    {
        handle = url_actor.to_string();
    }

    let mut name = String::new();
    for css in [
        r#"[data-testid*="DisplayName"]"#,
        r#"[data-testid*="displayName"]"#,
        r#"[itemprop="author"] [itemprop="name"]"#,
    ] {
        if let Some(el) = post.select(&selector(css)).next() {
            let candidate = element_text(&el).trim().to_string();
            if !candidate.is_empty() {
                name = candidate;
                break;
            }
        }
    }
    if name.is_empty() {
        for anchor in post.select(&selector("a[href]")) {
            let Some(actor) = profile_actor(anchor.value().attr("href").unwrap_or("")) else {
                continue;
            };
            if !handle.is_empty() && !actor.eq_ignore_ascii_case(&handle) {
                continue;
            }
            let candidate = element_text(&anchor).trim().to_string();
            if candidate.is_empty()
                || candidate.starts_with('@')
                || candidate.eq_ignore_ascii_case(&actor)
            {
                continue;
            }
            name = candidate;
            if handle.is_empty() && valid_handle(&actor) {
                handle = actor;
            }
            break;
        }
    }
    if name.is_empty() {
        for avatar in post.select(&selector(r#"[aria-label$="'s avatar"]"#)) {
            let label = avatar.value().attr("aria-label").unwrap_or("").trim();
            let candidate = label.strip_suffix("'s avatar").unwrap_or(label).trim();
            if !candidate.is_empty() {
                name = candidate.to_string();
                break;
            }
        }
    }
    (sanitize_text(&name), sanitize_text(&handle))
}

fn profile_actor(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let path = if let Ok(url) = Url::parse(trimmed) {
        if !is_bluesky_host(url.host_str().unwrap_or("")) {
            return None;
        }
        url.path().to_string()
    } else {
        trimmed.to_string()
    };
    let parts: Vec<&str> = path
        .trim_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    if parts.len() != 2 || parts[0] != "profile" {
        return None;
    }
    let actor = percent_encoding::percent_decode_str(parts[1])
        .decode_utf8()
        .ok()?;
    if actor.is_empty() {
        return None;
    }
    Some(actor.to_string())
}

fn rendered_published(post: &ElementRef, post_url: &str, base: &Url) -> String {
    if let Some(time_el) = post.select(&selector("time[datetime]")).next() {
        let published = time_el.value().attr("datetime").unwrap_or("").trim();
        if !published.is_empty() {
            return sanitize_text(published);
        }
    }
    for anchor in post.select(&selector("a[href]")) {
        let raw = anchor.value().attr("href").unwrap_or("");
        let Some((candidate_url, _, _)) = canonical_post_url(raw, base) else {
            continue;
        };
        if candidate_url != post_url {
            continue;
        }
        let mut value = anchor
            .value()
            .attr("data-tooltip")
            .unwrap_or("")
            .trim()
            .to_string();
        if value.is_empty() {
            value = anchor
                .value()
                .attr("aria-label")
                .unwrap_or("")
                .trim()
                .to_string();
        }
        if !value.is_empty() {
            return sanitize_text(&value);
        }
    }
    String::new()
}

// --- page-level meta fallback --------------------------------------------

fn page_post_document(html: &Html, base: &Url) -> Option<Document> {
    let (post_url, actor, _) = canonical_post_url(base.as_str(), base)?;

    let raw_text = first_page_meta(
        html,
        &[
            r#"meta[property="og:description"]"#,
            r#"meta[name="twitter:description"]"#,
            r#"meta[name="description"]"#,
        ],
    );
    let text = sanitize_text(&raw_text);
    if text.is_empty() {
        return None;
    }

    let (name, title_handle) = author_from_page_title(&first_page_meta(
        html,
        &[
            r#"meta[property="og:title"]"#,
            r#"meta[name="twitter:title"]"#,
            r#"meta[name="title"]"#,
        ],
    ));
    let mut handle = title_handle;
    if handle.is_empty() && !actor.to_ascii_lowercase().starts_with("did:") {
        handle = actor;
    }
    let author = format_author(&name, &handle);
    let image = safe_http_url(
        &first_page_meta(
            html,
            &[
                r#"meta[property="og:image"]"#,
                r#"meta[name="twitter:image"]"#,
            ],
        ),
        base,
    );
    let mut metadata = post_metadata(
        &author,
        &handle,
        &first_page_meta(html, &[r#"meta[property="article:published_time"]"#]),
    );
    let images: Vec<String> = if image.is_empty() {
        Vec::new()
    } else {
        vec![image.clone()]
    };
    if !image.is_empty() {
        metadata.insert("image".to_string(), image.into());
    }

    let mut doc = Document::new(post_url);
    doc.title = Some(post_title(&author));
    doc.text = Some(text.clone());
    doc.html = Some(schema_post_html(&text, &images));
    doc.metadata = metadata;
    Some(doc)
}

fn first_page_meta(html: &Html, selectors: &[&str]) -> String {
    for css in selectors {
        if let Some(el) = html.select(&selector(css)).next() {
            let content = el.value().attr("content").unwrap_or("").trim();
            if !content.is_empty() {
                return content.to_string();
            }
        }
    }
    String::new()
}

fn author_from_page_title(title: &str) -> (String, String) {
    let mut title = sanitize_text(title);
    for suffix in [" on Bluesky", " | Bluesky", " • Bluesky"] {
        if let Some(stripped) = title.strip_suffix(suffix) {
            title = stripped.to_string();
        }
    }
    if let Some(open) = title.rfind(" (@") {
        if let Some(handle) = title
            .strip_suffix(')')
            .and_then(|_| title.get(open + 3..title.len() - 1))
        {
            if valid_handle(handle) {
                let name = title[..open].trim().to_string();
                return (name, handle.to_string());
            }
        }
    }
    (title, String::new())
}

// --- shared helpers --------------------------------------------------------

fn post_metadata(author: &str, handle: &str, published: &str) -> Metadata {
    let mut metadata = Metadata::new();
    metadata.insert("type".to_string(), POST_TYPE.into());
    if !author.is_empty() {
        metadata.insert("author".to_string(), author.to_string().into());
    }
    if !handle.is_empty() {
        let stripped = handle.strip_prefix('@').unwrap_or(handle);
        metadata.insert("handle".to_string(), format!("@{stripped}").into());
    }
    let published = sanitize_text(published);
    if !published.is_empty() {
        metadata.insert("published".to_string(), published.into());
    }
    metadata
}

fn post_title(author: &str) -> String {
    if author.is_empty() {
        "Bluesky post".to_string()
    } else {
        format!("Bluesky post: {author}")
    }
}

fn format_author(name: &str, handle: &str) -> String {
    let name = name.trim();
    let handle = handle.trim().strip_prefix('@').unwrap_or(handle.trim());
    match (name.is_empty(), handle.is_empty()) {
        (false, false) => format!("{name} (@{handle})"),
        (false, true) => name.to_string(),
        (true, false) => format!("@{handle}"),
        (true, true) => String::new(),
    }
}

/// Validates `raw` is a Bluesky post permalink shape
/// (`/profile/<actor>/post/<id>` on a known Bluesky host, after
/// resolving it against `base`) and returns its canonical
/// `https://bsky.app/...` form plus the decoded actor and post id.
/// Matches Go's `canonicalPostURL`.
fn canonical_post_url(raw: &str, base: &Url) -> Option<(String, String, String)> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let resolved = resolve_url(base, raw);
    let url = Url::parse(&resolved).ok()?;
    if !is_bluesky_host(url.host_str().unwrap_or("")) {
        return None;
    }
    let parts: Vec<&str> = url
        .path()
        .trim_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    if parts.len() != 4 || parts[0] != "profile" || parts[2] != "post" {
        return None;
    }
    let actor = percent_encoding::percent_decode_str(parts[1])
        .decode_utf8()
        .ok()?;
    if !valid_actor(&actor) {
        return None;
    }
    let post_id = percent_encoding::percent_decode_str(parts[3])
        .decode_utf8()
        .ok()?;
    if !valid_post_id(&post_id) {
        return None;
    }
    let canonical = format!("https://bsky.app/profile/{actor}/post/{post_id}");
    Some((canonical, actor.to_string(), post_id.to_string()))
}

fn valid_actor(actor: &str) -> bool {
    if actor.to_ascii_lowercase().starts_with("did:") {
        return !actor.contains(['/', '?', '#', ' ']);
    }
    valid_handle(actor)
}

fn valid_handle(handle: &str) -> bool {
    let handle = handle.trim();
    if handle.is_empty() || handle.starts_with('.') || handle.ends_with('.') {
        return false;
    }
    handle
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

fn valid_post_id(post_id: &str) -> bool {
    !post_id.is_empty() && post_id.chars().all(|c| c.is_ascii_alphanumeric())
}

fn safe_http_url(raw: &str, base: &Url) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return String::new();
    }
    let resolved = resolve_url(base, raw);
    let Ok(url) = Url::parse(&resolved) else {
        return String::new();
    };
    if (url.scheme() != "http" && url.scheme() != "https")
        || url.host_str().unwrap_or("").is_empty()
    {
        return String::new();
    }
    url.to_string()
}

fn paragraph_html(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n");
    let escaped = html_escape(&normalized);
    format!("<p>{}</p>", escaped.replace('\n', "<br>"))
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
    fn matches_bluesky_hosts_and_post_documents() {
        let ext = BlueskyExtractor::default();
        assert!(ext.matches(&doc("https://bsky.app/profile/jane.bsky.social", "")));
        assert!(ext.matches(&doc("https://www.bsky.app/profile/jane.bsky.social", "")));
        assert!(!ext.matches(&doc("https://example.com/profile/jane", "")));

        let mut post = doc("https://bsky.app/profile/jane.bsky.social/post/abc123", "");
        post.metadata
            .insert("type".to_string(), "bluesky".to_string().into());
        assert!(ext.matches(&post));
    }

    const SCHEMA_HTML: &str = r#"<html><head>
        <script type="application/ld+json">
        {"@type": "SocialMediaPosting", "url": "https://bsky.app/profile/jane.bsky.social/post/abc123",
         "text": "Hello from Bluesky!", "author": {"name": "Jane Doe", "alternateName": "@jane.bsky.social"},
         "datePublished": "2024-01-01T00:00:00Z"}
        </script>
    </head><body></body></html>"#;

    #[test]
    fn extract_collects_a_schema_post_as_extra_document() {
        let ext = BlueskyExtractor::default();
        let d = doc("https://bsky.app/profile/jane.bsky.social", SCHEMA_HTML);
        let ExtractOutcome::Extracted(extracted) = ext.extract(&d) else {
            panic!("expected Extracted");
        };
        assert!(extracted.skip_indexing);
        assert_eq!(extracted.extra_documents.len(), 1);
        let post = &extracted.extra_documents[0];
        assert_eq!(
            post.url,
            "https://bsky.app/profile/jane.bsky.social/post/abc123"
        );
        assert_eq!(post.text.as_deref(), Some("Hello from Bluesky!"));
        assert_eq!(
            post.metadata.get("handle").and_then(|v| v.as_str()),
            Some("@jane.bsky.social")
        );
    }

    const RENDERED_HTML: &str = r#"<html><body>
        <article data-testid="feedItem-by-john.bsky.social">
            <a data-testid="postThreadItem-by-john" href="/profile/john.bsky.social">
                <span data-testid="authorDisplayName">John Roe</span>
            </a>
            <div data-testid="postText">A rendered Bluesky post.</div>
            <a href="/profile/john.bsky.social/post/xyz789"><time datetime="2024-02-02T00:00:00Z"></time></a>
        </article>
    </body></html>"#;

    #[test]
    fn extract_collects_a_rendered_post_as_extra_document() {
        let ext = BlueskyExtractor::default();
        let d = doc("https://bsky.app/profile/john.bsky.social", RENDERED_HTML);
        let ExtractOutcome::Extracted(extracted) = ext.extract(&d) else {
            panic!("expected Extracted");
        };
        assert_eq!(extracted.extra_documents.len(), 1);
        let post = &extracted.extra_documents[0];
        assert_eq!(
            post.url,
            "https://bsky.app/profile/john.bsky.social/post/xyz789"
        );
        assert!(post
            .text
            .as_deref()
            .unwrap()
            .contains("rendered Bluesky post"));
    }

    #[test]
    fn extract_falls_back_to_page_meta_when_nothing_else_matches() {
        let ext = BlueskyExtractor::default();
        let html = r#"<html><head>
            <meta property="og:title" content="Jane Doe (@jane.bsky.social)">
            <meta property="og:description" content="A single post page description.">
        </head><body></body></html>"#;
        let d = doc(
            "https://bsky.app/profile/jane.bsky.social/post/single1",
            html,
        );
        let ExtractOutcome::Extracted(extracted) = ext.extract(&d) else {
            panic!("expected Extracted");
        };
        assert_eq!(extracted.extra_documents.len(), 1);
        let post = &extracted.extra_documents[0];
        assert_eq!(
            post.url,
            "https://bsky.app/profile/jane.bsky.social/post/single1"
        );
        assert_eq!(
            post.text.as_deref(),
            Some("A single post page description.")
        );
    }

    #[test]
    fn extract_returns_post_document_unchanged_for_recursion_guard() {
        let ext = BlueskyExtractor::default();
        let mut post = doc("https://bsky.app/profile/jane.bsky.social/post/abc123", "");
        post.metadata
            .insert("type".to_string(), "bluesky".to_string().into());
        post.text = Some("already extracted".to_string());

        let ExtractOutcome::Extracted(extracted) = ext.extract(&post) else {
            panic!("expected Extracted");
        };
        assert!(!extracted.skip_indexing);
        assert_eq!(extracted.text.as_deref(), Some("already extracted"));
    }

    #[test]
    fn preview_renders_title_and_html_or_falls_back_to_text() {
        let ext = BlueskyExtractor::default();
        let mut d = doc("https://bsky.app/profile/jane.bsky.social/post/abc123", "");
        d.title = Some("Bluesky post: Jane Doe (@jane.bsky.social)".to_string());
        d.html = Some("<p>Hello world.</p>".to_string());
        let PreviewOutcome::Previewed(response) = ext.preview(&d) else {
            panic!("expected Previewed");
        };
        let content = response.html.unwrap();
        assert!(content.contains("<h2>Bluesky post: Jane Doe (@jane.bsky.social)</h2>"));
        assert!(content.contains("<p>Hello world.</p>"));
    }
}
