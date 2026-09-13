//! `EmbeddedVideo` — a Rust port of
//! `server/extractor/extractors/embeddedvideo/extractor.go`. Enrich-only:
//! scans a document's HTML for `<video>`/`<source>`/`<iframe>`/`<embed>`/
//! `<object>` elements that embed a video, and stores the discovered URLs
//! (deduplicated, in document order) as a JSON array at
//! `Metadata["videos"]`.
//!
//! First extractor in this crate to use `scraper` (added to this crate's
//! `Cargo.toml` after a sovereignty-loop pass found no first-party
//! `rusty_*` crate for HTML parsing — see `docs/PROJECT-STATUS.md`).
//! `<video>`/`<source>` URLs are trusted as-is (matching Go: a page's own
//! `<video>` tag is treated as first-party content); `<iframe>`/`<embed>`/
//! `<object>` URLs are only accepted when they match a known video-hosting
//! service by full `https://` prefix ([`KNOWN_VIDEO_HOSTS`]) — a prefix
//! check, not a substring check, so `https://evil.com/youtube.com/embed/`
//! is correctly rejected. No sanitization is needed here (unlike
//! `jsonld.rs`): URLs are stored as opaque strings, not rendered as HTML.

use rusty_hister_core::{
    Capabilities, Document, ExtractOutcome, Extractor, ExtractorConfig, HisterError, PreviewOutcome,
};
use rusty_json::Value;
use scraper::{ElementRef, Html, Selector};
use std::collections::HashSet;

/// Full `https://` URL prefixes for known video-hosting/embed services (Go:
/// `knownVideoHosts`). Prefix matching, not substring matching, so a
/// malicious URL can't smuggle a known host name in as a path segment.
const KNOWN_VIDEO_HOSTS: &[&str] = &[
    "https://youtube.com/embed/",
    "https://www.youtube.com/embed/",
    "https://youtube.com/v/",
    "https://www.youtube.com/v/",
    "https://youtu.be/",
    "https://player.vimeo.com/video/",
    "https://vimeo.com/video/",
    "https://www.dailymotion.com/embed/",
    "https://bitchute.com/embed/",
    "https://www.bitchute.com/embed/",
    "https://rumble.com/embed/",
    "https://player.twitch.tv/",
    "https://www.facebook.com/plugins/video",
    "https://www.instagram.com/p/",
    "https://www.tiktok.com/embed/",
    "https://ok.ru/videoembed/",
    "https://rutube.ru/play/embed/",
    "https://www.ted.com/talks/",
    "https://fast.wistia.com/embed/",
    "https://cdn.jwplayer.com/players/",
    "https://players.brightcove.net/",
    "https://www.metacafe.com/embed/",
    "https://streamable.com/e/",
    "https://odysee.com/$/embed/",
];

/// Byte substrings used for [`EmbeddedVideoExtractor::matches`]'s cheap
/// pre-scan (Go: `htmlQuickCheck`).
const HTML_QUICK_CHECK: &[&str] = &["<iframe", "<video", "<embed", "<object"];

/// One embedded video found in a document (Go: `videoEntry`).
#[derive(Debug, Clone, PartialEq)]
struct VideoEntry {
    url: String,
    /// `"iframe"`, `"video"`, `"embed"`, or `"object"`.
    kind: &'static str,
    mime: Option<String>,
}

impl VideoEntry {
    fn to_json(&self) -> Value {
        let mut object = rusty_json::Map::new();
        object.insert("url".to_string(), Value::String(self.url.clone()));
        object.insert("type".to_string(), Value::String(self.kind.to_string()));
        if let Some(mime) = &self.mime {
            object.insert("mime".to_string(), Value::String(mime.clone()));
        }
        Value::Object(object)
    }
}

/// Scans HTML for embedded video tags and stores discovered video URLs in
/// document metadata (Go: `EmbeddedVideoExtractor`).
#[derive(Debug, Default)]
pub struct EmbeddedVideoExtractor {
    config: ExtractorConfig,
}

impl Extractor for EmbeddedVideoExtractor {
    fn name(&self) -> &str {
        "embeddedvideo"
    }

    fn description(&self) -> &str {
        "Scans HTML for embedded video tags (iframe, video, embed, object) and stores discovered video URLs in document metadata."
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            enrich: true,
            extract: false,
            preview: false,
        }
    }

    /// A cheap substring pre-check avoiding the parse cost on pages with no
    /// video-embedding elements at all (Go's own `Match`).
    fn matches(&self, document: &Document) -> bool {
        let Some(html) = document.html.as_deref() else {
            return false;
        };
        if html.is_empty() {
            return false;
        }
        let lower = html.to_ascii_lowercase();
        HTML_QUICK_CHECK.iter().any(|needle| lower.contains(needle))
    }

    fn extract(&self, document: &Document) -> ExtractOutcome {
        let Some(html) = document.html.as_deref() else {
            return ExtractOutcome::Fallback(HisterError::Extraction(
                "document has no HTML".to_string(),
            ));
        };

        let videos = extract_videos(html);
        if videos.is_empty() {
            return ExtractOutcome::Fallback(HisterError::Extraction(
                "no embedded video elements found".to_string(),
            ));
        }

        let mut extracted = document.clone();
        let dump = Value::Array(videos.iter().map(VideoEntry::to_json).collect());
        extracted
            .metadata
            .insert("videos".to_string(), Value::String(dump.to_json_string()));
        ExtractOutcome::Extracted(extracted)
    }

    /// Not implemented — this extractor only enriches metadata (Go:
    /// `Preview` always returns `PreviewFallback(nil)`).
    fn preview(&self, _document: &Document) -> PreviewOutcome {
        PreviewOutcome::Fallback(HisterError::Extraction(
            "embeddedvideo does not render previews".to_string(),
        ))
    }

    fn config(&self) -> &ExtractorConfig {
        &self.config
    }

    fn set_config(&mut self, config: ExtractorConfig) -> Result<(), HisterError> {
        self.config = config;
        Ok(())
    }
}

/// Returns every embedded video found in `html`'s `<video>`/`<source>`/
/// `<iframe>`/`<embed>`/`<object>` elements, in document order, deduplicated
/// by URL (Go: `extractVideos`).
fn extract_videos(html: &str) -> Vec<VideoEntry> {
    let document = Html::parse_document(html);
    let selector =
        Selector::parse("video, source, iframe, embed, object").expect("static selector is valid");

    let mut videos = Vec::new();
    let mut seen = HashSet::new();

    for element in document.select(&selector) {
        let Some(entry) = video_entry_for(&element) else {
            continue;
        };
        let url = entry.url.trim();
        if url.is_empty() || !seen.insert(url.to_string()) {
            continue;
        }
        videos.push(VideoEntry {
            url: url.to_string(),
            ..entry
        });
    }

    videos
}

/// Builds a [`VideoEntry`] for `element` if it's a video-embedding element
/// this extractor recognizes, applying the known-host allowlist to
/// `iframe`/`embed`/`object` (but not `video`/`source`, matching Go).
fn video_entry_for(element: &ElementRef) -> Option<VideoEntry> {
    let value = element.value();
    match value.name() {
        "video" => Some(VideoEntry {
            url: value.attr("src")?.to_string(),
            kind: "video",
            mime: non_empty(value.attr("type")),
        }),
        "source" => {
            if !has_ancestor(element, "video") {
                return None;
            }
            Some(VideoEntry {
                url: value.attr("src")?.to_string(),
                kind: "video",
                mime: non_empty(value.attr("type")),
            })
        }
        "iframe" => {
            let url = value.attr("src")?;
            is_video_embed_url(url).then(|| VideoEntry {
                url: url.to_string(),
                kind: "iframe",
                mime: None,
            })
        }
        "embed" => {
            let url = value.attr("src")?;
            is_video_embed_url(url).then(|| VideoEntry {
                url: url.to_string(),
                kind: "embed",
                mime: non_empty(value.attr("type")),
            })
        }
        "object" => {
            let url = value.attr("data")?;
            is_video_embed_url(url).then(|| VideoEntry {
                url: url.to_string(),
                kind: "object",
                mime: non_empty(value.attr("type")),
            })
        }
        _ => None,
    }
}

fn non_empty(attr: Option<&str>) -> Option<String> {
    attr.filter(|s| !s.is_empty()).map(str::to_string)
}

/// True while `element` has an ancestor element named `tag` (used to gate
/// `<source>` on being nested inside a `<video>`, matching Go's
/// token-stream `inVideo` flag).
fn has_ancestor(element: &ElementRef, tag: &str) -> bool {
    element
        .ancestors()
        .any(|node| node.value().as_element().is_some_and(|el| el.name() == tag))
}

/// True when `url` starts with one of [`KNOWN_VIDEO_HOSTS`] (case-
/// insensitively), matching Go's `isVideoEmbedURL`.
fn is_video_embed_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    KNOWN_VIDEO_HOSTS
        .iter()
        .any(|prefix| lower.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extracted_videos(html: &str) -> Vec<Value> {
        let mut doc = Document::new("https://example.com/");
        doc.html = Some(html.to_string());
        match EmbeddedVideoExtractor::default().extract(&doc) {
            ExtractOutcome::Extracted(result) => match result.metadata.get("videos") {
                Some(Value::String(raw)) => match Value::parse(raw).unwrap() {
                    Value::Array(entries) => entries,
                    other => panic!("expected an array, got {other:?}"),
                },
                other => panic!("expected Metadata[\"videos\"] to be a string, got {other:?}"),
            },
            other => panic!("expected Extracted, got {other:?}"),
        }
    }

    fn field<'a>(entry: &'a Value, key: &str) -> Option<&'a str> {
        match entry.get(key) {
            Some(Value::String(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    #[test]
    fn extracts_a_native_video_element_with_its_source() {
        let html =
            r#"<video><source src="https://cdn.example.com/movie.mp4" type="video/mp4"></video>"#;
        let videos = extracted_videos(html);
        assert_eq!(videos.len(), 1);
        assert_eq!(
            field(&videos[0], "url"),
            Some("https://cdn.example.com/movie.mp4")
        );
        assert_eq!(field(&videos[0], "type"), Some("video"));
        assert_eq!(field(&videos[0], "mime"), Some("video/mp4"));
    }

    #[test]
    fn extracts_a_video_elements_own_src_too() {
        let html = r#"<video src="https://cdn.example.com/direct.mp4"></video>"#;
        let videos = extracted_videos(html);
        assert_eq!(videos.len(), 1);
        assert_eq!(
            field(&videos[0], "url"),
            Some("https://cdn.example.com/direct.mp4")
        );
        assert_eq!(field(&videos[0], "type"), Some("video"));
    }

    #[test]
    fn ignores_a_source_element_outside_any_video() {
        let html = r#"<source src="https://cdn.example.com/orphan.mp4">"#;
        let mut doc = Document::new("https://example.com/");
        doc.html = Some(html.to_string());
        match EmbeddedVideoExtractor::default().extract(&doc) {
            ExtractOutcome::Fallback(_) => {}
            other => panic!("expected Fallback, got {other:?}"),
        }
    }

    #[test]
    fn accepts_a_known_video_host_iframe() {
        let html = r#"<iframe src="https://www.youtube.com/embed/dQw4w9WgXcQ"></iframe>"#;
        let videos = extracted_videos(html);
        assert_eq!(videos.len(), 1);
        assert_eq!(
            field(&videos[0], "url"),
            Some("https://www.youtube.com/embed/dQw4w9WgXcQ")
        );
        assert_eq!(field(&videos[0], "type"), Some("iframe"));
    }

    #[test]
    fn rejects_an_iframe_that_only_looks_like_a_known_host() {
        // A malicious URL smuggling a known host name in as a path segment
        // must not pass the prefix check.
        let html = r#"<iframe src="https://evil.example.com/youtube.com/embed/x"></iframe>"#;
        let mut doc = Document::new("https://example.com/");
        doc.html = Some(html.to_string());
        match EmbeddedVideoExtractor::default().extract(&doc) {
            ExtractOutcome::Fallback(_) => {}
            other => panic!("expected Fallback, got {other:?}"),
        }
    }

    #[test]
    fn accepts_a_known_video_host_embed_and_object() {
        let html = concat!(
            r#"<embed src="https://player.vimeo.com/video/12345" type="video/mp4">"#,
            r#"<object data="https://player.twitch.tv/?video=12345" type="text/html"></object>"#,
        );
        let videos = extracted_videos(html);
        assert_eq!(videos.len(), 2);
        assert_eq!(field(&videos[0], "type"), Some("embed"));
        assert_eq!(field(&videos[1], "type"), Some("object"));
    }

    #[test]
    fn deduplicates_repeated_urls() {
        let html = concat!(
            r#"<iframe src="https://youtu.be/dQw4w9WgXcQ"></iframe>"#,
            r#"<iframe src="https://youtu.be/dQw4w9WgXcQ"></iframe>"#,
        );
        let videos = extracted_videos(html);
        assert_eq!(videos.len(), 1);
    }

    #[test]
    fn extract_falls_back_when_no_video_elements_are_present() {
        let mut doc = Document::new("https://example.com/");
        doc.html = Some("<html><body><p>hi</p></body></html>".to_string());
        match EmbeddedVideoExtractor::default().extract(&doc) {
            ExtractOutcome::Fallback(_) => {}
            other => panic!("expected Fallback, got {other:?}"),
        }
    }

    #[test]
    fn matches_only_when_a_quick_check_tag_is_present() {
        let ext = EmbeddedVideoExtractor::default();

        let mut empty = Document::new("https://example.com/");
        empty.html = Some(String::new());
        assert!(!ext.matches(&empty));

        let mut no_tags = Document::new("https://example.com/");
        no_tags.html = Some("<html><body><p>hi</p></body></html>".to_string());
        assert!(!ext.matches(&no_tags));

        let mut with_iframe = Document::new("https://example.com/");
        with_iframe.html = Some(r#"<iframe src="https://example.com"></iframe>"#.to_string());
        assert!(ext.matches(&with_iframe));
    }
}
