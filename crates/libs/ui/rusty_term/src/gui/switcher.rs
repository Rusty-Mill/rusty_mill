//! An attached window's session switcher (gui-design F4, D60): a panel
//! under the session chip listing the server's sessions, the current one
//! marked, then the menu's items (D45 b), then a footer of keys.
//!
//! Pure: [`paint`] draws a [`Menu`] into a see-through [`Pop`] with the
//! rows a click can hit; [`start`], [`step`] and [`pick`] are its keyboard
//! rules. Sizes are the pack's logical px times the DPI scale.

use super::chrome::Chrome;
use super::paint::{Pixmap, Rect, icons, mix};
use super::ribbon::{Cap, keycap};
use super::source::SessionChoice;
use super::text::{UiText, Weight};

/// Panel width, corner radius and border.
const WIDTH: f32 = 360.0;
const RADIUS: f32 = 9.0;
const BORDER: f32 = 1.0;
/// Where the panel sits: its left edge on the session chip's, its top
/// 3 px under the chip (y 46 in the 52 px strip).
pub(crate) const AT: (f32, f32) = (14.0, 46.0);
/// Host header: padding (top, left, bottom) and text size.
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
/// Footer height, side padding, space between groups and before a label.
const FOOT_H: f32 = 34.0;
const FOOT_PAD: f32 = 14.0;
const FOOT_GAP: f32 = 16.0;
const KEY_GAP: f32 = 6.0;
/// The selection ring, inset in its row.
const RING: f32 = 2.0;
/// A disabled row's opacity (D26).
const DISABLED: f32 = 0.45;

/// The footer's keys and what they do.
const FOOTER: [(&[&str], &str); 3] = [
    (&["Up", "Down"], "Move"),
    (&["Enter"], "Switch"),
    (&["Esc"], "Close"),
];

/// What the switcher lists: the sessions on `host`, then menu `items`.
/// Rows count sessions first, then items.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Menu {
    pub host: String,
    pub sessions: Vec<SessionChoice>,
    pub items: Vec<String>,
}

/// What a row asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pick {
    /// Show the session with this key.
    Session(u64),
    /// Run menu item `i`.
    Item(usize),
}

/// A painted switcher: its pixels and each row's box, by row.
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

impl Menu {
    fn len(&self) -> usize {
        self.sessions.len() + self.items.len()
    }

    /// Whether row `i` can be picked: every item, and a session with
    /// blocks to show (D60).
    fn enabled(&self, i: usize) -> bool {
        self.sessions.get(i).is_none_or(|s| s.can_show) && i < self.len()
    }
}

/// The row the keyboard starts on: the current session's.
pub(crate) fn start(menu: &Menu) -> usize {
    menu.sessions.iter().position(|s| s.current).unwrap_or(0)
}

/// The next row from `from` that can be picked, wrapping; `from` itself
/// when no other can.
pub(crate) fn step(menu: &Menu, from: usize, forward: bool) -> usize {
    let n = menu.len();
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
    match menu.sessions.get(i) {
        Some(s) => Some(Pick::Session(s.key)),
        None => Some(Pick::Item(i - menu.sessions.len())),
    }
}

/// An attached window's switcher: closed, or open on a menu with a row
/// selected and its last painting.
pub(crate) struct Switcher {
    text: UiText,
    open: Option<Open>,
}

/// An open switcher.
struct Open {
    menu: Menu,
    sel: usize,
    hover: Option<usize>,
    painted: Option<((Chrome, f32), Pop)>,
}

impl Switcher {
    /// A closed switcher; `None` if the interface font is broken.
    pub(crate) fn new() -> Option<Self> {
        Some(Self {
            text: UiText::embedded()?,
            open: None,
        })
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Open on `menu`, the current session selected.
    pub(crate) fn open(&mut self, menu: Menu) {
        let sel = start(&menu);
        self.open = Some(Open {
            menu,
            sel,
            hover: None,
            painted: None,
        });
    }

    pub(crate) fn close(&mut self) {
        self.open = None;
    }

    /// The server's sessions now, while open; the selection stays on its
    /// session, or its item (D60: rows update live).
    pub(crate) fn update(&mut self, sessions: Vec<SessionChoice>) {
        let Some(open) = &mut self.open else {
            return;
        };
        if open.menu.sessions == sessions {
            return;
        }
        let was = open.menu.sessions.len();
        let picked = open.menu.sessions.get(open.sel).map(|s| s.key);
        open.menu.sessions = sessions;
        let now = open.menu.sessions.len();
        open.sel = match picked {
            Some(key) => open
                .menu
                .sessions
                .iter()
                .position(|s| s.key == key)
                .unwrap_or_else(|| start(&open.menu)),
            None => open.sel - was + now,
        };
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

    /// What row `row` asks for.
    pub(crate) fn pick(&self, row: usize) -> Option<Pick> {
        pick(&self.open.as_ref()?.menu, row)
    }

    /// Whether session `key` is the one shown.
    pub(crate) fn is_current(&self, key: u64) -> bool {
        self.open
            .as_ref()
            .is_some_and(|o| o.menu.sessions.iter().any(|s| s.key == key && s.current))
    }

    /// The row under the pointer; whether that changed.
    pub(crate) fn hover(&mut self, row: Option<usize>) -> bool {
        let Some(open) = &mut self.open else {
            return false;
        };
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

    /// The last painting while open.
    pub(crate) fn painted(&self) -> Option<&Pop> {
        self.open.as_ref()?.painted.as_ref().map(|(_, p)| p)
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

/// The panel's height in px at `s`.
fn height(menu: &Menu, s: f32) -> f32 {
    let rule = if menu.items.is_empty() { 0.0 } else { BORDER };
    (2.0 * BORDER
        + HEAD_PAD.0
        + HEAD_LINE
        + HEAD_PAD.2
        + menu.sessions.len() as f32 * ROW_H
        + rule
        + menu.items.len() as f32 * ITEM_H
        + FOOT_H)
        * s
}

/// Paint `menu` with row `sel` selected and `hover` under the pointer.
pub(crate) fn paint(
    menu: &Menu,
    sel: usize,
    hover: Option<usize>,
    (c, s): (&Chrome, f32),
    font: &mut UiText,
) -> Pop {
    let (w, h) = ((WIDTH * s).round(), height(menu, s).round());
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
    let host = font.fit(&menu.host, Weight::Regular, HEAD_PX * s, room);
    out.text(
        font,
        &host,
        Weight::Regular,
        HEAD_PX * s,
        HEAD_PAD.1 * s,
        base,
        c.muted,
    );
    y += (HEAD_LINE + HEAD_PAD.2) * s;

    for (i, session) in menu.sessions.iter().enumerate() {
        let r = inner(y, ROW_H * s);
        paint_session(&mut out, session, r, hover == Some(i), (c, s), font);
        rows.push((r, i));
        y += ROW_H * s;
    }
    if !menu.items.is_empty() {
        out.fill(inner(y, BORDER * s), 0.0, c.line);
        y += BORDER * s;
    }
    for (j, item) in menu.items.iter().enumerate() {
        let i = menu.sessions.len() + j;
        let r = inner(y, ITEM_H * s);
        if hover == Some(i) {
            out.fill(r, 0.0, c.hover);
        }
        let base = r.y + r.h / 2.0 + font.centre_drop(NAME_PX * s);
        let x = r.x + ROW_PAD.0 * s;
        out.text(font, item, Weight::Regular, NAME_PX * s, x, base, c.text);
        rows.push((r, i));
        y += ITEM_H * s;
    }
    if let Some(&(r, _)) = rows.iter().find(|&&(_, i)| i == sel) {
        out.border(r, 0.0, RING * s, c.accent);
    }

    // The footer, its bottom corners the panel's.
    let foot = Rect {
        x: 0.0,
        y,
        w,
        h: h - y,
    };
    out.fill_part(panel, RADIUS * s, foot, c.sunk);
    out.fill(inner(y, BORDER * s), 0.0, c.line);
    let mid = y + BORDER * s + (FOOT_H - BORDER) * s / 2.0;
    let base = mid + font.centre_drop(DETAIL_PX * s);
    let mut x = FOOT_PAD * s;
    for (keys, label) in FOOTER {
        for k in keys {
            x += keycap(&mut out, font, k, (x, mid), Cap::Plain, (c, s)) + KEY_GAP * s;
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
    out.border(panel, RADIUS * s, BORDER * s, c.line);
    Pop { pixmap: out, rows }
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
    let ink = |color: u32| {
        if session.can_show {
            color
        } else {
            mix(under, color, DISABLED)
        }
    };
    if session.current {
        out.fill(Rect { w: BAR * s, ..r }, 0.0, c.accent);
    }
    let mut right = r.x + r.w - ROW_PAD.1 * s;
    if session.current {
        let label_w = font.width("Current", Weight::Medium, DETAIL_PX * s);
        right -= label_w;
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
    let x = r.x + ROW_PAD.0 * s;
    let room = (right - x).max(0.0);
    let top = r.y + (r.h - (NAME_LINE + LINE_GAP + DETAIL_LINE) * s) / 2.0;
    let name = font.fit(&session.name, Weight::Medium, NAME_PX * s, room);
    let base = top + NAME_LINE * s / 2.0 + font.centre_drop(NAME_PX * s);
    out.text(
        font,
        &name,
        Weight::Medium,
        NAME_PX * s,
        x,
        base,
        ink(c.text),
    );
    let line = font.fit(&detail(session), Weight::Regular, DETAIL_PX * s, room);
    let mid = top + (NAME_LINE + LINE_GAP + DETAIL_LINE / 2.0) * s;
    let base = mid + font.centre_drop(DETAIL_PX * s);
    out.text(
        font,
        &line,
        Weight::Regular,
        DETAIL_PX * s,
        x,
        base,
        ink(c.muted),
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

    fn session(key: u64, name: &str, current: bool, can_show: bool) -> SessionChoice {
        SessionChoice {
            key,
            name: name.into(),
            windows: 3,
            clients: 1,
            current,
            can_show,
        }
    }

    fn menu() -> Menu {
        Menu {
            host: "baileyai".into(),
            sessions: vec![
                session(7, "rusty_mill", true, true),
                session(8, "empty", false, false),
                session(9, "agents-night", false, true),
            ],
            items: vec!["Settings".into(), "Open config file".into()],
        }
    }

    #[test]
    fn rows_say_windows_and_clients_in_the_singular_for_one() {
        let mut s = session(1, "a", false, true);
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
        assert_eq!(pick(&m, 5), None);
        let none = Menu {
            sessions: Vec::new(),
            items: Vec::new(),
            ..m
        };
        assert_eq!(step(&none, 0, true), 0);
    }

    #[test]
    fn the_panel_is_360_wide_with_a_row_per_session_and_item() {
        let c = chrome();
        let mut font = UiText::embedded().expect("the embedded faces parse");
        let pop = paint(&menu(), 0, None, (&c, 1.0), &mut font);
        assert_eq!(pop.pixmap.width, 360);
        let want = 2.0 + 29.0 + 3.0 * 44.0 + 1.0 + 2.0 * 36.0 + 34.0;
        assert_eq!(pop.pixmap.height, want as usize);
        let rows: Vec<usize> = pop.rows.iter().map(|r| r.1).collect();
        assert_eq!(rows, [0, 1, 2, 3, 4]);
        let first = pop.row(100.0, 31.0 + 22.0);
        assert_eq!(first, Some(0));
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
}
