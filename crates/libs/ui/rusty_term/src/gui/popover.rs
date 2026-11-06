//! An attached window's popovers: the session switcher under the session
//! chip (gui-design F4, D60) and the detached blocks under their chip (F6,
//! D61). Each is a panel with a header and rows, and the switcher a footer
//! of keys.
//!
//! Pure: [`paint`] draws a [`Menu`] into a see-through [`Pop`] with the
//! rows a click can hit; [`start`], [`step`] and [`pick`] are its keyboard
//! rules, and [`Popover`] keeps one open. Sizes are the pack's logical px
//! times the DPI scale.

use super::blocks::{Glyph, Ink, ink, paint_glyph};
use super::chrome::Chrome;
use super::paint::{Pixmap, Rect, icons, mix};
use super::ribbon::{Cap, keycap};
use super::source::SessionChoice;
use super::text::{UiText, Weight};
use std::time::Duration;

/// The switcher's and the detached list's widths.
pub(crate) const SWITCHER_W: f32 = 360.0;
pub(crate) const DETACHED_W: f32 = 340.0;
/// Corner radius and border.
const RADIUS: f32 = 9.0;
const BORDER: f32 = 1.0;
/// The panels' top, 3 px under the strip's chips (y 46 in the 52 px
/// strip), and the switcher's left edge, on the session chip's.
pub(crate) const TOP: f32 = 46.0;
pub(crate) const SWITCHER_X: f32 = 14.0;
/// Header: padding (top, left, bottom) and text size.
const HEAD_PAD: (f32, f32, f32) = (10.0, 14.0, 4.0);
const HEAD_PX: f32 = 11.0;
/// A line of 11 px text.
const HEAD_LINE: f32 = 15.0;
/// Session rows: least height, padding (left, right), the current row's
/// bar, and the gap between parts.
const ROW_H: f32 = 44.0;
const ROW_PAD: (f32, f32) = (17.0, 14.0);
const BAR: f32 = 3.0;
const GAP: f32 = 10.0;
/// Detached rows: least height and side padding.
const DETACHED_H: f32 = 52.0;
const DETACHED_PAD: f32 = 14.0;
/// Name and detail sizes, and their line heights with the 2 px between.
const NAME_PX: f32 = 13.0;
const DETAIL_PX: f32 = 12.0;
const NAME_LINE: f32 = 17.0;
const DETAIL_LINE: f32 = 16.0;
const LINE_GAP: f32 = 2.0;
const ICON: f32 = 16.0;
const CHECK_STROKE: f32 = 1.8;
/// Menu items' height under the sessions' rule.
const ITEM_H: f32 = 36.0;
/// A primary button: height, side padding, radius and text size.
const BUTTON_H: f32 = 28.0;
const BUTTON_PAD: f32 = 12.0;
const BUTTON_R: f32 = 6.0;
/// Footer height, side padding, space between groups and before a label.
const FOOT_H: f32 = 34.0;
const FOOT_PAD: f32 = 14.0;
const FOOT_GAP: f32 = 16.0;
const KEY_GAP: f32 = 6.0;
/// The selection ring, inset in its row.
const RING: f32 = 2.0;
/// A disabled row's opacity (D26).
const DISABLED: f32 = 0.45;

/// The switcher's footer: its keys and what they do.
const FOOTER: [(&[&str], &str); 3] = [
    (&["Up", "Down"], "Move"),
    (&["Enter"], "Switch"),
    (&["Esc"], "Close"),
];

/// Which popover a menu is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// The session switcher (D60).
    Switcher,
    /// The detached blocks (D61).
    Detached,
}

/// A popover's content: a header over rows.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Menu {
    pub kind: Kind,
    pub header: String,
    pub rows: Vec<Row>,
}

/// One row of a popover.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Row {
    /// A session to switch to.
    Session(SessionChoice),
    /// A menu item, under a rule after the sessions (D45 b).
    Item(String),
    /// A detached block to reattach.
    Detached(DetachedRow),
}

/// A detached block as its row shows it (D61).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DetachedRow {
    pub key: u64,
    pub name: String,
    /// "{state}, detached {age}".
    pub detail: String,
    pub lead: Option<(Glyph, Ink)>,
}

/// What a row asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pick {
    /// Show the session with this key.
    Session(u64),
    /// Run menu item `i`, counted among the items.
    Item(usize),
    /// Put the block with this key back in the layout.
    Reattach(u64),
}

/// A painted popover: its pixels and each row's box, by row.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Pop {
    pub pixmap: Pixmap,
    pub rows: Vec<(Rect, usize)>,
}

impl Pop {
    /// The row under `(x, y)`, in the pop's own px, if any.
    pub(crate) fn row(&self, x: f32, y: f32) -> Option<usize> {
        self.rows
            .iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|&(_, i)| i)
    }

    /// Whether `(x, y)`, in the pop's own px, is on it.
    pub(crate) fn contains(&self, x: f32, y: f32) -> bool {
        x >= 0.0 && y >= 0.0 && x < self.pixmap.width as f32 && y < self.pixmap.height as f32
    }
}

/// What tells two versions of a row apart as the same row.
#[derive(Clone, Debug, PartialEq, Eq)]
enum RowId {
    Session(u64),
    Item(String),
    Detached(u64),
}

impl Row {
    fn id(&self) -> RowId {
        match self {
            Row::Session(s) => RowId::Session(s.key),
            Row::Item(label) => RowId::Item(label.clone()),
            Row::Detached(d) => RowId::Detached(d.key),
        }
    }

    /// Whether it can be picked: a session needs blocks to show (D60).
    fn enabled(&self) -> bool {
        match self {
            Row::Session(s) => s.can_show,
            Row::Item(_) | Row::Detached(_) => true,
        }
    }

    fn height(&self) -> f32 {
        match self {
            Row::Session(_) => ROW_H,
            Row::Item(_) => ITEM_H,
            Row::Detached(_) => DETACHED_H,
        }
    }
}

impl Menu {
    fn enabled(&self, i: usize) -> bool {
        self.rows.get(i).is_some_and(Row::enabled)
    }

    fn width(&self) -> f32 {
        match self.kind {
            Kind::Switcher => SWITCHER_W,
            Kind::Detached => DETACHED_W,
        }
    }

    /// Whether a rule goes before row `i`: the first item after other
    /// rows.
    fn rule_before(&self, i: usize) -> bool {
        i > 0 && matches!(self.rows[i], Row::Item(_)) && !matches!(self.rows[i - 1], Row::Item(_))
    }

    /// The panel's height in px at `s`.
    fn height(&self, s: f32) -> f32 {
        let rules = (0..self.rows.len())
            .filter(|&i| self.rule_before(i))
            .count();
        let rows: f32 = self.rows.iter().map(Row::height).sum();
        let foot = match self.kind {
            Kind::Switcher => FOOT_H,
            Kind::Detached => 0.0,
        };
        (2.0 * BORDER + HEAD_PAD.0 + HEAD_LINE + HEAD_PAD.2 + rows + rules as f32 * BORDER + foot)
            * s
    }
}

/// The row the keyboard starts on: the current session's, else the
/// first.
pub(crate) fn start(menu: &Menu) -> usize {
    menu.rows
        .iter()
        .position(|r| matches!(r, Row::Session(s) if s.current))
        .unwrap_or(0)
}

/// The next row from `from` that can be picked, wrapping; `from` itself
/// when no other can.
pub(crate) fn step(menu: &Menu, from: usize, forward: bool) -> usize {
    let n = menu.rows.len();
    if n == 0 {
        return 0;
    }
    (1..n)
        .map(|d| {
            if forward {
                (from + d) % n
            } else {
                (from + n - d % n) % n
            }
        })
        .find(|&i| menu.enabled(i))
        .unwrap_or(from)
}

/// What row `i` asks for; `None` for a disabled or missing row.
pub(crate) fn pick(menu: &Menu, i: usize) -> Option<Pick> {
    if !menu.enabled(i) {
        return None;
    }
    Some(match &menu.rows[i] {
        Row::Session(s) => Pick::Session(s.key),
        Row::Item(_) => Pick::Item(
            menu.rows[..i]
                .iter()
                .filter(|r| matches!(r, Row::Item(_)))
                .count(),
        ),
        Row::Detached(d) => Pick::Reattach(d.key),
    })
}

/// An attached window's popover: closed, or open on a menu at a place,
/// with a row selected and its last painting.
pub(crate) struct Popover {
    text: UiText,
    open: Option<Open>,
}

/// An open popover.
struct Open {
    menu: Menu,
    /// Its top-left corner, window px.
    at: (usize, usize),
    sel: usize,
    hover: Option<usize>,
    painted: Option<((Chrome, f32), Pop)>,
}

impl Popover {
    /// A closed popover; `None` if the interface font is broken.
    pub(crate) fn new() -> Option<Self> {
        Some(Self {
            text: UiText::embedded()?,
            open: None,
        })
    }

    /// Which popover is open, if any.
    pub(crate) fn kind(&self) -> Option<Kind> {
        self.open.as_ref().map(|o| o.menu.kind)
    }

    /// Open `menu` at window px `at`, the [`start`] row selected.
    pub(crate) fn open(&mut self, menu: Menu, at: (usize, usize)) {
        let sel = start(&menu);
        self.open = Some(Open {
            menu,
            at,
            sel,
            hover: None,
            painted: None,
        });
    }

    pub(crate) fn close(&mut self) {
        self.open = None;
    }

    /// The open menu's rows now (D60: rows update live); the selection
    /// stays on its row while it is there. A menu left with no rows
    /// closes.
    pub(crate) fn update(&mut self, rows: Vec<Row>) {
        let Some(open) = &mut self.open else {
            return;
        };
        if open.menu.rows == rows {
            return;
        }
        if rows.is_empty() {
            self.open = None;
            return;
        }
        let picked = open.menu.rows.get(open.sel).map(Row::id);
        open.menu.rows = rows;
        open.sel = picked
            .and_then(|id| open.menu.rows.iter().position(|r| r.id() == id))
            .unwrap_or_else(|| start(&open.menu));
        open.hover = None;
        open.painted = None;
    }

    /// Move the selection to the next row that can be picked.
    pub(crate) fn step(&mut self, forward: bool) {
        if let Some(open) = &mut self.open {
            open.sel = step(&open.menu, open.sel, forward);
            open.painted = None;
        }
    }

    /// What the selected row asks for.
    pub(crate) fn selected(&self) -> Option<Pick> {
        let open = self.open.as_ref()?;
        pick(&open.menu, open.sel)
    }

    /// Whether session `key` is the one shown.
    pub(crate) fn is_current(&self, key: u64) -> bool {
        self.open.as_ref().is_some_and(|o| {
            o.menu
                .rows
                .iter()
                .any(|r| matches!(r, Row::Session(s) if s.key == key && s.current))
        })
    }

    /// The row at window px `(x, y)`, and whether the point is on the
    /// popover at all.
    pub(crate) fn hit(&self, x: f64, y: f64) -> (Option<Pick>, bool) {
        let Some(open) = &self.open else {
            return (None, false);
        };
        let Some((_, pop)) = &open.painted else {
            return (None, false);
        };
        let (px, py) = ((x - open.at.0 as f64) as f32, (y - open.at.1 as f64) as f32);
        let row = pop.row(px, py).and_then(|i| pick(&open.menu, i));
        (row, pop.contains(px, py))
    }

    /// The pointer moved to window px `(x, y)`; whether the lit row
    /// changed.
    pub(crate) fn hover(&mut self, x: f64, y: f64) -> bool {
        let Some(open) = &mut self.open else {
            return false;
        };
        let row = open.painted.as_ref().and_then(|(_, pop)| {
            pop.row((x - open.at.0 as f64) as f32, (y - open.at.1 as f64) as f32)
        });
        let row = row.filter(|&i| open.menu.enabled(i));
        if open.hover == row {
            return false;
        }
        open.hover = row;
        open.painted = None;
        true
    }

    /// Paint for `chrome` at `scale`, unless the last painting is that.
    pub(crate) fn repaint(&mut self, chrome: Chrome, scale: f32) {
        let Some(open) = &mut self.open else {
            return;
        };
        let look = (chrome, scale);
        if matches!(&open.painted, Some((l, _)) if *l == look) {
            return;
        }
        let pop = paint(
            &open.menu,
            open.sel,
            open.hover,
            (&chrome, scale),
            &mut self.text,
        );
        open.painted = Some((look, pop));
    }

    /// The last painting and its place, while open.
    pub(crate) fn painted(&self) -> Option<(&Pixmap, (usize, usize))> {
        let open = self.open.as_ref()?;
        open.painted.as_ref().map(|(_, p)| (&p.pixmap, open.at))
    }
}

/// A detached row's detail (D61): "{state}, detached {age}", or
/// "{state}, detached" when the block was detached before this client
/// saw it go.
pub(crate) fn detached_detail(state: &str, age: Option<Duration>) -> String {
    match age {
        Some(age) => format!("{state}, detached {}", ago(age)),
        None => format!("{state}, detached"),
    }
}

/// "just now" under a minute, then "{n} min ago", then "{n} h ago".
fn ago(age: Duration) -> String {
    let secs = age.as_secs();
    match secs {
        0..60 => "just now".to_owned(),
        60..3600 => format!("{} min ago", secs / 60),
        _ => format!("{} h ago", secs / 3600),
    }
}

/// "{n} windows, {m} clients", singular for one.
fn detail(s: &SessionChoice) -> String {
    let plural = |n: usize, one: &str| match n {
        1 => format!("1 {one}"),
        n => format!("{n} {one}s"),
    };
    format!(
        "{}, {}",
        plural(s.windows, "window"),
        plural(s.clients, "client")
    )
}

/// Paint `menu` with row `sel` selected and `hover` under the pointer.
pub(crate) fn paint(
    menu: &Menu,
    sel: usize,
    hover: Option<usize>,
    (c, s): (&Chrome, f32),
    font: &mut UiText,
) -> Pop {
    let (w, h) = ((menu.width() * s).round(), menu.height(s).round());
    let mut out = Pixmap::clear(w as usize, h as usize);
    let panel = Rect {
        x: 0.0,
        y: 0.0,
        w,
        h,
    };
    out.fill(panel, RADIUS * s, c.raised);
    let inner = |y: f32, rh: f32| Rect {
        x: BORDER * s,
        y,
        w: w - 2.0 * BORDER * s,
        h: rh,
    };
    let mut rows = Vec::new();

    let mut y = BORDER * s + HEAD_PAD.0 * s;
    let base = y + HEAD_LINE * s / 2.0 + font.centre_drop(HEAD_PX * s);
    let room = w - 2.0 * HEAD_PAD.1 * s;
    let header = font.fit(&menu.header, Weight::Regular, HEAD_PX * s, room);
    out.text(
        font,
        &header,
        Weight::Regular,
        HEAD_PX * s,
        HEAD_PAD.1 * s,
        base,
        c.muted,
    );
    y += (HEAD_LINE + HEAD_PAD.2) * s;

    for (i, row) in menu.rows.iter().enumerate() {
        if menu.rule_before(i) {
            out.fill(inner(y, BORDER * s), 0.0, c.line);
            y += BORDER * s;
        }
        let r = inner(y, row.height() * s);
        let hovered = hover == Some(i);
        match row {
            Row::Session(session) => paint_session(&mut out, session, r, hovered, (c, s), font),
            Row::Item(label) => paint_item(&mut out, label, r, hovered, (c, s), font),
            Row::Detached(d) => paint_detached(&mut out, d, r, hovered, (c, s), font),
        }
        rows.push((r, i));
        y += row.height() * s;
    }
    if let Some(&(r, _)) = rows.iter().find(|&&(_, i)| i == sel) {
        out.border(r, 0.0, RING * s, c.accent);
    }
    if menu.kind == Kind::Switcher {
        paint_footer(&mut out, panel, y, (c, s), font);
    }
    out.border(panel, RADIUS * s, BORDER * s, c.line);
    Pop { pixmap: out, rows }
}

/// The switcher's footer from `y` down, its bottom corners the panel's.
fn paint_footer(out: &mut Pixmap, panel: Rect, y: f32, (c, s): (&Chrome, f32), font: &mut UiText) {
    let foot = Rect {
        x: 0.0,
        y,
        w: panel.w,
        h: panel.h - y,
    };
    out.fill_part(panel, RADIUS * s, foot, c.sunk);
    out.fill(
        Rect {
            x: BORDER * s,
            y,
            w: panel.w - 2.0 * BORDER * s,
            h: BORDER * s,
        },
        0.0,
        c.line,
    );
    let mid = y + BORDER * s + (FOOT_H - BORDER) * s / 2.0;
    let base = mid + font.centre_drop(DETAIL_PX * s);
    let mut x = FOOT_PAD * s;
    for (keys, label) in FOOTER {
        for k in keys {
            x += keycap(out, font, k, (x, mid), Cap::Plain, (c, s)) + KEY_GAP * s;
        }
        x += out.text(
            font,
            label,
            Weight::Regular,
            DETAIL_PX * s,
            x,
            base,
            c.muted,
        );
        x += FOOT_GAP * s;
    }
}

/// A name over a detail line in `r`, from `x` to `right`, in `name_ink`
/// and `detail_ink`.
#[allow(clippy::too_many_arguments)]
fn two_lines(
    out: &mut Pixmap,
    (name, detail): (&str, &str),
    r: Rect,
    (x, right): (f32, f32),
    (name_ink, detail_ink): (u32, u32),
    s: f32,
    font: &mut UiText,
) {
    let room = (right - x).max(0.0);
    let top = r.y + (r.h - (NAME_LINE + LINE_GAP + DETAIL_LINE) * s) / 2.0;
    let name = font.fit(name, Weight::Medium, NAME_PX * s, room);
    let base = top + NAME_LINE * s / 2.0 + font.centre_drop(NAME_PX * s);
    out.text(font, &name, Weight::Medium, NAME_PX * s, x, base, name_ink);
    let line = font.fit(detail, Weight::Regular, DETAIL_PX * s, room);
    let mid = top + (NAME_LINE + LINE_GAP + DETAIL_LINE / 2.0) * s;
    let base = mid + font.centre_drop(DETAIL_PX * s);
    out.text(
        font,
        &line,
        Weight::Regular,
        DETAIL_PX * s,
        x,
        base,
        detail_ink,
    );
}

/// One session's row in `r`: its name over its windows and clients, and
/// the current one's bar, check and "Current".
fn paint_session(
    out: &mut Pixmap,
    session: &SessionChoice,
    r: Rect,
    hovered: bool,
    (c, s): (&Chrome, f32),
    font: &mut UiText,
) {
    let fill = match (session.current, hovered && session.can_show) {
        (_, true) => Some(c.hover),
        (true, false) => Some(c.base),
        (false, false) => None,
    };
    if let Some(fill) = fill {
        out.fill(r, 0.0, fill);
    }
    let under = fill.unwrap_or(c.raised);
    let shade = |color: u32| {
        if session.can_show {
            color
        } else {
            mix(under, color, DISABLED)
        }
    };
    let mut right = r.x + r.w - ROW_PAD.1 * s;
    if session.current {
        out.fill(Rect { w: BAR * s, ..r }, 0.0, c.accent);
        right -= font.width("Current", Weight::Medium, DETAIL_PX * s);
        let base = r.y + r.h / 2.0 + font.centre_drop(DETAIL_PX * s);
        out.text(
            font,
            "Current",
            Weight::Medium,
            DETAIL_PX * s,
            right,
            base,
            c.text,
        );
        right -= (GAP + ICON) * s;
        let iy = r.y + (r.h - ICON * s) / 2.0;
        out.icon(
            icons::CHECK,
            right,
            iy,
            ICON * s,
            CHECK_STROKE * s,
            c.accent,
        );
        right -= GAP * s;
    }
    two_lines(
        out,
        (&session.name, &detail(session)),
        r,
        (r.x + ROW_PAD.0 * s, right),
        (shade(c.text), shade(c.muted)),
        s,
        font,
    );
}

/// A menu item's row in `r`.
fn paint_item(
    out: &mut Pixmap,
    label: &str,
    r: Rect,
    hovered: bool,
    (c, s): (&Chrome, f32),
    font: &mut UiText,
) {
    if hovered {
        out.fill(r, 0.0, c.hover);
    }
    let base = r.y + r.h / 2.0 + font.centre_drop(NAME_PX * s);
    let x = r.x + ROW_PAD.0 * s;
    out.text(font, label, Weight::Regular, NAME_PX * s, x, base, c.text);
}

/// A detached block's row in `r`: its lead glyph, its name over its state
/// and age, and a primary "Reattach" button.
fn paint_detached(
    out: &mut Pixmap,
    d: &DetachedRow,
    r: Rect,
    hovered: bool,
    (c, s): (&Chrome, f32),
    font: &mut UiText,
) {
    if hovered {
        out.fill(r, 0.0, c.hover);
    }
    let label = "Reattach";
    let bw = font.width(label, Weight::Medium, DETAIL_PX * s) + 2.0 * BUTTON_PAD * s;
    let button = Rect {
        x: r.x + r.w - DETACHED_PAD * s - bw,
        y: r.y + (r.h - BUTTON_H * s) / 2.0,
        w: bw,
        h: BUTTON_H * s,
    };
    out.fill(button, BUTTON_R * s, c.accent);
    let base = button.y + button.h / 2.0 + font.centre_drop(DETAIL_PX * s);
    out.text(
        font,
        label,
        Weight::Medium,
        DETAIL_PX * s,
        button.x + BUTTON_PAD * s,
        base,
        c.on_accent,
    );
    let mut x = r.x + DETACHED_PAD * s;
    if let Some((glyph, i)) = d.lead {
        let iy = r.y + (r.h - ICON * s) / 2.0;
        paint_glyph(out, glyph, x, iy, s, ink(c, i), c.line);
        x += (ICON + GAP) * s;
    }
    two_lines(
        out,
        (&d.name, &d.detail),
        r,
        (x, button.x - GAP * s),
        (c.text, c.muted),
        s,
        font,
    );
}

/// The footer's least width, for tests: every group fits the panel.
#[cfg(test)]
fn footer_width(s: f32, font: &UiText) -> f32 {
    use super::ribbon::keycap_width;
    FOOTER
        .iter()
        .map(|(keys, label)| {
            keys.iter()
                .map(|k| keycap_width(k, s, font) + KEY_GAP * s)
                .sum::<f32>()
                + font.width(label, Weight::Regular, DETAIL_PX * s)
        })
        .sum::<f32>()
        + 2.0 * (FOOT_GAP + FOOT_PAD) * s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chrome() -> Chrome {
        Chrome::from_theme(&crate::core::Theme::default())
    }

    fn session(key: u64, name: &str, current: bool, can_show: bool) -> Row {
        Row::Session(SessionChoice {
            key,
            name: name.into(),
            windows: 3,
            clients: 1,
            current,
            can_show,
        })
    }

    fn menu() -> Menu {
        Menu {
            kind: Kind::Switcher,
            header: "baileyai".into(),
            rows: vec![
                session(7, "rusty_mill", true, true),
                session(8, "empty", false, false),
                session(9, "agents-night", false, true),
                Row::Item("Settings".into()),
                Row::Item("Open config file".into()),
            ],
        }
    }

    fn detached() -> Menu {
        Menu {
            kind: Kind::Detached,
            header: "Detached blocks".into(),
            rows: vec![Row::Detached(DetachedRow {
                key: 4,
                name: "cargo watch -x check".into(),
                detail: "Idle, detached 4 min ago".into(),
                lead: Some((Glyph::Check, Ink::Success)),
            })],
        }
    }

    #[test]
    fn rows_say_windows_and_clients_in_the_singular_for_one() {
        let mut s = SessionChoice {
            key: 1,
            name: "a".into(),
            windows: 3,
            clients: 1,
            current: false,
            can_show: true,
        };
        assert_eq!(detail(&s), "3 windows, 1 client");
        (s.windows, s.clients) = (1, 0);
        assert_eq!(detail(&s), "1 window, 0 clients");
    }

    #[test]
    fn the_keyboard_starts_on_current_and_skips_what_cannot_be_picked() {
        let m = menu();
        assert_eq!(start(&m), 0);
        assert_eq!(step(&m, 0, true), 2, "the empty session is skipped");
        assert_eq!(step(&m, 4, true), 0, "wraps");
        assert_eq!(step(&m, 0, false), 4, "wraps back to the last item");
        assert_eq!(pick(&m, 1), None);
        assert_eq!(pick(&m, 2), Some(Pick::Session(9)));
        assert_eq!(pick(&m, 3), Some(Pick::Item(0)));
        assert_eq!(pick(&m, 4), Some(Pick::Item(1)));
        assert_eq!(pick(&m, 5), None);
        assert_eq!(pick(&detached(), 0), Some(Pick::Reattach(4)));
        let none = Menu {
            rows: Vec::new(),
            ..m
        };
        assert_eq!(step(&none, 0, true), 0);
    }

    #[test]
    fn the_switcher_is_360_wide_with_a_row_per_session_and_item() {
        let c = chrome();
        let mut font = UiText::embedded().expect("the embedded faces parse");
        let pop = paint(&menu(), 0, None, (&c, 1.0), &mut font);
        assert_eq!(pop.pixmap.width, 360);
        let want = 2.0 + 29.0 + 3.0 * 44.0 + 1.0 + 2.0 * 36.0 + 34.0;
        assert_eq!(pop.pixmap.height, want as usize);
        let rows: Vec<usize> = pop.rows.iter().map(|r| r.1).collect();
        assert_eq!(rows, [0, 1, 2, 3, 4]);
        assert_eq!(pop.row(100.0, 31.0 + 22.0), Some(0));
        assert!(footer_width(1.0, &font) < 360.0, "the footer fits");
        let at = |x: usize, y: usize| pop.pixmap.pixels()[y * 360 + x];
        assert_eq!(at(0, 0), crate::gui::paint::CLEAR, "a rounded corner");
        assert_eq!(at(180, 0), c.line, "the border");
    }

    #[test]
    fn current_has_the_bar_and_the_selection_a_ring() {
        let c = chrome();
        let mut font = UiText::embedded().expect("the embedded faces parse");
        let pop = paint(&menu(), 2, Some(3), (&c, 1.0), &mut font);
        let at = |x: f32, y: f32| pop.pixmap.pixels()[y as usize * 360 + x as usize];
        let (cur, _) = pop.rows[0];
        assert_eq!(at(cur.x + 1.0, cur.y + 20.0), c.accent, "the bar");
        assert_eq!(at(cur.x + 200.0, cur.y + 4.0), c.base, "the fill");
        let (sel, _) = pop.rows[2];
        assert_eq!(at(sel.x + 100.0, sel.y + 0.5), c.accent, "the ring");
        assert_eq!(at(sel.x + 100.0, sel.y + 4.0), c.raised, "inside it");
        let (hov, _) = pop.rows[3];
        assert_eq!(at(hov.x + 300.0, hov.y + 4.0), c.hover);
    }

    #[test]
    fn the_detached_list_is_340_wide_with_no_footer_and_a_reattach_button() {
        let c = chrome();
        let mut font = UiText::embedded().expect("the embedded faces parse");
        let pop = paint(&detached(), 0, None, (&c, 1.0), &mut font);
        assert_eq!(pop.pixmap.width, 340);
        assert_eq!(pop.pixmap.height, (2.0 + 29.0 + 52.0) as usize);
        let (r, _) = pop.rows[0];
        let at = |x: f32, y: f32| pop.pixmap.pixels()[y as usize * 340 + x as usize];
        // The button's left padding, at its middle: the accent fill.
        let right = r.x + r.w - DETACHED_PAD - 4.0;
        assert_eq!(at(right, r.y + r.h / 2.0), c.accent);
    }

    #[test]
    fn an_update_keeps_the_selection_on_its_row_and_an_empty_one_closes() {
        let mut p = Popover::new().expect("the embedded faces parse");
        p.open(menu(), (0, 0));
        p.step(true);
        assert_eq!(p.selected(), Some(Pick::Session(9)));
        let mut rows = menu().rows;
        rows.remove(1);
        p.update(rows);
        assert_eq!(
            p.selected(),
            Some(Pick::Session(9)),
            "still the same session"
        );
        assert!(p.is_current(7));
        p.update(Vec::new());
        assert_eq!(p.kind(), None);
    }

    #[test]
    fn a_detached_row_says_its_state_and_age() {
        let d = |s: u64| Some(Duration::from_secs(s));
        assert_eq!(detached_detail("Idle", None), "Idle, detached");
        assert_eq!(detached_detail("Idle", d(5)), "Idle, detached just now");
        assert_eq!(
            detached_detail("Working", d(240)),
            "Working, detached 4 min ago"
        );
        assert_eq!(detached_detail("Done", d(7300)), "Done, detached 2 h ago");
    }
}
