//! Painting the session overview (gui-design D97) into a see-through
//! pixmap that covers the body under the strip, with the tiles a click can
//! hit. Sizes are logical px times the DPI scale; the colours are the
//! chrome's. No tile size is in the pack's frames, so those are D97's own.

use super::blocks::{ink, paint_glyph};
use super::chrome::Chrome;
use super::overview::{Card, Out, Overview, Tile};
use super::paint::{Pixmap, Rect, icons, mix};
use super::text::{UiText, Weight};

/// The strip above the view and the ribbon under it, which stay (D97).
const STRIP_H: f32 = 52.0;
const RIBBON_H: f32 = 34.0;
/// The view's inset from the window's sides and the strip (D47).
const INSET: f32 = 8.0;
/// Space between the view's edge and its content.
const MARGIN: f32 = 16.0;
const TILE_W: f32 = 240.0;
const TILE_H: f32 = 72.0;
const TILE_PAD: f32 = 12.0;
const TILE_GAP: f32 = 8.0;
const ROW_GAP: f32 = 16.0;
const HEAD_H: f32 = 32.0;
const RADIUS: f32 = 9.0;
const NAME_PX: f32 = 13.0;
const SMALL_PX: f32 = 12.0;
const ICON: f32 = 16.0;
const STROKE: f32 = 1.4;
const GAP: f32 = 6.0;
const RING: f32 = 2.0;
const FOCUS: f32 = 1.5;
/// A disabled tile's opacity (D26).
const DISABLED: f32 = 0.45;

/// A painted overview: its pixels and each tile's box, by pane key.
pub(crate) struct Painted {
    pub pixmap: Pixmap,
    pub tiles: Vec<(Rect, u64)>,
    /// The scroll the painting ended with, so the selected tile shows.
    pub scroll: f32,
}

impl Painted {
    /// The tile under `(x, y)`, in the pixmap's own px.
    pub(crate) fn hit(&self, x: f32, y: f32) -> Option<u64> {
        self.tiles
            .iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|&(_, k)| k)
    }
}

/// The view's size in px for a window `view` px wide and high at `s`.
fn size(view: (f32, f32), s: f32) -> (f32, f32) {
    let w = (view.0 - 2.0 * INSET * s).max(1.0);
    let h = (view.1 - (STRIP_H + RIBBON_H + INSET) * s).max(1.0);
    (w.round(), h.round())
}

/// How many tiles fit a row of the view `w` px wide.
fn per_row(w: f32, s: f32) -> usize {
    let room = w - 2.0 * MARGIN * s + TILE_GAP * s;
    ((room / ((TILE_W + TILE_GAP) * s)).floor() as usize).max(1)
}

/// Paints `o`, `hover` the tile under the pointer, `scroll` px down.
pub(crate) fn paint(
    o: &Overview,
    hover: Option<u64>,
    scroll: f32,
    (c, s): (&Chrome, f32),
    font: &mut UiText,
    view: (f32, f32),
) -> Painted {
    let (w, h) = size(view, s);
    let mut out = Pixmap::clear(w as usize, h as usize);
    let panel = Rect {
        x: 0.0,
        y: 0.0,
        w,
        h,
    };
    out.fill(panel, RADIUS * s, c.frame);
    let mut tiles = Vec::new();
    let cols = per_row(w, s);
    let x0 = MARGIN * s;
    let mut y = MARGIN * s;
    let mut selected_at = None;
    let mut scroll = scroll;
    // Two passes: lay out to find the selected tile, then paint scrolled.
    let mut places: Vec<(&Tile, Rect)> = Vec::new();
    let mut heads: Vec<(&Card, f32)> = Vec::new();
    for card in o.cards() {
        heads.push((card, y));
        y += HEAD_H * s;
        for (i, tile) in card.tiles.iter().enumerate() {
            let (col, row) = (i % cols, i / cols);
            let r = Rect {
                x: x0 + col as f32 * (TILE_W + TILE_GAP) * s,
                y: y + row as f32 * (TILE_H + TILE_GAP) * s,
                w: TILE_W * s,
                h: TILE_H * s,
            };
            if Some(tile.src.key) == o.selected() {
                selected_at = Some(r);
            }
            places.push((tile, r));
        }
        let rows = card.tiles.len().div_ceil(cols) as f32;
        y += (rows * (TILE_H + TILE_GAP) - TILE_GAP + ROW_GAP) * s;
    }
    if let Some(r) = selected_at {
        let m = MARGIN * s;
        if r.y - m < scroll {
            scroll = (r.y - m).max(0.0);
        } else if r.y + r.h + m > scroll + h {
            scroll = r.y + r.h + m - h;
        }
    }
    scroll = scroll.clamp(0.0, (y - h).max(0.0));
    if o.cards().is_empty() {
        paint_empty(&mut out, (c, s), font);
    }
    for (card, top) in heads {
        paint_head(
            &mut out,
            card,
            Rect {
                x: x0,
                y: top - scroll,
                w: w - 2.0 * x0,
                h: HEAD_H * s,
            },
            (c, s),
            font,
        );
    }
    for (tile, r) in places {
        let r = Rect {
            y: r.y - scroll,
            ..r
        };
        let lit = hover == Some(tile.src.key) && o.online();
        let sel = o.selected() == Some(tile.src.key);
        paint_tile(&mut out, tile, r, (lit, sel, o.online()), (c, s), font);
        tiles.push((r, tile.src.key));
    }
    out.border(panel, RADIUS * s, s, c.line);
    Painted {
        pixmap: out,
        tiles,
        scroll,
    }
}

/// "No agents in this session" in the D28 empty-state style.
fn paint_empty(out: &mut Pixmap, (c, s): (&Chrome, f32), font: &mut UiText) {
    let x = MARGIN * s;
    let base = MARGIN * s + 4.0 * s + font.centre_drop(NAME_PX * s);
    out.text(
        font,
        "No agents in this session",
        Weight::Regular,
        NAME_PX * s,
        x,
        base,
        c.text,
    );
    let base = MARGIN * s + 24.0 * s + font.centre_drop(SMALL_PX * s);
    out.text(
        font,
        "Blocks appear here when an agent starts in one.",
        Weight::Regular,
        SMALL_PX * s,
        x,
        base,
        c.muted,
    );
}

/// A card's id and its block count; "No card" heads the rest.
fn paint_head(out: &mut Pixmap, card: &Card, r: Rect, (c, s): (&Chrome, f32), font: &mut UiText) {
    let px = NAME_PX * s;
    let mid = r.y + r.h / 2.0;
    let title = card.card.as_deref().unwrap_or("No card");
    let title = font.fit(title, Weight::Medium, px, r.w / 2.0);
    let used = out.text(
        font,
        &title,
        Weight::Medium,
        px,
        r.x,
        mid + font.centre_drop(px),
        c.text,
    );
    let n = card.tiles.len();
    let count = if n == 1 {
        "1 block".to_owned()
    } else {
        format!("{n} blocks")
    };
    out.text(
        font,
        &count,
        Weight::Regular,
        SMALL_PX * s,
        r.x + used + 8.0 * s,
        mid + font.centre_drop(SMALL_PX * s),
        c.muted,
    );
}

/// A tile: lead glyph and agent, role, state, and who else has input.
fn paint_tile(
    out: &mut Pixmap,
    t: &Tile,
    r: Rect,
    (lit, selected, online): (bool, bool, bool),
    (c, s): (&Chrome, f32),
    font: &mut UiText,
) {
    let under = if lit { c.hover } else { c.base };
    let shade = |color: u32| {
        if online {
            color
        } else {
            mix(under, color, DISABLED)
        }
    };
    out.fill(r, RADIUS * s, under);
    let edge = if t.focused { c.accent } else { c.line };
    out.border(
        r,
        RADIUS * s,
        if t.focused { FOCUS * s } else { s },
        shade(edge),
    );
    let x = r.x + TILE_PAD * s;
    let right = r.x + r.w - TILE_PAD * s;
    // Line 1: the lead glyph, then the agent.
    let line1 = r.y + TILE_PAD * s;
    let mut tx = x;
    if let Some((glyph, i)) = t.lead {
        paint_glyph(out, glyph, tx, line1, s, shade(ink(c, i)), shade(c.line));
        tx += (ICON + GAP) * s;
    }
    let px = NAME_PX * s;
    let name = if t.src.name.is_empty() {
        "Block"
    } else {
        &t.src.name
    };
    let name = font.fit(name, Weight::Medium, px, right - tx);
    let base = line1 + ICON * s / 2.0 + font.centre_drop(px);
    out.text(font, &name, Weight::Medium, px, tx, base, shade(c.text));
    // Line 2: the role.
    // Rows of 16 px each (the glyph's height) fill D97's 72 px tile exactly
    // between its 12 px paddings.
    let line2 = line1 + ICON * s;
    let spx = SMALL_PX * s;
    if !t.src.role.is_empty() {
        let role = font.fit(&t.src.role, Weight::Regular, spx, right - x);
        let base = line2 + 8.0 * s + font.centre_drop(spx);
        out.text(font, &role, Weight::Regular, spx, x, base, shade(c.muted));
    }
    // Line 3: the state, and whoever else has input on the right.
    let line3 = line2 + 16.0 * s;
    let mid = line3 + 8.0 * s;
    let mut end = right;
    if let Some(holder) = &t.src.holder {
        let text = font.fit(
            &format!("{holder} has input"),
            Weight::Regular,
            spx,
            (right - x) / 2.0,
        );
        let width = font.width(&text, Weight::Regular, spx);
        out.text(
            font,
            &text,
            Weight::Regular,
            spx,
            right - width,
            mid + font.centre_drop(spx),
            shade(c.muted),
        );
        let icon = (ICON + GAP) * s;
        out.icon(
            icons::EYE,
            right - width - icon,
            mid - ICON * s / 2.0,
            ICON * s,
            STROKE * s,
            shade(c.muted),
        );
        end = right - width - icon - GAP * s;
    }
    let state = match (t.src.exited, t.src.detached) {
        (true, _) => "Exited",
        (_, true) => "Detached",
        _ => t.state,
    };
    let state = font.fit(state, Weight::Regular, spx, (end - x).max(0.0));
    out.text(
        font,
        &state,
        Weight::Regular,
        spx,
        x,
        mid + font.centre_drop(spx),
        shade(c.muted),
    );
    if selected && online {
        out.border(r.inset(s), RADIUS * s, RING * s, c.accent);
    }
}

/// What a painting was made for.
type Look = (Chrome, f32, usize, usize);

/// An attached window's overview: closed, or open with its last painting.
pub(crate) struct OverviewUi {
    text: UiText,
    open: Option<Open>,
}

struct Open {
    overview: Overview,
    hover: Option<u64>,
    scroll: f32,
    painted: Option<(Look, Painted, (usize, usize))>,
}

impl OverviewUi {
    /// A closed overview; `None` if the interface font is broken.
    pub(crate) fn new() -> Option<Self> {
        Some(Self {
            text: UiText::embedded()?,
            open: None,
        })
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open.is_some()
    }

    pub(crate) fn open(&mut self, cards: Vec<Card>, online: bool) {
        self.open = Some(Open {
            overview: Overview::open(cards, online),
            hover: None,
            scroll: 0.0,
            painted: None,
        });
    }

    pub(crate) fn close(&mut self) {
        self.open = None;
    }

    /// The cards' new state (D60: the view updates live).
    pub(crate) fn update(&mut self, cards: Vec<Card>, online: bool) {
        if let Some(open) = &mut self.open {
            open.overview.update(cards, online);
            open.painted = None;
        }
    }

    /// A key; what the window does next, or `None` when closed.
    pub(crate) fn key(&mut self, key: super::overview::Key) -> Option<Out> {
        let open = self.open.as_mut()?;
        open.hover = None;
        open.painted = None;
        Some(open.overview.key(key))
    }

    /// A click at window px `(x, y)`: what it asks. A click off the view
    /// closes it.
    pub(crate) fn click(&mut self, x: f64, y: f64) -> Option<Out> {
        let open = self.open.as_mut()?;
        let Some((_, painted, at)) = &open.painted else {
            return None;
        };
        let (px, py) = ((x - at.0 as f64) as f32, (y - at.1 as f64) as f32);
        let on = painted.pixmap.width as f32 > px
            && px >= 0.0
            && painted.pixmap.height as f32 > py
            && py >= 0.0;
        if !on {
            return Some(Out::Close);
        }
        let key = painted.hit(px, py)?;
        open.painted = None;
        Some(open.overview.click(key))
    }

    /// The pointer moved; whether the lit tile changed.
    pub(crate) fn hover(&mut self, x: f64, y: f64) -> bool {
        let Some(open) = &mut self.open else {
            return false;
        };
        let key = open
            .painted
            .as_ref()
            .and_then(|(_, p, at)| p.hit((x - at.0 as f64) as f32, (y - at.1 as f64) as f32));
        if open.hover == key {
            return false;
        }
        open.hover = key;
        open.painted = None;
        true
    }

    /// Paint for `chrome` at `scale` in a window `view` px wide and high,
    /// unless the last painting is that.
    pub(crate) fn repaint(&mut self, chrome: Chrome, scale: f32, view: (f32, f32)) {
        let Some(open) = &mut self.open else {
            return;
        };
        let look = (chrome, scale, view.0 as usize, view.1 as usize);
        if matches!(&open.painted, Some((l, ..)) if *l == look) {
            return;
        }
        let painted = paint(
            &open.overview,
            open.hover,
            open.scroll,
            (&chrome, scale),
            &mut self.text,
            view,
        );
        open.scroll = painted.scroll;
        let at = (
            (INSET * scale).round() as usize,
            ((STRIP_H + INSET) * scale).round() as usize,
        );
        open.painted = Some((look, painted, at));
    }

    /// The last painting and its place, while open.
    pub(crate) fn painted(&self) -> Option<(&Pixmap, (usize, usize))> {
        let (_, painted, at) = self.open.as_ref()?.painted.as_ref()?;
        Some((&painted.pixmap, *at))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Theme;
    use crate::gui::source::OverviewTile;

    fn tile(key: u64, holder: Option<&str>) -> Tile {
        Tile {
            src: OverviewTile {
                key,
                window: 1,
                name: format!("agent {key}"),
                role: "builder".into(),
                exited: false,
                detached: false,
                holder: holder.map(str::to_owned),
            },
            lead: None,
            state: "Idle",
            focused: key == 1,
        }
    }

    fn cards(n: u64) -> Vec<Card> {
        vec![
            Card {
                card: Some("#1572".into()),
                tiles: (1..=n)
                    .map(|k| tile(k, (k == 2).then_some("bob")))
                    .collect(),
            },
            Card {
                card: None,
                tiles: vec![tile(100, None)],
            },
        ]
    }

    fn paint_at(o: &Overview, scale: f32, view: (f32, f32)) -> Painted {
        let chrome = Chrome::from_theme(&Theme::default());
        let mut font = UiText::embedded().expect("font");
        paint(o, None, 0.0, (&chrome, scale), &mut font, view)
    }

    #[test]
    fn the_view_covers_the_body_between_strip_and_ribbon() {
        let o = Overview::open(cards(2), true);
        let p = paint_at(&o, 1.0, (1000.0, 700.0));
        assert_eq!((p.pixmap.width, p.pixmap.height), (984, 606));
        let p = paint_at(&o, 2.0, (2000.0, 1400.0));
        assert_eq!((p.pixmap.width, p.pixmap.height), (1968, 1212));
    }

    #[test]
    fn every_tile_is_hittable_and_a_wide_row_wraps() {
        let o = Overview::open(cards(8), true);
        let p = paint_at(&o, 1.0, (1000.0, 1200.0));
        assert_eq!(p.tiles.len(), 9);
        for (r, key) in &p.tiles {
            assert_eq!(p.hit(r.x + 3.0, r.y + 3.0), Some(*key));
        }
        let first = p.tiles[0].0;
        let last_of_card = p.tiles[7].0;
        assert!(
            last_of_card.y > first.y,
            "eight 240 px tiles can't share 968 px"
        );
        assert_eq!(p.hit(1.0, 1.0), None);
    }

    #[test]
    fn the_selected_tile_is_scrolled_into_view() {
        let mut o = Overview::open(cards(2), true);
        o.click(100);
        let p = paint_at(&o, 1.0, (1000.0, 300.0));
        assert!(p.scroll > 0.0);
        let (r, _) = p.tiles.iter().find(|(_, k)| *k == 100).expect("tile");
        assert!(r.y >= 0.0 && r.y + r.h <= p.pixmap.height as f32);
    }

    #[test]
    fn an_empty_session_paints_and_nothing_is_hittable() {
        let o = Overview::open(Vec::new(), true);
        let p = paint_at(&o, 1.0, (800.0, 600.0));
        assert!(p.tiles.is_empty());
    }

    #[test]
    fn a_click_off_the_view_closes_it() {
        let mut ui = OverviewUi::new().expect("font");
        ui.open(cards(2), true);
        ui.repaint(Chrome::from_theme(&Theme::default()), 1.0, (1000.0, 700.0));
        assert_eq!(ui.click(2.0, 2.0), Some(Out::Close));
        let (r, key) = ui
            .open
            .as_ref()
            .expect("open")
            .painted
            .as_ref()
            .expect("painted")
            .1
            .tiles[0];
        let (x, y) = (r.x as f64 + 8.0 + 4.0, r.y as f64 + 60.0 + 4.0);
        assert_eq!(
            ui.click(x, y),
            Some(Out::Open {
                key,
                window: 1,
                detached: false
            })
        );
    }
}
