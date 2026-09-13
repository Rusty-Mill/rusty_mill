//! A Rust port of `server/extractor/textutil/textutil.go`: flattens an HTML
//! subtree to plain text while preserving block-level line breaks — unlike
//! a plain concatenation of text nodes (e.g. `StackExchangeExtractor`'s own
//! `element_text` helper), which runs the paragraphs of multi-paragraph
//! content together. Needed by extractors whose comment/post bodies wrap
//! multiple paragraphs in `<p>`/`<div>`/... elements. Go shares this helper
//! across `hackernews`, `discourse`, and `reddit`; ported once here for the
//! same reuse across this crate's own future Discourse/Reddit extractors.

use ego_tree::NodeRef;
use scraper::{ElementRef, Node};

/// Elements whose boundaries become line breaks when markup is flattened to
/// text.
const BLOCK_ELEMENTS: &[&str] = &[
    "address",
    "article",
    "blockquote",
    "dd",
    "div",
    "dl",
    "dt",
    "figcaption",
    "figure",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "li",
    "main",
    "ol",
    "p",
    "pre",
    "section",
    "table",
    "td",
    "th",
    "tr",
    "ul",
];

/// Maximum DOM nesting depth walked when flattening a subtree to text.
/// (`hackernews::selection_text` calls this over untrusted crawled-page
/// markup); a pathologically deep DOM would
/// otherwise recurse `write_node_text` without bound and overflow the
/// stack. 64 matches this workspace's established recursion-cap
/// convention (e.g. `nexus-database::formula::eval::MAX_RECURSION_DEPTH`,
/// `rusty_jinja::parser::MAX_NESTING_DEPTH`).
const MAX_NODE_DEPTH: usize = 64;

/// Flattens `element`'s subtree to plain text. Unlike a plain
/// text-node concatenation, block elements and `<br>` become line breaks
/// so the shape of the original content survives; `<script>`/`<style>`/
/// `<svg>`/`<button>` contents are skipped entirely.
pub fn selection_text(element: &ElementRef) -> String {
    let mut out = String::new();
    write_node_text(&mut out, **element, 0);
    normalize_text(&out)
}

fn write_node_text(out: &mut String, node: NodeRef<Node>, depth: usize) {
    if depth > MAX_NODE_DEPTH {
        return;
    }
    match node.value() {
        Node::Text(text) => out.push_str(text),
        Node::Element(el) => {
            let name = el.name();
            if matches!(name, "script" | "style" | "svg" | "button") {
                return;
            }
            if name == "br" {
                write_text_break(out);
                return;
            }
            let is_block = BLOCK_ELEMENTS.contains(&name);
            if is_block {
                write_text_break(out);
            }
            for child in node.children() {
                write_node_text(out, child, depth + 1);
            }
            if is_block {
                write_text_break(out);
            }
        }
        _ => {
            for child in node.children() {
                write_node_text(out, child, depth + 1);
            }
        }
    }
}

fn write_text_break(out: &mut String) {
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
}

/// Collapses runs of whitespace within lines and runs of blank lines down
/// to one, and trims the result.
pub(crate) fn normalize_text(text: &str) -> String {
    let text = text
        .replace('\u{a0}', " ")
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut cleaned: Vec<String> = Vec::new();
    let mut blank = false;
    for line in text.split('\n') {
        let joined = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if joined.is_empty() {
            if !cleaned.is_empty() && !blank {
                cleaned.push(String::new());
                blank = true;
            }
            continue;
        }
        cleaned.push(joined);
        blank = false;
    }
    cleaned.join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use scraper::{Html, Selector};

    fn text_of(html: &str, css: &str) -> String {
        let doc = Html::parse_document(html);
        let selector = Selector::parse(css).unwrap();
        let element = doc
            .select(&selector)
            .next()
            .expect("selector matched nothing");
        selection_text(&element)
    }

    #[test]
    fn separates_paragraphs_with_a_line_break() {
        let text = text_of("<div id='x'><p>First.</p><p>Second.</p></div>", "#x");
        assert_eq!(text, "First.\nSecond.");
    }

    #[test]
    fn br_becomes_a_line_break() {
        let text = text_of("<div id='x'>one<br>two</div>", "#x");
        assert_eq!(text, "one\ntwo");
    }

    #[test]
    fn collapses_internal_whitespace_runs() {
        let text = text_of("<div id='x'>a   b \t c</div>", "#x");
        assert_eq!(text, "a b c");
    }

    #[test]
    fn an_embedded_newline_still_acts_as_a_line_break() {
        // A literal newline in a text node is whitespace like any other,
        // but `normalize_text` only ever operates line-by-line (splitting
        // on `\n` first) — so an embedded one still separates lines,
        // matching the Go original's own behavior.
        let text = text_of("<div id='x'>a\nb</div>", "#x");
        assert_eq!(text, "a\nb");
    }

    #[test]
    fn collapses_blank_line_runs_to_one() {
        let text = text_of(
            "<div id='x'><p>a</p><div></div><div></div><p>b</p></div>",
            "#x",
        );
        assert_eq!(text, "a\nb");
    }

    #[test]
    fn skips_script_style_svg_and_button_content() {
        let text = text_of(
            "<div id='x'>keep<script>drop()</script><style>.c{}</style><svg><text>drop</text></svg><button>drop</button></div>",
            "#x",
        );
        assert_eq!(text, "keep");
    }

    #[test]
    fn nested_inline_markup_does_not_insert_breaks() {
        let text = text_of("<p id='x'>see <a href='/x'>this link</a> here</p>", "#x");
        assert_eq!(text, "see this link here");
    }

    #[test]
    fn deeply_nested_dom_recursion_is_capped_instead_of_overflowing_the_stack() {
        let total_depth = 5000;
        let mut html = String::from("<div id='x'>");
        for i in 0..total_depth {
            html.push_str(&format!("<div>L{i}"));
        }
        for _ in 0..total_depth {
            html.push_str("</div>");
        }
        html.push_str("</div>");
        let text = text_of(&html, "#x");
        assert!(text.contains("L0"), "shallow content must be present");
        assert!(
            text.contains(&format!("L{}", MAX_NODE_DEPTH - 2)),
            "content at the cap boundary must still be present"
        );
        assert!(
            !text.contains(&format!("L{}", MAX_NODE_DEPTH - 1)),
            "recursion past the cap must stop instead of walking arbitrarily deep"
        );
    }
}
