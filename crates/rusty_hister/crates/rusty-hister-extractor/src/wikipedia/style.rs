//! A Rust port of `server/extractor/extractors/wikipedia/style.go`:
//! injects inline `style` attributes onto Wikipedia's structural elements
//! (infoboxes, wikitables, figures, galleries, quotes, legends, pie-chart
//! legends, standalone images) so they still render richly in
//! [`super::WikipediaExtractor::preview`] after the sanitizer strips
//! Wikipedia's own CSS classes. All colours are semi-transparent `rgba()`
//! values (theme-neutral) rather than Wikipedia's light-mode palette, so
//! they adapt to both light and dark hosts.
//!
//! Every pass here follows the same shape: collect the `NodeId`s a
//! selector matches (via [`super::ids_within`], scoped to the article
//! content), then mutate each one by id (via [`super::set_style`]) — see
//! `mod.rs`'s own doc for why an immutable `ElementRef` pass and a
//! mutating pass can't interleave on the same borrow.
//!
//! `styleWikitables`' `tbl.WrapHtml(...)` (wrapping a wikitable in a
//! horizontally-scrolling `<div>`) is the one Go behavior this port
//! doesn't reproduce — see `mod.rs`'s module doc for why.

use ego_tree::NodeId;
use scraper::{CaseSensitivity, ElementRef, Html, Node};
use std::collections::HashSet;

use super::{ids_within, selector, set_style, style_selector};

// Colour tokens (rgba, theme-neutral) — inlined directly into the style
// strings below rather than reassembled with `concat!`, since Go's own
// versions are `const` string concatenations for the same reason: these
// never change independently of the rules that use them.
// rgba(128,128,128,0.25) = border, rgba(128,128,128,0.06) = surface,
// rgba(128,128,128,0.12) = surface (hi), rgba(100,150,220,0.18) = accent.

const STYLE_INFOBOX: &str = "float: right; clear: right; width: 22em; max-width: 100%; margin: 0 0 1em 1.5em; padding: 0; border: 1px solid rgba(128,128,128,0.25); border-collapse: collapse; background-color: rgba(128,128,128,0.06); font-size: 0.875em; line-height: 1.5";
const STYLE_INFOBOX_CAPTION: &str = "background-color: rgba(100,150,220,0.18); padding: 0.5em; text-align: center; font-weight: bold; font-size: 1.1em";
const STYLE_INFOBOX_TH: &str = "padding: 0.25em 0.5em; text-align: left; vertical-align: top; font-weight: bold; width: 40%; border-top: 1px solid rgba(128,128,128,0.25)";
const STYLE_INFOBOX_TD: &str =
    "padding: 0.25em 0.5em; vertical-align: top; border-top: 1px solid rgba(128,128,128,0.25)";
const STYLE_INFOBOX_IMAGE: &str =
    "padding: 0.4em; text-align: center; border-top: 1px solid rgba(128,128,128,0.25)";

const STYLE_WIKITABLE: &str = "border-collapse: collapse; border: 1px solid rgba(128,128,128,0.25); margin: 1em 0; background-color: rgba(128,128,128,0.06); font-size: 0.875em";
const STYLE_WIKITABLE_TH: &str = "background-color: rgba(128,128,128,0.12); border: 1px solid rgba(128,128,128,0.25); padding: 0.35em 0.65em; text-align: left; font-weight: bold";
const STYLE_WIKITABLE_TD: &str =
    "border: 1px solid rgba(128,128,128,0.25); padding: 0.35em 0.65em; vertical-align: top";
const STYLE_WIKITABLE_CAPTION: &str =
    "font-weight: bold; padding: 0.5em; text-align: left; font-size: 1.05em";

const STYLE_THUMB_RIGHT: &str = "float: right; clear: right; margin: 0 0 0.8em 1.4em; max-width: 220px; border: 1px solid rgba(128,128,128,0.25); background-color: rgba(128,128,128,0.06); padding: 3px; overflow: hidden";
const STYLE_THUMB_LEFT: &str = "float: left; clear: left; margin: 0 1.4em 0.8em 0; max-width: 220px; border: 1px solid rgba(128,128,128,0.25); background-color: rgba(128,128,128,0.06); padding: 3px; overflow: hidden";
const STYLE_THUMB_CENTER: &str = "margin: 1em auto; display: table; max-width: 100%; border: 1px solid rgba(128,128,128,0.25); background-color: rgba(128,128,128,0.06); padding: 3px";
const STYLE_THUMB_INLINE: &str = "display: inline-block; vertical-align: top; margin: 4px; max-width: 180px; border: 1px solid rgba(128,128,128,0.25); background-color: rgba(128,128,128,0.06); padding: 3px; overflow: hidden";
const STYLE_FIGCAPTION: &str = "font-size: 0.85em; line-height: 1.4; padding: 4px";
const STYLE_THUMB_IMG: &str = "max-width: 100%; height: auto; display: block";

const STYLE_HATNOTE: &str =
    "font-style: italic; padding-left: 1.6em; margin-bottom: 0.5em; font-size: 0.9em";

const STYLE_H2: &str =
    "border-bottom: 1px solid rgba(128,128,128,0.25); padding-bottom: 0.25em; margin-top: 1.5em";
const STYLE_H3: &str = "margin-top: 1.2em";

const STYLE_SIDEBAR: &str = "float: right; clear: right; width: 18em; max-width: 100%; margin: 0 0 1em 1.5em; padding: 0.5em; border: 1px solid rgba(128,128,128,0.25); background-color: rgba(128,128,128,0.06); font-size: 0.85em";

const STYLE_REFERENCES_WRAP: &str =
    "font-size: 0.8em; line-height: 1.6; margin-top: 0.5em; padding-top: 0.5em";
const STYLE_REFERENCES_LIST: &str = "margin: 0; padding-left: 2em; list-style-type: decimal";

const STYLE_GALLERY: &str =
    "display: flex; flex-wrap: wrap; gap: 4px; margin: 1em 0; padding: 0; list-style-type: none";
const STYLE_GALLERY_BOX: &str = "width: 155px; text-align: center";
const STYLE_GALLERY_THUMB: &str = "width: 150px; height: 150px; display: flex; align-items: center; justify-content: center; border: 1px solid rgba(128,128,128,0.25); background-color: rgba(128,128,128,0.06); padding: 3px; overflow: hidden";
const STYLE_GALLERY_TEXT: &str =
    "font-size: 0.85em; line-height: 1.4; padding: 2px 4px; max-width: 150px";

const STYLE_QUOTE_MARK: &str = "vertical-align: top; border: none; font-size: 2em; line-height: 0.6em; padding: 0.2em 0.3em; opacity: 0.3";
const STYLE_QUOTE_BODY: &str = "vertical-align: top; border: none; padding: 0.25em 0.5em";
const STYLE_QUOTE_TABLE: &str =
    "margin: 1em auto; border-collapse: collapse; border: none; width: auto";

const STYLE_LEGEND_COLOR: &str = "display: inline-block; width: 1.2em; height: 1.2em; margin-right: 0.4em; vertical-align: middle; border: 1px solid rgba(128,128,128,0.25)";

const STYLE_PIE_LEGEND: &str = "margin: 0.5em 0; padding: 0";

/// Selector → style pairs applied verbatim, in order, before the more
/// involved per-element passes below.
const SIMPLE_STYLES: &[(&str, &str)] = &[
    (".hatnote", STYLE_HATNOTE),
    ("h2", STYLE_H2),
    ("h3", STYLE_H3),
    ("table.sidebar, .sidebar", STYLE_SIDEBAR),
    (".mw-references-wrap", STYLE_REFERENCES_WRAP),
    ("ol.references", STYLE_REFERENCES_LIST),
];

const FIGURE_ALIGNMENTS: &[(&str, &str)] = &[
    ("mw-halign-left", STYLE_THUMB_LEFT),
    ("mw-halign-center", STYLE_THUMB_CENTER),
];
const THUMB_ALIGNMENTS: &[(&str, &str)] =
    &[("tleft", STYLE_THUMB_LEFT), ("tcenter", STYLE_THUMB_CENTER)];

pub(super) fn style_content(html: &mut Html, scope_id: NodeId) {
    for (css, style) in SIMPLE_STYLES {
        style_selector(html, scope_id, css, style);
    }
    style_infoboxes(html, scope_id);
    style_wikitables(html, scope_id);
    style_quotes(html, scope_id);
    style_figures(html, scope_id);
    style_galleries(html, scope_id);
    style_legends(html, scope_id);
    style_pie_charts(html, scope_id);
    style_images(html, scope_id);
}

fn style_infoboxes(html: &mut Html, scope_id: NodeId) {
    for table_id in ids_within(html, scope_id, "table.infobox") {
        set_style(html, table_id, STYLE_INFOBOX);
        for tr_id in ids_within(html, table_id, "tr") {
            style_infobox_row(html, tr_id);
        }
    }
}

fn style_infobox_row(html: &mut Html, tr_id: NodeId) {
    let th_ids = ids_within(html, tr_id, "th");
    let td_ids = ids_within(html, tr_id, "td");
    let th_colspan = th_ids
        .first()
        .is_some_and(|id| has_nonempty_attr(html, *id, "colspan"));
    let td_colspan = td_ids
        .first()
        .is_some_and(|id| has_nonempty_attr(html, *id, "colspan"));
    let td_has_img = td_ids
        .iter()
        .any(|id| !ids_within(html, *id, "img").is_empty());

    if !th_ids.is_empty() && td_ids.is_empty() {
        let style = if th_colspan {
            STYLE_INFOBOX_CAPTION
        } else {
            STYLE_INFOBOX_TH
        };
        for id in &th_ids {
            set_style(html, *id, style);
        }
    }
    if !td_ids.is_empty() && th_ids.is_empty() {
        if td_colspan && td_has_img {
            for id in &td_ids {
                set_style(html, *id, STYLE_INFOBOX_IMAGE);
            }
        } else if td_colspan {
            for id in &td_ids {
                set_style(html, *id, STYLE_INFOBOX_CAPTION);
            }
        }
    }
    if !th_ids.is_empty() && !td_ids.is_empty() {
        for id in &th_ids {
            set_style(html, *id, STYLE_INFOBOX_TH);
        }
        for id in &td_ids {
            set_style(html, *id, STYLE_INFOBOX_TD);
        }
    }
}

fn style_wikitables(html: &mut Html, scope_id: NodeId) {
    for table_id in ids_within(html, scope_id, "table.wikitable, table.sortable") {
        if has_class(html, table_id, "infobox") {
            continue;
        }
        set_style(html, table_id, STYLE_WIKITABLE);
        style_selector(html, table_id, "caption", STYLE_WIKITABLE_CAPTION);
        style_selector(html, table_id, "th", STYLE_WIKITABLE_TH);
        style_selector(html, table_id, "td", STYLE_WIKITABLE_TD);
        // Go also wraps the table in a horizontally-scrolling `<div>`
        // here (`tbl.WrapHtml(...)`) — not reproduced, see mod.rs's doc.
    }
}

fn style_quotes(html: &mut Html, scope_id: NodeId) {
    for table_id in ids_within(html, scope_id, "table.cquote, table.pullquote") {
        set_style(html, table_id, STYLE_QUOTE_TABLE);
        for td_id in ids_within(html, table_id, "td") {
            let (text, has_cite) = {
                let td = element(html, td_id);
                let text = td.text().collect::<String>().trim().to_string();
                let has_cite = td.select(&selector("cite")).next().is_some();
                (text, has_cite)
            };
            if text == "\u{201c}" || text == "\u{201d}" || text == "\"" {
                set_style(html, td_id, STYLE_QUOTE_MARK);
            } else if !has_cite {
                set_style(html, td_id, STYLE_QUOTE_BODY);
            }
        }
    }
}

fn style_figures(html: &mut Html, scope_id: NodeId) {
    let clustered = clustered_figure_ids(html, scope_id);

    for fig_id in ids_within(html, scope_id, "figure") {
        let style = if clustered.contains(&fig_id) {
            STYLE_THUMB_INLINE.to_string()
        } else {
            let class = attr(html, fig_id, "class");
            alignment_style(&class, FIGURE_ALIGNMENTS)
        };
        set_style(html, fig_id, &style);
        style_selector(html, fig_id, "figcaption, .thumbcaption", STYLE_FIGCAPTION);
        style_selector(html, fig_id, "img", STYLE_THUMB_IMG);
    }

    // Legacy `div.thumb` thumbnails (galleries style their own div.thumb
    // children separately, in style_galleries).
    for div_id in ids_within(html, scope_id, "div.thumb") {
        if has_gallery_ancestor(html, div_id) {
            continue;
        }
        let class = attr(html, div_id, "class");
        let style = alignment_style(&class, THUMB_ALIGNMENTS);
        set_style(html, div_id, &style);
        style_selector(html, div_id, "figcaption, .thumbcaption", STYLE_FIGCAPTION);
        style_selector(html, div_id, "img", STYLE_THUMB_IMG);
    }
}

/// Figures with another figure as an immediate next sibling get inline
/// rather than floated layout, so clusters of them wrap naturally
/// instead of stacking as floats.
fn clustered_figure_ids(html: &Html, scope_id: NodeId) -> HashSet<NodeId> {
    let mut clustered = HashSet::new();
    let Some(scope) = ElementRef::wrap(html.tree.get(scope_id).expect("scope node")) else {
        return clustered;
    };
    for fig in scope.select(&selector("figure")) {
        if let Some(next) = next_sibling_element(*fig) {
            if next.value().name() == "figure" {
                clustered.insert(fig.id());
                clustered.insert(next.id());
            }
        }
    }
    clustered
}

fn next_sibling_element(node: ego_tree::NodeRef<'_, Node>) -> Option<ElementRef<'_>> {
    node.next_siblings().find_map(ElementRef::wrap)
}

fn alignment_style(class: &str, mappings: &[(&str, &str)]) -> String {
    for (needle, style) in mappings {
        if class.contains(needle) {
            return (*style).to_string();
        }
    }
    STYLE_THUMB_RIGHT.to_string()
}

fn style_galleries(html: &mut Html, scope_id: NodeId) {
    for ul_id in ids_within(html, scope_id, "ul.gallery") {
        set_style(html, ul_id, STYLE_GALLERY);
        for li_id in ids_within(html, ul_id, "li.gallerybox") {
            set_style(html, li_id, STYLE_GALLERY_BOX);
            style_selector(html, li_id, "div.thumb", STYLE_GALLERY_THUMB);
            style_selector(html, li_id, ".gallerytext", STYLE_GALLERY_TEXT);
            style_selector(html, li_id, "img", STYLE_THUMB_IMG);
        }
    }
}

fn style_legends(html: &mut Html, scope_id: NodeId) {
    for id in ids_within(html, scope_id, ".legend-color") {
        let existing = attr(html, id, "style");
        let combined = format!("{STYLE_LEGEND_COLOR}; {existing}");
        set_style(html, id, &combined);
    }
}

fn style_pie_charts(html: &mut Html, scope_id: NodeId) {
    let mut ids = ids_within(html, scope_id, ".smooth-pie");
    ids.extend(ids_within(html, scope_id, ".smooth-pie-border"));
    for id in ids {
        if let Some(mut node) = html.tree.get_mut(id) {
            node.detach();
        }
    }
    style_selector(html, scope_id, ".smooth-pie-legend", STYLE_PIE_LEGEND);
}

/// Constrains standalone images (ones outside a `<figure>`/`.thumb`,
/// which already size their own images) so they don't blow out the
/// preview — only images that already declare a `width` are touched,
/// matching Go exactly.
fn style_images(html: &mut Html, scope_id: NodeId) {
    let targets: Vec<NodeId> = {
        let Some(scope) = ElementRef::wrap(html.tree.get(scope_id).expect("scope node")) else {
            return;
        };
        scope
            .select(&selector("img"))
            .filter(|img| {
                let boxed = img.ancestors().any(|a| {
                    a.value().as_element().is_some_and(|el| {
                        el.name() == "figure"
                            || el.has_class("thumb", CaseSensitivity::CaseSensitive)
                    })
                });
                !boxed && img.value().attr("width").is_some()
            })
            .map(|img| img.id())
            .collect()
    };
    for id in targets {
        set_style(html, id, "max-width: 100%; height: auto");
    }
}

fn element(html: &Html, id: NodeId) -> ElementRef<'_> {
    ElementRef::wrap(html.tree.get(id).expect("node")).expect("element")
}

fn attr(html: &Html, id: NodeId, name: &str) -> String {
    element(html, id)
        .value()
        .attr(name)
        .unwrap_or("")
        .to_string()
}

fn has_nonempty_attr(html: &Html, id: NodeId, name: &str) -> bool {
    !attr(html, id, name).is_empty()
}

fn has_class(html: &Html, id: NodeId, class: &str) -> bool {
    element(html, id)
        .value()
        .has_class(class, CaseSensitivity::CaseSensitive)
}

fn has_gallery_ancestor(html: &Html, id: NodeId) -> bool {
    element(html, id).ancestors().any(|a| {
        a.value().as_element().is_some_and(|el| {
            el.name() == "ul" && el.has_class("gallery", CaseSensitivity::CaseSensitive)
        })
    })
}
