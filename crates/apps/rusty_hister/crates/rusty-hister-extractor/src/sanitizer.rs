//! A Rust port of `server/sanitizer/sanitizer.go`: an HTML allow-list policy
//! built on `ammonia` (in place of Go's `bluemonday`) for extractors whose
//! `preview()` output embeds real, attacker-controlled HTML from the source
//! page. `jsonld.rs`'s own hand-rolled `sanitize_text` predates this module
//! (written before `ammonia` was a dependency of this crate) and is left as
//! it is — it only ever produces plain-text metadata, never rendered HTML,
//! so it has no need for this module's tag/attribute allow-listing; its
//! entity-decoding helper is reused here via `pub(crate)` rather than
//! duplicated (see [`crate::jsonld::unescape_html_entities`]).
//!
//! Ports:
//! - [`sanitize_html`] — `sanitizer.SanitizeHTML`: the general-purpose
//!   policy for arbitrary third-party HTML.
//! - [`sanitize_trusted_html`] — `sanitizer.SanitizeTrustedHTML`: additionally
//!   permits layout CSS (`display`/`position`/`float`/`top`/`left`/`right`/
//!   `bottom`) for extractors whose source is editorially moderated (e.g.
//!   Wikipedia). Never use this for arbitrary third-party HTML.
//! - [`sanitize_text`] — `sanitizer.SanitizeText`: strips every tag, decodes
//!   entities, and trims, for safe plain-text metadata.
//!
//! Approximations from the `bluemonday` original, deliberately documented
//! rather than silently dropped:
//! - `bluemonday`'s per-attribute-value regex validation (used throughout
//!   the SVG attribute allow-list, e.g. `d` on `<path>` must look like path
//!   data) is reproduced with hand-rolled character-class predicates driven
//!   through `ammonia::Builder::attribute_filter`, rather than a `regex`
//!   dependency — this crate doesn't otherwise need general regex support,
//!   and the checks a `<svg>` attribute allow-list needs are all simple
//!   character-class/enum tests. A few of Go's own regexes are themselves
//!   loose (documented at each predicate below); this port matches that
//!   looseness rather than quietly tightening it, since these are
//!   presentation-only attributes with no injection risk either way.
//! - Go's `AllowDataURIImages()` scopes `data:` URIs to `<img src>` only.
//!   `ammonia` validates a URL attribute's scheme once, before any
//!   attribute-level callback runs, with no per-element carve-out at that
//!   stage — so `data` is allowed workspace-wide in `url_schemes` and then
//!   [`filter_attribute`] strips any `data:` value that isn't on
//!   `img[src]`, reproducing the same net effect from the other direction.
//! - Two of Go's own `svgAttrRules` entries scope to elements
//!   (`symbol`, `col`, `colgroup`) that are not themselves in this policy's
//!   (or Go's) allowed-tags list, making those particular element scopes
//!   dead code in the source being ported — an element not on the tag
//!   allow-list is stripped entirely before its attributes ever matter.
//!   Reproduced as-is (harmlessly) rather than special-cased away.

use crate::jsonld::unescape_html_entities;
use ammonia::Builder;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

/// Non-SVG elements allow-listed by both sanitizer variants (`bluemonday`
/// policy's `AllowElements` call, minus the SVG element set below).
const ALLOWED_TAGS: &[&str] = &[
    "a",
    "abbr",
    "b",
    "br",
    "canvas",
    "caption",
    "center",
    "cite",
    "code",
    "dd",
    "del",
    "details",
    "div",
    "dl",
    "dt",
    "em",
    "figcaption",
    "figure",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "i",
    "img",
    "ins",
    "kbd",
    "label",
    "li",
    "math",
    "marquee",
    "media",
    "mediagroup",
    "noscript",
    "ol",
    "p",
    "pre",
    "source",
    "span",
    "strong",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "title",
    "tr",
    "tt",
    "u",
    "ul",
    "video",
];

/// SVG elements allow-listed by both sanitizer variants. `foreignObject`,
/// `use`, and `symbol` are excluded because they can embed or reference
/// third-party content (HTML subtrees, external SVG via `xlink:href`).
const SVG_ELEMENTS: &[&str] = &[
    "svg",
    "g",
    "path",
    "rect",
    "circle",
    "ellipse",
    "line",
    "polyline",
    "polygon",
    "text",
    "tspan",
    "defs",
    "clipPath",
    "mask",
    "linearGradient",
    "radialGradient",
    "stop",
    "marker",
    "pattern",
];

/// `style` attribute properties allowed by both sanitizer variants. Styles
/// that can take a `url()` value (background-image, `list-style-image`,
/// `cursor`, ...) are intentionally excluded — they can pull in third-party
/// content.
const ALLOWED_STYLES: &[&str] = &[
    "text-decoration",
    "color",
    "font-size",
    "font-weight",
    "font-style",
    "text-align",
    "background-color",
    "border",
    "border-top",
    "border-bottom",
    "border-left",
    "border-right",
    "border-collapse",
    "border-spacing",
    "border-color",
    "border-radius",
    "padding",
    "padding-top",
    "padding-bottom",
    "padding-left",
    "padding-right",
    "margin",
    "margin-top",
    "margin-bottom",
    "margin-left",
    "margin-right",
    "width",
    "max-width",
    "min-width",
    "height",
    "max-height",
    "clear",
    "z-index",
    "vertical-align",
    "line-height",
    "white-space",
    "overflow",
    "overflow-x",
    "overflow-y",
    "flex-wrap",
    "align-items",
    "justify-content",
    "gap",
    "caption-side",
    "empty-cells",
    "transform",
    "opacity",
    "list-style-type",
    "-webkit-print-color-adjust",
    "print-color-adjust",
];

/// Positioning/layout CSS properties denied by [`sanitize_html`] because
/// attacker-controlled content could use them to overlap the host page
/// (e.g. a fake URL bar positioned over the real one). Permitted only by
/// [`sanitize_trusted_html`], for extractors whose source is editorially
/// moderated (e.g. Wikipedia).
const TRUSTED_LAYOUT_STYLES: &[&str] = &[
    "display", "position", "float", "top", "left", "right", "bottom",
];

const VIEWBOX_ELEMENTS: &[&str] = &["svg", "symbol", "pattern", "marker"];
const TEXT_ANCHOR_ELEMENTS: &[&str] = &["text", "tspan"];
const FONT_ELEMENTS: &[&str] = &["text", "tspan", "svg"];
const GRADIENT_ELEMENTS: &[&str] = &["linearGradient", "radialGradient"];
const GRADIENT_UNITS_ELEMENTS: &[&str] = &["linearGradient", "radialGradient", "pattern"];
const MARKER_ELEMENTS: &[&str] = &["marker"];

/// Sanitizes HTML from an arbitrary, untrusted third-party source.
pub fn sanitize_html(html: &str) -> String {
    build_policy(false).clean(html).to_string()
}

/// Sanitizes HTML from an editorially-moderated source (e.g. Wikipedia),
/// additionally permitting layout CSS. Do not use for arbitrary third-party
/// HTML.
pub fn sanitize_trusted_html(html: &str) -> String {
    build_policy(true).clean(html).to_string()
}

/// Strips every HTML tag, decodes entities, and trims surrounding
/// whitespace — safe plain text suitable for storing in metadata or
/// displaying verbatim.
pub fn sanitize_text(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let stripped = Builder::empty().clean(trimmed).to_string();
    unescape_html_entities(&stripped).trim().to_string()
}

fn build_policy(trusted: bool) -> Builder<'static> {
    let mut builder = Builder::empty();

    let mut tags: HashSet<&str> = ALLOWED_TAGS.iter().copied().collect();
    tags.extend(SVG_ELEMENTS.iter().copied());
    builder.tags(tags);

    builder.generic_attributes(
        ["alt", "title", "aria-hidden", "style"]
            .into_iter()
            .collect(),
    );

    let mut tag_attributes: HashMap<&str, HashSet<&str>> = HashMap::new();
    tag_attributes.insert("a", ["href"].into_iter().collect());
    tag_attributes.insert("img", ["src", "srcset"].into_iter().collect());
    tag_attributes.insert("source", ["src", "srcset"].into_iter().collect());
    for el in SVG_ELEMENTS {
        let set = tag_attributes.entry(el).or_default();
        set.insert("id");
        set.extend([
            "x", "y", "x1", "y1", "x2", "y2", "cx", "cy", "r", "rx", "ry", "width", "height", "dx",
            "dy",
        ]);
        set.extend([
            "fill",
            "stroke",
            "stop-color",
            "fill-opacity",
            "stroke-opacity",
            "opacity",
            "stop-opacity",
            "stroke-width",
            "stroke-dasharray",
            "stroke-linecap",
            "stroke-linejoin",
            "fill-rule",
            "clip-rule",
            "transform",
            "pointer-events",
            "clip-path",
            "mask",
        ]);
    }
    for el in VIEWBOX_ELEMENTS {
        tag_attributes
            .entry(el)
            .or_default()
            .extend(["viewBox", "preserveAspectRatio"]);
    }
    tag_attributes.entry("path").or_default().insert("d");
    for el in ["polyline", "polygon"] {
        tag_attributes.entry(el).or_default().insert("points");
    }
    for el in TEXT_ANCHOR_ELEMENTS {
        tag_attributes.entry(el).or_default().insert("text-anchor");
    }
    for el in FONT_ELEMENTS {
        tag_attributes
            .entry(el)
            .or_default()
            .extend(["font-size", "font-family", "font-weight"]);
    }
    tag_attributes.entry("stop").or_default().insert("offset");
    for el in GRADIENT_ELEMENTS {
        tag_attributes
            .entry(el)
            .or_default()
            .insert("gradientTransform");
    }
    for el in GRADIENT_UNITS_ELEMENTS {
        tag_attributes
            .entry(el)
            .or_default()
            .extend(["gradientUnits", "patternUnits"]);
    }
    for el in MARKER_ELEMENTS {
        tag_attributes.entry(el).or_default().extend([
            "markerWidth",
            "markerHeight",
            "refX",
            "refY",
            "orient",
        ]);
    }
    tag_attributes.entry("svg").or_default().extend([
        "baseProfile",
        "xmlns",
        "xmlns:xlink",
        "version",
    ]);
    builder.tag_attributes(tag_attributes);

    builder.url_schemes(["mailto", "http", "https", "data"].into_iter().collect());
    builder.link_rel(None);

    let mut styles: HashSet<&str> = ALLOWED_STYLES.iter().copied().collect();
    if trusted {
        styles.extend(TRUSTED_LAYOUT_STYLES.iter().copied());
    }
    builder.filter_style_properties(styles);

    builder.attribute_filter(filter_attribute);
    builder
}

fn filter_attribute<'v>(element: &str, attribute: &str, value: &'v str) -> Option<Cow<'v, str>> {
    let keep = match attribute {
        "id" => is_id(value),
        "href" => !is_data_uri(value),
        "src" => element == "img" || !is_data_uri(value),
        "x" | "y" | "x1" | "y1" | "x2" | "y2" | "cx" | "cy" | "r" | "rx" | "ry" | "width"
        | "height" | "dx" | "dy" | "fill-opacity" | "stroke-opacity" | "opacity"
        | "stop-opacity" | "stroke-width" | "markerWidth" | "markerHeight" | "refX" | "refY" => {
            is_num(value)
        }
        "fill" | "stroke" | "stop-color" => is_fill(value),
        "stroke-dasharray" => is_dasharray(value),
        "stroke-linecap" => matches!(value, "butt" | "round" | "square"),
        "stroke-linejoin" => matches!(value, "miter" | "round" | "bevel"),
        "fill-rule" | "clip-rule" => matches!(value, "nonzero" | "evenodd"),
        "transform" | "gradientTransform" => is_transform(value),
        "pointer-events" => matches!(
            value,
            "none"
                | "auto"
                | "visible"
                | "visiblePainted"
                | "visibleFill"
                | "visibleStroke"
                | "painted"
                | "fill"
                | "stroke"
                | "all"
        ),
        "clip-path" | "mask" => is_url_ref(value),
        "viewBox" => is_viewbox(value),
        "preserveAspectRatio" => is_preserve_aspect_ratio(value),
        "d" => is_path_data(value),
        "points" => is_points(value),
        "text-anchor" => matches!(value, "start" | "middle" | "end"),
        "font-size" | "font-family" | "font-weight" => is_paragraph_safe(value),
        "offset" => value == "none" || is_num(value),
        "gradientUnits" | "patternUnits" => matches!(value, "userSpaceOnUse" | "objectBoundingBox"),
        "orient" => is_orient(value),
        "baseProfile" => is_alpha(value),
        "xmlns" | "xmlns:xlink" => is_xmlns_url(value),
        "version" => is_num_pair(value),
        _ => true,
    };
    keep.then_some(Cow::Borrowed(value))
}

fn is_data_uri(value: &str) -> bool {
    value.to_ascii_lowercase().starts_with("data:")
}

/// Mirrors `^[a-zA-Z0-9_-]+$` (bluemonday's own `id` allow-list pattern).
fn is_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Mirrors `^-?[\d.]+(%|em|ex|px|pt|cm|mm|in)?$`.
fn is_num(value: &str) -> bool {
    let body = value.strip_prefix('-').unwrap_or(value);
    for unit in ["%", "em", "ex", "px", "pt", "cm", "mm", "in", ""] {
        if let Some(digits) = body.strip_suffix(unit) {
            if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit() || c == '.') {
                return true;
            }
        }
    }
    false
}

fn is_plain_num(value: &str) -> bool {
    let body = value.strip_prefix('-').unwrap_or(value);
    !body.is_empty() && body.chars().all(|c| c.is_ascii_digit() || c == '.')
}

/// Mirrors `^-?[\d.]+ -?[\d.]+$`.
fn is_num_pair(value: &str) -> bool {
    let mut parts = value.split(' ');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(a), Some(b), None) => is_plain_num(a) && is_plain_num(b),
        _ => false,
    }
}

/// Mirrors `^[\d.\- ]+$`.
fn is_viewbox(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | ' '))
}

/// Mirrors `^[a-zA-Z(),.\d\s\-]+$`.
fn is_transform(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|c| {
            c.is_ascii_alphabetic()
                || c.is_ascii_digit()
                || c.is_whitespace()
                || matches!(c, '(' | ')' | ',' | '.' | '-')
        })
}

/// Mirrors `^(none|currentColor|transparent|#[0-9a-fA-F]{3,8}|[a-zA-Z]+|var\(...\))$`.
fn is_fill(value: &str) -> bool {
    if matches!(value, "none" | "currentColor" | "transparent") {
        return true;
    }
    if let Some(hex) = value.strip_prefix('#') {
        if matches!(hex.len(), 3..=8) && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return true;
        }
    }
    if !value.is_empty() && value.chars().all(|c| c.is_ascii_alphabetic()) {
        return true;
    }
    if let Some(inner) = value
        .strip_prefix("var(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        let inner = inner.trim();
        if let Some(rest) = inner.strip_prefix("--") {
            let (name, fallback) = match rest.split_once(',') {
                Some((n, f)) => (n.trim(), Some(f.trim())),
                None => (rest.trim(), None),
            };
            let name_ok =
                !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
            let fallback_ok = fallback.is_none_or(|f| {
                let f = f.strip_prefix('#').unwrap_or(f);
                !f.is_empty() && f.chars().all(|c| c.is_ascii_alphanumeric())
            });
            return name_ok && fallback_ok;
        }
    }
    false
}

/// Mirrors Go's own `^[\d., ]+|none$` — an *unanchored* alternation (loose
/// by construction in the source being ported): valid if the value starts
/// with a run of digit/dot/comma/space characters, OR ends with "none",
/// not necessarily both/only. Kept loose to match, since this only gates a
/// presentation attribute (dash pattern), not anything injectable.
fn is_dasharray(value: &str) -> bool {
    let starts_numeric = value
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | ' '));
    starts_numeric || value.ends_with("none")
}

/// Mirrors `^[MmZzLlHhVvCcSsQqTtAaEe\d\s,.\-+]+$`.
fn is_path_data(value: &str) -> bool {
    const COMMANDS: &str = "MmZzLlHhVvCcSsQqTtAaEe";
    !value.is_empty()
        && value.chars().all(|c| {
            COMMANDS.contains(c)
                || c.is_ascii_digit()
                || c.is_whitespace()
                || matches!(c, ',' | '.' | '-' | '+')
        })
}

/// Mirrors `^[\d\s,.\-]+$`.
fn is_points(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_digit() || c.is_whitespace() || matches!(c, ',' | '.' | '-'))
}

/// Mirrors `^(none|url\(#[a-zA-Z0-9_-]+\))$`.
fn is_url_ref(value: &str) -> bool {
    if value == "none" {
        return true;
    }
    if let Some(inner) = value
        .strip_prefix("url(#")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        return !inner.is_empty()
            && inner
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'));
    }
    false
}

/// Mirrors bluemonday's `Paragraph` pattern:
/// `^[\p{L}\p{N}\s\-_',\[\]!\./\\\(\)]*$` — letters, numbers, whitespace,
/// and a fixed set of punctuation, no markup or script-y characters.
fn is_paragraph_safe(value: &str) -> bool {
    value.chars().all(|c| {
        c.is_alphanumeric()
            || c.is_whitespace()
            || matches!(
                c,
                '-' | '_' | '\'' | ',' | '[' | ']' | '!' | '.' | '/' | '\\' | '(' | ')'
            )
    })
}

/// Mirrors `^(none|xMi[dn]YMi[dn]|xMa[dx]YMa[dx]|xMi[dn]YMa[dx]|xMa[dx]YMi[dn])(\s+(meet|slice))?$`.
/// Faithfully reproduces a `[dx]`/`[dn]` quirk in the pattern being ported
/// (it permits some non-standard tokens like `xMadYMad` alongside the real
/// SVG alignment keywords) rather than tightening it — this only gates a
/// presentation attribute, and tightening here would silently narrow
/// behavior relative to the Go source this module ports.
fn is_preserve_aspect_ratio(value: &str) -> bool {
    let (align, rest) = match value.split_once(char::is_whitespace) {
        Some((a, r)) => (a, r.trim_start()),
        None => (value, ""),
    };
    let align_ok = align == "none" || is_align_token(align);
    let rest_ok = rest.is_empty() || matches!(rest, "meet" | "slice");
    align_ok && rest_ok
}

fn is_align_token(value: &str) -> bool {
    let Some(rest) = value.strip_prefix('x') else {
        return false;
    };
    for x_part in ["Mid", "Min", "Mad", "Max"] {
        if let Some(y_part) = rest.strip_prefix(x_part).and_then(|r| r.strip_prefix('Y')) {
            return matches!(y_part, "Mid" | "Min" | "Mad" | "Max");
        }
    }
    false
}

/// Mirrors `^(auto|auto-start-reverse|[\d.]+)$`.
fn is_orient(value: &str) -> bool {
    matches!(value, "auto" | "auto-start-reverse")
        || (!value.is_empty() && value.chars().all(|c| c.is_ascii_digit() || c == '.'))
}

/// Mirrors `^[a-zA-Z]+$`.
fn is_alpha(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|c| c.is_ascii_alphabetic())
}

/// Mirrors `^https?://` (a prefix check, not a full-string match, matching
/// the Go source's own unanchored-at-the-end pattern).
fn is_xmlns_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_html_strips_script_tags_and_their_content() {
        let out = sanitize_html("<p>hi</p><script>alert(1)</script>");
        assert_eq!(out, "<p>hi</p>");
    }

    #[test]
    fn sanitize_html_strips_disallowed_tags_but_keeps_their_text() {
        let out = sanitize_html("<marquee>keep</marquee><blink>drop-tag-keep-text</blink>");
        assert!(out.contains("keep"));
        assert!(out.contains("drop-tag-keep-text"));
        assert!(!out.contains("<blink"));
    }

    #[test]
    fn sanitize_html_strips_event_handler_attributes() {
        let out = sanitize_html(r#"<img src="pic.png" onerror="alert(1)">"#);
        assert!(!out.contains("onerror"));
        assert!(out.contains(r#"src="pic.png""#));
    }

    #[test]
    fn sanitize_html_keeps_href_on_anchors_only() {
        let out = sanitize_html(r#"<a href="https://example.com">link</a>"#);
        assert!(out.contains(r#"href="https://example.com""#));
    }

    #[test]
    fn sanitize_html_rejects_javascript_scheme_links() {
        let out = sanitize_html(r#"<a href="javascript:alert(1)">bad</a>"#);
        assert!(!out.contains("javascript:"));
    }

    #[test]
    fn sanitize_html_allows_data_uri_only_on_img_src() {
        let img = sanitize_html(r#"<img src="data:image/png;base64,AAAA">"#);
        assert!(img.contains("data:image/png"));

        let anchor = sanitize_html(r#"<a href="data:text/html,hi">bad</a>"#);
        assert!(!anchor.contains("data:"));
    }

    #[test]
    fn sanitize_html_filters_style_to_allowed_properties_and_strips_layout() {
        let out = sanitize_html(r#"<p style="color: red; position: absolute; top: 0;">x</p>"#);
        assert!(out.contains("color:red") || out.contains("color: red"));
        assert!(!out.contains("position"));
        assert!(!out.contains("top"));
    }

    #[test]
    fn sanitize_trusted_html_permits_layout_styles() {
        let out = sanitize_trusted_html(r#"<p style="position: absolute;">x</p>"#);
        assert!(out.contains("position"));
    }

    #[test]
    fn sanitize_html_keeps_valid_svg_path_data() {
        let out = sanitize_html(r#"<svg><path d="M10 10 L20 20 Z"/></svg>"#);
        assert!(out.contains(r#"d="M10 10 L20 20 Z""#));
    }

    #[test]
    fn sanitize_html_strips_svg_path_data_that_is_not_path_syntax() {
        let out = sanitize_html(r#"<svg><path d="javascript:alert(1)"/></svg>"#);
        assert!(!out.contains("javascript"));
    }

    #[test]
    fn sanitize_html_enforces_id_pattern_on_svg_elements() {
        let ok = sanitize_html(r#"<svg><rect id="valid-id_1"/></svg>"#);
        assert!(ok.contains(r#"id="valid-id_1""#));

        let bad = sanitize_html(r#"<svg><rect id="bad id!"/></svg>"#);
        assert!(!bad.contains("bad id!"));
    }

    #[test]
    fn sanitize_text_strips_tags_and_decodes_entities() {
        assert_eq!(sanitize_text("  <b>Tom &amp; Jerry</b>  "), "Tom & Jerry");
    }

    #[test]
    fn sanitize_text_of_empty_or_blank_input_is_empty() {
        assert_eq!(sanitize_text(""), "");
        assert_eq!(sanitize_text("   \n\t "), "");
    }
}
