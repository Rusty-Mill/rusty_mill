//! A Rust port of `server/extractor/extractors/wikipedia/text.go`: renders
//! infobox key/value pairs and the main article body (headings,
//! paragraphs, list items, definition terms, and wikitables) as plain,
//! searchable text. Purely read-only — unlike `style.rs`, nothing here
//! needs to mutate the parsed document.

use scraper::ElementRef;

use super::selector;

const HEADING_TAGS: &[&str] = &["h2", "h3", "h4", "h5", "h6"];

/// Writes every `table.infobox`'s rows as `"key: value"` lines (or just
/// `key`/`value` alone when the other side is empty), reading straight
/// from the original, un-cleaned content — matching Go, which runs this
/// pass before the noise-removing clone exists.
pub(super) fn write_infobox_text(out: &mut String, content: &ElementRef) {
    for table in content.select(&selector("table.infobox")) {
        for tr in table.select(&selector("tr")) {
            let key = collect_text(&tr, "th");
            let value = collect_text(&tr, "td");
            if let Some(line) = join_key_value(&key, &value) {
                out.push_str(&line);
                out.push('\n');
            }
        }
        out.push('\n');
    }
}

fn join_key_value(key: &str, value: &str) -> Option<String> {
    match (key.is_empty(), value.is_empty()) {
        (false, false) => Some(format!("{key}: {value}")),
        (false, true) => Some(key.to_string()),
        (true, false) => Some(value.to_string()),
        (true, true) => None,
    }
}

/// Walks the cleaned content in document order, writing headings,
/// paragraphs, list items, definition terms, and wikitables as
/// structured plain text. The combined selector (rather than one pass
/// per tag) is what keeps this in document order — matching Go's
/// `goquery` `Find("h2, h3, ..., table.wikitable, table.sortable")`,
/// which visits the tree once rather than tag-by-tag.
pub(super) fn write_article_text(out: &mut String, content: &ElementRef) {
    let sel = selector("h2, h3, h4, h5, h6, p, li, dt, dd, table.wikitable, table.sortable");
    for el in content.select(&sel) {
        let tag = el.value().name();
        if tag == "table" {
            write_table_text(out, &el);
            continue;
        }
        let is_heading = HEADING_TAGS.contains(&tag);
        // Skip list items etc. that live inside a wikitable — `writeTableText`
        // already rendered them as part of that table's cells.
        if !is_heading && has_wikitable_ancestor(&el) {
            continue;
        }
        let text = el.text().collect::<String>();
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        if is_heading {
            out.push('\n');
        }
        out.push_str(text);
        out.push('\n');
    }
}

fn has_wikitable_ancestor(el: &ElementRef) -> bool {
    el.ancestors().any(|node| {
        node.value().as_element().is_some_and(|e| {
            e.name() == "table"
                && (e.has_class("wikitable", scraper::CaseSensitivity::CaseSensitive)
                    || e.has_class("sortable", scraper::CaseSensitivity::CaseSensitive))
        })
    })
}

/// Renders a wikitable as tab-separated text with a header row, preceded
/// by its caption if it has one.
fn write_table_text(out: &mut String, table: &ElementRef) {
    let caption = collect_text(table, "caption");
    if !caption.is_empty() {
        out.push_str(&caption);
        out.push('\n');
    }
    for tr in table.select(&selector("tr")) {
        let cells: Vec<String> = tr
            .select(&selector("th, td"))
            .map(|cell| cell.text().collect::<String>().trim().to_string())
            .collect();
        if !cells.is_empty() {
            out.push_str(&cells.join("\t"));
            out.push('\n');
        }
    }
    out.push('\n');
}

fn collect_text(scope: &ElementRef, css: &str) -> String {
    scope
        .select(&selector(css))
        .flat_map(|el| el.text())
        .collect::<String>()
        .trim()
        .to_string()
}
