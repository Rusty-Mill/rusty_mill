//! `Mastodon` — a Rust port of
//! `server/extractor/extractors/mastodon/extractor.go` (capability
//! inventory §4.5.13). Decomposes a Mastodon timeline/status page into
//! one [`Document`] per visible toot, the first extractor in this crate
//! to use [`Document::extra_documents`]/[`Document::skip_indexing`] — a
//! `rusty-hister-core` capability added specifically to unblock this and
//! the two other one-page-to-many-documents extractors (Bluesky,
//! Twitter). The containing timeline page itself carries no standalone
//! content worth indexing, so `extract` always sets `skip_indexing`
//! before doing anything else; only the toot documents it appends to
//! `extra_documents` matter to a caller that walks that list.
//!
//! [`matches`](Extractor::matches) accepts two shapes: a real Mastodon
//! page (fingerprinted the same way Go does, by a `"repository":
//! "mastodon/mastodon"` substring Mastodon's own web frontend embeds),
//! and a toot document this extractor already produced (`metadata.type
//! == "toot"`) — a recursion guard, matching Go's own, so a future
//! indexer walking `extra_documents` and re-running the chain over each
//! entry doesn't re-explode an already-extracted toot.
//!
//! [`original_status_url`] ports Go's `originalStatusURL` byte-for-byte
//! in spirit: Mastodon's own UI links to a *remote* toot using a
//! federated-handle path shape (`/@user@remote.host/12345`, one
//! account's local proxy for someone else's post), and canonicalizes it
//! to that toot's real home-instance URL (`https://remote.host/@user/
//! 12345`) whenever the URL matches that exact shape — otherwise the
//! original URL is returned unchanged, matching Go's many narrow
//! validation guards (no userinfo, no extra path/query/fragment on the
//! reconstructed host, no `/?#` inside the decoded username/status-id).
//!
//! [`preview`](Extractor::preview) is a direct, deliberately-unenhanced
//! port of Go's own — Go's source carries a `// TODO enhance the toot
//! preview` comment on this exact method, and its actual behavior really
//! is that unfinished: it renders an optional `<h1>`-derived heading
//! followed by the *entire original page's* raw HTML, sanitized, rather
//! than anything scoped to a specific toot. Reproduced faithfully rather
//! than "fixed", since that would silently diverge from Go's own,
//! currently-shipped behavior.

use crate::sanitizer::sanitize_html;
use crate::stackexchange::{element_text, selector};
use crate::urlutil::{resolve_url, rewrite_urls};
use ammonia::Url;
use percent_encoding::percent_decode_str;
use rusty_hister_core::{
    Capabilities, Document, ExtractOutcome, Extractor, ExtractorConfig, HisterError,
    PreviewOutcome, PreviewResponse,
};
use scraper::{ElementRef, Html, Selector};

const MASTODON_FINGERPRINT: &str = r#""repository":"mastodon/mastodon""#;

fn status_selector() -> Selector {
    selector(".status, .detailed-status")
}

fn permalink_selector() -> Selector {
    selector(".status__relative-time, .detailed-status__datetime")
}

#[derive(Debug, Default)]
pub struct MastodonExtractor {
    config: ExtractorConfig,
}

impl Extractor for MastodonExtractor {
    fn name(&self) -> &str {
        "Mastodon"
    }

    fn description(&self) -> &str {
        "Extracts toots as individual documents from Mastodon websites."
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            enrich: false,
            extract: true,
            preview: true,
        }
    }

    fn matches(&self, document: &Document) -> bool {
        if document
            .html
            .as_deref()
            .is_some_and(|h| h.contains(MASTODON_FINGERPRINT))
        {
            return true;
        }
        is_toot_document(document)
    }

    fn extract(&self, document: &Document) -> ExtractOutcome {
        if is_toot_document(document) {
            return ExtractOutcome::Extracted(document.clone());
        }

        let mut extracted = document.clone();
        extracted.skip_indexing = true;

        let html_str = document.html.as_deref().unwrap_or("");
        let html = Html::parse_document(html_str);
        let statuses: Vec<ElementRef> = html.select(&status_selector()).collect();
        if statuses.is_empty() {
            return ExtractOutcome::Extracted(extracted);
        }

        let Ok(base) = Url::parse(&document.url) else {
            return ExtractOutcome::Fallback(HisterError::Extraction(
                "invalid document URL".to_string(),
            ));
        };

        for status in statuses {
            if let Some(toot) = build_toot_document(&status, &base) {
                extracted.extra_documents.push(toot);
            }
        }
        ExtractOutcome::Extracted(extracted)
    }

    fn preview(&self, document: &Document) -> PreviewOutcome {
        let html_str = document.html.as_deref().unwrap_or("");
        let html = Html::parse_document(html_str);

        let mut out = String::new();
        if let Some(title) = html
            .select(&selector("h1"))
            .next()
            .map(|el| element_text(&el).trim().to_string())
            .filter(|t| !t.is_empty())
        {
            out.push_str(&format!("<h2>{title}</h2>\n"));
        }
        out.push_str(html_str);

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

fn is_toot_document(document: &Document) -> bool {
    document.metadata.get("type").and_then(|v| v.as_str()) == Some("toot")
}

/// Builds one toot [`Document`] from a `.status`/`.detailed-status`
/// element, or `None` when it has no discoverable permalink — matching
/// Go's own early return (via a closure it just returns from) when the
/// relative-time/datetime element's `href` attribute is missing.
fn build_toot_document(status: &ElementRef, base: &Url) -> Option<Document> {
    let content = status.select(&selector(".status__content")).next()?;
    let href = status
        .select(&permalink_selector())
        .next()
        .and_then(|el| el.value().attr("href"))?;

    // Reparse the content subtree as its own fragment before rewriting
    // URLs, rather than mutating the shared `html` document in place —
    // the same "reparse as an independent document" trick
    // `WikipediaExtractor::extract`'s noise-removal clone and every
    // later extractor's own body-HTML handling uses, since
    // `scraper::ElementRef` has no in-place mutation API of its own.
    let mut fragment = Html::parse_fragment(&format!("<div>{}</div>", content.inner_html()));
    rewrite_urls(&mut fragment, base);
    let rewritten = fragment.select(&selector("div")).next()?;
    let content_html = rewritten.inner_html();
    let content_text = element_text(&rewritten);

    let status_url = resolve_url(base, href);
    let canonical_url = original_status_url(&status_url);

    let display_name = status
        .select(&selector(".display-name"))
        .next()
        .map(|el| element_text(&el))
        .unwrap_or_default();

    let mut toot = Document::new(canonical_url);
    toot.title = Some(format!("Mastodon toot: {display_name}"));
    toot.text = Some(content_text);
    toot.html = Some(content_html);
    toot.metadata
        .insert("type".to_string(), "toot".to_string().into());
    Some(toot)
}

/// Canonicalizes a Mastodon federated-handle permalink
/// (`https://local.instance/@user@remote.host/12345`) to that toot's
/// actual home-instance URL (`https://remote.host/@user/12345`) when the
/// URL matches that exact shape. Returns `raw_url` unchanged for
/// anything else — a same-instance toot URL (`/@user/12345`, no `@` in
/// the account segment) never matches and passes through untouched.
fn original_status_url(raw_url: &str) -> String {
    let Ok(status_url) = Url::parse(raw_url) else {
        return raw_url.to_string();
    };
    if status_url.scheme() != "http" && status_url.scheme() != "https" {
        return raw_url.to_string();
    }

    let parts: Vec<&str> = status_url
        .path()
        .trim_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    if parts.len() != 2 {
        return raw_url.to_string();
    }

    let Ok(account) = percent_decode_str(parts[0]).decode_utf8() else {
        return raw_url.to_string();
    };
    if !account.starts_with('@') {
        return raw_url.to_string();
    }

    let Some(separator) = account.rfind('@') else {
        return raw_url.to_string();
    };
    if separator <= 1 || separator == account.len() - 1 {
        return raw_url.to_string();
    }

    let username = &account[1..separator];
    let remote_host = &account[separator + 1..];
    let Ok(status_id) = percent_decode_str(parts[1]).decode_utf8() else {
        return raw_url.to_string();
    };
    if format!("{username}{status_id}").contains(['/', '?', '#']) {
        return raw_url.to_string();
    }

    let Ok(remote_url) = Url::parse(&format!("https://{remote_host}")) else {
        return raw_url.to_string();
    };
    let Some(host) = remote_url.host_str().filter(|h| !h.is_empty()) else {
        return raw_url.to_string();
    };
    let reconstructed_host = match remote_url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    };
    if !remote_url.username().is_empty()
        || remote_url.password().is_some()
        || reconstructed_host != remote_host
        || remote_url.path() != "/"
        || remote_url.query().is_some()
        || remote_url.fragment().is_some()
    {
        return raw_url.to_string();
    }

    format!("https://{remote_host}/@{username}/{status_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(url: &str, html: &str) -> Document {
        let mut d = Document::new(url);
        d.html = Some(html.to_string());
        d
    }

    const TIMELINE_HTML: &str = r#"<html><body>
        <script>window.data = {"repository":"mastodon/mastodon"};</script>
        <div class="status">
            <a class="display-name">Jane Doe</a>
            <div class="status__content">Hello <a href="/tags/rust">#rust</a> world!</div>
            <a class="status__relative-time" href="/@jane/109999999999999999"></a>
        </div>
        <div class="detailed-status">
            <a class="display-name">John Roe</a>
            <div class="status__content">A detailed toot with <a href="/tags/mastodon">#mastodon</a>.</div>
            <a class="detailed-status__datetime" href="https://example.social/@john@remote.example/42"></a>
        </div>
    </body></html>"#;

    #[test]
    fn matches_mastodon_fingerprint_and_toot_documents() {
        let ext = MastodonExtractor::default();
        assert!(ext.matches(&doc("https://example.social/@jane", TIMELINE_HTML)));

        let mut toot = doc("https://example.social/@jane/1", "");
        toot.metadata
            .insert("type".to_string(), "toot".to_string().into());
        assert!(ext.matches(&toot));

        assert!(!ext.matches(&doc("https://example.social/@jane", "<html></html>")));
    }

    #[test]
    fn extract_sets_skip_indexing_and_collects_toots_as_extra_documents() {
        let ext = MastodonExtractor::default();
        let d = doc("https://example.social/@jane", TIMELINE_HTML);
        let ExtractOutcome::Extracted(extracted) = ext.extract(&d) else {
            panic!("expected Extracted");
        };
        assert!(extracted.skip_indexing);
        assert_eq!(extracted.extra_documents.len(), 2);

        let first = &extracted.extra_documents[0];
        assert_eq!(first.title.as_deref(), Some("Mastodon toot: Jane Doe"));
        assert!(first.text.as_deref().unwrap().contains("Hello"));
        assert!(first
            .html
            .as_deref()
            .unwrap()
            .contains(r#"href="https://example.social/tags/rust""#));
        assert_eq!(
            first.metadata.get("type").and_then(|v| v.as_str()),
            Some("toot")
        );
    }

    #[test]
    fn extract_canonicalizes_a_federated_permalink() {
        let ext = MastodonExtractor::default();
        let d = doc("https://example.social/@jane", TIMELINE_HTML);
        let ExtractOutcome::Extracted(extracted) = ext.extract(&d) else {
            panic!("expected Extracted");
        };
        let second = &extracted.extra_documents[1];
        assert_eq!(second.url, "https://remote.example/@john/42");
    }

    #[test]
    fn extract_returns_toot_document_unchanged_for_recursion_guard() {
        let ext = MastodonExtractor::default();
        let mut toot = doc("https://remote.example/@john/42", "");
        toot.metadata
            .insert("type".to_string(), "toot".to_string().into());
        toot.text = Some("already extracted".to_string());

        let ExtractOutcome::Extracted(extracted) = ext.extract(&toot) else {
            panic!("expected Extracted");
        };
        assert!(!extracted.skip_indexing);
        assert_eq!(extracted.text.as_deref(), Some("already extracted"));
    }

    #[test]
    fn extract_marks_skip_indexing_even_with_no_visible_toots() {
        let ext = MastodonExtractor::default();
        let d = doc(
            "https://example.social/@jane",
            r#"<html><body><script>{"repository":"mastodon/mastodon"}</script></body></html>"#,
        );
        let ExtractOutcome::Extracted(extracted) = ext.extract(&d) else {
            panic!("expected Extracted");
        };
        assert!(extracted.skip_indexing);
        assert!(extracted.extra_documents.is_empty());
    }

    #[test]
    fn preview_renders_a_heading_and_the_whole_raw_page_html() {
        let ext = MastodonExtractor::default();
        let d = doc(
            "https://example.social/@jane",
            r#"<html><body><h1>My Timeline</h1><p onclick="evil()">hi</p></body></html>"#,
        );
        let PreviewOutcome::Previewed(response) = ext.preview(&d) else {
            panic!("expected Previewed");
        };
        let content = response.html.unwrap();
        assert!(content.contains("<h2>My Timeline</h2>"));
        assert!(content.contains("<p>hi</p>"));
        assert!(!content.contains("onclick"));
    }
}
