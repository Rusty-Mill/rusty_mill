//! `Reddit` — a Rust port of `server/extractor/extractors/reddit/reddit.go`
//! (capability inventory §4.5.6). Extract and preview for Reddit post pages.
//!
//! Reddit has shipped at least three different markups for the same post
//! over the years — the modern `shreddit-*` web components, the legacy
//! `old.reddit.com` DOM, and a `schema.org` JSON-LD block many pages embed
//! regardless of which HTML renders — and a crawled page could be any of
//! them. Go's [`find_post`]/[`comment_body`]/etc. cope with this the same
//! way throughout: an ordered list of CSS-selector candidates, first
//! non-empty/first-match wins ([`first_value`], [`first_attr`],
//! [`first_selection_text`], [`first_selection_attr`], [`first_page_meta`]).
//! This port keeps that shape rather than branching on "which Reddit era is
//! this" up front, matching Go exactly.
//!
//! [`crate::textutil::selection_text`] is the crate's third real caller
//! (Go itself shares `textutil` across `hackernews`/`discourse`/`reddit`).
//! Two things `scraper::ElementRef`'s read-only API needs a workaround for,
//! both already established by earlier extractors:
//! - URL rewriting on a specific subtree (a post/comment body) rather than
//!   the whole document: [`rewritten_inner_html`] re-parses the subtree's
//!   own HTML as a standalone `Html::parse_fragment`, runs
//!   [`crate::urlutil::rewrite_urls`] on *that*, and serializes it back —
//!   the same "reparse as its own document" trick `WikipediaExtractor`'s
//!   `Extract` uses for its noise-removal clone.
//! - [`fallback_comment_text`] needs Go's `clone.Find(sel).Remove()`
//!   (used when no dedicated comment-body element exists, so the comment's
//!   own text has to be read from its whole subtree minus UI chrome and
//!   minus any *nested* reply, which would otherwise double-count that
//!   reply's text): [`build_filtered_fragment`] copies only the kept nodes
//!   into a fresh `ego_tree` fragment, the same approach
//!   `ChatGptExtractor` uses for its own content-cleaning.

use crate::sanitizer::{sanitize_html, sanitize_text};
use crate::stackexchange::{html_escape, selector};
use crate::textutil;
use crate::urlutil::{resolve_url, rewrite_urls};
use ammonia::Url;
use ego_tree::NodeMut;
use rusty_hister_core::{
    Capabilities, Document, ExtractOutcome, Extractor, ExtractorConfig, HisterError,
    PreviewOutcome, PreviewResponse,
};
use rusty_json::{Map, Number, Value};
use scraper::{ElementRef, Html, Node};

#[derive(Debug, Default)]
pub struct RedditExtractor {
    config: ExtractorConfig,
}

#[derive(Debug, Default, Clone)]
struct RedditPost {
    title: String,
    author: String,
    subreddit: String,
    score: String,
    published: String,
    flair: String,
    link: String,
    body_text: String,
    body_html: String,
    post_id: String,
    comment_count: String,
    comments: Vec<RedditComment>,
}

#[derive(Debug, Default, Clone)]
struct RedditComment {
    id: String,
    author: String,
    score: String,
    published: String,
    text: String,
    html: String,
    depth: i32,
}

impl Extractor for RedditExtractor {
    fn name(&self) -> &str {
        "Reddit"
    }

    fn description(&self) -> &str {
        "Extracts a Reddit post and every comment already present on its post page."
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            enrich: false,
            extract: true,
            preview: true,
        }
    }

    fn matches(&self, document: &Document) -> bool {
        reddit_post_url(&document.url).is_some()
    }

    fn extract(&self, document: &Document) -> ExtractOutcome {
        let post = match parse_reddit_post(document) {
            Ok(post) => post,
            Err(message) => return ExtractOutcome::Fallback(HisterError::Extraction(message)),
        };

        let mut extracted = document.clone();
        if !post.title.is_empty() {
            extracted.title = Some(post.title.clone());
        }
        extracted.text = Some(reddit_post_text(&post));
        extracted
            .metadata
            .insert("type".to_string(), Value::String("reddit".to_string()));
        set_metadata(&mut extracted, "author", &post.author);
        set_metadata(&mut extracted, "subreddit", &post.subreddit);
        set_metadata(&mut extracted, "published", &post.published);
        set_metadata(&mut extracted, "score", &post.score);
        set_metadata(&mut extracted, "flair", &post.flair);
        set_metadata(&mut extracted, "link", &post.link);
        set_metadata(&mut extracted, "post_id", &post.post_id);
        extracted.metadata.insert(
            "comments".to_string(),
            Value::Number(Number::from(post.comments.len())),
        );
        ExtractOutcome::Extracted(extracted)
    }

    fn preview(&self, document: &Document) -> PreviewOutcome {
        let post = match parse_reddit_post(document) {
            Ok(post) => post,
            Err(message) => return PreviewOutcome::Fallback(HisterError::Extraction(message)),
        };

        let mut out = String::new();
        if !post.title.is_empty() {
            out.push_str(&format!(
                r#"<h2><a href="{}">{}</a></h2>"#,
                html_escape(&document.url),
                html_escape(&post.title)
            ));
        }
        let mut parts = Vec::new();
        if !post.author.is_empty() {
            parts.push(format!(
                "submitted by <strong>{}</strong>",
                html_escape(&post.author)
            ));
        }
        if !post.subreddit.is_empty() {
            parts.push(format!("to {}", html_escape(&post.subreddit)));
        }
        if !post.score.is_empty() {
            parts.push(format!("{} points", html_escape(&post.score)));
        }
        if !post.published.is_empty() {
            parts.push(html_escape(&post.published));
        }
        if !post.flair.is_empty() {
            parts.push(format!("flair: {}", html_escape(&post.flair)));
        }
        if !parts.is_empty() {
            out.push_str(&format!("<p>{}</p>", parts.join(" &middot; ")));
        }
        if !post.body_html.is_empty() {
            out.push_str(&post.body_html);
        }
        if !post.link.is_empty() {
            out.push_str(&format!(
                r#"<p><a href="{}">View linked content</a></p>"#,
                html_escape(&post.link)
            ));
        }
        if !post.comments.is_empty() {
            out.push_str("<hr><h2>Comments</h2>");
            let base_depth = post.comments.iter().map(|c| c.depth).min().unwrap_or(0);
            for comment in &post.comments {
                let depth = (comment.depth - base_depth).clamp(0, 12);
                out.push_str(&format!(r#"<div style="margin-left:{}em">"#, depth * 2));
                let mut comment_parts = Vec::new();
                if !comment.author.is_empty() {
                    comment_parts
                        .push(format!("<strong>{}</strong>", html_escape(&comment.author)));
                }
                if !comment.score.is_empty() {
                    comment_parts.push(format!("{} points", html_escape(&comment.score)));
                }
                if !comment.published.is_empty() {
                    comment_parts.push(html_escape(&comment.published));
                }
                if !comment_parts.is_empty() {
                    out.push_str(&format!("<p>{}</p>", comment_parts.join(" &middot; ")));
                }
                out.push_str(&comment.html);
                out.push_str("</div>");
            }
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

/// Recognizes a Reddit post-page URL, returning its `(post_id, subreddit)`
/// on success. `subreddit` is only known from a `/r/<subreddit>/comments/...`
/// URL shape; the shorter shapes leave it empty for [`parse_post_element`]
/// to fill in from the page itself.
fn reddit_post_url(raw_url: &str) -> Option<(String, String)> {
    let url = Url::parse(raw_url).ok()?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return None;
    }
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    let host = host.strip_suffix('.').unwrap_or(&host);
    let parts = path_parts(url.path());

    if host == "redd.it" {
        return match parts.as_slice() {
            [id] if valid_reddit_id(id) => Some((id.to_ascii_lowercase(), String::new())),
            _ => None,
        };
    }
    if host != "reddit.com" && !host.ends_with(".reddit.com") {
        return None;
    }

    let trimmed_path = url.path().trim_end_matches('/').to_ascii_lowercase();
    if [".json", ".rss", ".xml"]
        .iter()
        .any(|suffix| trimmed_path.ends_with(suffix))
    {
        return None;
    }

    match parts.as_slice() {
        [r, subreddit, comments, id, ..]
            if r.eq_ignore_ascii_case("r")
                && comments.eq_ignore_ascii_case("comments")
                && valid_reddit_id(id) =>
        {
            Some((id.to_ascii_lowercase(), subreddit.clone()))
        }
        [comments, id, ..] if comments.eq_ignore_ascii_case("comments") && valid_reddit_id(id) => {
            Some((id.to_ascii_lowercase(), String::new()))
        }
        _ => None,
    }
}

fn path_parts(path: &str) -> Vec<String> {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        return Vec::new();
    }
    trimmed.split('/').map(str::to_string).collect()
}

fn valid_reddit_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric())
}

fn parse_reddit_post(document: &Document) -> Result<RedditPost, String> {
    let Some((post_id, subreddit)) = reddit_post_url(&document.url) else {
        return Err("not a Reddit post page".to_string());
    };
    let html_str = document.html.as_deref().unwrap_or("");
    let html = Html::parse_document(html_str);
    let structured = structured_reddit_post(&html);
    let root = find_post(&html, &post_id);
    if root.is_none() && structured.is_none() {
        return Err("no Reddit post found".to_string());
    }

    let mut post = RedditPost {
        post_id: post_id.clone(),
        subreddit,
        ..Default::default()
    };
    let base = Url::parse(&document.url).ok();
    if let Some(root) = root {
        parse_post_element(&mut post, root, &html, base.as_ref());
    }
    post.comments = parse_comments(&html, base.as_ref());
    merge_structured_post(&mut post, structured);

    post.subreddit = normalize_subreddit(&post.subreddit);
    post.title = clean_reddit_title(&post.title, &post.subreddit);
    post.link = meaningful_post_link(&post.link, &document.url, &post.post_id);
    if post.title.is_empty()
        && post.body_text.is_empty()
        && post.link.is_empty()
        && post.comments.is_empty()
    {
        return Err("reddit post has no content".to_string());
    }
    Ok(post)
}

const POST_SELECTORS: &[&str] = &[
    "shreddit-post",
    r#"[data-fullname^="t3_"]"#,
    r#"[data-testid="post-container"]"#,
    r#"[itemtype$="/DiscussionForumPosting"]"#,
    r#"[itemtype$="/SocialMediaPosting"]"#,
    "main article",
];

fn find_post<'a>(html: &'a Html, post_id: &str) -> Option<ElementRef<'a>> {
    let title_probe = selector(r#"h1[slot="title"], [itemprop="headline"], p.title a.title"#);
    for css in POST_SELECTORS {
        let candidates: Vec<ElementRef> = html.select(&selector(css)).collect();
        if candidates.is_empty() {
            continue;
        }
        let mut context_candidate = None;
        let mut title_candidate = None;
        let mut exact_candidate = None;
        for candidate in &candidates {
            if !post_id.is_empty() && selection_post_id(candidate) == post_id {
                exact_candidate = Some(*candidate);
                break;
            }
            if candidate
                .value()
                .attr("view-context")
                .is_some_and(|v| v.eq_ignore_ascii_case("CommentsPage"))
            {
                context_candidate = Some(*candidate);
            }
            if title_candidate.is_none() && candidate.select(&title_probe).next().is_some() {
                title_candidate = Some(*candidate);
            }
        }
        if let Some(c) = exact_candidate {
            return Some(c);
        }
        if let Some(c) = context_candidate {
            return Some(c);
        }
        if candidates.len() == 1 {
            return Some(candidates[0]);
        }
        if let Some(c) = title_candidate {
            return Some(c);
        }
    }
    None
}

fn selection_post_id(s: &ElementRef) -> String {
    for attr in ["post-id", "postid", "data-post-id"] {
        let id = normalize_thing_id(s.value().attr(attr).unwrap_or(""), "t3_");
        if !id.is_empty() {
            return id;
        }
    }
    for attr in ["id", "thingid", "thing-id", "data-fullname"] {
        let value = s
            .value()
            .attr(attr)
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if !value.starts_with("t3_") {
            continue;
        }
        let id = normalize_thing_id(&value, "t3_");
        if !id.is_empty() {
            return id;
        }
    }
    for attr in ["permalink", "data-permalink"] {
        let raw = s.value().attr(attr).unwrap_or("").trim();
        if raw.is_empty() {
            continue;
        }
        // Go's `url.Parse` happily parses a bare path like this permalink
        // usually is; `url::Url::parse` requires a scheme, so an absolute
        // URL is parsed normally and anything else is treated as already
        // being the path.
        let path = Url::parse(raw)
            .map(|u| u.path().to_string())
            .unwrap_or_else(|_| raw.to_string());
        if let Some((id, _)) = reddit_post_url(&format!("https://reddit.com{path}")) {
            return id;
        }
    }
    String::new()
}

fn normalize_thing_id(id: &str, prefix: &str) -> String {
    let lower = id.trim().to_ascii_lowercase();
    let stripped = if let Some(rest) = lower.strip_prefix(prefix) {
        rest.to_string()
    } else if lower.starts_with("t1_") || lower.starts_with("t2_") || lower.starts_with("t3_") {
        return String::new();
    } else {
        lower
    };
    if !valid_reddit_id(&stripped) {
        return String::new();
    }
    stripped
}

fn parse_post_element(post: &mut RedditPost, root: ElementRef, html: &Html, base: Option<&Url>) {
    post.title = first_value(&[
        first_attr(root, &["post-title", "data-title"]),
        first_selection_text(
            root,
            &[
                r#"h1[slot="title"]"#,
                r#"[data-testid="post-title"]"#,
                r#"[itemprop="headline"]"#,
                "p.title a.title",
                "h1",
            ],
        ),
        first_page_meta(
            html,
            &[
                r#"meta[property="og:title"]"#,
                r#"meta[name="twitter:title"]"#,
            ],
        ),
        html.select(&selector("title"))
            .next()
            .map(|el| textutil::selection_text(&el))
            .unwrap_or_default(),
    ]);
    post.author = first_value(&[
        first_attr(root, &["author", "data-author"]),
        first_selection_text(
            root,
            &[
                r#"[slot="authorName"]"#,
                r#"[noun="post_author"]"#,
                r#"[data-testid="post_author_link"]"#,
                ".tagline .author",
                r#"[rel="author"]"#,
            ],
        ),
    ]);
    post.subreddit = first_value(&[
        first_attr(
            root,
            &[
                "subreddit-prefixed-name",
                "data-subreddit-prefixed-name",
                "subreddit",
            ],
        ),
        post.subreddit.clone(),
    ]);
    post.score = first_value(&[
        first_attr(root, &["score", "data-score"]),
        first_selection_text(
            root,
            &[
                r#"[data-testid="post-score"]"#,
                r#"[data-click-id="score"]"#,
                ".score.unvoted",
                r#"[itemprop="upvoteCount"]"#,
            ],
        ),
    ]);
    post.comment_count = first_attr(
        root,
        &["comment-count", "data-comment-count", "num-comments"],
    );
    post.published = first_value(&[
        first_attr(root, &["created-timestamp", "date-created", "published"]),
        first_selection_attr(
            root,
            "datetime",
            &["time[datetime]", r#"[itemprop="datePublished"]"#],
        ),
    ]);
    post.flair = first_value(&[
        first_attr(root, &["post-flair", "post-flair-text", "data-flair"]),
        first_selection_text(
            root,
            &[
                "shreddit-post-flair",
                r#"[slot="post-flair"]"#,
                ".linkflairlabel",
            ],
        ),
    ]);

    if let Some(body) = first_body_selection(root) {
        post.body_text = textutil::selection_text(&body);
        post.body_html = rewritten_inner_html(&body, base);
    }
    if post.body_text.is_empty() {
        post.body_text = first_page_meta(
            root,
            &[
                r#"meta[itemprop="articleBody"]"#,
                r#"meta[itemprop="text"]"#,
            ],
        );
        if !post.body_text.is_empty() {
            post.body_html = paragraph_html(&post.body_text);
        }
    }

    post.link = first_value(&[
        first_attr(root, &["content-href", "data-url", "url"]),
        first_selection_attr(
            root,
            "href",
            &[
                "p.title a.title",
                r#"a[data-click-id="body"]"#,
                r#"[slot="outbound-link"]"#,
            ],
        ),
    ]);
    post.link = match base {
        Some(base) => resolve_url(base, post.link.trim()),
        None => post.link.trim().to_string(),
    };
    let id = selection_post_id(&root);
    if !id.is_empty() {
        post.post_id = id;
    }
}

const BODY_SELECTORS: &[&str] = &[
    r#"[slot="text-body"]"#,
    "shreddit-post-text-body",
    r#"[data-testid="post-content"]"#,
    r#"[data-click-id="text"]"#,
    ".usertext-body .md",
    r#"[itemprop="articleBody"]:not(meta)"#,
    r#"[itemprop="text"]:not(meta)"#,
];

fn first_body_selection(root: ElementRef<'_>) -> Option<ElementRef<'_>> {
    for css in BODY_SELECTORS {
        for candidate in root.select(&selector(css)) {
            let has_media = candidate
                .select(&selector("img, video, audio"))
                .next()
                .is_some();
            if textutil::selection_text(&candidate).is_empty() && !has_media {
                continue;
            }
            return Some(candidate);
        }
    }
    None
}

const COMMENT_ROOT_SELECTORS: &[&str] = &[
    "shreddit-comment",
    r#"[data-fullname^="t1_"]"#,
    r#"[data-testid="comment"]"#,
    r#"[itemtype$="/Comment"]"#,
];

fn parse_comments(html: &Html, base: Option<&Url>) -> Vec<RedditComment> {
    let scope = comment_scope(html);
    for css in COMMENT_ROOT_SELECTORS {
        let roots: Vec<ElementRef> = match scope {
            Some(scope) => scope.select(&selector(css)).collect(),
            None => html.select(&selector(css)).collect(),
        };
        if roots.is_empty() {
            continue;
        }
        return roots
            .into_iter()
            .filter_map(|root| parse_comment(root, css, base))
            .collect();
    }
    Vec::new()
}

fn comment_scope(html: &Html) -> Option<ElementRef<'_>> {
    const SELECTORS: &[&str] = &[
        "shreddit-comment-tree",
        "#comment-tree",
        ".commentarea",
        r#"[data-testid="comment-tree"]"#,
    ];
    SELECTORS
        .iter()
        .find_map(|css| html.select(&selector(css)).next())
}

fn parse_comment(
    root: ElementRef,
    root_selector: &str,
    base: Option<&Url>,
) -> Option<RedditComment> {
    let mut comment = RedditComment {
        id: comment_id(root),
        author: first_value(&[
            first_attr(root, &["author", "data-author"]),
            first_selection_text(
                root,
                &[
                    r#"[noun="comment_author"]"#,
                    r#"[data-testid="comment_author_link"]"#,
                    ".tagline .author",
                    r#"[rel="author"]"#,
                    r#"[slot="commentMeta"] a"#,
                ],
            ),
        ]),
        score: first_value(&[
            first_attr(root, &["score", "data-score"]),
            first_selection_text(
                root,
                &[
                    r#"[data-testid="comment-score"]"#,
                    ".score.unvoted",
                    r#"[itemprop="upvoteCount"]"#,
                ],
            ),
        ]),
        published: first_value(&[
            first_attr(root, &["created-timestamp", "date-created", "published"]),
            first_selection_attr(
                root,
                "datetime",
                &["time[datetime]", r#"[itemprop="datePublished"]"#],
            ),
        ]),
        depth: comment_depth(root, root_selector),
        ..Default::default()
    };
    if let Some(body) = comment_body(root) {
        comment.text = textutil::selection_text(&body);
        comment.html = rewritten_inner_html(&body, base);
    } else {
        comment.text = fallback_comment_text(root, root_selector);
        if !comment.text.is_empty() {
            comment.html = paragraph_html(&comment.text);
        }
    }
    if comment.text.is_empty() && comment.author.is_empty() && comment.id.is_empty() {
        return None;
    }
    Some(comment)
}

const COMMENT_BODY_SELECTORS: &[&str] = &[
    r#"[slot="comment"]"#,
    r#"[data-testid="comment-body"]"#,
    r#"[data-click-id="text"]"#,
    ".entry .usertext-body .md",
    r#"[id$="-comment-rtjson-content"]"#,
    r#"[itemprop="text"]"#,
    r#"[itemprop="commentText"]"#,
];

fn comment_body(root: ElementRef<'_>) -> Option<ElementRef<'_>> {
    let direct_slot = selector(r#"[slot="comment"]"#);
    if let Some(direct) = root.child_elements().find(|c| direct_slot.matches(c)) {
        return Some(direct);
    }
    COMMENT_BODY_SELECTORS
        .iter()
        .find_map(|css| root.select(&selector(css)).next())
}

/// Reads a comment's own text when no dedicated body element was found,
/// by copying its subtree into a fresh fragment with nested replies (any
/// descendant also matching `root_selector`) and UI chrome stripped —
/// see this module's own doc for why a copy rather than in-place removal.
fn fallback_comment_text(root: ElementRef, root_selector: &str) -> String {
    let skip = selector(&format!(
        r#"{root_selector}, button, script, style, svg, shreddit-comment-action-row, [slot="actionRow"], [slot="commentMeta"], [slot="commentAvatar"]"#
    ));
    let fragment = build_filtered_fragment(root, &skip);
    fragment
        .select(&selector("*"))
        .next()
        .map(|el| textutil::selection_text(&el))
        .unwrap_or_default()
}

fn build_filtered_fragment(root: ElementRef, skip: &scraper::Selector) -> Html {
    let mut html = Html::new_fragment();
    let mut root_mut = html.tree.root_mut();
    let mut clone_mut = root_mut.append(Node::Element(root.value().clone()));
    for child in root.children() {
        copy_filtered(&mut clone_mut, child, skip);
    }
    html
}

fn copy_filtered(
    parent: &mut NodeMut<Node>,
    source: ego_tree::NodeRef<Node>,
    skip: &scraper::Selector,
) {
    match source.value() {
        Node::Text(text) => {
            parent.append(Node::Text(text.clone()));
        }
        Node::Element(element) => {
            if ElementRef::wrap(source).is_some_and(|el| skip.matches(&el)) {
                return;
            }
            let mut child_mut = parent.append(Node::Element(element.clone()));
            for grandchild in source.children() {
                copy_filtered(&mut child_mut, grandchild, skip);
            }
        }
        _ => {
            for child in source.children() {
                copy_filtered(parent, child, skip);
            }
        }
    }
}

fn comment_id(root: ElementRef) -> String {
    let id = normalize_thing_id(root.value().attr("comment-id").unwrap_or(""), "t1_");
    if !id.is_empty() {
        return id;
    }
    for attr in ["thingid", "thing-id", "data-fullname", "id"] {
        let value = root
            .value()
            .attr(attr)
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if !value.starts_with("t1_") {
            continue;
        }
        let id = normalize_thing_id(&value, "t1_");
        if !id.is_empty() {
            return id;
        }
    }
    String::new()
}

fn comment_depth(root: ElementRef, root_selector: &str) -> i32 {
    for attr in ["depth", "data-depth"] {
        if let Some(depth) = root
            .value()
            .attr(attr)
            .and_then(|v| v.trim().parse::<i32>().ok())
            .filter(|d| *d >= 0)
        {
            return depth;
        }
    }
    let sel = selector(root_selector);
    root.ancestors()
        .filter(|a| ElementRef::wrap(*a).is_some_and(|el| sel.matches(&el)))
        .count() as i32
}

fn reddit_post_text(post: &RedditPost) -> String {
    let mut out = String::new();
    out.push_str(&post.title);
    if !post.author.is_empty() {
        out.push_str("\nsubmitted by ");
        out.push_str(&post.author);
    }
    if !post.subreddit.is_empty() {
        out.push_str("\nsubreddit: ");
        out.push_str(&post.subreddit);
    }
    if !post.score.is_empty() {
        out.push_str("\nscore: ");
        out.push_str(&post.score);
    }
    if !post.flair.is_empty() {
        out.push_str("\nflair: ");
        out.push_str(&post.flair);
    }
    if !post.body_text.is_empty() {
        out.push_str("\n\n");
        out.push_str(&post.body_text);
    }
    if !post.link.is_empty() {
        out.push_str("\n\nlink: ");
        out.push_str(&post.link);
    }
    if !post.comments.is_empty() {
        out.push_str("\n\nComments");
        let base_depth = post.comments.iter().map(|c| c.depth).min().unwrap_or(0);
        for comment in &post.comments {
            let indent = "  ".repeat((comment.depth - base_depth).max(0) as usize);
            out.push_str("\n\n");
            out.push_str(&indent);
            if !comment.author.is_empty() {
                out.push_str(&comment.author);
            }
            if !comment.score.is_empty() {
                out.push_str(&format!(" [{} points]", comment.score));
            }
            if !comment.text.is_empty() {
                for line in comment.text.split('\n') {
                    out.push('\n');
                    out.push_str(&indent);
                    out.push_str(line);
                }
            }
        }
    }
    out.trim().to_string()
}

fn meaningful_post_link(raw_link: &str, page_url: &str, post_id: &str) -> String {
    let raw_link = raw_link.trim();
    if raw_link.is_empty() {
        return String::new();
    }
    let Ok(link_url) = Url::parse(raw_link) else {
        return String::new();
    };
    if link_url.scheme() != "http" && link_url.scheme() != "https" {
        return String::new();
    }
    if let Some((id, _)) = reddit_post_url(raw_link) {
        if id == post_id {
            return String::new();
        }
    }
    if let Ok(page) = Url::parse(page_url) {
        if page.scheme() == link_url.scheme()
            && page
                .host_str()
                .unwrap_or("")
                .eq_ignore_ascii_case(link_url.host_str().unwrap_or(""))
            && page.path() == link_url.path()
        {
            return String::new();
        }
    }
    link_url.to_string()
}

fn clean_reddit_title(title: &str, subreddit: &str) -> String {
    let mut title = sanitize_text(title);
    if !subreddit.is_empty() {
        for suffix in [format!(" : {subreddit}"), format!(" | {subreddit}")] {
            title = title
                .strip_suffix(suffix.as_str())
                .unwrap_or(&title)
                .trim()
                .to_string();
        }
    }
    for suffix in [" - Reddit", " : Reddit"] {
        title = title
            .strip_suffix(suffix)
            .unwrap_or(&title)
            .trim()
            .to_string();
    }
    title
}

fn normalize_subreddit(subreddit: &str) -> String {
    let trimmed = subreddit.trim().trim_matches('/');
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.len() >= 2 && trimmed[..2].eq_ignore_ascii_case("r/") {
        format!("r/{}", &trimmed[2..])
    } else {
        format!("r/{trimmed}")
    }
}

fn first_value(values: &[String]) -> String {
    for value in values {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return sanitize_text(trimmed);
        }
    }
    String::new()
}

fn first_attr(s: ElementRef, attrs: &[&str]) -> String {
    for attr in attrs {
        let value = s.value().attr(attr).unwrap_or("").trim();
        if !value.is_empty() {
            return value.to_string();
        }
    }
    String::new()
}

fn first_selection_text(s: ElementRef, selectors: &[&str]) -> String {
    for css in selectors {
        if let Some(el) = s.select(&selector(css)).next() {
            let text = textutil::selection_text(&el);
            if !text.is_empty() {
                return text;
            }
        }
    }
    String::new()
}

fn first_selection_attr(s: ElementRef, attr: &str, selectors: &[&str]) -> String {
    for css in selectors {
        if let Some(el) = s.select(&selector(css)).next() {
            let value = el.value().attr(attr).unwrap_or("").trim();
            if !value.is_empty() {
                return value.to_string();
            }
        }
    }
    String::new()
}

fn first_page_meta<'a, S>(scope: S, selectors: &[&str]) -> String
where
    S: scraper::selectable::Selectable<'a> + Copy,
{
    for css in selectors {
        if let Some(value) = scope
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

fn paragraph_html(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n");
    let escaped = html_escape(&normalized);
    format!("<p>{}</p>", escaped.replace('\n', "<br>"))
}

/// Rewrites relative URLs within `body`'s own subtree to absolute, by
/// re-parsing its serialized HTML as an independent fragment (rather than
/// mutating the shared document `body` was read from) and serializing
/// that back — the same trick `WikipediaExtractor::extract`'s
/// noise-removal clone uses for the same underlying reason (`scraper`
/// gives no in-place mutation API scoped to one subtree).
fn rewritten_inner_html(body: &ElementRef, base: Option<&Url>) -> String {
    let mut fragment = Html::parse_fragment(&body.inner_html());
    if let Some(base) = base {
        rewrite_urls(&mut fragment, base);
    }
    fragment.html()
}

fn structured_reddit_post(html: &Html) -> Option<RedditPost> {
    for script in html.select(&selector(r#"script[type="application/ld+json"]"#)) {
        let text = script.text().collect::<String>();
        let Ok(payload) = Value::parse(&text) else {
            continue;
        };
        let Some(node) = find_structured_posting(&payload) else {
            continue;
        };
        return Some(post_from_structured_node(node));
    }
    None
}

fn find_structured_posting(value: &Value) -> Option<&Map> {
    match value {
        Value::Array(items) => items.iter().find_map(find_structured_posting),
        Value::Object(map) => {
            if schema_type_matches(map.get("@type"), "DiscussionForumPosting")
                || schema_type_matches(map.get("@type"), "SocialMediaPosting")
            {
                return Some(map);
            }
            ["mainEntity", "@graph", "itemListElement"]
                .into_iter()
                .find_map(|key| map.get(key).and_then(find_structured_posting))
        }
        _ => None,
    }
}

fn schema_type_matches(value: Option<&Value>, want: &str) -> bool {
    match value {
        Some(Value::String(s)) => {
            let s = s.trim_end_matches('/');
            let s = match s.rfind(['/', '#']) {
                Some(i) => &s[i + 1..],
                None => s,
            };
            s.eq_ignore_ascii_case(want)
        }
        Some(Value::Array(items)) => items
            .iter()
            .any(|item| schema_type_matches(Some(item), want)),
        Some(Value::Object(map)) => schema_type_matches(map.get("@type"), want),
        _ => false,
    }
}

fn post_from_structured_node(node: &Map) -> RedditPost {
    let body_text = structured_string(node, &["articleBody", "text"]);
    let body_html = if body_text.is_empty() {
        String::new()
    } else {
        paragraph_html(&body_text)
    };
    let mut post = RedditPost {
        title: structured_string(node, &["headline", "name"]),
        author: structured_author(node.get("author")),
        published: structured_string(node, &["datePublished", "dateCreated"]),
        body_text,
        body_html,
        score: structured_count(node, "upvoteCount"),
        comment_count: structured_count(node, "commentCount"),
        ..Default::default()
    };
    for key in ["comment", "comments"] {
        if let Some(value) = node.get(key) {
            post.comments.extend(structured_comments(value, 0));
        }
    }
    post
}

fn structured_comments(value: &Value, depth: i32) -> Vec<RedditComment> {
    match value {
        Value::Array(items) => items
            .iter()
            .flat_map(|item| structured_comments(item, depth))
            .collect(),
        Value::Object(map) => {
            if !schema_type_matches(map.get("@type"), "Comment") {
                return Vec::new();
            }
            let text = structured_string(map, &["text", "commentText", "articleBody"]);
            let html = if text.is_empty() {
                String::new()
            } else {
                paragraph_html(&text)
            };
            let comment = RedditComment {
                id: structured_string(map, &["identifier", "@id"]),
                author: structured_author(map.get("author")),
                published: structured_string(map, &["datePublished", "dateCreated"]),
                score: structured_count(map, "upvoteCount"),
                text,
                html,
                depth,
            };
            let mut comments = vec![comment];
            for key in ["comment", "comments"] {
                if let Some(v) = map.get(key) {
                    comments.extend(structured_comments(v, depth + 1));
                }
            }
            comments
        }
        _ => Vec::new(),
    }
}

fn merge_structured_post(post: &mut RedditPost, structured: Option<RedditPost>) {
    let Some(structured) = structured else {
        return;
    };
    if post.title.is_empty() {
        post.title = structured.title;
    }
    if post.author.is_empty() {
        post.author = structured.author;
    }
    if post.published.is_empty() {
        post.published = structured.published;
    }
    if post.score.is_empty() {
        post.score = structured.score;
    }
    if post.comment_count.is_empty() {
        post.comment_count = structured.comment_count;
    }
    if post.body_text.is_empty() {
        post.body_text = structured.body_text;
        post.body_html = structured.body_html;
    }
    if post.comments.is_empty() {
        post.comments = structured.comments;
    }
}

fn structured_string(node: &Map, keys: &[&str]) -> String {
    for key in keys {
        if let Some(Value::String(s)) = node.get(*key) {
            let s = sanitize_text(s);
            if !s.is_empty() {
                return s;
            }
        }
    }
    String::new()
}

fn structured_author(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => sanitize_text(s),
        Some(Value::Object(map)) => structured_string(map, &["name", "alternateName"]),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| structured_author(Some(item)))
            .find(|a| !a.is_empty())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn structured_count(node: &Map, key: &str) -> String {
    if let Some(value) = node.get(key) {
        return format_structured_number(value);
    }
    let Some(statistics) = node.get("interactionStatistic") else {
        return String::new();
    };
    let entries: Vec<&Value> = match statistics {
        Value::Array(items) => items.iter().collect(),
        other => vec![other],
    };
    for entry in entries {
        let Value::Object(stat) = entry else { continue };
        if key == "commentCount"
            && !schema_type_matches(stat.get("interactionType"), "CommentAction")
        {
            continue;
        }
        if key == "upvoteCount" && !schema_type_matches(stat.get("interactionType"), "LikeAction") {
            continue;
        }
        if let Some(count) = stat
            .get("userInteractionCount")
            .map(format_structured_number)
        {
            if !count.is_empty() {
                return count;
            }
        }
    }
    String::new()
}

fn format_structured_number(value: &Value) -> String {
    match value {
        Value::String(s) => sanitize_text(s),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
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
    fn matches_only_reddit_post_pages() {
        let ext = RedditExtractor::default();
        let cases: &[(&str, bool)] = &[
            (
                "https://www.reddit.com/r/golang/comments/18ujt6g/new_at_go_start_here/",
                true,
            ),
            ("https://old.reddit.com/r/golang/comments/18ujt6g/", true),
            (
                "https://reddit.com/r/golang/comments/18ujt6g/title/kf123ab/?context=3",
                true,
            ),
            ("https://www.reddit.com/comments/18ujt6g/title/", true),
            ("https://redd.it/18ujt6g", true),
            ("https://www.reddit.com/r/golang/", false),
            ("https://www.reddit.com/user/alice/", false),
            ("https://www.reddit.com/search/?q=golang", false),
            ("https://www.reddit.com/r/golang/comments/", false),
            (
                "https://www.reddit.com/r/golang/comments/18ujt6g/title.json",
                false,
            ),
            (
                "https://reddit.com.example/r/golang/comments/18ujt6g/title",
                false,
            ),
            ("https://example.com/r/golang/comments/18ujt6g/title", false),
        ];
        for (url, want) in cases {
            assert_eq!(ext.matches(&Document::new(*url)), *want, "url = {url}");
        }
    }

    #[test]
    fn extract_collects_shreddit_post_and_nested_comments_exactly_once() {
        let ext = RedditExtractor::default();
        let html = r#"<html><body>
            <aside>
                <shreddit-post id="t3_other" post-title="A recommendation" permalink="/r/other/comments/other/a_recommendation/"></shreddit-post>
            </aside>
            <main>
                <article>
                    <shreddit-post id="t3_abc123" post-title="A durable parser" author="post_author"
                        subreddit-prefixed-name="r/golang" score="42"
                        created-timestamp="2026-08-12T10:00:00Z" post-flair-text="Discussion"
                        content-href="https://example.com/articles/parser">
                        <div slot="text-body">
                            <p>Post <strong>body</strong>.</p>
                            <script>alert("post")</script>
                        </div>
                    </shreddit-post>
                </article>
                <shreddit-comment-tree>
                    <shreddit-comment thingid="t1_first" author="alice" score="10" depth="0">
                        <div slot="comment"><p>First comment.</p></div>
                        <shreddit-comment thingid="t1_reply" author="bob" score="3" depth="1">
                            <div slot="comment"><p>A nested reply.</p></div>
                        </shreddit-comment>
                    </shreddit-comment>
                </shreddit-comment-tree>
            </main>
        </body></html>"#;
        let d = doc(
            "https://www.reddit.com/r/golang/comments/abc123/a_durable_parser/",
            html,
        );

        let ExtractOutcome::Extracted(extracted) = ext.extract(&d) else {
            panic!("expected Extracted");
        };
        assert_eq!(extracted.title.as_deref(), Some("A durable parser"));
        let text = extracted.text.unwrap();
        assert!(text.contains("Post body."));
        assert!(text.contains("First comment."));
        assert!(text.contains("A nested reply."));
        assert!(text.contains("https://example.com/articles/parser"));
        assert!(!text.contains("A recommendation"));
        assert!(!text.contains("alert("));
        assert_eq!(text.matches("A nested reply").count(), 1);

        assert_eq!(
            extracted.metadata.get("author"),
            Some(&Value::String("post_author".to_string()))
        );
        assert_eq!(
            extracted.metadata.get("subreddit"),
            Some(&Value::String("r/golang".to_string()))
        );
        assert_eq!(
            extracted.metadata.get("post_id"),
            Some(&Value::String("abc123".to_string()))
        );
        assert_eq!(
            extracted.metadata.get("comments"),
            Some(&Value::Number(Number::from(2u64)))
        );
    }

    #[test]
    fn extract_collects_legacy_reddit_markup_with_nested_comment_once() {
        let ext = RedditExtractor::default();
        let html = r#"<html><body>
            <div id="siteTable">
                <div class="thing link self" data-fullname="t3_abc123" data-author="legacy_author" data-score="17">
                    <div class="entry">
                        <p class="title"><a class="title" href="/r/golang/comments/abc123/legacy_post/">Legacy post title</a></p>
                        <div class="usertext-body"><div class="md"><p>Legacy post body.</p></div></div>
                    </div>
                </div>
            </div>
            <div class="commentarea">
                <div class="thing comment" data-fullname="t1_parent" data-author="parent_author" data-score="8">
                    <div class="entry"><div class="usertext-body"><div class="md"><p>Legacy parent comment.</p></div></div></div>
                    <div class="child"><div class="sitetable nestedlisting">
                        <div class="thing comment" data-fullname="t1_child" data-author="child_author" data-score="2">
                            <div class="entry"><div class="usertext-body"><div class="md"><p>Legacy child comment.</p></div></div></div>
                        </div>
                    </div></div>
                </div>
            </div>
        </body></html>"#;
        let d = doc(
            "https://old.reddit.com/r/golang/comments/abc123/legacy_post/",
            html,
        );

        let ExtractOutcome::Extracted(extracted) = ext.extract(&d) else {
            panic!("expected Extracted");
        };
        assert_eq!(extracted.title.as_deref(), Some("Legacy post title"));
        let text = extracted.text.unwrap();
        assert!(text.contains("Legacy post body."));
        assert!(text.contains("Legacy parent comment."));
        assert!(text.contains("Legacy child comment."));
        assert_eq!(text.matches("Legacy child comment.").count(), 1);
        assert_eq!(
            extracted.metadata.get("comments"),
            Some(&Value::Number(Number::from(2u64)))
        );
    }

    #[test]
    fn extract_falls_back_to_json_ld_structured_data() {
        let ext = RedditExtractor::default();
        let html = r#"<html><head><script type="application/ld+json">{
            "@context": "https://schema.org",
            "@type": "DiscussionForumPosting",
            "headline": "Schema post",
            "text": "Schema post body.",
            "author": {"@type": "Person", "name": "schema_author"},
            "datePublished": "2026-08-10T09:00:00Z",
            "upvoteCount": 12,
            "comment": [{
                "@type": "Comment",
                "identifier": "first",
                "text": "Schema comment.",
                "author": {"name": "comment_author"},
                "comment": [{"@type": "Comment", "identifier": "reply", "text": "Schema reply.", "author": "reply_author"}]
            }]
        }</script></head><body></body></html>"#;
        let d = doc(
            "https://www.reddit.com/r/golang/comments/schema1/schema_post/",
            html,
        );

        let ExtractOutcome::Extracted(extracted) = ext.extract(&d) else {
            panic!("expected Extracted");
        };
        assert_eq!(extracted.title.as_deref(), Some("Schema post"));
        let text = extracted.text.unwrap();
        assert!(text.contains("Schema post body."));
        assert!(text.contains("Schema comment."));
        assert!(text.contains("Schema reply."));
        assert_eq!(
            extracted.metadata.get("comments"),
            Some(&Value::Number(Number::from(2u64)))
        );
    }

    #[test]
    fn extract_falls_back_when_post_markup_is_missing() {
        let ext = RedditExtractor::default();
        let d = doc(
            "https://www.reddit.com/r/golang/comments/abc123/title/",
            "<html><head><title>Reddit block page</title></head><body>blocked</body></html>",
        );
        assert!(matches!(ext.extract(&d), ExtractOutcome::Fallback(_)));
    }

    #[test]
    fn extract_rejects_a_non_post_reddit_page() {
        let ext = RedditExtractor::default();
        let d = doc(
            "https://www.reddit.com/r/golang/",
            r#"<main><shreddit-post id="t3_abc123" post-title="Feed post"></shreddit-post></main>"#,
        );
        assert!(matches!(ext.extract(&d), ExtractOutcome::Fallback(_)));
    }

    #[test]
    fn preview_indents_nested_comments_and_rewrites_relative_links() {
        let ext = RedditExtractor::default();
        let html = r#"<html><body>
            <main><article>
                <shreddit-post id="t3_abc123" post-title="Links post" author="post_author">
                    <div slot="text-body"><p>See <a href="/r/golang/wiki/rules">rules</a>.</p></div>
                </shreddit-post>
            </article></main>
            <shreddit-comment-tree>
                <shreddit-comment thingid="t1_first" author="alice" depth="0">
                    <div slot="comment"><p>First comment.</p></div>
                    <shreddit-comment thingid="t1_reply" author="bob" depth="1">
                        <div slot="comment"><p>Reply with <a href="/user/bob">a link</a>.</p></div>
                    </shreddit-comment>
                </shreddit-comment>
            </shreddit-comment-tree>
        </body></html>"#;
        let d = doc(
            "https://www.reddit.com/r/golang/comments/abc123/links_post/",
            html,
        );

        let PreviewOutcome::Previewed(response) = ext.preview(&d) else {
            panic!("expected Previewed");
        };
        let content = response.html.unwrap();
        assert!(content.contains(r#"href="https://www.reddit.com/r/golang/wiki/rules""#));
        assert!(content.contains(r#"href="https://www.reddit.com/user/bob""#));
        assert!(content.contains(r#"style="margin-left:2em""#));
        assert!(!content.contains("<script"));
    }

    #[test]
    fn preview_falls_back_for_a_non_post_page() {
        let ext = RedditExtractor::default();
        let d = doc("https://www.reddit.com/r/golang/", "<main></main>");
        assert!(matches!(ext.preview(&d), PreviewOutcome::Fallback(_)));
    }
}
