//! A Rust port of `server/extractor/urlutil/urlutil.go`: shared URL helpers
//! for extractors whose `preview()` rewrites relative links in third-party
//! HTML to absolute ones before rendering it back to the caller.
//!
//! [`rewrite_urls`] mutates the whole document rather than one scoped
//! `goquery.Selection` the way Go's `RewriteURLs` does — `scraper`'s
//! `ElementRef` only borrows immutably from its `Html`, so scoping a
//! mutation pass to an arbitrary subtree while a selection over that same
//! tree is still borrowed isn't expressible without also carrying node-tree
//! types (`ego_tree::NodeId`) into this module's public API. A whole-document
//! pass is harmless for every current caller: each one serializes only the
//! specific subtree it cares about afterward (`ElementRef::html()`), so
//! rewriting attributes elsewhere in the tree has no observable effect —
//! but a future caller relying on some *other* part of the document being
//! left untouched would need a scoped variant added back.

use ammonia::Url;
use scraper::node::Element;
use scraper::{Html, Selector};

const URL_ATTRS: [&str; 3] = ["href", "src", "srcset"];

/// Resolves `reference` against `base`. Returns `reference` unchanged if it
/// is already absolute, a fragment, or a `data:` URI. Protocol-relative
/// URLs (`//host/...`) are resolved using `base`'s scheme.
pub fn resolve_url(base: &Url, reference: &str) -> String {
    if reference.is_empty() || reference.starts_with('#') || reference.starts_with("data:") {
        return reference.to_string();
    }
    if reference.starts_with("//") {
        return format!("{}:{}", base.scheme(), reference);
    }
    if Url::parse(reference).is_ok() {
        // Already absolute (has its own scheme) — returned as-is, not
        // re-serialized, matching Go's `if u.IsAbs() { return ref }`.
        return reference.to_string();
    }
    base.join(reference)
        .map(|joined| joined.to_string())
        .unwrap_or_else(|_| reference.to_string())
}

/// Rewrites each URL in a `srcset` attribute value against `base`.
pub fn resolve_srcset(base: &Url, srcset: &str) -> String {
    srcset
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let mut fields: Vec<String> = entry.split_whitespace().map(str::to_string).collect();
            if let Some(first) = fields.first_mut() {
                *first = resolve_url(base, first);
            }
            fields.join(" ")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Rewrites every relative `href`, `src`, and `srcset` attribute in `html`
/// to an absolute URL against `base`. See the module doc for why this is a
/// whole-document pass rather than one scoped to a subtree.
pub fn rewrite_urls(html: &mut Html, base: &Url) {
    let selector = Selector::parse("[href], [src], [srcset]").expect("static selector is valid");
    let ids: Vec<_> = html.select(&selector).map(|element| element.id()).collect();
    for id in ids {
        let Some(mut node_mut) = html.tree.get_mut(id) else {
            continue;
        };
        let scraper::Node::Element(element) = node_mut.value() else {
            continue;
        };
        for attr in URL_ATTRS {
            let Some(current) = attr_value(element, attr) else {
                continue;
            };
            let updated = if attr == "srcset" {
                resolve_srcset(base, &current)
            } else {
                resolve_url(base, &current)
            };
            set_attr_value(element, attr, updated);
        }
    }
}

fn attr_value(element: &Element, name: &str) -> Option<String> {
    element
        .attrs
        .iter()
        .find(|(qualname, _)| &*qualname.local == name)
        .map(|(_, value)| value.to_string())
}

fn set_attr_value(element: &mut Element, name: &str, value: String) {
    if let Some((_, existing)) = element
        .attrs
        .iter_mut()
        .find(|(qualname, _)| &*qualname.local == name)
    {
        *existing = value.into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://example.com/dir/page.html").unwrap()
    }

    #[test]
    fn resolve_url_leaves_empty_fragment_and_data_uri_refs_unchanged() {
        assert_eq!(resolve_url(&base(), ""), "");
        assert_eq!(resolve_url(&base(), "#section"), "#section");
        assert_eq!(
            resolve_url(&base(), "data:image/png;base64,AAAA"),
            "data:image/png;base64,AAAA"
        );
    }

    #[test]
    fn resolve_url_leaves_absolute_urls_unchanged() {
        assert_eq!(
            resolve_url(&base(), "https://other.example/x"),
            "https://other.example/x"
        );
    }

    #[test]
    fn resolve_url_resolves_protocol_relative_urls_with_base_scheme() {
        assert_eq!(
            resolve_url(&base(), "//cdn.example/img.png"),
            "https://cdn.example/img.png"
        );
    }

    #[test]
    fn resolve_url_resolves_relative_paths_against_base() {
        assert_eq!(
            resolve_url(&base(), "img.png"),
            "https://example.com/dir/img.png"
        );
        assert_eq!(
            resolve_url(&base(), "/img.png"),
            "https://example.com/img.png"
        );
        assert_eq!(
            resolve_url(&base(), "../up.png"),
            "https://example.com/up.png"
        );
    }

    #[test]
    fn resolve_srcset_resolves_each_url_and_keeps_descriptors() {
        let out = resolve_srcset(&base(), "small.png 1x, /big.png 2x");
        assert_eq!(
            out,
            "https://example.com/dir/small.png 1x, https://example.com/big.png 2x"
        );
    }

    #[test]
    fn resolve_srcset_skips_empty_entries() {
        assert_eq!(
            resolve_srcset(&base(), "a.png 1x, , b.png 2x"),
            "https://example.com/dir/a.png 1x, https://example.com/dir/b.png 2x"
        );
    }

    #[test]
    fn rewrite_urls_rewrites_relative_href_and_src_across_the_document() {
        let mut html = Html::parse_document(
            r#"<a href="page2.html">next</a><img src="/pic.png"><source srcset="a.png 1x, b.png 2x">"#,
        );
        rewrite_urls(&mut html, &base());
        let out = html.html();
        assert!(out.contains(r#"href="https://example.com/dir/page2.html""#));
        assert!(out.contains(r#"src="https://example.com/pic.png""#));
        assert!(out.contains("https://example.com/dir/a.png 1x, https://example.com/dir/b.png 2x"));
    }

    #[test]
    fn rewrite_urls_leaves_absolute_and_fragment_urls_unchanged() {
        let mut html =
            Html::parse_document(r##"<a href="https://other.example/">x</a><a href="#top">y</a>"##);
        rewrite_urls(&mut html, &base());
        let out = html.html();
        assert!(out.contains(r#"href="https://other.example/""#));
        assert!(out.contains(r##"href="#top""##));
    }
}
