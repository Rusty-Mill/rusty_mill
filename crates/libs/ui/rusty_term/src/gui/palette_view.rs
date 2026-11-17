//! Painting the command palette (gui-design D77, D78) into a see-through
//! pixmap with the rows a click can hit. Sizes are the pack's logical px
//! times the DPI scale; colours are the chrome's, so dark and light follow
//! the terminal theme (D5, D25, D34).

use super::chrome::Chrome;
use super::paint::{Pixmap, Rect, icons, mix};
use super::palette::{Entry, Item, Key, Line, Note, Out, Palette};
use super::popover::{FOOT_H, Pop, paint_footer};
use super::source::Listed;
use super::text::{UiText, Weight};

/// The panel's width and the space above it, from the window's top (12
/// under the 52 px strip).
pub(crate) const WIDTH: f32 = 560.0;
pub(crate) const TOP: f32 = 64.0;
const RADIUS: f32 = 9.0;
const BORDER: f32 = 1.0;
/// The search row's height and side padding, and the browse's path line.
const INPUT_H: f32 = 44.0;
const PATH_H: f32 = 28.0;
const PAD: f32 = 14.0;
const ICON: f32 = 16.0;
const GAP: f32 = 10.0;
/// Text sizes: the typed query, a row's name, its detail and a header.
const QUERY_PX: f32 = 14.0;
const NAME_PX: f32 = 13.0;
const SMALL_PX: f32 = 12.0;
const HEAD_PX: f32 = 11.0;
const RING: f32 = 2.0;
const BAR: f32 = 3.0;
/// A disabled row's opacity (D26).
const DISABLED: f32 = 0.45;
const STROKE: f32 = 1.4;
/// The width of a folder row's right end that opens it (D78).
const CHEVRON_W: f32 = 40.0;

/// The commands' footer, and the browse's (D77, D78).
const COMMANDS: [(&[&str], &str); 3] = [
    (&["Up", "Down"], "Move"),
    (&["Enter"], "Run"),
    (&["Esc"], "Close"),
];
const BROWSE: [(&[&str], &str); 5] = [
    (&["Up", "Down"], "Move"),
    (&["Enter"], "Choose"),
    (&["Tab"], "Open folder"),
    (&["Backspace"], "Up"),
    (&["Esc"], "Close"),
];

/// A painted palette: its pixels, each row's box by line, and the boxes
/// of the folder rows' chevrons.
pub(crate) struct Painted {
    pub pop: Pop,
    pub chevrons: Vec<(Rect, usize)>,
}

impl Painted {
    /// The line under `(x, y)` in the panel's own px, and whether the
    /// point is on that row's chevron.
    pub(crate) fn hit(&self, x: f32, y: f32) -> Option<(usize, bool)> {
        let line = self.pop.row(x, y)?;
        let chevron = self
            .chevrons
            .iter()
            .any(|(r, i)| *i == line && r.contains(x, y));
        Some((line, chevron))
    }
}

/// The panel's height in px at `s` for the lines shown.
fn height(p: &Palette, lines: &[Line], s: f32) -> f32 {
    let list: f32 = p.visible(lines).map(|i| lines[i].height()).sum();
    let path = if p.browsing() { PATH_H } else { 0.0 };
    (2.0 * BORDER + path + INPUT_H + BORDER + list + FOOT_H) * s
}

/// Paints `p` with `hover` the line under the pointer.
pub(crate) fn paint(
    p: &Palette,
    hover: Option<usize>,
    (c, s): (&Chrome, f32),
    font: &mut UiText,
) -> Painted {
    let lines = p.lines();
    let (w, h) = ((WIDTH * s).round(), height(p, &lines, s).round());
    let mut out = Pixmap::clear(w as usize, h as usize);
    let panel = Rect {
        x: 0.0,
        y: 0.0,
        w,
        h,
    };
    out.fill(panel, RADIUS * s, c.raised);
    let mut y = BORDER * s;
    if p.browsing() {
        paint_path(&mut out, p.folder(), (y, w), (c, s), font);
        y += PATH_H * s;
    }
    paint_input(&mut out, p, (y, w), (c, s), font);
    y += INPUT_H * s;
    out.fill(
        Rect {
            x: BORDER * s,
            y,
            w: w - 2.0 * BORDER * s,
            h: BORDER * s,
        },
        0.0,
        c.line,
    );
    y += BORDER * s;

    let mut rows = Vec::new();
    let mut chevrons = Vec::new();
    for i in p.visible(&lines) {
        let r = Rect {
            x: BORDER * s,
            y,
            w: w - 2.0 * BORDER * s,
            h: lines[i].height() * s,
        };
        match &lines[i] {
            Line::Head(text) => paint_head(&mut out, text, r, (c, s), font),
            Line::Item(item) => {
                let selected = i == p.selected_line();
                paint_item(
                    &mut out,
                    item,
                    r,
                    (hover == Some(i), selected),
                    (c, s),
                    font,
                );
                rows.push((r, i));
                if item.folder {
                    chevrons.push((
                        Rect {
                            x: r.x + r.w - CHEVRON_W * s,
                            w: CHEVRON_W * s,
                            ..r
                        },
                        i,
                    ));
                }
            }
            Line::Note(note) => paint_note(&mut out, note, r, (c, s), font),
        }
        y += r.h;
    }
    let keys: &[(&[&str], &str)] = if p.browsing() { &BROWSE } else { &COMMANDS };
    paint_footer(&mut out, panel, y, (c, s), font, keys);
    out.border(panel, RADIUS * s, BORDER * s, c.line);
    Painted {
        pop: Pop { pixmap: out, rows },
        chevrons,
    }
}

/// The folder being browsed, over the search row (D78).
fn paint_path(
    out: &mut Pixmap,
    folder: Option<&str>,
    (y, w): (f32, f32),
    (c, s): (&Chrome, f32),
    font: &mut UiText,
) {
    let text = folder.unwrap_or("…");
    let room = w - 2.0 * PAD * s;
    let text = font.fit_tail(text, Weight::Regular, SMALL_PX * s, room);
    let base = y + PATH_H * s / 2.0 + font.centre_drop(SMALL_PX * s) + 3.0 * s;
    out.text(
        font,
        &text,
        Weight::Regular,
        SMALL_PX * s,
        PAD * s,
        base,
        c.muted,
    );
}

/// The search icon and the typed text with its caret, or the placeholder.
fn paint_input(
    out: &mut Pixmap,
    p: &Palette,
    (y, w): (f32, f32),
    (c, s): (&Chrome, f32),
    font: &mut UiText,
) {
    let mid = y + INPUT_H * s / 2.0;
    out.icon(
        icons::SEARCH,
        PAD * s,
        mid - ICON * s / 2.0,
        ICON * s,
        STROKE * s,
        c.muted,
    );
    let x = (PAD + ICON + GAP) * s;
    let base = mid + font.centre_drop(QUERY_PX * s);
    let query = p.query();
    if query.is_empty() {
        let hint = if p.browsing() {
            "Filter this folder"
        } else {
            "Type a command, session or tab"
        };
        out.text(font, hint, Weight::Regular, QUERY_PX * s, x, base, c.muted);
        caret(out, x, mid, (c, s));
        return;
    }
    let room = w - x - PAD * s;
    let shown = font.fit_tail(query, Weight::Regular, QUERY_PX * s, room);
    let used = out.text(font, &shown, Weight::Regular, QUERY_PX * s, x, base, c.text);
    caret(out, x + used, mid, (c, s));
}

fn caret(out: &mut Pixmap, x: f32, mid: f32, (c, s): (&Chrome, f32)) {
    let h = 18.0 * s;
    out.fill(
        Rect {
            x: x.round(),
            y: (mid - h / 2.0).round(),
            w: (1.5 * s).max(1.0).round(),
            h,
        },
        0.0,
        c.accent,
    );
}

fn paint_head(out: &mut Pixmap, text: &str, r: Rect, (c, s): (&Chrome, f32), font: &mut UiText) {
    let base = r.y + r.h / 2.0 + font.centre_drop(HEAD_PX * s) + 3.0 * s;
    out.text(
        font,
        text,
        Weight::Regular,
        HEAD_PX * s,
        r.x + (PAD - 1.0) * s,
        base,
        c.muted,
    );
}

/// A row: its icon, its label with the matched letters in the heavier
/// face, its detail or chevron, and the selection ring.
fn paint_item(
    out: &mut Pixmap,
    item: &Item,
    r: Rect,
    (hovered, selected): (bool, bool),
    (c, s): (&Chrome, f32),
    font: &mut UiText,
) {
    if hovered && item.enabled {
        out.fill(r, 0.0, c.hover);
    }
    let under = if hovered && item.enabled {
        c.hover
    } else {
        c.raised
    };
    let shade = |color: u32| match item.enabled {
        true => color,
        false => mix(under, color, DISABLED),
    };
    if item.current {
        out.fill(Rect { w: BAR * s, ..r }, 0.0, c.accent);
    }
    let mid = r.y + r.h / 2.0;
    let mut x = r.x + PAD * s;
    let mut right = r.x + r.w - PAD * s;
    if item.folder {
        out.icon(
            icons::FOLDER,
            x,
            mid - ICON * s / 2.0,
            ICON * s,
            STROKE * s,
            shade(c.muted),
        );
        x += (ICON + GAP) * s;
        right -= (ICON + GAP) * s;
        out.icon(
            icons::CHEVRON_RIGHT,
            r.x + r.w - (PAD + ICON) * s,
            mid - ICON * s / 2.0,
            ICON * s,
            STROKE * s,
            shade(c.muted),
        );
    }
    if !item.detail.is_empty() {
        let room = (right - x) / 2.0;
        let detail = font.fit(&item.detail, Weight::Regular, SMALL_PX * s, room);
        let width = font.width(&detail, Weight::Regular, SMALL_PX * s);
        let base = mid + font.centre_drop(SMALL_PX * s);
        out.text(
            font,
            &detail,
            Weight::Regular,
            SMALL_PX * s,
            right - width,
            base,
            shade(c.muted),
        );
        right -= width + GAP * s;
    }
    paint_label(out, item, (x, right, mid), shade(c.text), s, font);
    if selected && item.enabled {
        out.border(r, 0.0, RING * s, c.accent);
    }
}

/// `item`'s label from `x`, cut to `right`; letters the query matched in
/// the heavier face when there is a query, else the whole name in it.
fn paint_label(
    out: &mut Pixmap,
    item: &Item,
    (x, right, mid): (f32, f32, f32),
    ink: u32,
    s: f32,
    font: &mut UiText,
) {
    let px = NAME_PX * s;
    let base = mid + font.centre_drop(px);
    let room = (right - x).max(0.0);
    let label = font.fit(&item.label, Weight::Medium, px, room);
    if item.at.is_empty() {
        out.text(font, &label, Weight::Medium, px, x, base, ink);
        return;
    }
    let mut x = x;
    let mut run = String::new();
    let mut lit = false;
    let mut flush = |run: &mut String, lit: bool, x: &mut f32, out: &mut Pixmap| {
        if run.is_empty() {
            return;
        }
        let weight = if lit { Weight::Medium } else { Weight::Regular };
        *x += out.text(font, run, weight, px, *x, base, ink);
        run.clear();
    };
    for (i, ch) in label.chars().enumerate() {
        let now = item.at.contains(&i);
        if now != lit {
            flush(&mut run, lit, &mut x, out);
            lit = now;
        }
        run.push(ch);
    }
    flush(&mut run, lit, &mut x, out);
}

/// An empty state, "Reading", or an unreadable folder: an icon and a title
/// over a slate hint, as the pack's empty states (D28).
fn paint_note(out: &mut Pixmap, note: &Note, r: Rect, (c, s): (&Chrome, f32), font: &mut UiText) {
    let mut x = r.x + PAD * s;
    let icon_y = r.y + (if note.hint.is_some() { 22.0 } else { 20.0 } - ICON / 2.0) * s;
    if note.alert {
        out.icon(icons::ALERT, x, icon_y, ICON * s, STROKE * s, c.failure);
        x += (ICON + GAP) * s;
    } else if note.spinner {
        // Static (D27): the ring with its quarter arc.
        out.icon(icons::SPINNER_RING, x, icon_y, ICON * s, STROKE * s, c.line);
        out.icon(
            icons::SPINNER_ARC,
            x,
            icon_y,
            ICON * s,
            STROKE * s,
            c.accent,
        );
        x += (ICON + GAP) * s;
    }
    let room = r.x + r.w - PAD * s - x;
    let title = font.fit(&note.title, Weight::Regular, NAME_PX * s, room);
    let line = r.y + 20.0 * s;
    let base = line + font.centre_drop(NAME_PX * s);
    let ink = if note.spinner { c.muted } else { c.text };
    out.text(font, &title, Weight::Regular, NAME_PX * s, x, base, ink);
    if let Some(hint) = &note.hint {
        let hint = font.fit(hint, Weight::Regular, SMALL_PX * s, room);
        let base = r.y + 40.0 * s + font.centre_drop(SMALL_PX * s);
        out.text(font, &hint, Weight::Regular, SMALL_PX * s, x, base, c.muted);
    }
}

/// What a painting was made for.
type Look = (Chrome, f32, usize);

/// An attached window's palette: closed, or open with its last painting.
pub(crate) struct PaletteUi {
    text: UiText,
    open: Option<Open>,
}

struct Open {
    palette: Palette,
    hover: Option<usize>,
    painted: Option<(Look, Painted, (usize, usize))>,
}

impl PaletteUi {
    /// A closed palette; `None` if the interface font is broken.
    pub(crate) fn new() -> Option<Self> {
        Some(Self {
            text: UiText::embedded()?,
            open: None,
        })
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open.is_some()
    }

    pub(crate) fn open(&mut self, entries: Vec<Entry>) {
        self.open = Some(Open {
            palette: Palette::open(entries),
            hover: None,
            painted: None,
        });
    }

    pub(crate) fn close(&mut self) {
        self.open = None;
    }

    /// The rows' new state (D77: the list updates live).
    pub(crate) fn update(&mut self, entries: Vec<Entry>) {
        if let Some(open) = &mut self.open {
            open.palette.update(entries);
            open.painted = None;
        }
    }

    /// A key; what the window does next, or `None` when closed.
    pub(crate) fn key(&mut self, key: Key) -> Option<Out> {
        let open = self.open.as_mut()?;
        open.hover = None;
        open.painted = None;
        Some(open.palette.key(key))
    }

    /// A folder read finished.
    pub(crate) fn listed(&mut self, ask: u64, listed: Listed) {
        if let Some(open) = &mut self.open {
            open.palette.listed(ask, listed);
            open.painted = None;
        }
    }

    /// A click at window px `(x, y)`: what it asks, and whether it was on
    /// the panel at all (a click off it closes, D77).
    pub(crate) fn click(&mut self, x: f64, y: f64) -> (Option<Out>, bool) {
        let Some(open) = &mut self.open else {
            return (None, false);
        };
        let Some((_, painted, at)) = &open.painted else {
            return (None, false);
        };
        let (px, py) = ((x - at.0 as f64) as f32, (y - at.1 as f64) as f32);
        let on = painted.pop.contains(px, py);
        let Some((line, chevron)) = painted.hit(px, py) else {
            return (None, on);
        };
        open.painted = None;
        (Some(open.palette.click(line, chevron)), true)
    }

    /// The pointer moved; whether the lit row changed.
    pub(crate) fn hover(&mut self, x: f64, y: f64) -> bool {
        let Some(open) = &mut self.open else {
            return false;
        };
        let line = open.painted.as_ref().and_then(|(_, p, at)| {
            p.hit((x - at.0 as f64) as f32, (y - at.1 as f64) as f32)
                .map(|(line, _)| line)
        });
        if open.hover == line {
            return false;
        }
        open.hover = line;
        open.painted = None;
        true
    }

    /// Paint for `chrome` at `scale` in a window `view` px wide and high,
    /// unless the last painting is that.
    pub(crate) fn repaint(&mut self, chrome: Chrome, scale: f32, view: (f32, f32)) {
        let Some(open) = &mut self.open else {
            return;
        };
        let look = (chrome, scale, view.0 as usize);
        if matches!(&open.painted, Some((l, ..)) if *l == look) {
            return;
        }
        let painted = paint(&open.palette, open.hover, (&chrome, scale), &mut self.text);
        let width = painted.pop.pixmap.width as f32;
        let at = (
            ((view.0 - width) / 2.0).max(0.0).round() as usize,
            (TOP * scale).round() as usize,
        );
        open.painted = Some((look, painted, at));
    }

    /// The last painting and its place, while open.
    pub(crate) fn painted(&self) -> Option<(&Pixmap, (usize, usize))> {
        let (_, painted, at) = self.open.as_ref()?.painted.as_ref()?;
        Some((&painted.pop.pixmap, *at))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Theme;
    use crate::gui::palette::{Cmd, Entry, Id, Key, Out};

    fn entries(n: usize) -> Vec<Entry> {
        (0..n)
            .map(|i| Entry {
                id: Id::Cmd(Cmd::Settings),
                label: format!("Command {i}"),
                detail: "detail".into(),
                enabled: i != 1,
                current: i == 0,
            })
            .collect()
    }

    fn paint_at(p: &Palette, scale: f32) -> Painted {
        let chrome = Chrome::from_theme(&Theme::default());
        let mut font = UiText::embedded().expect("font");
        paint(p, None, (&chrome, scale), &mut font)
    }

    #[test]
    fn the_panel_is_560_wide_and_rows_are_hittable() {
        let p = Palette::open(entries(3));
        let painted = paint_at(&p, 1.0);
        assert_eq!(painted.pop.pixmap.width, 560);
        let (rect, line) = painted.pop.rows[0];
        assert_eq!(painted.hit(rect.x + 5.0, rect.y + 5.0), Some((line, false)));
        assert_eq!(painted.hit(5.0, 5.0), None, "the search row is no row");
    }

    #[test]
    fn a_long_list_is_capped_at_eight_rows() {
        let few = paint_at(&Palette::open(entries(3)), 1.0);
        let many = paint_at(&Palette::open(entries(40)), 1.0);
        assert!(many.pop.pixmap.height > few.pop.pixmap.height);
        let shorter = paint_at(&Palette::open(entries(5)), 1.0);
        let taller = paint_at(&Palette::open(entries(60)), 1.0);
        assert!(taller.pop.pixmap.height >= shorter.pop.pixmap.height);
        assert!(
            f32::from(taller.pop.pixmap.height as u16)
                <= 2.0 + 44.0 + 1.0 + 28.0 + 8.0 * 40.0 + 34.0 + 40.0
        );
    }

    #[test]
    fn the_browse_adds_a_path_line_and_folder_rows_have_a_chevron() {
        let mut p = Palette::open(vec![Entry {
            id: Id::GoTo,
            label: "Go to directory".into(),
            detail: String::new(),
            enabled: true,
            current: false,
        }]);
        let plain = paint_at(&p, 1.0).pop.pixmap.height;
        let Out::List { ask, .. } = p.key(Key::Enter) else {
            panic!("no listing asked");
        };
        p.listed(ask, Ok("/home/me\nd\tsrc".into()));
        let painted = paint_at(&p, 1.0);
        assert!(!painted.chevrons.is_empty());
        assert!(painted.pop.pixmap.height > plain - 40);
        let (rect, line) = painted.chevrons[0];
        assert_eq!(
            painted.hit(rect.x + rect.w - 5.0, rect.y + 5.0),
            Some((line, true))
        );
    }

    #[test]
    fn a_larger_scale_scales_the_panel() {
        let p = Palette::open(entries(3));
        let one = paint_at(&p, 1.0);
        let two = paint_at(&p, 2.0);
        assert_eq!(two.pop.pixmap.width, 2 * one.pop.pixmap.width);
        assert!(two.pop.pixmap.height >= 2 * one.pop.pixmap.height - 2);
    }
}
