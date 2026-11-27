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
use super::link_policy::{Link, Loopback};
use super::paint::{Pixmap, Rect, mix};
use super::text::{UiText, Weight};

/// The panel's width, padding and radius (gui_lib `pop_takeover`).
pub(crate) const WIDTH: f32 = 300.0;
/// The link prompt is wider: three buttons and a URL (D70).
pub(crate) const LINK_WIDTH: f32 = 380.0;
const PAD: f32 = 16.0;
const RADIUS: f32 = 9.0;
const BORDER: f32 = 1.0;
/// Its top's space under the header (D64 2).
pub(crate) const DROP: f32 = 8.0;
/// The least space kept between the panel and the window's edge (D70).
const EDGE: f32 = 8.0;
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

/// What a confirm is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Subject {
    /// Take block `key` over (gui-design F3).
    Block(u64),
    /// Open a link (C63, D70).
    Link(Link),
    /// Forward a loopback port of pane `key`'s host, then open the link
    /// here (mux C81, D341).
    Forward(u64, Loopback),
}

/// What a key or click on an open confirm did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    /// Nothing that closes it.
    Stay,
    /// Closed without acting (the first button, Esc, a click off it).
    Cancel,
    /// Button `n` of [`Confirm::new`]'s `buttons` was pressed (n >= 1).
    Pick(usize),
}

/// A key the confirm answers (D64 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConfirmKey {
    Tab,
    BackTab,
    Enter,
    Escape,
    /// Scroll a long URL by a line or a page (D70).
    Up,
    Down,
    PageUp,
    PageDown,
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

/// The copy for opening `link` (C63, D70): the whole URL, never cut, then
/// what "Always trust" would remember.
pub(crate) fn link_copy(link: &Link) -> (String, String) {
    let mut body = format!("Opens {}.", link.url);
    if !link.origin.is_empty() {
        body.push_str(&format!(" Always trust remembers {}.", link.origin));
    }
    // Lead with the host: it is what the user decides on.
    let title = match link.origin.split_once("://") {
        Some((_, host)) => format!("Open link to {host}?"),
        None => "Open this link?".to_owned(),
    };
    (title, body)
}

/// The copy for forwarding `target` (C81, D341): says where the port is,
/// since `localhost` in a remote pane is not this machine.
pub(crate) fn forward_copy(target: &Loopback) -> (String, String) {
    (
        format!("Forward port {} from this pane's host?", target.port),
        format!(
            "{}://localhost:{} is on the machine this pane runs on. This opens a tunnel to it \
             on this computer and opens it in your browser. It asks once per host and port, then \
             remembers.",
            target.scheme, target.port
        ),
    )
}

/// The forward prompt's buttons: Cancel (focused) and the accent one.
pub(crate) fn forward_buttons() -> Vec<String> {
    vec!["Cancel".to_owned(), "Forward and open".to_owned()]
}

/// The link prompt's "Always trust" button, as an [`Answer::Pick`] ("Open
/// once" is Pick(1)).
pub(crate) const ALWAYS: usize = 2;

/// Clicks this soon after the prompt opens are ignored, so the click that
/// opened it (or a double click) cannot press a button that appeared under
/// the pointer.
const CLICK_GUARD: std::time::Duration = std::time::Duration::from_millis(300);

/// The link prompt's buttons: Cancel (focused), Open once, and, where the
/// policy allows it, Always trust.
pub(crate) fn link_buttons(can_trust: bool) -> Vec<String> {
    let mut b = vec!["Cancel".to_owned(), "Open once".to_owned()];
    if can_trust {
        b.push("Always trust".to_owned());
    }
    b
}

/// An open confirm for block `key`, at window px `at`.
pub(crate) struct Confirm {
    pub subject: Subject,
    text: UiText,
    title: String,
    body: String,
    /// Button labels, left to right; the first cancels, the last has the
    /// accent unless `lead` says otherwise.
    buttons: Vec<String>,
    /// The accent button.
    lead: usize,
    width: f32,
    at: (usize, usize),
    focus: usize,
    hover: Option<usize>,
    /// Where it was asked: its right edge and top, window px. It is kept
    /// whole inside the window at paint time (D70).
    anchor: (f32, f32),
    /// The first body line shown, and the lines and rows of the last paint.
    scroll: usize,
    shown: (usize, usize),
    opened: std::time::Instant,
    painted: Option<(Look, Painted)>,
}

/// What a painting depends on besides the confirm's own state.
type Look = (Chrome, f32, (u32, u32));

/// A painted confirm and its buttons' boxes, in its own px.
pub(crate) struct Painted {
    pub pixmap: Pixmap,
    pub buttons: Vec<(Rect, usize)>,
    /// Body lines in all, and how many rows showed them.
    pub lines: (usize, usize),
}

impl Confirm {
    /// A confirm about `subject` saying `title` over `body`, its right
    /// edge at window px `right` and its top at `top`, `width` logical px
    /// wide. `buttons` run left to right, the first cancels, `lead` is the
    /// accent one and `focus` starts focused. `None` if the interface font
    /// is broken.
    pub(crate) fn new(
        subject: Subject,
        (title, body): (String, String),
        buttons: Vec<String>,
        (lead, focus, width): (usize, usize, f32),
        right: f32,
        top: f32,
        s: f32,
    ) -> Option<Self> {
        let x = (right - width * s).max(0.0).round() as usize;
        Some(Self {
            anchor: (right, top),
            scroll: 0,
            shown: (0, 0),
            opened: std::time::Instant::now(),
            subject,
            text: UiText::embedded()?,
            title,
            body,
            buttons,
            lead,
            width,
            at: (x, top.max(0.0).round() as usize),
            focus,
            hover: None,
            painted: None,
        })
    }

    /// The take-over confirm for block `key` (D64 2): "Take over" leads and
    /// has the focus.
    pub(crate) fn take_over(
        key: u64,
        copy: (String, String),
        right: f32,
        top: f32,
        s: f32,
    ) -> Option<Self> {
        let buttons = vec!["Cancel".to_owned(), "Take over".to_owned()];
        Self::new(
            Subject::Block(key),
            copy,
            buttons,
            (1, 1, WIDTH),
            right,
            top,
            s,
        )
    }

    fn step(&mut self, back: bool) {
        let n = self.buttons.len();
        self.focus = if back {
            (self.focus + n - 1) % n
        } else {
            (self.focus + 1) % n
        };
        self.painted = None;
    }

    /// Answer `key`: Tab moves the focus, Enter presses the focused
    /// button, Esc cancels.
    pub(crate) fn key(&mut self, key: ConfirmKey) -> Answer {
        match key {
            ConfirmKey::Tab => {
                self.step(false);
                Answer::Stay
            }
            ConfirmKey::BackTab => {
                self.step(true);
                Answer::Stay
            }
            ConfirmKey::Up | ConfirmKey::Down | ConfirmKey::PageUp | ConfirmKey::PageDown => {
                let page = self.shown.1.max(1) as isize;
                self.scroll_by(match key {
                    ConfirmKey::Up => -1,
                    ConfirmKey::Down => 1,
                    ConfirmKey::PageUp => -page,
                    _ => page,
                });
                Answer::Stay
            }
            ConfirmKey::Enter => press(self.focus),
            ConfirmKey::Escape => Answer::Cancel,
        }
    }

    /// Answer a click at window px `(x, y)`: a button presses, anywhere
    /// off the panel cancels.
    pub(crate) fn click(&self, x: f64, y: f64) -> Answer {
        if self.opened.elapsed() < CLICK_GUARD {
            return Answer::Stay;
        }
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

    /// Scroll the body by `by` lines (a long URL); no-op if it all shows.
    pub(crate) fn scroll_by(&mut self, by: isize) {
        let (lines, rows) = self.shown;
        let max = lines.saturating_sub(rows);
        let to = self.scroll.saturating_add_signed(by).min(max);
        if to != self.scroll {
            self.scroll = to;
            self.painted = None;
        }
    }

    /// Paint for `chrome` at `scale` in a window of `view` px, unless the
    /// last painting is that. The panel is narrowed, moved and its body
    /// cut to rows with a scroll hint so every button is inside the window
    /// (D70): the user can always see and reach what they approve.
    pub(crate) fn repaint(&mut self, chrome: Chrome, scale: f32, view: (f32, f32)) {
        let look = (chrome, scale, (view.0 as u32, view.1 as u32));
        if matches!(&self.painted, Some((l, _)) if *l == look) {
            return;
        }
        let margin = EDGE * scale;
        let width = (self.width * scale).min((view.0 - 2.0 * margin).max(1.0));
        let fixed = (2.0 * PAD + TITLE.1 + TITLE_GAP + ACTS_GAP + BUTTON.0) * scale;
        let rows = ((view.1 - 2.0 * margin - fixed) / (BODY.1 * scale))
            .floor()
            .max(1.0) as usize;
        let state = State {
            title: &self.title,
            body: &self.body,
            buttons: &self.buttons,
            lead: self.lead,
            width: width / scale,
            rows,
            scroll: self.scroll,
            focus: self.focus,
            hover: self.hover,
        };
        let painted = paint(&state, (&chrome, scale), &mut self.text);
        let (w, h) = (painted.pixmap.width as f32, painted.pixmap.height as f32);
        let x = (self.anchor.0 - w).min(view.0 - w - margin).max(0.0);
        let y = self.anchor.1.min(view.1 - h - margin).max(0.0);
        self.at = (x.round() as usize, y.round() as usize);
        self.shown = painted.lines;
        self.scroll = self.scroll.min(self.shown.0.saturating_sub(self.shown.1));
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

fn press(button: usize) -> Answer {
    match button {
        0 => Answer::Cancel,
        n => Answer::Pick(n),
    }
}

fn button_at(p: &Painted, x: f32, y: f32) -> Option<usize> {
    p.buttons
        .iter()
        .find(|(r, _)| r.contains(x, y))
        .map(|&(_, c)| c)
}

/// What a confirm shows.
struct State<'a> {
    title: &'a str,
    body: &'a str,
    buttons: &'a [String],
    lead: usize,
    width: f32,
    /// How many body rows fit, and the first line to show.
    rows: usize,
    scroll: usize,
    focus: usize,
    hover: Option<usize>,
}

/// The confirm panel: title, wrapped body, then "Cancel" and "Take over"
/// at the right, the focused one ringed (D64 2).
fn paint(confirm: &State<'_>, (c, s): (&Chrome, f32), font: &mut UiText) -> Painted {
    let w = (confirm.width * s).round();
    let room = w - 2.0 * PAD * s;
    let (title_px, title_h) = (TITLE.0 * s, TITLE.1 * s);
    let (body_px, body_h) = (BODY.0 * s, BODY.1 * s);
    let title = font.fit(confirm.title, Weight::Medium, title_px, room);
    let all = wrap_hard(font, confirm.body, body_px, room);
    let (lines, hint) = cut(&all, confirm.rows, confirm.scroll);
    let (btn_h, btn_pad, btn_r, btn_px) = BUTTON;
    let h = (2.0 * PAD * s
        + title_h
        + TITLE_GAP * s
        + body_h * (lines.len() + usize::from(hint.is_some())) as f32
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
    for line in lines {
        let base = y + body_h / 2.0 + font.centre_drop(body_px);
        out.text(font, line, Weight::Regular, body_px, PAD * s, base, c.muted);
        y += body_h;
    }
    if let Some(hint) = hint {
        let base = y + body_h / 2.0 + font.centre_drop(body_px);
        out.text(font, &hint, Weight::Medium, body_px, PAD * s, base, c.text);
        y += body_h;
    }
    y += ACTS_GAP * s;
    let mut right = w - PAD * s;
    let mut buttons = Vec::new();
    for (choice, label) in confirm.buttons.iter().enumerate().rev() {
        let bw = font.width(label, Weight::Medium, btn_px * s) + 2.0 * btn_pad * s;
        let r = Rect {
            x: right - bw,
            y,
            w: bw,
            h: btn_h * s,
        };
        let lit = confirm.hover == Some(choice);
        let (fill, ink) = match choice == confirm.lead {
            true if lit => (mix(c.accent, c.text, HOVER_MIX), c.on_accent),
            true => (c.accent, c.on_accent),
            false if lit => (c.hover, c.text),
            false => (c.raised, c.text),
        };
        out.fill(r, btn_r * s, fill);
        if choice != confirm.lead {
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
        lines: (all.len(), lines.len()),
    }
}

/// The lines of `all` to show in `rows` rows from line `scroll`, and, when
/// they do not all fit, a hint row saying which (it takes one of the rows).
fn cut(all: &[String], rows: usize, scroll: usize) -> (&[String], Option<String>) {
    if all.len() <= rows {
        return (all, None);
    }
    let rows = rows.saturating_sub(1).max(1);
    let from = scroll.min(all.len() - rows);
    let hint = format!(
        "Lines {}-{} of {}. Up and Down scroll.",
        from + 1,
        from + rows,
        all.len()
    );
    (&all[from..from + rows], Some(hint))
}

/// [`wrap`], then any line still wider than `room` (a long URL has no
/// spaces) cut at characters, so nothing is hidden or clipped (D70).
fn wrap_hard(font: &UiText, text: &str, px: f32, room: f32) -> Vec<String> {
    let mut out = Vec::new();
    for line in wrap(font, text, px, room) {
        let mut cur = String::new();
        for ch in line.chars() {
            cur.push(ch);
            if font.width(&cur, Weight::Regular, px) > room && cur.chars().count() > 1 {
                cur.pop();
                out.push(std::mem::take(&mut cur));
                cur.push(ch);
            }
        }
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open() -> Confirm {
        Confirm::take_over(4, copy("claude", "orch"), 600.0, 100.0, 1.0)
            .expect("the embedded faces parse")
    }

    fn link(u: &str) -> Link {
        crate::gui::link_policy::parse(u).expect("a valid link")
    }

    fn open_link(u: &str, can_trust: bool) -> Confirm {
        let l = link(u);
        Confirm::new(
            Subject::Link(l.clone()),
            link_copy(&l),
            link_buttons(can_trust),
            (1, 0, LINK_WIDTH),
            600.0,
            100.0,
            1.0,
        )
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
            Answer::Pick(1),
            "Take over has the focus"
        );
        assert_eq!(c.key(ConfirmKey::Tab), Answer::Stay);
        assert_eq!(
            c.key(ConfirmKey::Enter),
            Answer::Cancel,
            "Cancel has it now"
        );
        assert_eq!(c.key(ConfirmKey::BackTab), Answer::Stay);
        assert_eq!(c.key(ConfirmKey::Enter), Answer::Pick(1));
        assert_eq!(c.key(ConfirmKey::Escape), Answer::Cancel);
    }

    #[test]
    fn the_link_prompt_focuses_cancel_and_cycles_its_buttons() {
        let mut c = open_link("https://example.com/a", true);
        assert_eq!(
            c.key(ConfirmKey::Enter),
            Answer::Cancel,
            "Cancel is focused"
        );
        c.key(ConfirmKey::Tab);
        assert_eq!(c.key(ConfirmKey::Enter), Answer::Pick(1), "Open once");
        c.key(ConfirmKey::Tab);
        assert_eq!(c.key(ConfirmKey::Enter), Answer::Pick(2), "Always trust");
        c.key(ConfirmKey::Tab);
        assert_eq!(c.key(ConfirmKey::Enter), Answer::Cancel, "wraps to Cancel");
        c.key(ConfirmKey::BackTab);
        assert_eq!(c.key(ConfirmKey::Enter), Answer::Pick(2), "and back");
    }

    #[test]
    fn the_title_leads_with_the_host_and_the_body_ends_sentences() {
        let (title, body) = link_copy(&link("https://docs.example.com:8443/a"));
        assert_eq!(title, "Open link to docs.example.com:8443?");
        assert_eq!(
            body,
            "Opens https://docs.example.com:8443/a. Always trust remembers https://docs.example.com:8443."
        );
        let (title, body) = link_copy(&link("mailto:a@b.c"));
        assert_eq!(
            (title.as_str(), body.as_str()),
            ("Open this link?", "Opens mailto:a@b.c.")
        );
    }

    #[test]
    fn untrustable_links_offer_no_always_trust() {
        assert_eq!(link_buttons(false), ["Cancel", "Open once"]);
        let mut c = open_link("file:///etc/hosts", false);
        c.key(ConfirmKey::BackTab);
        assert_eq!(
            c.key(ConfirmKey::Enter),
            Answer::Pick(1),
            "wraps past Open once only"
        );
    }

    #[test]
    fn a_long_url_is_shown_whole() {
        let url = format!("https://example.com/{}", "a".repeat(300));
        let l = link(&url);
        let font = UiText::embedded().expect("faces");
        let lines = wrap_hard(&font, &link_copy(&l).1, BODY.0, 300.0);
        assert!(lines.len() > 1);
        assert!(
            lines
                .iter()
                .all(|l| font.width(l, Weight::Regular, BODY.0) <= 300.0)
        );
        let joined: String = lines.concat();
        assert!(
            joined.contains(&url.replace(' ', "")),
            "every character of the URL is kept"
        );
    }

    #[test]
    fn it_sits_right_aligned_and_clicks_hit_its_buttons_or_cancel() {
        let mut c = open();
        c.repaint(
            Chrome::from_theme(&crate::core::Theme::default()),
            1.0,
            (10000.0, 10000.0),
        );
        let (pixmap, at) = c.painted().expect("painted");
        assert_eq!(at, (300, 100), "its right edge on the button's");
        assert_eq!(pixmap.width, 300);
        let (_, p) = c.painted.as_ref().expect("painted");
        let take = p
            .buttons
            .iter()
            .find(|(_, b)| *b == 1)
            .expect("Take over")
            .0;
        let cancel = p.buttons.iter().find(|(_, b)| *b == 0).expect("Cancel").0;
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
        assert_eq!(c.click(x, y), Answer::Stay, "too soon after it opened");
        c.opened -= std::time::Duration::from_secs(1);
        assert_eq!(c.click(x, y), Answer::Pick(1));
        let (x, y) = mid(cancel);
        assert_eq!(c.click(x, y), Answer::Cancel);
        assert_eq!(c.click(310.0, 110.0), Answer::Stay, "on the panel");
        assert_eq!(c.click(10.0, 10.0), Answer::Cancel, "off it");
    }

    /// Paint `c` in `view` at `scale`; panic unless the whole panel, buttons
    /// included, is inside the window.
    fn assert_inside(c: &mut Confirm, scale: f32, view: (f32, f32)) {
        c.repaint(
            Chrome::from_theme(&crate::core::Theme::default()),
            scale,
            view,
        );
        let (pm, at) = c.painted().expect("painted");
        assert!(
            at.0 as f32 + pm.width as f32 <= view.0,
            "right edge {at:?} in {view:?}"
        );
        assert!(
            at.1 as f32 + pm.height as f32 <= view.1,
            "bottom edge {at:?} in {view:?}"
        );
        let (_, p) = c.painted.as_ref().expect("painted");
        for (r, _) in &p.buttons {
            assert!(
                r.y + r.h <= pm.height as f32 && r.x >= 0.0,
                "button inside the panel"
            );
        }
    }

    fn open_at(url: &str, right: f32, top: f32, s: f32) -> Confirm {
        let l = link(url);
        Confirm::new(
            Subject::Link(l.clone()),
            link_copy(&l),
            link_buttons(true),
            (1, 0, LINK_WIDTH),
            right,
            top,
            s,
        )
        .expect("faces")
    }

    #[test]
    fn a_click_near_the_bottom_or_right_edge_keeps_the_panel_inside() {
        for scale in [1.0, 1.5, 2.0] {
            for (x, y) in [(790.0, 590.0), (5.0, 595.0), (400.0, 580.0), (800.0, 0.0)] {
                let mut c = open_at(
                    "https://example.com/a/b?c=d",
                    x + 190.0 * scale,
                    y + 12.0 * scale,
                    scale,
                );
                assert_inside(&mut c, scale, (800.0, 600.0));
            }
        }
    }

    #[test]
    fn a_maximum_length_url_scrolls_with_its_buttons_in_view() {
        let url = format!("https://example.com/{}", "ab/".repeat(675));
        assert!(url.len() > 2000);
        for (view, scale) in [
            ((800.0, 600.0), 1.0),
            ((320.0, 240.0), 1.0),
            ((900.0, 700.0), 2.0),
        ] {
            let mut c = open_at(&url, view.0, view.1, scale);
            assert_inside(&mut c, scale, view);
            let (total, rows) = c.shown;
            assert!(total > rows, "{view:?}: it needs scrolling");
            // Scrolling to the end shows the tail; the panel stays inside.
            c.scroll_by(isize::MAX / 2);
            assert_inside(&mut c, scale, view);
            assert_eq!(c.scroll, total - rows, "clamped at the last page");
            c.scroll_by(-(isize::MAX / 2));
            assert_eq!(c.scroll, 0, "clamped at the top");
        }
    }

    #[test]
    fn the_keys_scroll_a_long_url_and_a_short_one_does_not() {
        let url = format!("https://example.com/{}", "ab/".repeat(675));
        let mut c = open_at(&url, 800.0, 100.0, 1.0);
        assert_inside(&mut c, 1.0, (800.0, 400.0));
        assert_eq!(c.key(ConfirmKey::Down), Answer::Stay);
        assert_eq!(c.scroll, 1);
        c.key(ConfirmKey::PageDown);
        assert_eq!(c.scroll, 1 + c.shown.1);
        c.key(ConfirmKey::Up);
        c.key(ConfirmKey::PageUp);
        assert_eq!(c.scroll, 0);
        let mut short = open_at("https://example.com/", 800.0, 100.0, 1.0);
        assert_inside(&mut short, 1.0, (800.0, 600.0));
        short.key(ConfirmKey::Down);
        assert_eq!(short.scroll, 0, "nothing to scroll");
    }

    #[test]
    fn a_window_narrower_than_the_panel_narrows_it() {
        let mut c = open_at("https://example.com/", 300.0, 10.0, 1.0);
        assert_inside(&mut c, 1.0, (320.0, 600.0));
        let (pm, _) = c.painted().expect("painted");
        assert!(pm.width <= 304, "narrowed to the window less its edges");
    }

    #[test]
    fn a_new_window_size_or_scale_repaints() {
        let mut c = open_at("https://example.com/a", 790.0, 590.0, 1.0);
        assert_inside(&mut c, 1.0, (800.0, 600.0));
        assert_inside(&mut c, 1.0, (800.0, 300.0));
        assert_inside(&mut c, 2.0, (800.0, 300.0));
    }
}
