//! `Discourse` — a Rust port of
//! `server/extractor/extractors/discourse/discourse.go` (capability
//! inventory §4.5.4). Extract and preview for Discourse forum topic pages.
//!
//! Like `RedditExtractor`, a Discourse topic page can carry the same
//! content in up to three places at once, and this port merges them the
//! same way Go does rather than picking just one:
//! - a `#data-preloaded` JSON blob Discourse embeds for its own
//!   client-side app to hydrate from (often *double*-encoded: the value
//!   itself is a JSON string containing the real payload — see
//!   [`parse_preloaded_topic`]);
//! - the already-rendered post DOM ([`parse_rendered_posts`]);
//! - a `schema.org` `QAPage` JSON-LD block ([`parse_schema_topic`]).
//!
//! [`merge_posts`] combines all three by post id/number, preferring the
//! highest-fidelity source available per field (`source_rank`: rendered
//! DOM > preloaded JSON > JSON-LD, matching Go's own ranking) rather than
//! just picking one source and discarding the others.
//!
//! [`clean_and_serialize_body`] is this module's one recurring
//! `scraper::ElementRef`-is-read-only workaround, used everywhere Go
//! mutates a `goquery.Selection` in place (`.Find(...).Remove()` for UI
//! chrome, then `urlutil.RewriteURLs`): reparse the content as its own
//! `Html::parse_fragment` wrapped in a synthetic `<div>` (so multiple
//! top-level nodes, the common case for post bodies, survive), detach the
//! noise by `NodeId`, rewrite URLs, then read back the wrapper's own
//! `inner_html()` — the same "reparse as an independent document" trick
//! `WikipediaExtractor::extract` and `RedditExtractor`'s body/comment
//! HTML use for the same underlying reason.

use crate::sanitizer::{sanitize_html, sanitize_text};
use crate::stackexchange::{html_escape, selector};
use crate::textutil;
use crate::urlutil::rewrite_urls;
use ammonia::Url;
use rusty_hister_core::{
    Capabilities, Document, ExtractOutcome, Extractor, ExtractorConfig, HisterError,
    PreviewOutcome, PreviewResponse,
};
use rusty_json::{Map, Number, Value};
use scraper::{ElementRef, Html, Node};

#[derive(Debug, Default)]
pub struct DiscourseExtractor {
    config: ExtractorConfig,
}

#[derive(Debug, Default, Clone)]
struct DiscourseTopic {
    id: i64,
    title: String,
    category: String,
    tags: Vec<String>,
    posts: Vec<DiscoursePost>,
}

#[derive(Debug, Default, Clone)]
struct DiscoursePost {
    id: i64,
    number: i32,
    reply_to: i32,
    author: String,
    published: String,
    text: String,
    html: String,
    likes: i32,
    reactions: i32,
    accepted: bool,
    source_rank: i32,
}

const NOISE_SELECTOR: &str = "script, style, button, .cooked-selection-barrier, .post-menu-area";

impl Extractor for DiscourseExtractor {
    fn name(&self) -> &str {
        "Discourse"
    }

    fn description(&self) -> &str {
        "Extracts a Discourse topic and every post already present in the page."
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            enrich: false,
            extract: true,
            preview: true,
        }
    }

    fn matches(&self, document: &Document) -> bool {
        discourse_topic_url(&document.url).is_some()
            && is_discourse_html(document.html.as_deref().unwrap_or(""))
    }

    fn extract(&self, document: &Document) -> ExtractOutcome {
        let topic = match parse_discourse_topic(document) {
            Ok(topic) => topic,
            Err(message) => return ExtractOutcome::Fallback(HisterError::Extraction(message)),
        };

        let mut extracted = document.clone();
        if !topic.title.is_empty() {
            extracted.title = Some(topic.title.clone());
        }
        extracted.text = Some(topic_text(&topic));
        extracted
            .metadata
            .insert("type".to_string(), Value::String("discourse".to_string()));
        extracted
            .metadata
            .insert("topic_id".to_string(), Value::String(topic.id.to_string()));
        extracted.metadata.insert(
            "posts".to_string(),
            Value::Number(Number::from(topic.posts.len())),
        );
        extracted.metadata.insert(
            "replies".to_string(),
            Value::Number(Number::from(topic.posts.len())),
        );
        for post in &topic.posts {
            if post.number == 1 {
                extracted.metadata.insert(
                    "replies".to_string(),
                    Value::Number(Number::from(topic.posts.len().saturating_sub(1))),
                );
                set_metadata(&mut extracted, "author", &post.author);
                set_metadata(&mut extracted, "published", &post.published);
                break;
            }
        }
        set_metadata(&mut extracted, "category", &topic.category);
        if !topic.tags.is_empty() {
            extracted
                .metadata
                .insert("tags".to_string(), Value::String(topic.tags.join(", ")));
        }
        for post in &topic.posts {
            if post.accepted && post.number > 0 {
                extracted.metadata.insert(
                    "accepted_answer".to_string(),
                    Value::Number(Number::from(post.number)),
                );
                break;
            }
        }
        ExtractOutcome::Extracted(extracted)
    }

    fn preview(&self, document: &Document) -> PreviewOutcome {
        let topic = match parse_discourse_topic(document) {
            Ok(topic) => topic,
            Err(message) => return PreviewOutcome::Fallback(HisterError::Extraction(message)),
        };

        let mut out = String::new();
        if !topic.title.is_empty() {
            out.push_str(&format!(
                r#"<h2><a href="{}">{}</a></h2>"#,
                html_escape(&document.url),
                html_escape(&topic.title)
            ));
        }
        let mut topic_parts = Vec::new();
        if !topic.category.is_empty() {
            topic_parts.push(format!("category: {}", html_escape(&topic.category)));
        }
        if !topic.tags.is_empty() {
            let escaped: Vec<String> = topic.tags.iter().map(|t| html_escape(t)).collect();
            topic_parts.push(format!("tags: {}", escaped.join(", ")));
        }
        if !topic_parts.is_empty() {
            out.push_str(&format!("<p>{}</p>", topic_parts.join(" &middot; ")));
        }

        for (index, post) in topic.posts.iter().enumerate() {
            if index > 0 {
                out.push_str("<hr>");
            }
            let mut heading = if post.number == 1 {
                "Original post".to_string()
            } else if post.number > 1 {
                format!("Reply #{}", post.number)
            } else {
                "Post".to_string()
            };
            if post.accepted {
                heading.push_str(" (accepted solution)");
            }
            out.push_str(&format!("<h3>{}</h3>", html_escape(&heading)));
            let parts = post_metadata_parts(post);
            if !parts.is_empty() {
                out.push_str(&format!("<p>{}</p>", parts.join(" &middot; ")));
            }
            out.push_str(&post.html);
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

fn set_metadata(document: &mut Document, key: &str, value: &str) {
    if !value.is_empty() {
        document
            .metadata
            .insert(key.to_string(), Value::String(value.to_string()));
    }
}

fn discourse_topic_url(raw_url: &str) -> Option<i64> {
    let url = Url::parse(raw_url).ok()?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return None;
    }
    if url.host_str().unwrap_or("").is_empty() {
        return None;
    }
    let parts = path_parts(url.path());
    for (index, part) in parts.iter().enumerate() {
        if part != "t" || index + 1 >= parts.len() {
            continue;
        }
        let mut id_index = index + 1;
        if !is_positive_integer(&parts[id_index]) {
            id_index += 1;
        }
        if id_index >= parts.len() || !is_positive_integer(&parts[id_index]) {
            return None;
        }
        if parts.len() > id_index + 2
            || (parts.len() == id_index + 2 && !is_positive_integer(&parts[id_index + 1]))
        {
            return None;
        }
        let id: i64 = parts[id_index].parse().ok()?;
        return (id > 0).then_some(id);
    }
    None
}

fn path_parts(path: &str) -> Vec<String> {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        return Vec::new();
    }
    trimmed.split('/').map(str::to_string).collect()
}

fn is_positive_integer(value: &str) -> bool {
    !value.is_empty() && value != "0" && value.chars().all(|c| c.is_ascii_digit())
}

/// Sniffs whether `raw_html` was generated by Discourse, via the same
/// `<meta>` markers Go's own tokenizer-based `isDiscourseHTML` looks for
/// (`id="data-discourse-setup"`, `name="discourse/config/environment"`, or
/// a `generator` meta whose content starts with `discourse`) — ported here
/// as a CSS-selector scan over the already-available `scraper` parse
/// rather than a hand-rolled byte-stream tokenizer, since this crate
/// already depends on `scraper` for every other markup-aware extractor.
fn is_discourse_html(raw_html: &str) -> bool {
    let html = Html::parse_document(raw_html);
    for meta in html.select(&selector("meta")) {
        let id = meta.value().attr("id").unwrap_or("");
        let name = meta.value().attr("name").unwrap_or("");
        let content = meta.value().attr("content").unwrap_or("");
        if id.eq_ignore_ascii_case("data-discourse-setup")
            || name.eq_ignore_ascii_case("discourse/config/environment")
        {
            return true;
        }
        let generator = content.trim().to_ascii_lowercase();
        if name.eq_ignore_ascii_case("generator")
            && (generator == "discourse" || generator.starts_with("discourse "))
        {
            return true;
        }
    }
    false
}

fn parse_discourse_topic(document: &Document) -> Result<DiscourseTopic, String> {
    let Some(topic_id) = discourse_topic_url(&document.url) else {
        return Err("not a Discourse topic page".to_string());
    };
    let html_str = document.html.as_deref().unwrap_or("");
    if !is_discourse_html(html_str) {
        return Err("page is not generated by Discourse".to_string());
    }
    let html = Html::parse_document(html_str);
    let base = Url::parse(&document.url).ok();

    let mut topic =
        parse_preloaded_topic(&html, topic_id, base.as_ref()).unwrap_or(DiscourseTopic {
            id: topic_id,
            ..Default::default()
        });
    topic.id = topic_id;
    read_topic_header(&mut topic, &html);
    merge_posts(&mut topic, parse_rendered_posts(&html, base.as_ref()));
    merge_schema_topic(&mut topic, parse_schema_topic(&html, base.as_ref()));
    finalize_topic(&mut topic);

    if topic.title.is_empty() || topic.posts.is_empty() {
        return Err("discourse topic has no posts".to_string());
    }
    Ok(topic)
}

fn parse_preloaded_topic(html: &Html, topic_id: i64, base: Option<&Url>) -> Option<DiscourseTopic> {
    let raw = html
        .select(&selector("#data-preloaded"))
        .next()?
        .text()
        .collect::<String>();
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let entries = Value::parse(raw).ok()?;
    let entries = entries.as_object()?;
    let mut entry = entries.get(&format!("topic_{topic_id}"))?.clone();
    // Discourse's preloaded payload frequently double-encodes: the value
    // for each key is itself a JSON string containing the real object.
    if let Value::String(inner) = &entry {
        if let Ok(parsed) = Value::parse(inner) {
            entry = parsed;
        }
    }
    let source = entry.as_object()?;

    let title = first_nonempty(&[
        source
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        sanitize_text(
            source
                .get("fancy_title")
                .and_then(Value::as_str)
                .unwrap_or(""),
        ),
    ]);
    let tags = parse_preloaded_tags(source.get("tags"));
    let mut posts = Vec::new();
    if let Some(Value::Object(post_stream)) = source.get("post_stream") {
        if let Some(Value::Array(raw_posts)) = post_stream.get("posts") {
            for raw_post in raw_posts {
                if let Some(post) = preloaded_post(raw_post, base) {
                    posts.push(post);
                }
            }
        }
    }
    Some(DiscourseTopic {
        id: topic_id,
        title,
        category: String::new(),
        tags,
        posts,
    })
}

fn preloaded_post(raw: &Value, base: Option<&Url>) -> Option<DiscoursePost> {
    let obj = raw.as_object()?;
    let post_type = obj.get("post_type").and_then(value_as_i64).unwrap_or(0);
    let hidden = matches!(obj.get("hidden"), Some(Value::Bool(true)));
    let deleted = obj.get("deleted_at").is_some_and(has_json_value);
    if post_type > 1 || hidden || deleted {
        return None;
    }
    let cooked = obj.get("cooked").and_then(Value::as_str).unwrap_or("");
    let (body_text, body_html) = extract_content_html(cooked, base);
    if body_text.is_empty() {
        return None;
    }
    let name = obj.get("name").and_then(Value::as_str).unwrap_or("");
    let username = obj.get("username").and_then(Value::as_str).unwrap_or("");
    let display_username = obj
        .get("display_username")
        .and_then(Value::as_str)
        .unwrap_or("");
    let mut post = DiscoursePost {
        id: obj.get("id").and_then(value_as_i64).unwrap_or(0),
        number: obj.get("post_number").and_then(value_as_i64).unwrap_or(0) as i32,
        author: format_author(name, display_username, username),
        published: obj
            .get("created_at")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        text: body_text,
        html: body_html,
        accepted: matches!(obj.get("accepted_answer"), Some(Value::Bool(true))),
        source_rank: 2,
        ..Default::default()
    };
    if let Some(reply_to) = obj.get("reply_to_post_number").and_then(value_as_i64) {
        post.reply_to = reply_to as i32;
    }
    if let Some(Value::Array(actions)) = obj.get("actions_summary") {
        for action in actions {
            if let Some(action) = action.as_object() {
                if action.get("id").and_then(value_as_i64) == Some(2) {
                    post.likes = action.get("count").and_then(value_as_i64).unwrap_or(0) as i32;
                }
            }
        }
    }
    if let Some(Value::Array(reactions)) = obj.get("reactions") {
        for reaction in reactions {
            if let Some(reaction) = reaction.as_object() {
                let id = reaction.get("id").and_then(Value::as_str).unwrap_or("");
                if post.likes > 0 && (id == "heart" || id == "like") {
                    continue;
                }
                post.reactions += reaction.get("count").and_then(value_as_i64).unwrap_or(0) as i32;
            }
        }
    }
    Some(post)
}

fn value_as_i64(value: &Value) -> Option<i64> {
    let Value::Number(n) = value else { return None };
    n.as_i64()
        .or_else(|| n.as_u64().map(|u| u as i64))
        .or(Some(n.as_f64() as i64))
}

fn has_json_value(value: &Value) -> bool {
    !matches!(value, Value::Null)
}

fn parse_preloaded_tags(value: Option<&Value>) -> Vec<String> {
    let Some(Value::Array(items)) = value else {
        return Vec::new();
    };
    let as_strings: Option<Vec<String>> = items
        .iter()
        .map(|v| v.as_str().map(str::to_string))
        .collect();
    if let Some(strings) = as_strings {
        return clean_strings(&strings);
    }
    let tags: Vec<String> = items
        .iter()
        .filter_map(|item| {
            let obj = item.as_object()?;
            for key in ["name", "id"] {
                if let Some(s) = obj.get(key).and_then(Value::as_str) {
                    if !s.is_empty() {
                        return Some(s.to_string());
                    }
                }
            }
            None
        })
        .collect();
    clean_strings(&tags)
}

fn clean_strings(values: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut cleaned = Vec::new();
    for value in values {
        let value = sanitize_text(value);
        if value.is_empty() {
            continue;
        }
        let key = value.to_ascii_lowercase();
        if !seen.insert(key) {
            continue;
        }
        cleaned.push(value);
    }
    cleaned
}

fn read_topic_header(topic: &mut DiscourseTopic, html: &Html) {
    topic.title = first_nonempty(&[
        html.select(&selector("#topic-title h1 a"))
            .next()
            .map(|el| textutil::selection_text(&el))
            .unwrap_or_default(),
        html.select(&selector(r#"h1[data-topic-id]"#))
            .next()
            .map(|el| textutil::selection_text(&el))
            .unwrap_or_default(),
        meta_content(
            html,
            &[
                r#"meta[property="og:title"]"#,
                r#"meta[name="twitter:title"]"#,
            ],
        ),
        topic.title.clone(),
    ]);
    if topic.category.is_empty() {
        topic.category = first_nonempty(&[
            html.select(&selector("#topic-title .badge-category__name"))
                .next()
                .map(|el| textutil::selection_text(&el))
                .unwrap_or_default(),
            html.select(&selector("#topic-title .category-name"))
                .next()
                .map(|el| textutil::selection_text(&el))
                .unwrap_or_default(),
            meta_content(html, &[r#"meta[property="og:article:section"]"#]),
        ]);
    }
    if topic.tags.is_empty() {
        for tag in html.select(&selector("#topic-title .discourse-tag")) {
            let value = textutil::selection_text(&tag);
            if !value.is_empty() {
                topic.tags.push(value);
            }
        }
    }
}

fn meta_content(html: &Html, selectors: &[&str]) -> String {
    for css in selectors {
        if let Some(value) = html
            .select(&selector(css))
            .next()
            .and_then(|el| el.value().attr("content"))
        {
            let value = value.trim();
            if !value.is_empty() {
                return value.to_string();
            }
        }
    }
    String::new()
}

fn first_nonempty(values: &[String]) -> String {
    for value in values {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    String::new()
}

const HIDDEN_SELECTOR: &str = "[hidden], .post-hidden, .deleted";

fn parse_rendered_posts(html: &Html, base: Option<&Url>) -> Vec<DiscoursePost> {
    let hidden_sel = selector(HIDDEN_SELECTOR);
    let mut posts = Vec::new();
    for root in html.select(&selector(
        ".post-stream article[data-post-id], article[data-post-id], .crawler-post",
    )) {
        if hidden_sel.matches(&root) {
            continue;
        }
        if let Some(container) = closest(root, ".topic-post") {
            if hidden_sel.matches(&container) {
                continue;
            }
        }
        let Some(body) = first_body(root) else {
            continue;
        };
        let (body_text, body_html) = clean_and_serialize_body(&body.inner_html(), base);
        if body_text.is_empty() {
            continue;
        }
        let mut post = DiscoursePost {
            id: root
                .value()
                .attr("data-post-id")
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0),
            number: rendered_post_number(root),
            reply_to: root
                .value()
                .attr("data-reply-to-post-number")
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0),
            author: rendered_author(root),
            published: rendered_published(root),
            text: body_text,
            html: body_html,
            accepted: rendered_post_accepted(root),
            source_rank: 3,
            ..Default::default()
        };
        post.likes = first_selection_int(
            root,
            &[
                r#"[data-action-id="2"]"#,
                ".like-count",
                ".post-action-menu__like-count",
            ],
        );
        posts.push(post);
    }
    posts
}

fn closest<'a>(el: ElementRef<'a>, css: &str) -> Option<ElementRef<'a>> {
    let sel = selector(css);
    let mut current: Option<ego_tree::NodeRef<'a, Node>> = Some(*el);
    while let Some(node) = current {
        if let Some(element) = ElementRef::wrap(node) {
            if sel.matches(&element) {
                return Some(element);
            }
        }
        current = node.parent();
    }
    None
}

fn first_body(root: ElementRef<'_>) -> Option<ElementRef<'_>> {
    for css in [
        ".cooked",
        r#".post[itemprop="text"]"#,
        r#"[itemprop="text"]"#,
    ] {
        if let Some(body) = root.select(&selector(css)).next() {
            return Some(body);
        }
    }
    None
}

fn rendered_author(root: ElementRef) -> String {
    let display_name = first_selection_text(
        root,
        &[
            ".names .full-name a",
            ".names .full-name",
            r#".creator [itemprop="name"]"#,
            r#"[itemprop="author"] [itemprop="name"]"#,
        ],
    );
    let mut username = first_selection_text(root, &[".names .username a", ".names .username"]);
    if username.is_empty() {
        username = first_selection_attr(root, "data-user-card", &[".names [data-user-card]"]);
    }
    format_author(&display_name, "", &username)
}

fn rendered_published(root: ElementRef) -> String {
    let published = first_selection_attr(
        root,
        "datetime",
        &["time[datetime]", r#"[itemprop="datePublished"][datetime]"#],
    );
    if !published.is_empty() {
        return published;
    }
    let value = first_selection_attr(root, "data-time", &[".relative-date[data-time]"]);
    if !value.is_empty() {
        if let Ok(timestamp) = value.parse::<i64>() {
            let dt = if timestamp < 1_000_000_000_000 {
                chrono::DateTime::from_timestamp(timestamp, 0)
            } else {
                chrono::DateTime::from_timestamp_millis(timestamp)
            };
            if let Some(dt) = dt {
                return dt.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true);
            }
        }
    }
    first_selection_attr(root, "title", &[".relative-date[title]"])
}

fn rendered_post_accepted(root: ElementRef) -> bool {
    let sel = selector(r#"[itemprop="acceptedAnswer"], .accepted-answer, .accepted-text"#);
    sel.matches(&root) || root.select(&sel).next().is_some()
}

fn rendered_post_number(root: ElementRef) -> i32 {
    if let Some(n) = root
        .value()
        .attr("data-post-number")
        .and_then(|v| v.trim().parse::<i32>().ok())
    {
        if n > 0 {
            return n;
        }
    }
    if let Some(container) = closest(root, "[data-post-number]") {
        if let Some(n) = container
            .value()
            .attr("data-post-number")
            .and_then(|v| v.trim().parse::<i32>().ok())
        {
            if n > 0 {
                return n;
            }
        }
    }
    if let Some(id) = root.value().attr("id") {
        if let Some(rest) = id.strip_prefix("post_") {
            return rest.trim().parse().unwrap_or(0);
        }
    }
    0
}

fn first_selection_text(root: ElementRef, selectors: &[&str]) -> String {
    for css in selectors {
        if let Some(el) = root.select(&selector(css)).next() {
            let text = textutil::selection_text(&el);
            if !text.is_empty() {
                return text;
            }
        }
    }
    String::new()
}

fn first_selection_attr(root: ElementRef, attr: &str, selectors: &[&str]) -> String {
    for css in selectors {
        if let Some(el) = root.select(&selector(css)).next() {
            let value = el.value().attr(attr).unwrap_or("").trim();
            if !value.is_empty() {
                return value.to_string();
            }
        }
    }
    String::new()
}

fn first_selection_int(root: ElementRef, selectors: &[&str]) -> i32 {
    for css in selectors {
        if let Some(el) = root.select(&selector(css)).next() {
            let text = textutil::selection_text(&el);
            for field in text.split_whitespace() {
                let trimmed = field.trim_matches(|c: char| "()[]{}.,".contains(c));
                if let Ok(n) = trimmed.parse::<i32>() {
                    if n > 0 {
                        return n;
                    }
                }
            }
        }
    }
    0
}

fn format_author(name: &str, display_name: &str, username: &str) -> String {
    let name = first_nonempty(&[display_name.to_string(), name.to_string()]);
    let name = sanitize_text(&name);
    let username = sanitize_text(username);
    if name.is_empty() {
        return username;
    }
    if username.is_empty() || name.eq_ignore_ascii_case(&username) {
        return name;
    }
    format!("{name} (@{username})")
}

fn parse_schema_topic(html: &Html, base: Option<&Url>) -> Option<DiscourseTopic> {
    for script in html.select(&selector(r#"script[type="application/ld+json"]"#)) {
        let text = script.text().collect::<String>();
        let Ok(value) = Value::parse(&text) else {
            continue;
        };
        let Some(page) = find_schema_type(&value, "QAPage") else {
            continue;
        };
        return Some(schema_topic(page, base));
    }
    None
}

fn find_schema_type<'a>(value: &'a Value, wanted: &str) -> Option<&'a Map> {
    match value {
        Value::Array(items) => items.iter().find_map(|item| find_schema_type(item, wanted)),
        Value::Object(map) => {
            if schema_type_matches(map.get("@type"), wanted) {
                return Some(map);
            }
            ["@graph", "mainEntity"]
                .into_iter()
                .find_map(|key| map.get(key).and_then(|v| find_schema_type(v, wanted)))
        }
        _ => None,
    }
}

fn schema_type_matches(value: Option<&Value>, wanted: &str) -> bool {
    match value {
        Some(Value::String(s)) => {
            let s = s.trim_end_matches('/');
            let s = match s.rfind(['/', '#']) {
                Some(i) => &s[i + 1..],
                None => s,
            };
            s.eq_ignore_ascii_case(wanted)
        }
        Some(Value::Array(items)) => items
            .iter()
            .any(|item| schema_type_matches(Some(item), wanted)),
        Some(Value::Object(map)) => schema_type_matches(map.get("@type"), wanted),
        _ => false,
    }
}

fn schema_topic(page: &Map, base: Option<&Url>) -> DiscourseTopic {
    let mut topic = DiscourseTopic {
        title: schema_string(page, &["name", "headline"]),
        ..Default::default()
    };
    let Some(Value::Object(question)) = page.get("mainEntity") else {
        return topic;
    };
    if !schema_type_matches(question.get("@type"), "Question") {
        return topic;
    }
    topic.posts.push(schema_post(question, 1, false, base));
    for value in schema_objects(question.get("acceptedAnswer")) {
        topic.posts.push(schema_post(value, 0, true, base));
    }
    for value in schema_objects(question.get("suggestedAnswer")) {
        topic.posts.push(schema_post(value, 0, false, base));
    }
    topic
}

fn schema_post(
    value: &Map,
    fallback_number: i32,
    accepted: bool,
    base: Option<&Url>,
) -> DiscoursePost {
    let raw_html = schema_string(value, &["text", "articleBody"]);
    let (body_text, body_html) = extract_content_html(&raw_html, base);
    let mut post = DiscoursePost {
        number: schema_post_number(&schema_string(value, &["url", "@id"])),
        author: schema_author(value.get("author")),
        published: schema_string(value, &["datePublished", "dateCreated"]),
        text: body_text,
        html: body_html,
        likes: schema_int(value.get("upvoteCount")),
        accepted,
        source_rank: 1,
        ..Default::default()
    };
    if post.number == 0 {
        post.number = fallback_number;
    }
    post
}

fn schema_post_number(raw_url: &str) -> i32 {
    let path = Url::parse(raw_url)
        .map(|u| u.path().to_string())
        .unwrap_or_else(|_| raw_url.to_string());
    let parts = path_parts(&path);
    parts.last().and_then(|s| s.parse().ok()).unwrap_or(0)
}

fn schema_objects(value: Option<&Value>) -> Vec<&Map> {
    match value {
        Some(Value::Object(map)) => vec![map],
        Some(Value::Array(items)) => items.iter().filter_map(Value::as_object).collect(),
        _ => Vec::new(),
    }
}

fn schema_string(value: &Map, keys: &[&str]) -> String {
    for key in keys {
        if let Some(Value::String(s)) = value.get(*key) {
            let s = s.trim();
            if !s.is_empty() {
                return s.to_string();
            }
        }
    }
    String::new()
}

fn schema_author(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => sanitize_text(s),
        Some(Value::Object(map)) => sanitize_text(&schema_string(map, &["name", "alternateName"])),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| schema_author(Some(item)))
            .find(|a| !a.is_empty())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn schema_int(value: Option<&Value>) -> i32 {
    match value {
        Some(Value::Number(n)) => n.as_f64() as i32,
        Some(Value::String(s)) => s.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

fn merge_schema_topic(topic: &mut DiscourseTopic, schema: Option<DiscourseTopic>) {
    let Some(schema) = schema else { return };
    if topic.title.is_empty() {
        topic.title = schema.title;
    }
    merge_posts(topic, schema.posts);
}

fn merge_posts(topic: &mut DiscourseTopic, incoming: Vec<DiscoursePost>) {
    for candidate in incoming {
        if candidate.text.is_empty() {
            continue;
        }
        let matched = topic.posts.iter().position(|existing| {
            (candidate.id > 0 && existing.id == candidate.id)
                || (candidate.number > 0 && existing.number == candidate.number)
        });
        let Some(matched) = matched else {
            topic.posts.push(candidate);
            continue;
        };
        topic.posts[matched].accepted = topic.posts[matched].accepted || candidate.accepted;
        if candidate.source_rank >= topic.posts[matched].source_rank {
            merge_post_content(&mut topic.posts[matched], candidate);
        } else {
            fill_post_metadata(&mut topic.posts[matched], candidate);
        }
    }
}

fn merge_post_content(destination: &mut DiscoursePost, mut source: DiscoursePost) {
    let accepted = destination.accepted;
    if source.id == 0 {
        source.id = destination.id;
    }
    if source.number == 0 {
        source.number = destination.number;
    }
    if source.reply_to == 0 {
        source.reply_to = destination.reply_to;
    }
    if source.author.is_empty() {
        source.author = destination.author.clone();
    }
    if source.published.is_empty() {
        source.published = destination.published.clone();
    }
    if source.likes == 0 {
        source.likes = destination.likes;
    }
    if source.reactions == 0 {
        source.reactions = destination.reactions;
    }
    *destination = source;
    destination.accepted = accepted || destination.accepted;
}

fn fill_post_metadata(destination: &mut DiscoursePost, source: DiscoursePost) {
    if destination.author.is_empty() {
        destination.author = source.author;
    }
    if destination.published.is_empty() {
        destination.published = source.published;
    }
    if destination.reply_to == 0 {
        destination.reply_to = source.reply_to;
    }
    if destination.likes == 0 {
        destination.likes = source.likes;
    }
    if destination.reactions == 0 {
        destination.reactions = source.reactions;
    }
}

fn finalize_topic(topic: &mut DiscourseTopic) {
    topic.title = sanitize_text(&topic.title);
    topic.category = sanitize_text(&topic.category);
    topic.tags = clean_strings(&topic.tags);
    topic
        .posts
        .sort_by(|left, right| match (left.number == 0, right.number == 0) {
            (true, true) => std::cmp::Ordering::Equal,
            (true, false) => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            (false, false) => left.number.cmp(&right.number),
        });
}

fn topic_text(topic: &DiscourseTopic) -> String {
    let mut out = String::new();
    out.push_str(&topic.title);
    if !topic.category.is_empty() {
        out.push_str("\ncategory: ");
        out.push_str(&topic.category);
    }
    if !topic.tags.is_empty() {
        out.push_str("\ntags: ");
        out.push_str(&topic.tags.join(", "));
    }
    for post in &topic.posts {
        out.push_str("\n\n");
        if post.number > 0 {
            out.push_str(&format!("#{} ", post.number));
        }
        if !post.author.is_empty() {
            out.push_str(&post.author);
        }
        if post.accepted {
            out.push_str(" [Accepted Solution]");
        }
        if post.reply_to > 0 {
            out.push_str(&format!(" [reply to #{}]", post.reply_to));
        }
        if post.likes > 0 {
            out.push_str(&format!(" [{} likes]", post.likes));
        }
        if post.reactions > 0 {
            out.push_str(&format!(" [{} reactions]", post.reactions));
        }
        out.push('\n');
        out.push_str(&post.text);
    }
    out.trim().to_string()
}

fn post_metadata_parts(post: &DiscoursePost) -> Vec<String> {
    let mut parts = Vec::new();
    if !post.author.is_empty() {
        parts.push(html_escape(&post.author));
    }
    if !post.published.is_empty() {
        parts.push(html_escape(&post.published));
    }
    if post.reply_to > 0 {
        parts.push(html_escape(&format!("reply to #{}", post.reply_to)));
    }
    if post.likes > 0 {
        parts.push(html_escape(&format!("{} likes", post.likes)));
    }
    if post.reactions > 0 {
        parts.push(html_escape(&format!("{} reactions", post.reactions)));
    }
    parts
}

fn extract_content_html(raw_html: &str, base: Option<&Url>) -> (String, String) {
    if raw_html.trim().is_empty() {
        return (String::new(), String::new());
    }
    clean_and_serialize_body(raw_html, base)
}

/// Wraps `inner_html` in a synthetic `<div>` (so multiple top-level nodes
/// survive as siblings rather than being silently limited to the first),
/// strips UI chrome ([`NOISE_SELECTOR`]), rewrites relative URLs to
/// absolute, and returns `(plain_text, cleaned_inner_html)` — matching
/// Go's `body.Find(...).Remove()` + `urlutil.RewriteURLs` + `body.Html()`
/// sequence, but as a reparse of an independent fragment rather than an
/// in-place mutation (see this module's own doc for why).
fn clean_and_serialize_body(inner_html: &str, base: Option<&Url>) -> (String, String) {
    let mut fragment = Html::parse_fragment(&format!("<div>{inner_html}</div>"));
    let Some(wrapper_id) = fragment.select(&selector("div")).next().map(|el| el.id()) else {
        return (String::new(), String::new());
    };
    let noise_ids: Vec<_> = fragment
        .select(&selector(NOISE_SELECTOR))
        .map(|el| el.id())
        .collect();
    for id in noise_ids {
        if let Some(mut node) = fragment.tree.get_mut(id) {
            node.detach();
        }
    }
    if let Some(base) = base {
        rewrite_urls(&mut fragment, base);
    }
    let wrapper = ElementRef::wrap(fragment.tree.get(wrapper_id).expect("wrapper node"))
        .expect("wrapper is an element");
    let text = textutil::selection_text(&wrapper);
    let html_out = wrapper.inner_html();
    (text, html_out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(url: &str, html: &str) -> Document {
        let mut d = Document::new(url);
        d.html = Some(html.to_string());
        d
    }

    const DISCOURSE_MARKER: &str =
        r#"<meta id="data-discourse-setup" data-base-url="https://forum.example.com">"#;

    #[test]
    fn matches_only_discourse_topic_pages() {
        let ext = DiscourseExtractor::default();
        let discourse_html = format!("<html><head>{DISCOURSE_MARKER}</head><body></body></html>");
        let cases: &[(&str, bool)] = &[
            ("https://forum.example.com/t/some-topic/123", true),
            ("https://forum.example.com/t/123", true),
            ("https://forum.example.com/t/some-topic/123/5", true),
            (
                "https://forum.example.com/t/some-topic/123/reply-anchor",
                false,
            ),
            ("https://forum.example.com/t/some-topic", false),
            ("https://forum.example.com/c/category-name", false),
            ("https://forum.example.com/", false),
        ];
        for (url, want) in cases {
            let d = doc(url, &discourse_html);
            assert_eq!(ext.matches(&d), *want, "url = {url}");
        }
        // Same URL shape, but not actually Discourse-flavored HTML.
        let non_discourse = doc(
            "https://forum.example.com/t/some-topic/123",
            "<html><body>plain forum software</body></html>",
        );
        assert!(!ext.matches(&non_discourse));
    }

    #[test]
    fn extract_collects_preloaded_topic_json_and_rendered_replies() {
        let ext = DiscourseExtractor::default();
        // The `data-preloaded` payload is genuinely double-encoded: the
        // value for "topic_123" is itself a JSON string (escaped quotes)
        // containing the real topic object. Built as its own literal
        // (rather than inline in the `format!` template below) so its
        // braces don't need `{{`/`}}` escaping.
        let json_payload = r#"{"topic_123": "{\"title\":\"A durable parser\",\"tags\":[\"go\",\"parsing\"],\"post_stream\":{\"posts\":[{\"id\":1,\"post_number\":1,\"name\":\"Author Name\",\"username\":\"author\",\"cooked\":\"<p>Original post body.</p>\",\"created_at\":\"2026-08-12T10:00:00Z\"}]}}"}"#;
        let html = format!(
            r#"<html><head>{DISCOURSE_MARKER}</head><body>
            <script type="application/json" id="data-preloaded">{json_payload}</script>
            <div id="topic-title"><h1 data-topic-id="123"><a>A durable parser</a></h1></div>
            <div class="post-stream">
                <article data-post-id="2" data-post-number="2">
                    <div class="names"><span class="username"><a>replier</a></span></div>
                    <div class="cooked"><p>A reply <a href="/wiki/rules">with a link</a>.</p></div>
                </article>
            </div>
        </body></html>"#
        );
        let d = doc("https://forum.example.com/t/a-durable-parser/123", &html);

        let ExtractOutcome::Extracted(extracted) = ext.extract(&d) else {
            panic!("expected Extracted");
        };
        assert_eq!(extracted.title.as_deref(), Some("A durable parser"));
        let text = extracted.text.unwrap();
        assert!(text.contains("Original post body."));
        assert!(text.contains("A reply"));
        assert_eq!(
            extracted.metadata.get("topic_id"),
            Some(&Value::String("123".to_string()))
        );
        assert_eq!(
            extracted.metadata.get("posts"),
            Some(&Value::Number(Number::from(2u64)))
        );
        let tags = extracted.metadata.get("tags").unwrap();
        let Value::String(tags) = tags else {
            panic!("expected string")
        };
        assert!(tags.contains("go") && tags.contains("parsing"));
    }

    #[test]
    fn extract_falls_back_to_json_ld_qa_page() {
        let ext = DiscourseExtractor::default();
        let html = format!(
            r#"<html><head>{DISCOURSE_MARKER}
            <script type="application/ld+json">{{
                "@type": "QAPage",
                "name": "Schema topic",
                "mainEntity": {{
                    "@type": "Question",
                    "text": "Question body.",
                    "author": {{"name": "asker"}},
                    "acceptedAnswer": {{"@type": "Answer", "text": "Accepted answer body.", "author": "answerer", "url": "https://forum.example.com/t/schema-topic/456/2"}}
                }}
            }}</script>
            </head><body></body></html>"#
        );
        let d = doc("https://forum.example.com/t/schema-topic/456", &html);

        let ExtractOutcome::Extracted(extracted) = ext.extract(&d) else {
            panic!("expected Extracted");
        };
        assert_eq!(extracted.title.as_deref(), Some("Schema topic"));
        let text = extracted.text.unwrap();
        assert!(text.contains("Question body."));
        assert!(text.contains("Accepted answer body."));
        assert_eq!(
            extracted.metadata.get("accepted_answer"),
            Some(&Value::Number(Number::from(2i32)))
        );
    }

    #[test]
    fn extract_falls_back_when_topic_has_no_posts() {
        let ext = DiscourseExtractor::default();
        let html = format!("<html><head>{DISCOURSE_MARKER}</head><body></body></html>");
        let d = doc("https://forum.example.com/t/empty-topic/789", &html);
        assert!(matches!(ext.extract(&d), ExtractOutcome::Fallback(_)));
    }

    #[test]
    fn preview_renders_headings_and_rewrites_relative_links() {
        let ext = DiscourseExtractor::default();
        let html = format!(
            r#"<html><head>{DISCOURSE_MARKER}</head><body>
            <div id="topic-title"><h1 data-topic-id="123"><a>Links topic</a></h1></div>
            <div class="post-stream">
                <article data-post-id="1" data-post-number="1">
                    <div class="names"><span class="username"><a>alice</a></span></div>
                    <div class="cooked"><p>See <a href="/t/other-topic/1">this</a>.</p></div>
                </article>
                <article data-post-id="2" data-post-number="2">
                    <div class="names"><span class="username"><a>bob</a></span></div>
                    <div class="cooked"><p>A reply.</p></div>
                </article>
            </div>
        </body></html>"#
        );
        let d = doc("https://forum.example.com/t/links-topic/123", &html);

        let PreviewOutcome::Previewed(response) = ext.preview(&d) else {
            panic!("expected Previewed");
        };
        let content = response.html.unwrap();
        assert!(content.contains("Original post"));
        assert!(content.contains("Reply #2"));
        assert!(content.contains(r#"href="https://forum.example.com/t/other-topic/1""#));
    }

    #[test]
    fn preview_falls_back_for_a_non_discourse_page() {
        let ext = DiscourseExtractor::default();
        let d = doc(
            "https://forum.example.com/t/some-topic/123",
            "<html><body>not discourse</body></html>",
        );
        assert!(matches!(ext.preview(&d), PreviewOutcome::Fallback(_)));
    }
}
