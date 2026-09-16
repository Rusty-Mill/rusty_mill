//! `Twitter` — a Rust port of
//! `server/extractor/extractors/twitter/extractor.go` (capability
//! inventory §4.5.15). Decomposes a Twitter/X profile/feed/tweet page
//! into one [`Document`] per visible tweet, reusing the
//! `extra_documents`/`skip_indexing` mechanism [`crate::MastodonExtractor`]/
//! [`crate::BlueskyExtractor`] established. Unlike Bluesky's own
//! three-source merge, Twitter has only one real source (the rendered
//! DOM) plus a page-meta fallback, and candidates are deduped by URL
//! (first match wins) rather than merged field-by-field.
//!
//! [`rewrite_tco_links`] is this module's own trick, with no equivalent
//! in Mastodon/Bluesky: Twitter/X shortens every link in a tweet's body
//! through its own `t.co` redirector, so the *visible* rendered anchor
//! usually carries the real destination in a `data-expanded-url` or
//! `title` attribute (or, for older markup, in the anchor's own visible
//! text) rather than in `href` at all. This rewrites each `t.co` anchor's
//! `href` to that real destination and returns the `(short, original)`
//! pairs so [`replace_tweet_link_urls`] can also patch the plain-text
//! version of the tweet (which still contains the literal short URL).
//! One Go behavior isn't reproduced: `anchor.SetText(...)`, which
//! replaces a `t.co`-only anchor's *visible text* with the expanded URL
//! — cosmetic only (the more important `href` fix and the plain-text
//! substitution both still happen), and `scraper`'s tree has no cheap
//! child-replacement primitive the way `goquery.Selection.SetText` does.
//!
//! Go mutates the *same* live selection twice — first `t.co` links
//! within the tweet-text content specifically, then every remaining
//! relative URL across the whole post — before reading back the final
//! HTML. `scraper::ElementRef` has no in-place mutation API, so this
//! port reparses the whole post subtree as its own fragment up front
//! (`WikipediaExtractor::extract`'s clone trick) and performs both
//! mutation passes against that one copy, in the same order, before
//! serializing the result — a `NodeId` stays valid across both passes
//! since neither one detaches the tweet-text content node itself.

use crate::sanitizer::{sanitize_html, sanitize_text};
use crate::stackexchange::{element_text, html_escape, selector};
use crate::urlutil::{resolve_url, rewrite_urls};
use ammonia::Url;
use ego_tree::NodeId;
use html5ever::{LocalName, Namespace, QualName};
use rusty_hister_core::{
    Capabilities, Document, ExtractOutcome, Extractor, ExtractorConfig, HisterError, Metadata,
    PreviewOutcome, PreviewResponse,
};
use scraper::{ElementRef, Html, Node, Selector, StrTendril};
use std::collections::HashSet;

const TWEET_TYPE: &str = "tweet";
const TWITTER_HOSTS: [&str; 7] = [
    "twitter.com",
    "www.twitter.com",
    "mobile.twitter.com",
    "m.twitter.com",
    "x.com",
    "www.x.com",
    "mobile.x.com",
];

#[derive(Debug, Default)]
pub struct TwitterExtractor {
    config: ExtractorConfig,
}

impl Extractor for TwitterExtractor {
    fn name(&self) -> &str {
        "Twitter"
    }

    fn description(&self) -> &str {
        "Extracts tweets as individual documents from Twitter and X feeds, profiles, and tweet pages."
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            enrich: false,
            extract: true,
            preview: true,
        }
    }

    fn matches(&self, document: &Document) -> bool {
        if is_tweet_document(document) {
            return true;
        }
        let Ok(url) = Url::parse(&document.url) else {
            return false;
        };
        is_twitter_host(url.host_str().unwrap_or(""))
    }

    fn extract(&self, document: &Document) -> ExtractOutcome {
        if is_tweet_document(document) {
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

        let mut seen = HashSet::new();
        let mut found = false;
        for post in find_tweet_selections(&html) {
            let Some(tweet) = tweet_document(&post, &base) else {
                continue;
            };
            if !seen.insert(tweet.url.clone()) {
                continue;
            }
            found = true;
            extracted.extra_documents.push(tweet);
        }

        if !found {
            if let Some(tweet) = tweet_document_from_page(&html, &base) {
                extracted.extra_documents.push(tweet);
            }
        }
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

fn is_tweet_document(document: &Document) -> bool {
    document.metadata.get("type").and_then(|v| v.as_str()) == Some(TWEET_TYPE)
}

fn is_twitter_host(host: &str) -> bool {
    TWITTER_HOSTS.iter().any(|h| h.eq_ignore_ascii_case(host))
}

fn find_tweet_selections(html: &Html) -> Vec<ElementRef<'_>> {
    let sel = selector(
        r#"[itemtype$="/SocialMediaPosting"], article[data-tweet-id], article[data-testid="tweet"], article[role="article"], [data-testid="tweet"]"#,
    );
    html.select(&sel).collect()
}

fn tweet_document(post: &ElementRef, base: &Url) -> Option<Document> {
    let mut fragment = Html::parse_fragment(&format!("<div>{}</div>", post.inner_html()));
    let root = fragment.select(&selector("div")).next()?;

    let (name, mut handle) = tweet_author(&root);
    let (tweet_url, url_handle, ok) = tweet_status_url(&root, base, &handle);
    if !ok {
        return None;
    }
    if !url_handle.is_empty() {
        handle = url_handle;
    }

    let (raw_text, content_id) = tweet_text(&root);
    let links = match content_id {
        Some(id) => rewrite_tco_links(&mut fragment, id, base),
        None => Vec::new(),
    };
    rewrite_urls(&mut fragment, base);
    let text = replace_tweet_link_urls(&raw_text, &links);

    let root = fragment.select(&selector("div")).next()?;
    let content = content_id
        .and_then(|id| fragment.tree.get(id))
        .and_then(ElementRef::wrap);
    let html = tweet_content_html(&root, content.as_ref(), &text);
    let published = tweet_published(&root);
    let author = format_author(&name, &handle);

    let mut metadata = Metadata::new();
    metadata.insert("type".to_string(), TWEET_TYPE.into());
    if !author.is_empty() {
        metadata.insert("author".to_string(), author.clone().into());
    }
    if !handle.is_empty() {
        metadata.insert("handle".to_string(), format!("@{handle}").into());
    }
    if !published.is_empty() {
        metadata.insert("published".to_string(), published.into());
    }

    let title = if author.is_empty() {
        "Twitter tweet".to_string()
    } else {
        format!("Twitter tweet: {author}")
    };

    let mut doc = Document::new(tweet_url);
    doc.title = Some(title);
    doc.text = Some(text);
    doc.html = Some(html);
    doc.metadata = metadata;
    Some(doc)
}

/// Walks the tweet-text priority list, returning both the matched text
/// and the `NodeId` of whichever element it came from (so callers can
/// mutate the same fragment afterward without holding a borrow of it).
/// Matches Go's `tweetText`.
fn tweet_text(root: &ElementRef) -> (String, Option<NodeId>) {
    let mut semantic_text = String::new();
    for css in [
        r#"meta[itemprop="articleBody"]"#,
        r#"meta[itemprop="text"]"#,
    ] {
        for el in root.select(&selector(css)) {
            let value = el.value().attr("content").unwrap_or("").trim();
            if !value.is_empty() {
                semantic_text = value.to_string();
                break;
            }
        }
        if !semantic_text.is_empty() {
            break;
        }
    }

    let mut content: Option<(NodeId, String)> = None;
    for css in [
        r#"[data-testid="tweetText"]"#,
        r#"[itemprop="articleBody"]:not(meta)"#,
        r#"[itemprop="text"]:not(meta)"#,
        r#"[data-contents="true"]"#,
        r#"[lang][dir="auto"]"#,
        r#"[dir="auto"]"#,
    ] {
        for el in root.select(&selector(css)) {
            let candidate = element_text(&el).trim().to_string();
            if candidate.is_empty() || !related_tweet_text(&semantic_text, &candidate) {
                continue;
            }
            content = Some((el.id(), candidate));
            break;
        }
        if content.is_some() {
            break;
        }
    }

    match content {
        Some((id, text)) => (text, Some(id)),
        None if !semantic_text.is_empty() => (semantic_text, None),
        None => (String::new(), None),
    }
}

fn related_tweet_text(semantic_text: &str, candidate: &str) -> bool {
    if semantic_text.is_empty() {
        return true;
    }
    let semantic_norm = normalize_tweet_text_links(semantic_text);
    let candidate_norm = normalize_tweet_text_links(candidate);
    semantic_norm.contains(&candidate_norm) || candidate_norm.contains(&semantic_norm)
}

fn normalize_tweet_text_links(text: &str) -> String {
    text.split_whitespace()
        .map(|field| {
            let candidate = field.trim_matches(|c: char| ".,!?;:()[]{}<>\"'".contains(c));
            if is_tco_url(candidate) || parse_original_tweet_link_url(candidate, true).is_some() {
                field.replacen(candidate, "{url}", 1)
            } else {
                field.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

struct TweetLink {
    short_url: String,
    original_url: String,
}

/// Rewrites every `t.co`-shortened anchor within the tweet-text content
/// node to its real destination, returning the `(short, original)` pairs
/// found. Matches Go's `rewriteTweetLinks`, minus the cosmetic
/// visible-text replacement (see the module doc).
fn rewrite_tco_links(fragment: &mut Html, content_id: NodeId, base: &Url) -> Vec<TweetLink> {
    let anchor_ids: Vec<NodeId> = match fragment.tree.get(content_id).and_then(ElementRef::wrap) {
        Some(content) => content
            .select(&selector("a[href]"))
            .map(|el| el.id())
            .collect(),
        None => return Vec::new(),
    };

    let mut links = Vec::new();
    for id in anchor_ids {
        let Some((href, data_expanded_url, title_attr, text)) =
            fragment.tree.get(id).and_then(ElementRef::wrap).map(|el| {
                (
                    el.value().attr("href").unwrap_or("").trim().to_string(),
                    el.value()
                        .attr("data-expanded-url")
                        .unwrap_or("")
                        .trim()
                        .to_string(),
                    el.value().attr("title").unwrap_or("").trim().to_string(),
                    element_text(&el).trim().to_string(),
                )
            })
        else {
            continue;
        };

        let short_url = resolve_url(base, &href);
        if !is_tco_url(&short_url) {
            continue;
        }

        let original_url = parse_original_tweet_link_url(&data_expanded_url, false)
            .or_else(|| parse_original_tweet_link_url(&title_attr, false))
            .or_else(|| parse_original_tweet_link_url(&text, true));

        let Some(original_url) = original_url else {
            if let Some(mut node) = fragment.tree.get_mut(id) {
                if let Node::Element(el) = node.value() {
                    el.attrs.remove(&attr_name("href"));
                }
            }
            continue;
        };

        if let Some(mut node) = fragment.tree.get_mut(id) {
            if let Node::Element(el) = node.value() {
                el.attrs
                    .insert(attr_name("href"), StrTendril::from(original_url.clone()));
            }
        }
        links.push(TweetLink {
            short_url,
            original_url,
        });
    }
    links
}

fn attr_name(name: &str) -> QualName {
    QualName::new(None, Namespace::from(""), LocalName::from(name))
}

fn parse_original_tweet_link_url(raw: &str, allow_missing_scheme: bool) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty()
        || raw.chars().any(char::is_whitespace)
        || raw.contains('…')
        || raw.contains("...")
    {
        return None;
    }
    let candidate = if allow_missing_scheme && !raw.contains("://") {
        format!("https://{raw}")
    } else {
        raw.to_string()
    };
    let url = Url::parse(&candidate).ok()?;
    if (url.scheme() != "http" && url.scheme() != "https")
        || !url.host_str().unwrap_or("").contains('.')
        || is_tco_url(url.as_str())
    {
        return None;
    }
    Some(url.to_string())
}

fn replace_tweet_link_urls(text: &str, links: &[TweetLink]) -> String {
    let mut result = text.to_string();
    for link in links {
        if !link.short_url.is_empty() {
            result = result.replace(&link.short_url, &link.original_url);
        }
    }
    result
}

fn is_tco_url(raw: &str) -> bool {
    let Ok(url) = Url::parse(raw) else {
        return false;
    };
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    host == "t.co" || host == "www.t.co"
}

fn tweet_content_html(root: &ElementRef, content: Option<&ElementRef>, text: &str) -> String {
    let mut out = String::new();
    if let Some(content) = content {
        let h = content.inner_html();
        if !h.trim().is_empty() {
            out.push_str(&h);
        }
    }
    if out.is_empty() && !text.is_empty() {
        out.push_str(&paragraph_html(text));
    }

    let media_sel = selector(
        r#"[data-testid="tweetPhoto"] img, img[src*="pbs.twimg.com/media/"], video[poster]"#,
    );
    let media: Vec<ElementRef> = root.select(&media_sel).collect();
    if !media.is_empty() {
        out.push_str("<figure>");
        for el in media {
            out.push_str(&el.html());
        }
        out.push_str("</figure>");
    }
    out
}

fn tweet_author(root: &ElementRef) -> (String, String) {
    if let Some(author) = root.select(&selector(r#"[itemprop="author"]"#)).next() {
        let name = first_meta_content(&author, r#"meta[itemprop="name"]"#);
        let handle = first_meta_content(&author, r#"meta[itemprop="alternateName"]"#);
        let handle = handle.strip_prefix('@').unwrap_or(&handle).to_string();
        if !name.is_empty() || !handle.is_empty() {
            return clean_author(&name, &handle);
        }
    }

    let mut name = String::new();
    let mut handle = String::new();
    if let Some(user_name) = root
        .select(&selector(r#"[data-testid="User-Name"]"#))
        .next()
    {
        for a in user_name.select(&selector("a")) {
            let text = element_text(&a).trim().to_string();
            if let Some(text_handle) = text.strip_prefix('@') {
                if handle.is_empty() {
                    handle = text_handle.to_string();
                }
            } else if !text.is_empty() && name.is_empty() {
                name = text;
            }
            if !name.is_empty() && !handle.is_empty() {
                break;
            }
        }
    }

    if name.is_empty() || handle.is_empty() {
        for a in root.select(&selector("a[href]")) {
            let text = element_text(&a).trim().to_string();
            let Some(profile_handle) = twitter_profile_handle(a.value().attr("href").unwrap_or(""))
            else {
                continue;
            };
            if text.is_empty() {
                continue;
            }
            if let Some(text_handle) = text.strip_prefix('@') {
                if handle.is_empty() {
                    handle = text_handle.to_string();
                }
            } else if name.is_empty()
                && (handle.is_empty() || handle.eq_ignore_ascii_case(&profile_handle))
            {
                name = text;
            }
            if handle.is_empty() {
                handle = profile_handle;
            }
            if !name.is_empty() && !handle.is_empty() {
                break;
            }
        }
    }
    clean_author(&name, &handle)
}

fn first_meta_content(el: &ElementRef, css: &str) -> String {
    el.select(&selector(css))
        .next()
        .and_then(|m| m.value().attr("content"))
        .unwrap_or("")
        .trim()
        .to_string()
}

fn twitter_profile_handle(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let path = if let Ok(url) = Url::parse(trimmed) {
        if !is_twitter_host(url.host_str().unwrap_or("")) {
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
    if parts.len() != 1 {
        return None;
    }
    let handle = percent_encoding::percent_decode_str(parts[0])
        .decode_utf8()
        .ok()?;
    if valid_handle(&handle) {
        Some(handle.to_string())
    } else {
        None
    }
}

fn clean_author(name: &str, handle: &str) -> (String, String) {
    let name = sanitize_text(name);
    let mut handle = sanitize_text(handle);
    if !valid_handle(&handle) {
        handle.clear();
    }
    (name, handle)
}

fn tweet_published(root: &ElementRef) -> String {
    let v = first_meta_content(root, r#"meta[itemprop="datePublished"]"#);
    if !v.is_empty() {
        return v;
    }
    let v = first_meta_content(root, r#"meta[itemprop="dateCreated"]"#);
    if !v.is_empty() {
        return v;
    }
    root.select(&selector("time[datetime]"))
        .next()
        .and_then(|t| t.value().attr("datetime"))
        .unwrap_or("")
        .trim()
        .to_string()
}

fn closest<'a>(el: ElementRef<'a>, sel: &Selector) -> Option<ElementRef<'a>> {
    if sel.matches(&el) {
        return Some(el);
    }
    for ancestor in el.ancestors() {
        if let Some(ancestor_el) = ElementRef::wrap(ancestor) {
            if sel.matches(&ancestor_el) {
                return Some(ancestor_el);
            }
        }
    }
    None
}

fn tweet_status_url(root: &ElementRef, base: &Url, handle: &str) -> (String, String, bool) {
    let mut candidates: Vec<String> = Vec::new();
    for el in root.select(&selector(r#"meta[itemprop="url"]"#)) {
        candidates.push(el.value().attr("content").unwrap_or("").to_string());
    }
    for el in root.select(&selector(r#"link[itemprop="url"]"#)) {
        candidates.push(el.value().attr("href").unwrap_or("").to_string());
    }
    if let Some(item_id) = root.value().attr("itemid") {
        candidates.push(item_id.to_string());
    }
    let anchor_href_sel = selector("a[href]");
    for time_el in root.select(&selector("time")) {
        if let Some(anchor) = closest(time_el, &anchor_href_sel) {
            if let Some(href) = anchor.value().attr("href") {
                candidates.push(href.to_string());
            }
        }
    }
    for el in root.select(&selector(r#"a[href*="/status/"]"#)) {
        candidates.push(el.value().attr("href").unwrap_or("").to_string());
    }

    for candidate in &candidates {
        if let Some((mut status_url, mut url_handle)) = canonical_status_url(candidate, base) {
            if url_handle.is_empty() && valid_handle(handle) {
                status_url = status_url_for(handle, &status_id(&status_url));
                url_handle = handle.to_string();
            }
            return (status_url, url_handle, true);
        }
    }

    if let Some(id) = root.value().attr("data-tweet-id") {
        if valid_status_id(id) {
            if valid_handle(handle) {
                return (status_url_for(handle, id), handle.to_string(), true);
            }
            return (status_url_for("", id), String::new(), true);
        }
    }
    if let Some((status_url, url_handle)) = canonical_status_url(base.as_str(), base) {
        return (status_url, url_handle, true);
    }
    (String::new(), String::new(), false)
}

/// Validates `raw` is (or resolves to) a `.../status/<id>` Twitter/X URL
/// and returns its canonical `https://x.com/...` form plus the decoded
/// handle when the URL carries one (`/handle/status/id`, as opposed to
/// the handle-less `/i/status/id` shape). Matches Go's
/// `canonicalStatusURL`.
fn canonical_status_url(raw: &str, base: &Url) -> Option<(String, String)> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let resolved = resolve_url(base, raw);
    let url = Url::parse(&resolved).ok()?;
    if !is_twitter_host(url.host_str().unwrap_or("")) {
        return None;
    }

    let parts: Vec<&str> = url
        .path()
        .trim_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    for (i, part) in parts.iter().enumerate() {
        if *part != "status" || i + 1 >= parts.len() {
            continue;
        }
        let id = percent_encoding::percent_decode_str(parts[i + 1])
            .decode_utf8()
            .ok()?;
        if !valid_status_id(&id) {
            return None;
        }
        let mut handle = String::new();
        if i == 1 {
            if let Ok(decoded) = percent_encoding::percent_decode_str(parts[0]).decode_utf8() {
                if valid_handle(&decoded) {
                    handle = decoded.to_string();
                }
            }
        }
        return Some((status_url_for(&handle, &id), handle));
    }
    None
}

fn valid_status_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_digit())
}

fn valid_handle(handle: &str) -> bool {
    if handle.is_empty() || handle.eq_ignore_ascii_case("i") || handle.eq_ignore_ascii_case("web") {
        return false;
    }
    handle
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn status_url_for(handle: &str, id: &str) -> String {
    if handle.is_empty() {
        format!("https://x.com/i/status/{id}")
    } else {
        format!("https://x.com/{handle}/status/{id}")
    }
}

fn status_id(status_url: &str) -> String {
    status_url
        .trim_matches('/')
        .split('/')
        .next_back()
        .unwrap_or("")
        .to_string()
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

fn tweet_document_from_page(html: &Html, base: &Url) -> Option<Document> {
    let (mut status_url, mut handle) = canonical_status_url(base.as_str(), base)?;

    let text = first_page_meta(
        html,
        &[
            r#"meta[property="og:description"]"#,
            r#"meta[name="twitter:description"]"#,
            r#"meta[name="description"]"#,
        ],
    );
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
    if !title_handle.is_empty() {
        handle = title_handle;
        status_url = status_url_for(&handle, &status_id(&status_url));
    }
    if handle.is_empty() {
        let author_url = first_page_meta(html, &[r#"meta[property="article:author"]"#]);
        if !author_url.is_empty() {
            if let Ok(u) = Url::parse(&author_url) {
                if is_twitter_host(u.host_str().unwrap_or("")) {
                    let candidate = u.path().trim_matches('/');
                    if valid_handle(candidate) {
                        handle = candidate.to_string();
                        status_url = status_url_for(&handle, &status_id(&status_url));
                    }
                }
            }
        }
    }
    let author = format_author(&name, &handle);

    let mut metadata = Metadata::new();
    metadata.insert("type".to_string(), TWEET_TYPE.into());
    if !author.is_empty() {
        metadata.insert("author".to_string(), author.clone().into());
    }
    if !handle.is_empty() {
        metadata.insert("handle".to_string(), format!("@{handle}").into());
    }
    let published = first_page_meta(html, &[r#"meta[property="article:published_time"]"#]);
    if !published.is_empty() {
        metadata.insert("published".to_string(), published.into());
    }
    let image = first_page_meta(
        html,
        &[
            r#"meta[property="og:image"]"#,
            r#"meta[name="twitter:image"]"#,
        ],
    );
    if !image.is_empty() {
        metadata.insert("image".to_string(), image.clone().into());
    }

    let title = if author.is_empty() {
        "Twitter tweet".to_string()
    } else {
        format!("Twitter tweet: {author}")
    };

    let mut doc = Document::new(status_url);
    doc.title = Some(title);
    doc.text = Some(text.clone());
    doc.html = Some(page_tweet_html(&text, &image, base));
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
    let mut title = title.trim().to_string();
    for marker in [" on X:", " on Twitter:"] {
        if let Some((before, _)) = title.split_once(marker) {
            title = before.to_string();
            break;
        }
    }
    for suffix in [" on X", " on Twitter"] {
        if let Some(stripped) = title.strip_suffix(suffix) {
            title = stripped.to_string();
        }
    }
    if let Some(open) = title.rfind(" (@") {
        if title.ends_with(')') {
            let name = title[..open].trim().to_string();
            let handle = title[open + 3..title.len() - 1].to_string();
            if valid_handle(&handle) {
                return (sanitize_text(&name), sanitize_text(&handle));
            }
        }
    }
    (sanitize_text(&title), String::new())
}

fn paragraph_html(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n");
    let escaped = html_escape(&normalized);
    format!("<p>{}</p>", escaped.replace('\n', "<br>"))
}

fn page_tweet_html(text: &str, image: &str, base: &Url) -> String {
    let mut out = paragraph_html(text);
    let resolved = resolve_url(base, image.trim());
    if let Ok(url) = Url::parse(&resolved) {
        if (url.scheme() == "http" || url.scheme() == "https")
            && !url.host_str().unwrap_or("").is_empty()
        {
            out.push_str(&format!(
                r#"<figure><img src="{}" alt=""></figure>"#,
                html_escape(&resolved)
            ));
        }
    }
    out
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
    fn matches_twitter_and_x_hosts_and_tweet_documents() {
        let ext = TwitterExtractor::default();
        assert!(ext.matches(&doc("https://twitter.com/jane/status/123", "")));
        assert!(ext.matches(&doc("https://x.com/jane/status/123", "")));
        assert!(!ext.matches(&doc("https://example.com/jane/status/123", "")));

        let mut tweet = doc("https://x.com/jane/status/123", "");
        tweet
            .metadata
            .insert("type".to_string(), "tweet".to_string().into());
        assert!(ext.matches(&tweet));
    }

    const RENDERED_HTML: &str = r#"<html><body>
        <article data-testid="tweet">
            <div data-testid="User-Name">
                <a href="/jane">Jane Doe</a>
                <a href="/jane">@jane</a>
            </div>
            <div data-testid="tweetText">Check this out <a href="https://t.co/abcd1234" data-expanded-url="https://example.com/real-article">https://t.co/abcd1234</a></div>
            <time datetime="2024-03-03T00:00:00Z"></time>
            <a href="/jane/status/998877"><time></time></a>
        </article>
    </body></html>"#;

    #[test]
    fn extract_collects_a_rendered_tweet_and_rewrites_tco_links() {
        let ext = TwitterExtractor::default();
        let d = doc("https://x.com/jane", RENDERED_HTML);
        let ExtractOutcome::Extracted(extracted) = ext.extract(&d) else {
            panic!("expected Extracted");
        };
        assert!(extracted.skip_indexing);
        assert_eq!(extracted.extra_documents.len(), 1);
        let tweet = &extracted.extra_documents[0];
        assert_eq!(tweet.url, "https://x.com/jane/status/998877");
        assert!(tweet
            .text
            .as_deref()
            .unwrap()
            .contains("https://example.com/real-article"));
        assert!(!tweet.text.as_deref().unwrap().contains("t.co"));
        assert!(tweet
            .html
            .as_deref()
            .unwrap()
            .contains(r#"href="https://example.com/real-article""#));
        assert_eq!(
            tweet.metadata.get("handle").and_then(|v| v.as_str()),
            Some("@jane")
        );
    }

    #[test]
    fn extract_falls_back_to_page_meta_when_nothing_else_matches() {
        let ext = TwitterExtractor::default();
        let html = r#"<html><head>
            <meta property="og:title" content="Jane Doe (@jane)">
            <meta property="og:description" content="A single tweet page description.">
        </head><body></body></html>"#;
        let d = doc("https://x.com/jane/status/555444", html);
        let ExtractOutcome::Extracted(extracted) = ext.extract(&d) else {
            panic!("expected Extracted");
        };
        assert_eq!(extracted.extra_documents.len(), 1);
        let tweet = &extracted.extra_documents[0];
        assert_eq!(tweet.url, "https://x.com/jane/status/555444");
        assert_eq!(
            tweet.text.as_deref(),
            Some("A single tweet page description.")
        );
    }

    #[test]
    fn extract_returns_tweet_document_unchanged_for_recursion_guard() {
        let ext = TwitterExtractor::default();
        let mut tweet = doc("https://x.com/jane/status/123", "");
        tweet
            .metadata
            .insert("type".to_string(), "tweet".to_string().into());
        tweet.text = Some("already extracted".to_string());

        let ExtractOutcome::Extracted(extracted) = ext.extract(&tweet) else {
            panic!("expected Extracted");
        };
        assert!(!extracted.skip_indexing);
        assert_eq!(extracted.text.as_deref(), Some("already extracted"));
    }

    #[test]
    fn extract_deduplicates_tweets_with_the_same_canonical_url() {
        let ext = TwitterExtractor::default();
        let html = r#"<html><body>
            <article role="article" data-testid="tweet">
                <div data-testid="tweetText">First copy</div>
                <a href="/jane/status/42"><time></time></a>
            </article>
            <article data-testid="tweet">
                <div data-testid="tweetText">Second copy, same URL</div>
                <a href="/jane/status/42"><time></time></a>
            </article>
        </body></html>"#;
        let d = doc("https://x.com/jane", html);
        let ExtractOutcome::Extracted(extracted) = ext.extract(&d) else {
            panic!("expected Extracted");
        };
        assert_eq!(extracted.extra_documents.len(), 1);
    }

    #[test]
    fn preview_renders_title_and_html_or_falls_back_to_text() {
        let ext = TwitterExtractor::default();
        let mut d = doc("https://x.com/jane/status/123", "");
        d.title = Some("Twitter tweet: Jane Doe (@jane)".to_string());
        d.html = Some("<p>Hello world.</p>".to_string());
        let PreviewOutcome::Previewed(response) = ext.preview(&d) else {
            panic!("expected Previewed");
        };
        let content = response.html.unwrap();
        assert!(content.contains("<h2>Twitter tweet: Jane Doe (@jane)</h2>"));
        assert!(content.contains("<p>Hello world.</p>"));
    }
}
