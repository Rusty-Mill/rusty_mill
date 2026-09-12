//! `JSONLD` — a Rust port of
//! `server/extractor/extractors/jsonld/jsonld.go`. Enrich-only: parses
//! every `<script type="application/ld+json">` block on a page, flattens
//! `@graph`/array wrappers, and stores the normalized node list plus two
//! classification fields (`type`, `headline`) on the document's metadata.
//! Readability (§4.4, not yet ported) already harvests author/
//! description/image/date fields from the same JSON-LD data plus
//! OpenGraph/meta tags and runs after this extractor in the default
//! chain, so this deliberately only writes the two fields Readability
//! doesn't: schema.org `@type` (for classification) and `headline`
//! (distinct from the HTML `<title>`).
//!
//! **Scoped, hand-rolled HTML/text handling, not a general-purpose
//! library.** Go's version tokenizes with `golang.org/x/net/html` and
//! sanitizes strings through `server/sanitizer`'s `bluemonday`-backed
//! strict text policy. Neither a general HTML parser nor a general HTML
//! sanitizer exists in this crate cluster yet, and most of the other 19
//! built-in extractors will eventually need one or both — a bigger,
//! cross-cutting dependency decision than this one extractor should make
//! unilaterally. This file instead hand-rolls exactly the two narrow
//! operations it needs: [`find_json_ld_blobs`] (locate
//! `<script type="application/ld+json">` bodies via a small state
//! machine, not a DOM parser) and [`sanitize_text`] (strip HTML tags,
//! decode a practical subset of HTML entities, trim — matching the
//! *observable behavior* of Go's `bluemonday.StrictPolicy()` +
//! `html.UnescapeString()` for plain strings, without depending on
//! either library). A real `sanitizer`/HTML-parsing module, once a later
//! extractor needs more than this, should replace both — flagged in
//! `docs/PROJECT-STATUS.md`.
//!
//! JSON parsing uses `rusty_json` (already this cluster's metadata JSON
//! type, `rusty_hister_core::Metadata = rusty_json::Map`), not `serde_json`
//! — no new dependency needed for that half of this extractor.

use rusty_hister_core::{
    Capabilities, Document, ExtractOutcome, Extractor, ExtractorConfig, HisterError, Metadata,
    PreviewOutcome,
};
use rusty_json::Value;

const SCRIPT_TYPE_MARKER: &str = "application/ld+json";

/// `@type` values that make a JSON-LD node a good source for the
/// document's classification fields, in descending priority order (Go:
/// `preferredTypes`).
const PREFERRED_TYPES: &[&str] = &[
    "Article",
    "NewsArticle",
    "BlogPosting",
    "WebPage",
    "Product",
    "Recipe",
    "VideoObject",
    "Event",
    "Person",
    "Organization",
];

/// Parses `application/ld+json` script tags and stores normalized
/// schema.org metadata on the document (Go: `JSONLDExtractor`).
#[derive(Debug, Default)]
pub struct JsonLdExtractor {
    config: ExtractorConfig,
}

impl Extractor for JsonLdExtractor {
    fn name(&self) -> &str {
        "jsonld"
    }

    fn description(&self) -> &str {
        "Parses application/ld+json script tags and stores normalized schema.org metadata on the document."
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            enrich: true,
            extract: false,
            preview: false,
        }
    }

    /// A cheap substring pre-check, mirroring Go's own `strings.Contains`
    /// guard: tokenizing the whole document just to rule out most pages
    /// would be wasteful, so this only runs the real (still narrow, see
    /// this module's doc comment) parse in `extract` once the marker is
    /// present at all.
    fn matches(&self, document: &Document) -> bool {
        document
            .html
            .as_deref()
            .is_some_and(|html| html.contains(SCRIPT_TYPE_MARKER))
    }

    fn extract(&self, document: &Document) -> ExtractOutcome {
        let Some(html) = document.html.as_deref() else {
            return ExtractOutcome::Fallback(HisterError::Extraction(
                "document has no HTML".to_string(),
            ));
        };

        let blobs = find_json_ld_blobs(html);
        if blobs.is_empty() {
            return ExtractOutcome::Fallback(HisterError::Extraction(
                "no application/ld+json script blocks found".to_string(),
            ));
        }

        let mut nodes: Vec<Value> = Vec::new();
        for blob in &blobs {
            if let Ok(parsed) = Value::parse(blob) {
                flatten_into(parsed, &mut nodes);
            }
        }
        if nodes.is_empty() {
            return ExtractOutcome::Fallback(HisterError::Extraction(
                "no valid JSON-LD nodes found".to_string(),
            ));
        }

        for node in &mut nodes {
            sanitize_node(node);
        }

        let mut extracted = document.clone();
        extracted.metadata.insert(
            "jsonld".to_string(),
            Value::String(Value::Array(nodes.clone()).to_json_string()),
        );

        let best = pick_best(&nodes);
        set_string(
            &mut extracted.metadata,
            "type",
            sanitize_text(&type_string(best)),
        );
        set_string(
            &mut extracted.metadata,
            "headline",
            sanitize_text(&first_string(best, &["headline", "name"])),
        );

        ExtractOutcome::Extracted(extracted)
    }

    /// Not implemented — Readability/Basic (§4.4, not yet ported) handle
    /// rendering (Go: `Preview` always returns `PreviewFallback(nil)`).
    fn preview(&self, _document: &Document) -> PreviewOutcome {
        PreviewOutcome::Fallback(HisterError::Extraction(
            "jsonld does not render previews".to_string(),
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

/// Finds every `<script type="application/ld+json">...</script>` block's
/// raw text content in `html`. A small, purpose-built scanner — not a
/// general HTML/DOM parser, see this module's doc comment — so it can be
/// fooled by pathological markup (e.g. an attribute literally named
/// `data-type` reads as `type` here); realistic pages don't do that.
fn find_json_ld_blobs(html: &str) -> Vec<String> {
    let lower = html.to_ascii_lowercase();
    let mut blobs = Vec::new();
    let mut pos = 0usize;

    while let Some(rel_start) = lower[pos..].find("<script") {
        let tag_start = pos + rel_start;
        let after_name = tag_start + "<script".len();
        match lower.as_bytes().get(after_name) {
            Some(b) if b.is_ascii_whitespace() || *b == b'>' || *b == b'/' => {}
            _ => {
                pos = after_name;
                continue;
            }
        }

        let Some(rel_tag_close) = lower[after_name..].find('>') else {
            break;
        };
        let tag_close = after_name + rel_tag_close;
        let is_json_ld = attr_value(&lower[after_name..tag_close], "type")
            .is_some_and(|v| v.trim() == SCRIPT_TYPE_MARKER);

        let content_start = tag_close + 1;
        let Some(rel_end_tag) = lower[content_start..].find("</script") else {
            break;
        };
        let content_end = content_start + rel_end_tag;
        let Some(rel_end_close) = lower[content_end..].find('>') else {
            break;
        };
        pos = content_end + rel_end_close + 1;

        if is_json_ld {
            let text = html[content_start..content_end].trim();
            if !text.is_empty() {
                blobs.push(text.to_string());
            }
        }
    }

    blobs
}

/// Extracts an attribute's raw value from `attrs` (a `<tag ...>`'s
/// attribute text, already lowercased) by a case-insensitive attribute
/// name. Handles `name="value"`, `name='value'`, and bare `name=value`.
fn attr_value<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{name}=");
    let start = attrs.find(&needle)? + needle.len();
    let rest = &attrs[start..];
    match rest.as_bytes().first() {
        Some(b'"') => rest[1..].find('"').map(|end| &rest[1..1 + end]),
        Some(b'\'') => rest[1..].find('\'').map(|end| &rest[1..1 + end]),
        _ => {
            let end = rest
                .find(|c: char| c.is_whitespace() || c == '>')
                .unwrap_or(rest.len());
            Some(&rest[..end])
        }
    }
}

/// Flattens a JSON-LD payload into a flat list of object nodes, unwrapping
/// `@graph` wrappers and top-level arrays (Go: `flatten`). Non-object,
/// non-array values (a malformed top-level string/number) contribute
/// nothing, same as Go's type switch falling through to `nil`.
fn flatten_into(value: Value, out: &mut Vec<Value>) {
    match value {
        Value::Object(mut map) => match map.remove("@graph") {
            Some(graph) => flatten_into(graph, out),
            None => out.push(Value::Object(map)),
        },
        Value::Array(items) => {
            for item in items {
                flatten_into(item, out);
            }
        }
        _ => {}
    }
}

/// Returns the first node whose `@type` matches one of [`PREFERRED_TYPES`],
/// falling back to the first node (Go: `pickBest`). `nodes` is never
/// empty at the call site (checked by `extract` beforehand).
fn pick_best(nodes: &[Value]) -> &Value {
    for want in PREFERRED_TYPES {
        if let Some(found) = nodes.iter().find(|n| type_matches(n, want)) {
            return found;
        }
    }
    &nodes[0]
}

fn type_matches(node: &Value, want: &str) -> bool {
    match node.get("@type") {
        Some(Value::String(s)) => s == want,
        Some(Value::Array(items)) => items
            .iter()
            .any(|item| matches!(item, Value::String(s) if s == want)),
        _ => false,
    }
}

fn type_string(node: &Value) -> String {
    match node.get("@type") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .find_map(|item| match item {
                Value::String(s) if !s.is_empty() => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn first_string(node: &Value, keys: &[&str]) -> String {
    for key in keys {
        if let Some(Value::String(s)) = node.get(key) {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    String::new()
}

fn set_string(metadata: &mut Metadata, key: &str, value: String) {
    if !value.is_empty() {
        metadata.insert(key.to_string(), Value::String(value));
    }
}

/// Sanitizes every non-`@`-prefixed field of a JSON-LD node in place, so
/// the raw dump stored at `Metadata["jsonld"]` cannot carry untrusted HTML
/// into downstream consumers (Go: `sanitizeNodes`/`sanitizeMap`).
/// `@`-prefixed keys (`@context`, `@type`, `@id`) are left untouched —
/// they're structural identifiers, not free-form text.
fn sanitize_node(node: &mut Value) {
    if let Value::Object(map) = node {
        for (key, value) in map.iter_mut() {
            if !key.starts_with('@') {
                sanitize_value(value);
            }
        }
    }
}

fn sanitize_value(value: &mut Value) {
    match value {
        Value::String(s) => *s = sanitize_text(s),
        Value::Object(map) => {
            for (key, v) in map.iter_mut() {
                if !key.starts_with('@') {
                    sanitize_value(v);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(sanitize_value),
        _ => {}
    }
}

/// Approximates Go's `sanitizer.SanitizeText` (`bluemonday.StrictPolicy()`
/// then `html.UnescapeString()` then trim) for a plain string: strips
/// every HTML tag, decodes a practical subset of HTML entities, and trims.
fn sanitize_text(s: &str) -> String {
    unescape_html_entities(&strip_html_tags(s))
        .trim()
        .to_string()
}

/// Removes everything between (and including) `<`/`>` pairs, keeping text
/// nodes concatenated as-is (no synthesized whitespace at tag boundaries,
/// matching Go's tag-stripping behavior).
fn strip_html_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// Decodes `&amp;`/`&lt;`/`&gt;`/`&quot;`/`&apos;` and numeric character
/// references (`&#NNN;`/`&#xHHHH;`). Not an exhaustive named-entity table
/// (there are 2000+ in the HTML5 spec) — the common subset that actually
/// appears in JSON-LD text fields in practice; an unrecognized entity is
/// left as literal text rather than dropped.
fn unescape_html_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp_pos) = rest.find('&') {
        out.push_str(&rest[..amp_pos]);
        let after_amp = &rest[amp_pos + 1..];
        if let Some(semi_pos) = after_amp.find(';') {
            let entity = &after_amp[..semi_pos];
            if let Some(decoded) = decode_entity(entity) {
                out.push(decoded);
                rest = &after_amp[semi_pos + 1..];
                continue;
            }
        }
        out.push('&');
        rest = after_amp;
    }
    out.push_str(rest);
    out
}

fn decode_entity(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            if let Some(hex) = entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
            {
                u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
            } else if let Some(dec) = entity.strip_prefix('#') {
                dec.parse::<u32>().ok().and_then(char::from_u32)
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extracted_metadata(html: &str) -> Metadata {
        let mut doc = Document::new("https://example.com/");
        doc.html = Some(html.to_string());
        match JsonLdExtractor::default().extract(&doc) {
            ExtractOutcome::Extracted(result) => result.metadata,
            other => panic!("expected Extracted, got {other:?}"),
        }
    }

    fn jsonld_nodes(metadata: &Metadata) -> Vec<Value> {
        match metadata.get("jsonld") {
            Some(Value::String(raw)) => match Value::parse(raw).unwrap() {
                Value::Array(nodes) => nodes,
                other => panic!("expected an array, got {other:?}"),
            },
            other => panic!("expected Metadata[\"jsonld\"] to be a string, got {other:?}"),
        }
    }

    fn metadata_str<'a>(metadata: &'a Metadata, key: &str) -> Option<&'a str> {
        match metadata.get(key) {
            Some(Value::String(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    #[test]
    fn extracts_type_and_headline_from_a_single_article_node() {
        let html = r#"<html><head><script type="application/ld+json">{
            "@context": "https://schema.org",
            "@type": "Article",
            "name": "Kristi Noem",
            "headline": "Kristi Noem",
            "author": {"@type": "Organization", "name": "Contributors"},
            "datePublished": "2010-01-01T00:00:00Z"
        }</script></head><body></body></html>"#;

        let metadata = extracted_metadata(html);
        assert_eq!(metadata_str(&metadata, "type"), Some("Article"));
        assert_eq!(metadata_str(&metadata, "headline"), Some("Kristi Noem"));
        // Fields readability owns are never written by this extractor.
        for key in ["author", "description", "image", "published", "modified"] {
            assert!(metadata.get(key).is_none(), "unexpected Metadata[{key}]");
        }
        assert_eq!(jsonld_nodes(&metadata).len(), 1);
    }

    #[test]
    fn flattens_an_at_graph_wrapper_into_its_member_nodes() {
        let html = r#"<html><head><script type="application/ld+json">{
            "@context": "https://schema.org",
            "@graph": [
                {"@type": "WebPage", "name": "About Us", "description": "The about page"},
                {"@type": "BreadcrumbList", "itemListElement": []},
                {"@type": "Organization", "name": "ACME"}
            ]
        }</script></head></html>"#;

        let metadata = extracted_metadata(html);
        assert_eq!(jsonld_nodes(&metadata).len(), 3);
        assert_eq!(metadata_str(&metadata, "type"), Some("WebPage"));
        assert_eq!(metadata_str(&metadata, "headline"), Some("About Us"));
    }

    #[test]
    fn collects_nodes_from_multiple_script_tags_and_prefers_article() {
        let html = r#"<html><head>
            <script type="application/ld+json">{"@type": "Organization", "name": "ACME"}</script>
            <script type="application/ld+json">{"@type": "Article", "headline": "Hello"}</script>
        </head></html>"#;

        let metadata = extracted_metadata(html);
        assert_eq!(jsonld_nodes(&metadata).len(), 2);
        assert_eq!(metadata_str(&metadata, "type"), Some("Article"));
    }

    #[test]
    fn accepts_a_top_level_array_of_nodes() {
        let html = r#"<html><head><script type="application/ld+json">[
            {"@type": "Person", "name": "Alice"},
            {"@type": "Person", "name": "Bob"}
        ]</script></head></html>"#;

        let metadata = extracted_metadata(html);
        assert_eq!(jsonld_nodes(&metadata).len(), 2);
    }

    #[test]
    fn skips_a_malformed_blob_and_keeps_the_valid_one() {
        let html = r#"<html><head>
            <script type="application/ld+json">{not valid json</script>
            <script type="application/ld+json">{"@type": "Article", "headline": "Survives"}</script>
        </head></html>"#;

        let metadata = extracted_metadata(html);
        assert_eq!(metadata_str(&metadata, "headline"), Some("Survives"));
        assert_eq!(jsonld_nodes(&metadata).len(), 1);
    }

    #[test]
    fn extract_falls_back_when_no_json_ld_is_present() {
        let mut doc = Document::new("https://example.com/");
        doc.html = Some("<html><body><p>hi</p></body></html>".to_string());
        match JsonLdExtractor::default().extract(&doc) {
            ExtractOutcome::Fallback(_) => {}
            other => panic!("expected Fallback, got {other:?}"),
        }
    }

    #[test]
    fn matches_only_when_the_marker_substring_is_present() {
        let ext = JsonLdExtractor::default();

        let mut empty = Document::new("https://example.com/");
        empty.html = Some(String::new());
        assert!(!ext.matches(&empty));

        let mut no_marker = Document::new("https://example.com/");
        no_marker.html = Some("<html><body><p>hi</p></body></html>".to_string());
        assert!(!ext.matches(&no_marker));

        let mut with_marker = Document::new("https://example.com/");
        with_marker.html = Some(r#"<script type="application/ld+json">{}</script>"#.to_string());
        assert!(ext.matches(&with_marker));
    }

    #[test]
    fn sanitizes_the_headline_stripping_tags_and_decoding_entities() {
        let html = r#"<script type="application/ld+json">{
            "@type": "Article",
            "headline": "Smith &amp; Jones: <i>an unlikely<\/i> story"
        }</script>"#;

        let metadata = extracted_metadata(html);
        assert_eq!(
            metadata_str(&metadata, "headline"),
            Some("Smith & Jones: an unlikely story")
        );
    }

    #[test]
    fn deep_sanitizes_the_raw_jsonld_dump_but_preserves_structural_keys() {
        let html = r#"<script type="application/ld+json">{
            "@type": "Article",
            "headline": "Smith &amp; Jones",
            "author": {"@type": "Person", "name": "<b>Jane<\/b>"},
            "keywords": ["<i>go<\/i>", "hister"]
        }</script>"#;

        let metadata = extracted_metadata(html);
        let nodes = jsonld_nodes(&metadata);
        assert_eq!(nodes.len(), 1);
        let node = &nodes[0];

        assert_eq!(
            node.get("headline"),
            Some(&Value::String("Smith & Jones".to_string()))
        );
        assert_eq!(
            node.get("author").and_then(|a| a.get("name")),
            Some(&Value::String("Jane".to_string()))
        );
        match node.get("keywords") {
            Some(Value::Array(items)) => {
                assert_eq!(
                    items,
                    &vec![
                        Value::String("go".to_string()),
                        Value::String("hister".to_string()),
                    ]
                );
            }
            other => panic!("expected an array, got {other:?}"),
        }
        // @-prefixed structural keys are preserved verbatim.
        assert_eq!(
            node.get("@type"),
            Some(&Value::String("Article".to_string()))
        );
    }

    #[test]
    fn strip_html_tags_keeps_text_between_and_outside_tags() {
        assert_eq!(strip_html_tags("<b>bold</b> plain"), "bold plain");
        assert_eq!(strip_html_tags("no tags here"), "no tags here");
    }

    #[test]
    fn unescape_html_entities_decodes_named_and_numeric_references() {
        assert_eq!(unescape_html_entities("a &amp; b"), "a & b");
        assert_eq!(unescape_html_entities("O&#39;Brien"), "O'Brien");
        assert_eq!(unescape_html_entities("&unknown; stays"), "&unknown; stays");
    }
}
