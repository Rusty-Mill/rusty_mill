//! `Wikipedia` — a Rust port of
//! `server/extractor/extractors/wikipedia/{wikipedia,style,text}.go`
//! (capability inventory §4.5.12). Extract and preview for
//! `*.wikipedia.org/wiki/...` article pages, excluding non-content
//! namespaces (`Special:`, `Talk:`, `User:`, ...; `Wikipedia:` itself is
//! allowed, since pages like `Wikipedia:Unusual_articles` are real content).
//!
//! `goquery` (Go's HTML library here) mutates its parse tree in place —
//! `.Remove()`, `.SetAttr()`, `.ReplaceWithHtml()`, `.WrapHtml()` all work
//! directly on the live document. `scraper::ElementRef` has no such API:
//! it's a read-only view over an `ego_tree::Tree`. This port mutates the
//! same tree by `NodeId` instead — `Tree::get_mut` for attribute/value
//! changes, `NodeMut::detach` for removals — always collecting the
//! `NodeId`s a `Selector` pass needs to touch into an owned `Vec` *before*
//! mutating, since an `ElementRef` (an immutable borrow of the tree) can't
//! stay alive across a `get_mut` call (a mutable borrow). See
//! [`ids_within`]/[`set_style`]/[`remove_within`] and `style.rs`'s own
//! helpers built on them. One Go capability isn't reproduced this way:
//! `styleWikitables`' `tbl.WrapHtml(...)` (wrapping a wikitable in a
//! horizontally-scrolling `<div>`) has no cheap `NodeId`-based equivalent
//! (`ego_tree` can graft subtrees but has no "insert a new parent between
//! a node and its existing parent" primitive) — skipped as a cosmetic-only
//! detail untested by Go's own test suite, not a content or search-quality
//! loss.
//!
//! Extract's text pipeline mirrors a real Go asymmetry rather than
//! "fixing" it: the infobox pass (`text::write_infobox_text`) reads
//! straight from the original, un-cleaned content, while the general
//! article-text pass (`text::write_article_text`) reads from a *cleaned
//! clone* with navboxes/references/etc. already removed — because Go
//! builds that clone by re-serializing and re-parsing the content
//! subtree (`cloneSelection`), this port does the same
//! (`ElementRef::inner_html()` + `Html::parse_fragment`) rather than
//! trying to eliminate the clone now that in-place mutation is possible;
//! `Preview`, which Go never clones, mutates the one parsed document
//! directly.

mod style;
mod text;

use ammonia::Url;
use ego_tree::NodeId;
use html5ever::{Attribute, LocalName, Namespace, QualName};
use rusty_hister_core::{
    Capabilities, Document, ExtractOutcome, Extractor, ExtractorConfig, HisterError,
    PreviewOutcome, PreviewResponse,
};
use rusty_json::Value;
use scraper::node::Element as ScraperElement;
use scraper::{ElementRef, Html, Node, Selector, StrTendril};

use crate::sanitizer::sanitize_trusted_html;
use crate::urlutil::rewrite_urls;

/// The HTML namespace `html5ever` tags every element with during normal
/// parsing; needed here only because a node built by hand (the
/// `<video>`-to-`<img>` swap) has to specify one explicitly.
const HTML_NS: &str = "http://www.w3.org/1999/xhtml";

/// Non-content Wikipedia namespaces excluded from extraction.
const EXCLUDED_NAMESPACES: &[&str] = &[
    "Special:",
    "Talk:",
    "User:",
    "User_talk:",
    "File:",
    "MediaWiki:",
    "Template:",
    "Help:",
    "Category:",
    "Portal:",
    "Draft:",
    "Module:",
    "Wikipedia_talk:",
    "Template_talk:",
    "Help_talk:",
    "Category_talk:",
];

/// Noise selectors shared by both indexing and preview removal.
const NOISE_BASE: &[&str] = &[
    ".navbox",
    ".navbar",
    ".toc",
    ".mw-editsection",
    ".noprint",
    "style",
    "script",
    ".mw-empty-elt",
    ".portal-bar",
    ".sistersitebox",
    ".authority-control",
    ".shortdescription",
];

/// Extra selectors stripped only during indexing (references, hatnotes, etc).
const NOISE_INDEX_ONLY: &[&str] = &[
    ".mw-references-wrap",
    "ol.references",
    ".reflist",
    ".sidebar",
    ".external.text",
    "#catlinks",
    ".hatnote",
    "sup.reference",
    ".reference-text",
];

/// Extra selectors stripped only during preview (admin notice boxes).
const NOISE_PREVIEW_ONLY: &[&str] = &[
    "table.ombox",
    "table.tmbox",
    "table.ambox",
    "table.cmbox",
    "table.fmbox",
    "table.imbox",
];

#[derive(Debug, Default)]
pub struct WikipediaExtractor {
    config: ExtractorConfig,
}

impl Extractor for WikipediaExtractor {
    fn name(&self) -> &str {
        "Wikipedia"
    }

    fn description(&self) -> &str {
        "Extracts article content, infoboxes, tables, and metadata from Wikipedia pages."
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            enrich: false,
            extract: true,
            preview: true,
        }
    }

    fn matches(&self, document: &Document) -> bool {
        is_wikipedia_url(&document.url)
    }

    fn extract(&self, document: &Document) -> ExtractOutcome {
        let Some(html_str) = document.html.as_deref() else {
            return ExtractOutcome::Fallback(HisterError::Extraction(
                "document has no HTML".to_string(),
            ));
        };
        let html = Html::parse_document(html_str);
        let Some(content) = article_content(&html) else {
            return ExtractOutcome::Fallback(HisterError::Extraction(
                "no mw-parser-output found".to_string(),
            ));
        };

        let title = article_title(&html);
        let description = short_description(&content);
        let cats = categories(&html);

        // Mirrors Go's `cloneSelection`: re-serialize the content subtree
        // and re-parse it as its own document so noise removal here can't
        // affect `content` (already used above) or the original `html`.
        let mut clone = Html::parse_fragment(&format!("<div>{}</div>", content.inner_html()));
        remove_all(&mut clone, &[NOISE_BASE, NOISE_INDEX_ONLY]);
        let clean_content = clone.select(&selector("div")).next().expect("wrapper div");

        let mut out = String::new();
        if !title.is_empty() {
            out.push_str(&title);
            out.push('\n');
        }
        text::write_infobox_text(&mut out, &content);
        text::write_article_text(&mut out, &clean_content);

        let text_out = out.trim().to_string();
        if text_out.is_empty() && title.is_empty() {
            return ExtractOutcome::Fallback(HisterError::Extraction(
                "no content found".to_string(),
            ));
        }

        let mut extracted = document.clone();
        if !title.is_empty() {
            extracted.title = Some(title);
        }
        if !text_out.is_empty() {
            extracted.text = Some(text_out);
        }
        extracted
            .metadata
            .insert("type".to_string(), Value::String("Article".to_string()));
        if !description.is_empty() {
            extracted
                .metadata
                .insert("description".to_string(), Value::String(description));
        }
        if !cats.is_empty() {
            extracted
                .metadata
                .insert("categories".to_string(), Value::String(cats.join(", ")));
        }
        ExtractOutcome::Extracted(extracted)
    }

    fn preview(&self, document: &Document) -> PreviewOutcome {
        let Some(html_str) = document.html.as_deref() else {
            return PreviewOutcome::Fallback(HisterError::Extraction(
                "document has no HTML".to_string(),
            ));
        };
        let mut html = Html::parse_document(html_str);
        let Some(content_id) = article_content(&html).map(|c| c.id()) else {
            return PreviewOutcome::Fallback(HisterError::Extraction(
                "no mw-parser-output found".to_string(),
            ));
        };
        let Ok(base) = Url::parse(&document.url) else {
            return PreviewOutcome::Fallback(HisterError::Extraction(
                "invalid document URL".to_string(),
            ));
        };

        remove_within(&mut html, content_id, &[NOISE_BASE, NOISE_PREVIEW_ONLY]);
        replace_videos(&mut html, content_id);
        style::style_content(&mut html, content_id);
        rewrite_urls(&mut html, &base);

        let content = ElementRef::wrap(html.tree.get(content_id).expect("content node"))
            .expect("content is an element");
        let sanitized = sanitize_trusted_html(&content.inner_html());
        PreviewOutcome::Previewed(PreviewResponse {
            html: Some(sanitized),
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

fn is_wikipedia_url(url: &str) -> bool {
    let Some((_, rest)) = url.split_once("wikipedia.org/wiki/") else {
        return false;
    };
    if rest.is_empty() {
        return false;
    }
    !EXCLUDED_NAMESPACES.iter().any(|ns| rest.starts_with(ns))
}

fn article_content(html: &Html) -> Option<ElementRef<'_>> {
    html.select(&selector("#mw-content-text .mw-parser-output"))
        .next()
        .or_else(|| html.select(&selector(".mw-parser-output")).next())
}

fn article_title(html: &Html) -> String {
    let specific = html
        .select(&selector("#firstHeading .mw-page-title-main"))
        .next()
        .map(|el| el.text().collect::<String>())
        .filter(|t| !t.trim().is_empty());
    let title = specific.unwrap_or_else(|| {
        html.select(&selector("#firstHeading"))
            .next()
            .map(|el| el.text().collect::<String>())
            .unwrap_or_default()
    });
    title.trim().to_string()
}

fn short_description(content: &ElementRef) -> String {
    content
        .select(&selector(".shortdescription"))
        .next()
        .map(|el| el.text().collect::<String>())
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn categories(html: &Html) -> Vec<String> {
    html.select(&selector("#catlinks .mw-normal-catlinks li a"))
        .filter_map(|el| {
            let text = el.text().collect::<String>();
            let text = text.trim();
            (!text.is_empty()).then(|| text.to_string())
        })
        .collect()
}

pub(super) fn selector(css: &str) -> Selector {
    Selector::parse(css).expect("valid CSS selector")
}

fn attr_name(name: &str) -> QualName {
    QualName::new(None, Namespace::from(""), LocalName::from(name))
}

/// Collects the `NodeId`s a selector matches within `scope_id`'s subtree.
/// A `Vec` rather than an iterator on purpose: every caller needs the
/// borrow of `html` to end before it can mutate anything found.
pub(super) fn ids_within(html: &Html, scope_id: NodeId, css: &str) -> Vec<NodeId> {
    ElementRef::wrap(html.tree.get(scope_id).expect("scope node"))
        .map(|scope| scope.select(&selector(css)).map(|el| el.id()).collect())
        .unwrap_or_default()
}

/// Sets `name="value"` on the element at `id`, if it still is one.
pub(super) fn set_attr(html: &mut Html, id: NodeId, name: &str, value: &str) {
    if let Some(mut node) = html.tree.get_mut(id) {
        if let Node::Element(el) = node.value() {
            el.attrs
                .insert(attr_name(name), StrTendril::from(value.to_string()));
        }
    }
}

/// Sets `style="value"` on the element at `id`.
pub(super) fn set_style(html: &mut Html, id: NodeId, style: &str) {
    set_attr(html, id, "style", style);
}

/// Applies `style` to every element `css` matches within `scope_id`.
pub(super) fn style_selector(html: &mut Html, scope_id: NodeId, css: &str, style: &str) {
    for id in ids_within(html, scope_id, css) {
        set_style(html, id, style);
    }
}

/// Removes (detaches) every element any of `selectors`' groups match,
/// document-wide. Used for `Extract`'s cleaned clone, which already *is*
/// just the content subtree, so no further scoping is needed.
fn remove_all(html: &mut Html, selectors: &[&[&str]]) {
    let mut ids = Vec::new();
    for group in selectors {
        for css in *group {
            ids.extend(html.select(&self::selector(css)).map(|el| el.id()));
        }
    }
    for id in ids {
        if let Some(mut node) = html.tree.get_mut(id) {
            node.detach();
        }
    }
}

/// Removes (detaches) every element any of `selectors`' groups match
/// within `scope_id`'s subtree.
fn remove_within(html: &mut Html, scope_id: NodeId, selectors: &[&[&str]]) {
    let mut ids = Vec::new();
    for group in selectors {
        for css in *group {
            ids.extend(ids_within(html, scope_id, css));
        }
    }
    for id in ids {
        if let Some(mut node) = html.tree.get_mut(id) {
            node.detach();
        }
    }
}

/// Replaces every `<video>` within `scope_id` with an `<img>` of its
/// poster frame (video elements lose their `controls`/`poster`/dimension
/// attributes to sanitization, rendering as unconstrained blobs — the
/// poster thumbnail preserves visual context), or removes it outright
/// when it has no poster.
fn replace_videos(html: &mut Html, scope_id: NodeId) {
    let targets: Vec<(NodeId, Option<String>)> =
        ElementRef::wrap(html.tree.get(scope_id).expect("scope node"))
            .map(|scope| {
                scope
                    .select(&selector("video"))
                    .map(|video| {
                        let poster = video
                            .value()
                            .attr("poster")
                            .filter(|p| !p.is_empty())
                            .map(|p| p.to_string());
                        (video.id(), poster)
                    })
                    .collect()
            })
            .unwrap_or_default();

    for (id, poster) in targets {
        match poster {
            Some(poster) => replace_with_poster_image(html, id, &poster),
            None => {
                if let Some(mut node) = html.tree.get_mut(id) {
                    node.detach();
                }
            }
        }
    }
}

fn replace_with_poster_image(html: &mut Html, id: NodeId, poster: &str) {
    let child_ids: Vec<NodeId> = html
        .tree
        .get(id)
        .map(|node| node.children().map(|c| c.id()).collect())
        .unwrap_or_default();
    for child_id in child_ids {
        if let Some(mut child) = html.tree.get_mut(child_id) {
            child.detach();
        }
    }
    if let Some(mut node) = html.tree.get_mut(id) {
        let name = QualName::new(None, Namespace::from(HTML_NS), LocalName::from("img"));
        let attrs = vec![
            Attribute {
                name: attr_name("src"),
                value: StrTendril::from(poster.to_string()),
            },
            Attribute {
                name: attr_name("style"),
                value: StrTendril::from(
                    "max-width: 100%; height: auto; display: block".to_string(),
                ),
            },
        ];
        *node.value() = Node::Element(ScraperElement::new(name, attrs));
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
    fn matches_wikipedia_article_urls_but_not_non_content_namespaces() {
        let ext = WikipediaExtractor::default();
        let cases: &[(&str, bool)] = &[
            (
                "https://en.wikipedia.org/wiki/Go_(programming_language)",
                true,
            ),
            ("https://de.wikipedia.org/wiki/Berlin", true),
            ("https://en.wikipedia.org/wiki/", false),
            ("https://en.wikipedia.org/", false),
            ("https://en.wikipedia.org/wiki/Special:Search", false),
            ("https://en.wikipedia.org/wiki/Wikipedia:About", true),
            ("https://en.wikipedia.org/wiki/Talk:Go", false),
            ("https://en.wikipedia.org/wiki/User:Example", false),
            ("https://en.wikipedia.org/wiki/Category:Programming", false),
            ("https://en.wikipedia.org/wiki/File:Example.jpg", false),
            ("https://en.wikipedia.org/wiki/Template:Infobox", false),
            ("https://stackoverflow.com/questions/1234", false),
            ("https://example.com/wiki/Foo", false),
        ];
        for (url, want) in cases {
            assert_eq!(ext.matches(&Document::new(*url)), *want, "url = {url}");
        }
    }

    const MINIMAL_ARTICLE: &str = r#"<html>
<head><title>Test - Wikipedia</title></head>
<body>
<h1 id="firstHeading"><span class="mw-page-title-main">Test Article</span></h1>
<div id="mw-content-text" class="mw-body-content">
<div class="mw-parser-output">
<div class="shortdescription">A test article for extraction</div>
<p>This is the lead paragraph of the test article. It contains important information.</p>
<h2 id="History">History</h2>
<p>The history section describes past events.</p>
<h3 id="Early_history">Early history</h3>
<p>Early history details go here.</p>
<table class="wikitable">
<caption>Test data table</caption>
<tr><th>Name</th><th>Value</th></tr>
<tr><td>Alpha</td><td>100</td></tr>
<tr><td>Beta</td><td>200</td></tr>
</table>
</div>
</div>
<div id="catlinks"><div class="mw-normal-catlinks">
<ul><li><a title="Category:Test">Test</a></li><li><a title="Category:Articles">Articles</a></li></ul>
</div></div>
</body></html>"#;

    #[test]
    fn extract_collects_title_text_and_metadata_from_a_minimal_article() {
        let ext = WikipediaExtractor::default();
        let d = doc(
            "https://en.wikipedia.org/wiki/Test_Article",
            MINIMAL_ARTICLE,
        );
        let ExtractOutcome::Extracted(extracted) = ext.extract(&d) else {
            panic!("expected Extracted");
        };
        assert_eq!(extracted.title.as_deref(), Some("Test Article"));
        let text = extracted.text.unwrap();
        assert!(text.contains("lead paragraph"));
        assert!(text.contains("History"));
        assert!(text.contains("Early history"));
        assert!(text.contains("Alpha") && text.contains("100"));
        assert!(text.contains("Test data table"));
        assert_eq!(
            extracted.metadata.get("type"),
            Some(&Value::String("Article".to_string()))
        );
        assert_eq!(
            extracted.metadata.get("description"),
            Some(&Value::String("A test article for extraction".to_string()))
        );
        let categories = extracted.metadata.get("categories").unwrap();
        let Value::String(categories) = categories else {
            panic!("expected string")
        };
        assert!(categories.contains("Test"));
    }

    const ARTICLE_WITH_INFOBOX: &str = r#"<html>
<body>
<h1 id="firstHeading"><span class="mw-page-title-main">Test Country</span></h1>
<div class="mw-parser-output">
<table class="infobox">
<tr><th colspan="2">Test Country</th></tr>
<tr><th>Capital</th><td>Testville</td></tr>
<tr><th>Population</th><td>1,000,000</td></tr>
<tr><th>Area</th><td>50,000 km2</td></tr>
</table>
<p>Test Country is a nation in the world.</p>
</div>
</body></html>"#;

    #[test]
    fn extract_collects_infobox_key_value_pairs() {
        let ext = WikipediaExtractor::default();
        let d = doc(
            "https://en.wikipedia.org/wiki/Test_Country",
            ARTICLE_WITH_INFOBOX,
        );
        let ExtractOutcome::Extracted(extracted) = ext.extract(&d) else {
            panic!("expected Extracted");
        };
        let text = extracted.text.unwrap();
        assert!(text.contains("Capital: Testville"));
        assert!(text.contains("Population: 1,000,000"));
    }

    const ARTICLE_WITH_NOISE: &str = r#"<html>
<body>
<h1 id="firstHeading"><span class="mw-page-title-main">Noisy Article</span></h1>
<div class="mw-parser-output">
<div class="shortdescription">Short desc</div>
<div class="hatnote">For other uses, see Foo.</div>
<p>Real content here.</p>
<div class="navbox">Navigation box content that should be removed.</div>
<div class="toc">Table of contents that should be removed.</div>
<div class="sidebar">Sidebar content to remove.</div>
<ol class="references"><li>Reference 1</li><li>Reference 2</li></ol>
<p>More real content.<sup class="reference">[1]</sup></p>
</div>
</body></html>"#;

    #[test]
    fn extract_strips_navigation_and_reference_noise() {
        let ext = WikipediaExtractor::default();
        let d = doc(
            "https://en.wikipedia.org/wiki/Noisy_Article",
            ARTICLE_WITH_NOISE,
        );
        let ExtractOutcome::Extracted(extracted) = ext.extract(&d) else {
            panic!("expected Extracted");
        };
        let text = extracted.text.unwrap();
        assert!(text.contains("Real content here"));
        assert!(!text.contains("Navigation box"));
        assert!(!text.contains("Reference 1"));
        assert!(!text.contains("[1]"));
    }

    #[test]
    fn extract_falls_back_when_there_is_no_content_at_all() {
        let ext = WikipediaExtractor::default();
        let d = doc(
            "https://en.wikipedia.org/wiki/Empty",
            "<html><body><h1 id='firstHeading'></h1></body></html>",
        );
        assert!(matches!(ext.extract(&d), ExtractOutcome::Fallback(_)));
    }

    #[test]
    fn preview_renders_content_and_styles_wikitables_and_headings() {
        let ext = WikipediaExtractor::default();
        let d = doc(
            "https://en.wikipedia.org/wiki/Test_Article",
            MINIMAL_ARTICLE,
        );
        let PreviewOutcome::Previewed(response) = ext.preview(&d) else {
            panic!("expected Previewed");
        };
        let html = response.html.unwrap();
        assert!(html.contains("lead paragraph"));
        assert!(html.contains("<table"));
        // `ammonia`'s CSS serializer drops the space after a property's
        // `:` (and adds one after each `,` inside a function like
        // `rgba(...)`) — these assertions match its actual output, not
        // the `style` string literals this module writes.
        assert!(html.contains("border-collapse:collapse"));
        assert!(html.contains("border:1px solid"));
        assert!(html.contains("border-bottom:1px solid"));
        assert!(!html.contains("navbox"));
    }

    #[test]
    fn preview_styles_infoboxes_with_float_and_accent_background() {
        let ext = WikipediaExtractor::default();
        let d = doc(
            "https://en.wikipedia.org/wiki/Test_Country",
            ARTICLE_WITH_INFOBOX,
        );
        let PreviewOutcome::Previewed(response) = ext.preview(&d) else {
            panic!("expected Previewed");
        };
        let html = response.html.unwrap();
        assert!(html.contains("float:right"));
        assert!(html.contains("background-color:rgba(128, 128, 128, 0.06)"));
        assert!(html.contains("background-color:rgba(100, 150, 220, 0.18)"));
        assert!(html.contains("Testville"));
    }

    #[test]
    fn preview_rewrites_relative_and_protocol_relative_urls() {
        let ext = WikipediaExtractor::default();
        let html = r#"<html><body>
<h1 id="firstHeading"><span class="mw-page-title-main">Links</span></h1>
<div class="mw-parser-output">
<p><a href="/wiki/Go_(programming_language)">Go</a></p>
<img src="//upload.wikimedia.org/image.png" />
</div>
</body></html>"#;
        let d = doc("https://en.wikipedia.org/wiki/Links", html);
        let PreviewOutcome::Previewed(response) = ext.preview(&d) else {
            panic!("expected Previewed");
        };
        let content = response.html.unwrap();
        assert!(content.contains("https://en.wikipedia.org/wiki/Go_(programming_language)"));
        assert!(content.contains("https://upload.wikimedia.org/image.png"));
    }

    #[test]
    fn preview_replaces_video_with_its_poster_image() {
        let ext = WikipediaExtractor::default();
        let html = r#"<html><body>
<h1 id="firstHeading"><span class="mw-page-title-main">Video</span></h1>
<div class="mw-parser-output">
<video poster="/media/poster.jpg"><source src="/media/clip.webm"></video>
</div>
</body></html>"#;
        let d = doc("https://en.wikipedia.org/wiki/Video", html);
        let PreviewOutcome::Previewed(response) = ext.preview(&d) else {
            panic!("expected Previewed");
        };
        let content = response.html.unwrap();
        assert!(content.contains("<img"));
        assert!(content.contains("poster.jpg"));
        assert!(!content.contains("<video"));
        assert!(!content.contains("<source"));
    }

    #[test]
    fn preview_styles_galleries_as_a_flex_container() {
        let ext = WikipediaExtractor::default();
        let html = r#"<html><body>
<h1 id="firstHeading"><span class="mw-page-title-main">Gallery</span></h1>
<div class="mw-parser-output">
<ul class="gallery mw-gallery-traditional">
<li class="gallerybox">
<div class="thumb"><a href="/wiki/File:Test.jpg"><img src="//upload.wikimedia.org/test.jpg" width="120" height="80" /></a></div>
<div class="gallerytext">A test image</div>
</li>
</ul>
</div>
</body></html>"#;
        let d = doc("https://en.wikipedia.org/wiki/Gallery", html);
        let PreviewOutcome::Previewed(response) = ext.preview(&d) else {
            panic!("expected Previewed");
        };
        let content = response.html.unwrap();
        assert!(content.contains("display:flex"));
        assert!(content.contains("A test image"));
        assert!(content.contains("https://upload.wikimedia.org/test.jpg"));
    }
}
