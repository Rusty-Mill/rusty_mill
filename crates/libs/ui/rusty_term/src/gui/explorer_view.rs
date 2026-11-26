//! Painting the workspace explorer panel (gui-design D99) into a
//! see-through pixmap, and finding what a click hit. Sizes are the pack's
//! logical px times the DPI scale; colours are the chrome's, so dark and
//! light follow the terminal theme.

use super::chrome::Chrome;
use super::explorer::{Key, Out, Panel, Row, Snapshot, State};
use super::paint::{Pixmap, Rect, icons, mix};
use super::popover::{FOOT_H, paint_footer};
use super::text::{UiText, Weight};

pub(crate) const WIDTH: f32 = 360.0;
const RADIUS: f32 = 9.0;
const BORDER: f32 = 1.0;
const HEAD_H: f32 = 38.0;
const ROW_H: f32 = 28.0;
const INDENT: f32 = 16.0;
const PAD: f32 = 12.0;
const ICON: f32 = 16.0;
const GAP: f32 = 6.0;
const NAME_PX: f32 = 13.0;
const SMALL_PX: f32 = 12.0;
const RING: f32 = 2.0;
const STROKE: f32 = 1.4;
const CLOSE_W: f32 = 26.0;
const CLOSE_H: f32 = 24.0;
const KEYS: [(&[&str], &str); 2] = [(&["y"], "Copy path"), (&["Esc"], "Close")];

/// What a painting was made for: chrome, scale and the panel's height in px.
type Look = (Chrome, f32, usize);

/// A painted panel: its pixels and the boxes a click can hit.
struct Painted {
    pixmap: Pixmap,
    /// Each drawn row's box, in the panel's own px, by index from the first drawn.
    rows: Vec<Rect>,
    close: Rect,
}

/// The panel in a window: view state, its font and its last painting.
pub(crate) struct ExplorerUi {
    text: UiText,
    panel: Panel,
    hover: Option<usize>,
    painted: Option<(Look, Painted, (usize, usize))>,
}

impl ExplorerUi {
    /// A closed panel; `None` if the interface font is broken.
    pub(crate) fn new() -> Option<Self> {
        Some(Self {
            text: UiText::embedded()?,
            panel: Panel::default(),
            hover: None,
            painted: None,
        })
    }

    pub(crate) fn is_open(&self) -> bool {
        self.panel.is_open()
    }

    /// The block the open panel belongs to.
    pub(crate) fn block(&self) -> Option<u64> {
        self.panel.block()
    }

    pub(crate) fn open(&mut self, key: u64) {
        self.panel.open(key);
        self.hover = None;
        self.painted = None;
    }

    pub(crate) fn close(&mut self) {
        self.panel.close();
        self.hover = None;
        self.painted = None;
    }

    /// The source's newest snapshot; whether it was for the open block.
    pub(crate) fn set(&mut self, key: u64, snapshot: Snapshot) -> bool {
        let kept = self.panel.set(key, snapshot);
        if kept {
            self.painted = None;
        }
        kept
    }

    /// A key; `None` when closed.
    pub(crate) fn key(&mut self, key: Key) -> Option<Out> {
        if !self.panel.is_open() {
            return None;
        }
        self.hover = None;
        self.painted = None;
        Some(self.panel.key(key))
    }

    /// A click at window px: what it asks, and whether it was on the panel.
    pub(crate) fn click(&mut self, x: f64, y: f64) -> (Option<Out>, bool) {
        let Some((_, painted, at)) = &self.painted else {
            return (None, false);
        };
        let (px, py) = ((x - at.0 as f64) as f32, (y - at.1 as f64) as f32);
        let on = px >= 0.0
            && py >= 0.0
            && px < painted.pixmap.width as f32
            && py < painted.pixmap.height as f32;
        if painted.close.contains(px, py) {
            return (Some(Out::Close), true);
        }
        let n = painted.rows.iter().position(|r| r.contains(px, py));
        let out = n.and_then(|n| self.panel.click(n));
        if out.is_some() {
            self.painted = None;
        }
        (out, on)
    }

    /// The pointer moved; whether the lit row changed.
    pub(crate) fn hover(&mut self, x: f64, y: f64) -> bool {
        let n = self.painted.as_ref().and_then(|(_, p, at)| {
            let (px, py) = ((x - at.0 as f64) as f32, (y - at.1 as f64) as f32);
            p.rows.iter().position(|r| r.contains(px, py))
        });
        if n == self.hover {
            return false;
        }
        self.hover = n;
        self.painted = None;
        true
    }

    /// Paints for `chrome` at `scale`, at the window's right edge between
    /// `top` and `bottom` px (D99: under the strip, over the ribbon), unless
    /// the last painting was that.
    pub(crate) fn repaint(
        &mut self,
        chrome: Chrome,
        scale: f32,
        view: (f32, f32),
        band: (f32, f32),
    ) {
        if !self.panel.is_open() {
            return;
        }
        let inset = (8.0 * scale).round();
        let h = (band.1 - band.0 - inset).max(0.0).round();
        let look = (chrome, scale, h as usize);
        if matches!(&self.painted, Some((l, ..)) if *l == look) {
            return;
        }
        let w = (WIDTH * scale).round();
        let list = (h / scale - HEAD_H - FOOT_H - 2.0 * BORDER).max(0.0);
        self.panel.fit((list / ROW_H) as usize);
        let painted = paint(
            &self.panel,
            self.hover,
            (&chrome, scale),
            (w, h),
            &mut self.text,
        );
        let at = (
            (view.0 - w - inset).max(0.0).round() as usize,
            band.0.round() as usize,
        );
        self.painted = Some((look, painted, at));
    }

    /// The last painting and its place, while open.
    pub(crate) fn painted(&self) -> Option<(&Pixmap, (usize, usize))> {
        let (_, painted, at) = self.painted.as_ref()?;
        Some((&painted.pixmap, *at))
    }
}

fn paint(
    panel: &Panel,
    hover: Option<usize>,
    (c, s): (&Chrome, f32),
    (w, h): (f32, f32),
    font: &mut UiText,
) -> Painted {
    let snap = panel.snapshot();
    let mut out = Pixmap::clear(w as usize, h as usize);
    let frame = Rect {
        x: 0.0,
        y: 0.0,
        w,
        h,
    };
    out.fill(frame, RADIUS * s, c.raised);
    let close = paint_head(&mut out, frame, (c, s), font);
    let foot_y = h - FOOT_H * s;
    let body = Rect {
        x: BORDER * s,
        y: (HEAD_H + BORDER) * s,
        w: w - 2.0 * BORDER * s,
        h: foot_y - (HEAD_H + BORDER) * s,
    };
    let rows = match &snap.state {
        State::Tree | State::Refused(_) if !snap.rows.is_empty() => {
            paint_rows(&mut out, panel, hover, body, (c, s), font)
        }
        state => {
            paint_state(&mut out, state, body, (c, s), font);
            Vec::new()
        }
    };
    paint_footer(&mut out, frame, foot_y, (c, s), font, &KEYS);
    if let Some(line) = &snap.footer {
        let px = SMALL_PX * s;
        let text = font.fit(line, Weight::Regular, px, w / 3.0);
        let width = font.width(&text, Weight::Regular, px);
        let base = foot_y + (FOOT_H * s) / 2.0 + font.centre_drop(px);
        out.text(
            font,
            &text,
            Weight::Regular,
            px,
            w - PAD * s - width,
            base,
            c.muted,
        );
    }
    out.border(frame, RADIUS * s, BORDER * s, c.line);
    Painted {
        pixmap: out,
        rows,
        close,
    }
}

/// The header: "Workspace" and the close button, whose box is returned.
fn paint_head(out: &mut Pixmap, frame: Rect, (c, s): (&Chrome, f32), font: &mut UiText) -> Rect {
    let head = Rect {
        h: HEAD_H * s,
        ..frame
    };
    out.fill_part(frame, RADIUS * s, head, c.base);
    out.fill(
        Rect {
            x: BORDER * s,
            y: HEAD_H * s,
            w: frame.w - 2.0 * BORDER * s,
            h: BORDER * s,
        },
        0.0,
        c.line,
    );
    let mid = HEAD_H * s / 2.0;
    let px = NAME_PX * s;
    let base = mid + font.centre_drop(px);
    out.text(font, "Workspace", Weight::Medium, px, PAD * s, base, c.text);
    let close = Rect {
        x: frame.w - (PAD + CLOSE_W) * s,
        y: mid - CLOSE_H * s / 2.0,
        w: CLOSE_W * s,
        h: CLOSE_H * s,
    };
    out.icon(
        icons::CLOSE,
        close.x + (CLOSE_W - ICON) * s / 2.0,
        mid - ICON * s / 2.0,
        ICON * s,
        STROKE * s,
        c.muted,
    );
    close
}

/// The rows from the panel's scroll, each a box by its index from the first.
fn paint_rows(
    out: &mut Pixmap,
    panel: &Panel,
    hover: Option<usize>,
    body: Rect,
    (c, s): (&Chrome, f32),
    font: &mut UiText,
) -> Vec<Rect> {
    let snap = panel.snapshot();
    let fit = (body.h / (ROW_H * s)).floor() as usize;
    let mut boxes = Vec::new();
    for (n, row) in snap.rows.iter().skip(panel.top()).take(fit).enumerate() {
        let r = Rect {
            y: body.y + n as f32 * ROW_H * s,
            h: ROW_H * s,
            ..body
        };
        let at = panel.top() + n;
        let (lit, selected) = (hover == Some(n) && !row.summary, at == snap.cursor);
        if lit {
            out.fill(r, 0.0, c.hover);
        }
        paint_row(out, row, r, (c, s), font);
        if selected && !row.summary {
            out.border(r, 0.0, RING * s, c.accent);
        }
        boxes.push(r);
    }
    boxes
}

fn paint_row(out: &mut Pixmap, row: &Row, r: Rect, (c, s): (&Chrome, f32), font: &mut UiText) {
    let mid = r.y + r.h / 2.0;
    let icon_y = mid - ICON * s / 2.0;
    let mut x = r.x + (PAD + f32::from(row.depth) * INDENT) * s;
    let mut right = r.x + r.w - PAD * s;
    if let Some(mark) = row.mark {
        let px = SMALL_PX * s;
        let ink = if mark == '*' { c.muted } else { c.text };
        let letter = mark.to_string();
        let width = font.width(&letter, Weight::Medium, px);
        let base = mid + font.centre_drop(px);
        out.text(font, &letter, Weight::Medium, px, right - width, base, ink);
        right -= width + GAP * s;
    }
    let px = if row.summary { SMALL_PX } else { NAME_PX } * s;
    if row.summary {
        x += (ICON + GAP) * 2.0 * s;
    } else {
        if row.dir {
            let chevron = if row.open {
                icons::CHEVRON_DOWN
            } else {
                icons::CHEVRON_RIGHT
            };
            out.icon(chevron, x, icon_y, ICON * s, STROKE * s, c.muted);
        }
        x += (ICON + GAP) * s;
        let kind = if row.dir { icons::FOLDER } else { icons::FILE };
        out.icon(kind, x, icon_y, ICON * s, STROKE * s, c.muted);
        x += (ICON + GAP) * s;
    }
    let name = if row.dir && !row.summary {
        format!("{}/", row.name)
    } else {
        row.name.clone()
    };
    let ink = if row.summary || !(row.dir || row.mark.is_some() || is_plain(row)) {
        c.muted
    } else {
        c.text
    };
    let text = font.fit(&name, Weight::Regular, px, (right - x).max(0.0));
    let base = mid + font.centre_drop(px);
    out.text(font, &text, Weight::Regular, px, x, base, ink);
}

/// Files and directories are bone; only a summary is slate.
fn is_plain(row: &Row) -> bool {
    !row.summary
}

/// A state with no rows: one slate line in the body.
fn paint_state(
    out: &mut Pixmap,
    state: &State,
    body: Rect,
    (c, s): (&Chrome, f32),
    font: &mut UiText,
) {
    let (line, spinner) = match state {
        State::Loading(path) => (format!("Reading {path}"), true),
        State::Empty => ("Empty folder".to_owned(), false),
        State::Refused(why) => (why.clone(), false),
        State::TakeOver => ("Take over first".to_owned(), false),
        State::Tree => return,
    };
    let mut x = body.x + PAD * s;
    let mid = body.y + 24.0 * s;
    if spinner {
        // Static (D27): the ring with its quarter arc.
        let y = mid - ICON * s / 2.0;
        out.icon(icons::SPINNER_RING, x, y, ICON * s, STROKE * s, c.line);
        out.icon(icons::SPINNER_ARC, x, y, ICON * s, STROKE * s, c.accent);
        x += (ICON + GAP) * s;
    }
    let px = SMALL_PX * s;
    let text = font.fit(&line, Weight::Regular, px, body.x + body.w - PAD * s - x);
    let base = mid + font.centre_drop(px);
    out.text(
        font,
        &text,
        Weight::Regular,
        px,
        x,
        base,
        mix(c.raised, c.muted, 1.0),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Theme;
    use crate::gui::explorer::{Intent, Row};

    const VIEW: (f32, f32) = (1200.0, 800.0);
    const BAND: (f32, f32) = (52.0, 770.0);

    fn row(name: &str, depth: u16, dir: bool, open: bool, mark: Option<char>) -> Row {
        Row {
            depth,
            name: name.into(),
            dir,
            open,
            mark,
            summary: false,
        }
    }

    fn tree() -> Snapshot {
        Snapshot {
            state: State::Tree,
            rows: vec![
                row(".", 0, true, true, Some('*')),
                row("src", 1, true, true, Some('*')),
                row("main.rs", 2, false, false, Some('M')),
                row("lib.rs", 2, false, false, None),
                Row {
                    summary: true,
                    ..row("3 not shown", 2, false, false, None)
                },
                row("Cargo.toml", 1, false, false, Some('?')),
            ],
            cursor: 2,
            path: "src/main.rs".into(),
            footer: Some("4.2 KiB, 3 h ago".into()),
        }
    }

    fn light() -> Theme {
        Theme {
            fg: 0x0022_2222,
            bg: 0x00FA_F8F3,
            ..Theme::default()
        }
    }

    fn open(snap: Snapshot, scale: f32, theme: &Theme) -> ExplorerUi {
        let mut ui = ExplorerUi::new().expect("font");
        ui.open(7);
        ui.set(7, snap);
        ui.repaint(Chrome::from_theme(theme), scale, VIEW, BAND);
        ui
    }

    #[test]
    fn closed_paints_nothing_and_takes_no_key() {
        let mut ui = ExplorerUi::new().expect("font");
        ui.repaint(Chrome::from_theme(&Theme::default()), 1.0, VIEW, BAND);
        assert!(ui.painted().is_none() && ui.key(Key::Down).is_none());
        assert_eq!(ui.click(900.0, 100.0), (None, false));
    }

    #[test]
    fn sits_at_the_right_edge_between_strip_and_ribbon() {
        let ui = open(tree(), 1.0, &Theme::default());
        let (pix, at) = ui.painted().expect("painted");
        assert_eq!(
            (pix.width, at.0 + pix.width),
            (360, 1192),
            "8 px from the right"
        );
        assert_eq!((at.1, at.1 + pix.height), (52, 762));
    }

    #[test]
    fn a_click_on_a_row_selects_it_and_off_the_panel_asks_nothing() {
        let mut ui = open(tree(), 1.0, &Theme::default());
        let (_, at) = ui.painted().map(|(p, a)| (p.width, a)).expect("painted");
        // Row 1 starts under the 38 px header and its hairline.
        let (x, y) = (at.0 as f64 + 100.0, at.1 as f64 + 39.0 + 28.0 + 5.0);
        assert_eq!(ui.click(x, y), (Some(Out::Ask(Intent::Select(1))), true));
        let mut ui = open(tree(), 1.0, &Theme::default());
        assert_eq!(ui.click(100.0, 300.0), (None, false), "a block to the left");
    }

    #[test]
    fn a_click_on_a_summary_asks_nothing_but_is_on_the_panel() {
        let mut ui = open(tree(), 1.0, &Theme::default());
        let at = ui.painted().map(|(_, a)| a).expect("painted");
        let (x, y) = (at.0 as f64 + 100.0, at.1 as f64 + 39.0 + 4.0 * 28.0 + 5.0);
        assert_eq!(ui.click(x, y), (None, true));
    }

    #[test]
    fn the_close_button_closes() {
        let mut ui = open(tree(), 1.0, &Theme::default());
        let at = ui.painted().map(|(_, a)| a).expect("painted");
        let (x, y) = (at.0 as f64 + 360.0 - 12.0 - 13.0, at.1 as f64 + 19.0);
        assert_eq!(ui.click(x, y), (Some(Out::Close), true));
    }

    #[test]
    fn states_without_rows_paint_and_hit_nothing() {
        for state in [
            State::Loading(".".into()),
            State::Empty,
            State::Refused("no workspace".into()),
            State::TakeOver,
        ] {
            let snap = Snapshot {
                state,
                ..Snapshot::default()
            };
            let mut ui = open(snap, 1.0, &Theme::default());
            let at = ui.painted().map(|(_, a)| a).expect("painted");
            assert_eq!(
                ui.click(at.0 as f64 + 50.0, at.1 as f64 + 60.0),
                (None, true)
            );
        }
    }

    #[test]
    fn a_new_snapshot_repaints_and_the_same_look_does_not() {
        let mut ui = open(tree(), 1.0, &Theme::default());
        let first = ui.painted().map(|(p, _)| p.clone());
        ui.repaint(Chrome::from_theme(&Theme::default()), 1.0, VIEW, BAND);
        assert_eq!(first.as_ref(), ui.painted().map(|(p, _)| p));
        let mut next = tree();
        next.cursor = 1;
        ui.set(7, next);
        assert!(ui.painted().is_none(), "dropped until the next repaint");
        ui.repaint(Chrome::from_theme(&Theme::default()), 1.0, VIEW, BAND);
        assert_ne!(first.as_ref(), ui.painted().map(|(p, _)| p));
    }

    #[test]
    fn scales_with_dpi() {
        let ui = open(tree(), 2.0, &Theme::default());
        let (pix, _) = ui.painted().expect("painted");
        assert_eq!(pix.width, 720);
    }

    /// Writes dark and light PPMs to `$EXPLORER_SHOTS` for a look by eye:
    /// `EXPLORER_SHOTS=/tmp/x cargo test -p rusty_term --features gui explorer_shots -- --ignored`.
    #[test]
    #[ignore = "writes files for a look by eye"]
    fn explorer_shots() {
        let dir = std::env::var("EXPLORER_SHOTS").expect("EXPLORER_SHOTS");
        for (name, theme) in [("dark", Theme::default()), ("light", light())] {
            let ui = open(tree(), 1.0, &theme);
            let (pix, _) = ui.painted().expect("painted");
            let mut ppm = format!("P6\n{} {}\n255\n", pix.width, pix.height).into_bytes();
            for p in pix.pixels() {
                ppm.extend([(p >> 16) as u8, (p >> 8) as u8, *p as u8]);
            }
            std::fs::write(format!("{dir}/explorer-{name}.ppm"), ppm).expect("write");
        }
    }
}
