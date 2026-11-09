//! The take-over confirm (gui-design F3, D19, D44, D64): a small panel
//! under an owner-only block's "Take over" button that says who decides
//! and asks before this window types there.
//!
//! Pure: [`paint`] draws a [`Confirm`] into a see-through pixmap with its
//! two buttons' boxes; [`Confirm`] keeps which button has the focus and
//! answers keys and clicks. Sizes are the pack's logical px times the DPI
//! scale.

use super::blocks::wrap;
use super::chrome::Chrome;
use super::paint::{Pixmap, Rect, mix};
use super::text::{UiText, Weight};

/// The panel's width, padding and radius (gui_lib `pop_takeover`).
pub(crate) const WIDTH: f32 = 300.0;
const PAD: f32 = 16.0;
const RADIUS: f32 = 9.0;
const BORDER: f32 = 1.0;
/// Its top's space under the header (D64 2).
pub(crate) const DROP: f32 = 8.0;
/// Title size and line, the gap under it, body size and line, the gap
/// before the buttons.
const TITLE: (f32, f32) = (14.0, 20.0);
const TITLE_GAP: f32 = 6.0;
const BODY: (f32, f32) = (12.0, 18.0);
const ACTS_GAP: f32 = 12.0;
/// Regular buttons: height, side padding, radius, text size, and the
/// space between them.
const BUTTON: (f32, f32, f32, f32) = (28.0, 12.0, 6.0, 12.0);
const BUTTON_GAP: f32 = 8.0;
/// The focus ring's width and its space outside the button (D27).
const RING: f32 = 2.0;
const RING_GAP: f32 = 2.0;
/// How far a hovered primary button lightens toward the text colour.
const HOVER_MIX: f32 = 0.12;

/// One of the confirm's buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Choice {
    Cancel,
    TakeOver,
}

impl Choice {
    fn label(self) -> &'static str {
        match self {
            Choice::Cancel => "Cancel",
            Choice::TakeOver => "Take over",
        }
    }

    fn other(self) -> Self {
        match self {
            Choice::Cancel => Choice::TakeOver,
            Choice::TakeOver => Choice::Cancel,
        }
    }
}

/// What a key or click on an open confirm did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    /// Nothing that closes it.
    Stay,
    /// Closed without taking over.
    Cancel,
    /// Confirmed: take block [`Confirm::key`] over.
    TakeOver,
}

/// A key the confirm answers (D64 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConfirmKey {
    Tab,
    BackTab,
    Enter,
    Escape,
}

/// The copy for taking `program` over from `owner` (D64 2).
pub(crate) fn copy(program: &str, owner: &str) -> (String, String) {
    (
        format!("Take input from {program}?"),
        format!(
            "{owner} started {program} and decides who types here. It sees everything you type."
        ),
    )
}

/// An open confirm for block `key`, at window px `at`.
pub(crate) struct Confirm {
    pub key: u64,
    text: UiText,
    title: String,
    body: String,
    at: (usize, usize),
    focus: Choice,
    hover: Option<Choice>,
    painted: Option<((Chrome, f32), Painted)>,
}

/// A painted confirm and its buttons' boxes, in its own px.
pub(crate) struct Painted {
    pub pixmap: Pixmap,
    pub buttons: Vec<(Rect, Choice)>,
}

impl Confirm {
    /// A confirm for block `key` saying `title` over `body`, its right
    /// edge at window px `right` and its top at `top`; "Take over" has
    /// the focus. `None` if the interface font is broken.
    pub(crate) fn new(
        key: u64,
        (title, body): (String, String),
        right: f32,
        top: f32,
        s: f32,
    ) -> Option<Self> {
        let x = (right - WIDTH * s).max(0.0).round() as usize;
        Some(Self {
            key,
            text: UiText::embedded()?,
            title,
            body,
            at: (x, top.max(0.0).round() as usize),
            focus: Choice::TakeOver,
            hover: None,
            painted: None,
        })
    }

    /// Answer `key`: Tab moves the focus, Enter presses the focused
    /// button, Esc cancels.
    pub(crate) fn key(&mut self, key: ConfirmKey) -> Answer {
        match key {
            ConfirmKey::Tab | ConfirmKey::BackTab => {
                self.focus = self.focus.other();
                self.painted = None;
                Answer::Stay
            }
            ConfirmKey::Enter => press(self.focus),
            ConfirmKey::Escape => Answer::Cancel,
        }
    }

    /// Answer a click at window px `(x, y)`: a button presses, anywhere
    /// off the panel cancels.
    pub(crate) fn click(&self, x: f64, y: f64) -> Answer {
        let Some((_, p)) = &self.painted else {
            return Answer::Cancel;
        };
        let (px, py) = self.local(x, y);
        if let Some(choice) = button_at(p, px, py) {
            return press(choice);
        }
        let on =
            px >= 0.0 && py >= 0.0 && px < p.pixmap.width as f32 && py < p.pixmap.height as f32;
        match on {
            true => Answer::Stay,
            false => Answer::Cancel,
        }
    }

    /// The pointer moved to window px `(x, y)`; whether the lit button
    /// changed.
    pub(crate) fn hover(&mut self, x: f64, y: f64) -> bool {
        let (px, py) = self.local(x, y);
        let lit = self
            .painted
            .as_ref()
            .and_then(|(_, p)| button_at(p, px, py));
        if lit == self.hover {
            return false;
        }
        self.hover = lit;
        self.painted = None;
        true
    }

    /// Paint for `chrome` at `scale`, unless the last painting is that.
    pub(crate) fn repaint(&mut self, chrome: Chrome, scale: f32) {
        let look = (chrome, scale);
        if matches!(&self.painted, Some((l, _)) if *l == look) {
            return;
        }
        let state = State {
            title: &self.title,
            body: &self.body,
            focus: self.focus,
            hover: self.hover,
        };
        let painted = paint(&state, (&chrome, scale), &mut self.text);
        self.painted = Some((look, painted));
    }

    /// The last painting and its place.
    pub(crate) fn painted(&self) -> Option<(&Pixmap, (usize, usize))> {
        self.painted.as_ref().map(|(_, p)| (&p.pixmap, self.at))
    }

    fn local(&self, x: f64, y: f64) -> (f32, f32) {
        ((x - self.at.0 as f64) as f32, (y - self.at.1 as f64) as f32)
    }
}

fn press(choice: Choice) -> Answer {
    match choice {
        Choice::Cancel => Answer::Cancel,
        Choice::TakeOver => Answer::TakeOver,
    }
}

fn button_at(p: &Painted, x: f32, y: f32) -> Option<Choice> {
    p.buttons
        .iter()
        .find(|(r, _)| r.contains(x, y))
        .map(|&(_, c)| c)
}

/// What a confirm shows.
struct State<'a> {
    title: &'a str,
    body: &'a str,
    focus: Choice,
    hover: Option<Choice>,
}

/// The confirm panel: title, wrapped body, then "Cancel" and "Take over"
/// at the right, the focused one ringed (D64 2).
fn paint(confirm: &State<'_>, (c, s): (&Chrome, f32), font: &mut UiText) -> Painted {
    let w = (WIDTH * s).round();
    let room = w - 2.0 * PAD * s;
    let (title_px, title_h) = (TITLE.0 * s, TITLE.1 * s);
    let (body_px, body_h) = (BODY.0 * s, BODY.1 * s);
    let title = font.fit(confirm.title, Weight::Medium, title_px, room);
    let lines = wrap(font, confirm.body, body_px, room);
    let (btn_h, btn_pad, btn_r, btn_px) = BUTTON;
    let h = (2.0 * PAD * s
        + title_h
        + TITLE_GAP * s
        + body_h * lines.len() as f32
        + (ACTS_GAP + btn_h) * s)
        .round();
    let mut out = Pixmap::clear(w as usize, h as usize);
    let panel = Rect {
        x: 0.0,
        y: 0.0,
        w,
        h,
    };
    out.fill(panel, RADIUS * s, c.raised);
    let mut y = PAD * s;
    let base = y + title_h / 2.0 + font.centre_drop(title_px);
    out.text(
        font,
        &title,
        Weight::Medium,
        title_px,
        PAD * s,
        base,
        c.text,
    );
    y += title_h + TITLE_GAP * s;
    for line in &lines {
        let base = y + body_h / 2.0 + font.centre_drop(body_px);
        out.text(font, line, Weight::Regular, body_px, PAD * s, base, c.muted);
        y += body_h;
    }
    y += ACTS_GAP * s;
    let mut right = w - PAD * s;
    let mut buttons = Vec::new();
    for choice in [Choice::TakeOver, Choice::Cancel] {
        let label = choice.label();
        let bw = font.width(label, Weight::Medium, btn_px * s) + 2.0 * btn_pad * s;
        let r = Rect {
            x: right - bw,
            y,
            w: bw,
            h: btn_h * s,
        };
        let lit = confirm.hover == Some(choice);
        let (fill, ink) = match choice {
            Choice::TakeOver if lit => (mix(c.accent, c.text, HOVER_MIX), c.on_accent),
            Choice::TakeOver => (c.accent, c.on_accent),
            Choice::Cancel if lit => (c.hover, c.text),
            Choice::Cancel => (c.raised, c.text),
        };
        out.fill(r, btn_r * s, fill);
        if choice == Choice::Cancel {
            out.border(r, btn_r * s, BORDER * s, c.line);
        }
        let base = y + r.h / 2.0 + font.centre_drop(btn_px * s);
        out.text(
            font,
            label,
            Weight::Medium,
            btn_px * s,
            r.x + btn_pad * s,
            base,
            ink,
        );
        if confirm.focus == choice {
            let o = RING_GAP * s;
            let ring = Rect {
                x: r.x - o,
                y: r.y - o,
                w: r.w + 2.0 * o,
                h: r.h + 2.0 * o,
            };
            out.border(ring, (btn_r + RING_GAP) * s, RING * s, c.accent);
        }
        buttons.push((r, choice));
        right = r.x - BUTTON_GAP * s;
    }
    out.border(panel, RADIUS * s, BORDER * s, c.line);
    Painted {
        pixmap: out,
        buttons,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open() -> Confirm {
        Confirm::new(4, copy("claude", "orch"), 600.0, 100.0, 1.0)
            .expect("the embedded faces parse")
    }

    #[test]
    fn the_copy_names_the_program_and_who_decides() {
        let (title, body) = copy("claude", "orch");
        assert_eq!(title, "Take input from claude?");
        assert_eq!(
            body,
            "orch started claude and decides who types here. It sees everything you type."
        );
    }

    #[test]
    fn keys_move_the_focus_press_it_or_cancel() {
        let mut c = open();
        assert_eq!(
            c.key(ConfirmKey::Enter),
            Answer::TakeOver,
            "Take over has the focus"
        );
        assert_eq!(c.key(ConfirmKey::Tab), Answer::Stay);
        assert_eq!(
            c.key(ConfirmKey::Enter),
            Answer::Cancel,
            "Cancel has it now"
        );
        assert_eq!(c.key(ConfirmKey::BackTab), Answer::Stay);
        assert_eq!(c.key(ConfirmKey::Enter), Answer::TakeOver);
        assert_eq!(c.key(ConfirmKey::Escape), Answer::Cancel);
    }

    #[test]
    fn it_sits_right_aligned_and_clicks_hit_its_buttons_or_cancel() {
        let mut c = open();
        c.repaint(Chrome::from_theme(&crate::core::Theme::default()), 1.0);
        let (pixmap, at) = c.painted().expect("painted");
        assert_eq!(at, (300, 100), "its right edge on the button's");
        assert_eq!(pixmap.width, 300);
        let (_, p) = c.painted.as_ref().expect("painted");
        let take = p
            .buttons
            .iter()
            .find(|(_, b)| *b == Choice::TakeOver)
            .expect("Take over")
            .0;
        let cancel = p
            .buttons
            .iter()
            .find(|(_, b)| *b == Choice::Cancel)
            .expect("Cancel")
            .0;
        assert!(cancel.x + cancel.w < take.x, "Cancel before Take over");
        assert!(
            (take.x + take.w - (300.0 - PAD)).abs() < 0.5,
            "right-aligned"
        );
        let mid = |r: Rect| {
            (
                300.0 + f64::from(r.x + r.w / 2.0),
                100.0 + f64::from(r.y + r.h / 2.0),
            )
        };
        let (x, y) = mid(take);
        assert_eq!(c.click(x, y), Answer::TakeOver);
        let (x, y) = mid(cancel);
        assert_eq!(c.click(x, y), Answer::Cancel);
        assert_eq!(c.click(310.0, 110.0), Answer::Stay, "on the panel");
        assert_eq!(c.click(10.0, 10.0), Answer::Cancel, "off it");
    }
}
