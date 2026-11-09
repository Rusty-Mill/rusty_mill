//! An attached window's blocks (gui-design F1; geometry D47, header D48):
//! the server's split tree laid out in pixels with gaps of the frame colour,
//! each block framed and headed by its folder and branch, or by its program
//! and status, with who else has input and its zoom button on the right.
//! A zoomed block fills the body beside a rail of the others (F7), the
//! armed prefix numbers the blocks (F2), and the ribbon runs along the
//! bottom (D49).
//!
//! Pure: [`grid_area`] says where a block's terminal sits, [`header`] what
//! its header says, and [`paint`] draws every frame, header, chip and the
//! ribbon into one pixmap under the strip, which both renderers present
//! behind the grids, with the spots a click can hit.
//! Sizes are the pack's logical px times the DPI scale, never the terminal
//! zoom.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use super::banner::{self, Action, Banner};
use super::chrome::Chrome;
use super::layout::Rect as Cells;
use super::paint::{Pixmap, Rect, icons};
use super::render::blend;
use super::ribbon::{self, Cap, Ribbon, keycap, keycap_width};
use super::source::{BlockInfo, Gate, Link, Refusal, State, Take, Wait};
use super::text::{UiText, Weight};
use crate::core::contrast_ratio;

/// Space between blocks, and around them but above.
const GAP: f32 = 8.0;
const RADIUS: f32 = 9.0;
/// Border widths: every block, and the focused one in the accent.
const BORDER: f32 = 1.0;
const FOCUS_BORDER: f32 = 1.5;
/// The focused border's accent opacity, unless that falls under 3:1.
const FOCUS_ALPHA: u8 = 179;
const FOCUS_MIN: f32 = 3.0;
const HEADER: f32 = 38.0;
/// Half the header, from a header control's middle to its bottom.
pub(crate) const HEADER_HALF: f32 = HEADER / 2.0;
const HAIRLINE: f32 = 1.0;
/// Header padding (left, right), space between its items, and the eye's
/// space before the lease text.
const HEADER_PAD: (f32, f32) = (14.0, 12.0);
const ITEM_GAP: f32 = 8.0;
const EYE_GAP: f32 = 6.0;
/// The terminal's padding inside the frame (sides, top and bottom).
const GRID_PAD: (f32, f32) = (16.0, 12.0);
const ICON: f32 = 16.0;
const ICON_STROKE: f32 = 1.5;
const TEXT_PX: f32 = 13.0;
const SMALL_PX: f32 = 12.0;
/// The least a cut text keeps before the next one is cut instead.
const TEXT_MIN: f32 = 48.0;
/// The zoom button (D47, D49): its box and radius.
const ZOOM_BTN: (f32, f32) = (26.0, 24.0);
const BTN_R: f32 = 5.0;
/// The zoomed header's "Zoomed" tag and "Exit zoom" button: heights, side
/// padding, radii and text sizes.
const TAG: (f32, f32, f32) = (20.0, 8.0, 5.0);
const TAG_PX: f32 = 11.0;
const EXIT: (f32, f32, f32) = (24.0, 10.0, 6.0);
/// The take-over button's space before the lease line (gui-design D64).
const TAKE_GAP: f32 = 10.0;
/// F2 badges: side, radius and digit size.
const BADGE: f32 = 32.0;
const BADGE_R: f32 = 8.0;
const BADGE_PX: f32 = 16.0;
/// F7's rail: its width, a chip's height and the space between chips, and
/// the space between a chip's glyph and its number.
const RAIL: f32 = 36.0;
const CHIP_H: f32 = 56.0;
const CHIP_GAP: f32 = 3.0;
/// How much of what is under it a down link's veil covers: the blocks
/// show at 70% (D62).
const DIM_VEIL: u8 = 77;
/// The empty state's title and line sizes and the gap between (D28).
const EMPTY_TITLE_PX: f32 = 14.0;
const EMPTY_LINE_PX: f32 = 12.0;
const EMPTY_GAP: f32 = 8.0;
/// The ended session panel (D28, D62; gui_lib `.panel`): its widest, the
/// space between its parts and before its button, its title and text
/// sizes and line heights, the alert's stroke and the title's gap.
const PANEL_MAX: f32 = 420.0;
const PANEL_GAP: f32 = 10.0;
const PANEL_ACTS: f32 = 4.0;
const PANEL_TITLE: (f32, f32) = (15.0, 20.0);
const PANEL_TEXT: (f32, f32) = (12.0, 18.0);
const PANEL_ICON_STROKE: f32 = 1.6;
/// A primary button: height, side padding, radius, text size.
const PRIMARY: (f32, f32, f32, f32) = (28.0, 12.0, 6.0, 12.0);
/// What the panel says of the other sessions.
const ENDED_TEXT: &str =
    "The server closed it, or the last block exited. Your other sessions are still running.";

/// The pixel area the blocks tile: the window under its top bars, inset
/// [`GAP`] but at the top, above `bottom` px of other bars.
pub(crate) fn area(width: usize, height: usize, top: usize, bottom: usize, scale: f32) -> Cells {
    let gap = px(GAP, scale);
    Cells::new(
        gap,
        top,
        width.saturating_sub(2 * gap),
        height.saturating_sub(top + bottom + gap),
    )
}

/// The gap between blocks, in px.
pub(crate) fn gap(scale: f32) -> usize {
    px(GAP, scale)
}

/// A zoomed block's box in `area`, and the rail beside it when `others`
/// blocks are hidden (F7, D49).
pub(crate) fn zoomed(area: Cells, others: usize, scale: f32) -> (Cells, Option<Cells>) {
    if others == 0 {
        return (area, None);
    }
    let rail = px(RAIL, scale);
    let side = rail + px(GAP, scale);
    let block = Cells {
        cols: area.cols.saturating_sub(side),
        ..area
    };
    let rail = Cells::new(
        area.col + area.cols.saturating_sub(rail),
        area.row,
        rail,
        area.rows,
    );
    (block, Some(rail))
}

/// The first `n` chip boxes down `rail`, as many as fit.
pub(crate) fn chips(rail: Cells, n: usize, scale: f32) -> Vec<Cells> {
    let (h, step) = (px(CHIP_H, scale), px(CHIP_H + GAP, scale));
    (0..n)
        .map(|i| Cells::new(rail.col, rail.row + i * step, rail.cols, h))
        .take_while(|c| c.row + c.rows <= rail.row + rail.rows)
        .collect()
}

/// Where a block's terminal sits inside `block`, both in px: under the
/// header and its hairline, padded inside the border.
pub(crate) fn grid_area(block: Cells, scale: f32) -> Cells {
    let side = px(BORDER + GRID_PAD.0, scale);
    let top = px(BORDER + HEADER + HAIRLINE + GRID_PAD.1, scale);
    let bottom = px(BORDER + GRID_PAD.1, scale);
    Cells::new(
        block.col + side,
        block.row + top,
        block.cols.saturating_sub(2 * side),
        block.rows.saturating_sub(top + bottom),
    )
}

fn px(logical: f32, scale: f32) -> usize {
    (logical * scale).round() as usize
}

/// A header icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Glyph {
    Folder,
    Branch,
    Spinner,
    Pause,
    Check,
    Cross,
}

/// A header colour, from the chrome's tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ink {
    Text,
    Muted,
    Accent,
    Success,
    Failure,
}

/// Which end of a text survives a cut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Keep {
    Start,
    End,
}

/// One item of a header's left group.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Part {
    Icon(Glyph, Ink),
    Text {
        text: String,
        weight: Weight,
        ink: Ink,
        keep: Keep,
    },
}

/// What a block's header says (D48): its left group, the lease line on
/// the right when another client has input or the block is its owner's,
/// and the take-over button on an owner-only block (gui-design D64).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Header {
    pub parts: Vec<Part>,
    pub lease: Option<Lease>,
    pub take: Option<TakeButton>,
}

/// The header's lease line: who has input, or why taking over failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Lease {
    pub text: String,
    /// A failure: an oxide alert and text instead of the slate eye (D64 4).
    pub alert: bool,
}

impl Lease {
    fn said(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            alert: false,
        }
    }

    fn alert(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            alert: true,
        }
    }
}

/// An owner-only block's header button (gui-design D64).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TakeButton {
    /// Opens the confirm.
    TakeOver,
    /// Stops waiting for the owner.
    CancelAsk,
    /// Gives the input back.
    GiveBack,
}

impl TakeButton {
    pub(crate) fn label(self) -> &'static str {
        match self {
            TakeButton::TakeOver => "Take over",
            TakeButton::CancelAsk => "Cancel ask",
            TakeButton::GiveBack => "Give back",
        }
    }
}

fn text(text: impl Into<String>, weight: Weight, ink: Ink, keep: Keep) -> Part {
    Part::Text {
        text: text.into(),
        weight,
        ink,
        keep,
    }
}

/// The header for `info` (D48). `take` is where this window's taking it
/// over stands, once its source can ask (D64); `branch` is the git branch
/// of its folder, when known; `home` shortens the folder to "~".
pub(crate) fn header(
    info: &BlockInfo,
    take: Option<Take>,
    branch: Option<&str>,
    home: Option<&str>,
) -> Header {
    let parts = if info.foreground.is_none() && info.status.is_none() {
        shell_parts(info, branch, home)
    } else {
        program_parts(info)
    };
    let (lease, take) = lease_line(info, take);
    Header { parts, lease, take }
}

/// The lease line and button (D48, D64): who has input on an open block;
/// on an owner-only one, where taking it over stands.
fn lease_line(info: &BlockInfo, take: Option<Take>) -> (Option<Lease>, Option<TakeButton>) {
    let holder = info.holder.as_ref().map(|h| format!("{h} has input"));
    let (owner, take) = match (&info.gate, take) {
        (Some(Gate::Orphaned), _) => {
            let text = format!("{} has no owner now", program(info));
            return (Some(Lease::alert(text)), None);
        }
        (Some(Gate::Owned(owner)), Some(take)) => (owner, take),
        _ => return (holder.map(Lease::said), None),
    };
    match take {
        Take::Rest => {
            let other = info.holder.as_ref().filter(|h| *h != owner);
            let text = match other {
                Some(h) => format!("{h} has input"),
                None => "Agent has input".to_owned(),
            };
            (Some(Lease::said(text)), Some(TakeButton::TakeOver))
        }
        Take::Asking => (
            Some(Lease::said(format!("Asking {owner}\u{2026}"))),
            Some(TakeButton::CancelAsk),
        ),
        Take::Held => (
            Some(Lease::said("You have input")),
            Some(TakeButton::GiveBack),
        ),
        Take::Refused(Refusal::Unreachable) => (
            Some(Lease::alert(format!("Can't reach {owner} right now"))),
            Some(TakeButton::TakeOver),
        ),
        Take::Refused(Refusal::Other) => (
            Some(Lease::alert(format!("Couldn't ask {owner}"))),
            Some(TakeButton::TakeOver),
        ),
        Take::Ended => (
            Some(Lease::said(format!("Your input from {owner} ended"))),
            Some(TakeButton::TakeOver),
        ),
    }
}

/// The block's program as its header names it (D48).
pub(crate) fn program(info: &BlockInfo) -> &str {
    info.foreground
        .as_deref()
        .or(info.shell.as_deref())
        .unwrap_or("program")
}

/// The folder and its branch; the shell's name with no folder.
fn shell_parts(info: &BlockInfo, branch: Option<&str>, home: Option<&str>) -> Vec<Part> {
    let mut parts = match &info.cwd {
        Some(cwd) => vec![
            Part::Icon(Glyph::Folder, Ink::Muted),
            text(tilde(cwd, home), Weight::Medium, Ink::Text, Keep::End),
        ],
        None => vec![text(
            info.shell.as_deref().unwrap_or("shell"),
            Weight::Medium,
            Ink::Text,
            Keep::Start,
        )],
    };
    if let Some(branch) = branch {
        parts.push(Part::Icon(Glyph::Branch, Ink::Accent));
        parts.push(text(branch, Weight::Regular, Ink::Text, Keep::Start));
    }
    parts
}

/// A block's lead glyph (D48): its status, else a spinner while a program
/// other than the shell runs, else none. Rail chips show it too (D49).
pub(crate) fn lead(info: &BlockInfo) -> Option<(Glyph, Ink)> {
    match info.status.as_ref().map(|s| s.state) {
        None if info.foreground.is_some() => Some((Glyph::Spinner, Ink::Accent)),
        None | Some(State::Idle) => None,
        Some(State::Working) => Some((Glyph::Spinner, Ink::Accent)),
        Some(State::Blocked(_)) => Some((Glyph::Pause, Ink::Muted)),
        Some(State::Done) => Some((Glyph::Check, Ink::Success)),
        Some(State::Error) => Some((Glyph::Cross, Ink::Failure)),
    }
}

/// A block's state in a word or two, as the detached popover says it
/// (D61): its status, else "Running" while a program other than the
/// shell runs, else "Idle".
pub(crate) fn state_word(info: &BlockInfo) -> &'static str {
    match info.status.as_ref().map(|s| s.state) {
        None if info.foreground.is_some() => "Running",
        None | Some(State::Idle) => "Idle",
        Some(State::Working) => "Working",
        Some(State::Blocked(wait)) => waiting(wait),
        Some(State::Done) => "Done",
        Some(State::Error) => "Stopped: error",
    }
}

/// The program's status glyph, its name and its status text.
fn program_parts(info: &BlockInfo) -> Vec<Part> {
    let name = program(info);
    let msg = info.status.as_ref().and_then(|s| s.msg.clone());
    let said = match info.status.as_ref().map(|s| s.state) {
        None => None,
        Some(State::Working) => Some((msg.unwrap_or_else(|| "Working".into()), Ink::Muted)),
        Some(State::Blocked(wait)) => {
            Some((msg.unwrap_or_else(|| waiting(wait).into()), Ink::Text))
        }
        Some(State::Done) => Some((msg.unwrap_or_else(|| "Done".into()), Ink::Muted)),
        Some(State::Idle) => Some(("Idle".into(), Ink::Muted)),
        Some(State::Error) => Some((msg.unwrap_or_else(|| "Stopped: error".into()), Ink::Failure)),
    };
    let mut parts: Vec<Part> = lead(info)
        .map(|(g, i)| Part::Icon(g, i))
        .into_iter()
        .collect();
    parts.push(text(name, Weight::Medium, Ink::Text, Keep::Start));
    if let Some((said, ink)) = said {
        parts.push(text(said, Weight::Regular, ink, Keep::Start));
    }
    parts
}

fn waiting(wait: Option<Wait>) -> &'static str {
    match wait {
        Some(Wait::Permission) => "Needs permission",
        Some(Wait::Question) => "Has a question",
        Some(Wait::SignIn) => "Needs sign-in",
        None => "Waiting for you",
    }
}

/// `cwd` with `home` shown as "~".
fn tilde(cwd: &str, home: Option<&str>) -> String {
    let Some(home) = home.filter(|h| !h.is_empty()) else {
        return cwd.to_owned();
    };
    match cwd.strip_prefix(home) {
        Some("") => "~".to_owned(),
        Some(rest) if rest.starts_with(['/', '\\']) => format!("~{rest}"),
        _ => cwd.to_owned(),
    }
}

/// How to draw the blocks: colours, DPI scale and pointer state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Look {
    pub chrome: Chrome,
    /// The terminal background, which the grids sit on (D47).
    pub bg: u32,
    pub scale: f32,
    pub hover: Option<Spot>,
    pub pressed: Option<Spot>,
}

/// What a spot on the body does when clicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Spot {
    /// Zoom block `key` (its header's zoom button).
    Zoom(u64),
    /// Show every block again (the zoomed header's "Exit zoom").
    Unzoom,
    /// Focus block `key` (its chip on the zoomed window's rail).
    Chip(u64),
    /// A connection banner's button (gui-design D62).
    Banner(Action),
    /// The ended session panel's "Open switcher" (D62, D60).
    OpenSwitcher,
    /// Block `key`'s take-over button (gui-design D64).
    Take(u64, TakeButton),
}

/// A block to draw: its box in the body's px, whether it has the focus or
/// is zoomed, its header, and its F2 number while the prefix is armed.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Shown {
    pub key: u64,
    pub rect: Cells,
    pub focused: bool,
    pub zoomed: bool,
    pub header: Header,
    pub badge: Option<usize>,
}

/// A hidden block's chip on the zoomed window's rail (F7, D49).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Chip {
    pub key: u64,
    pub rect: Cells,
    pub lead: Option<(Glyph, Ink)>,
    pub number: usize,
}

/// Everything the body shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Scene {
    pub blocks: Vec<Shown>,
    pub rail: Vec<Chip>,
    pub ribbon: Ribbon,
    /// Every block is detached: this many, said in the middle (D61).
    pub empty: Option<usize>,
    /// The connection banner under the strip (D62).
    pub banner: Option<Banner>,
    /// The blocks show at 70% and take no clicks: the link is down (D62).
    pub dim: bool,
    /// The shown session, by name, ended while the link was down: its
    /// panel fills the body (D28, D62).
    pub ended: Option<String>,
}

/// A painted body: its pixels, what is drawn over the grids (F2 badges),
/// and the spots, all in the body's px.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Body {
    pub pixmap: Pixmap,
    pub over: Vec<(Pixmap, (usize, usize))>,
    pub spots: Vec<(Rect, Spot)>,
}

impl Body {
    /// The spot under `(x, y)`, if any.
    pub(crate) fn spot(&self, x: f32, y: f32) -> Option<Spot> {
        self.spots
            .iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|&(_, s)| s)
    }
}

/// The body under the strip, `width × height` px: the frame colour, each
/// block framed and headed, the rail's chips, and the ribbon at the bottom.
pub(crate) fn paint(
    width: usize,
    height: usize,
    scene: &Scene,
    look: &Look,
    font: &mut UiText,
) -> Body {
    let mut out = Pixmap::new(width, height, look.chrome.frame);
    let mut spots = Vec::new();
    for block in &scene.blocks {
        paint_block(&mut out, &mut spots, block, look, font);
    }
    for chip in &scene.rail {
        paint_chip(&mut out, chip, look, font);
        spots.push((rect(chip.rect), Spot::Chip(chip.key)));
    }
    let y = height.saturating_sub(ribbon::height(look.scale)) as f32;
    if let Some(n) = scene.empty {
        paint_empty(&mut out, n, y, look, font);
    }
    if let Some(name) = &scene.ended {
        let r = paint_ended(&mut out, name, y, look, font);
        spots.push((r, Spot::OpenSwitcher));
    }
    ribbon::paint(&mut out, y, &scene.ribbon, (&look.chrome, look.scale), font);
    let mut over: Vec<(Pixmap, (usize, usize))> = scene
        .blocks
        .iter()
        .filter_map(|b| badge(b, look, font))
        .collect();
    if scene.dim {
        // The frame at 30% over the blocks shows them at 70%.
        spots.clear();
        over.push((
            Pixmap::veil(width, y as usize, look.chrome.frame, DIM_VEIL),
            (0, 0),
        ));
    }
    if let Some(b) = &scene.banner {
        let painted = banner::paint(
            b,
            &scene.ribbon.target,
            width,
            (&look.chrome, look.scale),
            font,
        );
        spots.extend(
            painted
                .buttons
                .into_iter()
                .map(|(r, a)| (r, Spot::Banner(a))),
        );
        over.push((painted.pixmap, (0, 0)));
    }
    Body {
        pixmap: out,
        over,
        spots,
    }
}

/// The D28 empty state over the body above `bottom`: "No blocks shown"
/// over what keeps running, centred (D61).
fn paint_empty(out: &mut Pixmap, detached: usize, bottom: f32, look: &Look, font: &mut UiText) {
    let (s, c) = (look.scale, &look.chrome);
    let title = "No blocks shown";
    let line = match detached {
        1 => "1 detached block keeps running. Reattach it from the strip.".to_owned(),
        n => format!("{n} detached blocks keep running. Reattach one from the strip."),
    };
    let (title_px, line_px) = (EMPTY_TITLE_PX * s, EMPTY_LINE_PX * s);
    let total = title_px + EMPTY_GAP * s + line_px;
    let top = (bottom - total) / 2.0;
    let centre = out.width as f32 / 2.0;
    let title_w = font.width(title, Weight::Medium, title_px);
    let title_base = top + title_px / 2.0 + font.centre_drop(title_px);
    out.text(
        font,
        title,
        Weight::Medium,
        title_px,
        centre - title_w / 2.0,
        title_base,
        c.text,
    );
    let line_w = font.width(&line, Weight::Regular, line_px);
    let line_mid = top + title_px + EMPTY_GAP * s + line_px / 2.0;
    let line_base = line_mid + font.centre_drop(line_px);
    out.text(
        font,
        &line,
        Weight::Regular,
        line_px,
        centre - line_w / 2.0,
        line_base,
        c.muted,
    );
}

/// The D28 "Session {name} ended" panel in a block filling the body above
/// `bottom`, centred: the alert and title, what happened wrapped at
/// [`PANEL_MAX`], and "Open switcher", whose box it returns (D62).
fn paint_ended(out: &mut Pixmap, name: &str, bottom: f32, look: &Look, font: &mut UiText) -> Rect {
    let (s, c) = (look.scale, &look.chrome);
    let gap = GAP * s;
    let frame = Rect {
        x: gap,
        y: 0.0,
        w: (out.width as f32 - 2.0 * gap).max(0.0),
        h: (bottom - gap).max(0.0),
    };
    out.fill(frame, RADIUS * s, c.base);
    out.border(frame, RADIUS * s, BORDER * s, c.line);
    let title = format!("Session {name} ended");
    let (title_px, title_h) = (PANEL_TITLE.0 * s, PANEL_TITLE.1 * s);
    let (text_px, text_h) = (PANEL_TEXT.0 * s, PANEL_TEXT.1 * s);
    let room = (PANEL_MAX * s).min(frame.w - 2.0 * gap);
    let lines = wrap(font, ENDED_TEXT, text_px, room);
    let label = "Open switcher";
    let (btn_h, btn_pad, btn_r, btn_px) = PRIMARY;
    let label_w = font.width(label, Weight::Medium, btn_px * s);
    let title_w = (ICON + ITEM_GAP) * s + font.width(&title, Weight::Medium, title_px);
    let widest = lines
        .iter()
        .map(|l| font.width(l, Weight::Regular, text_px))
        .fold(title_w, f32::max)
        .min(room);
    let height = title_h
        + PANEL_GAP * s
        + text_h * lines.len() as f32
        + (PANEL_GAP + PANEL_ACTS + btn_h) * s;
    let x = frame.x + (frame.w - widest) / 2.0;
    let mut y = frame.y + (frame.h - height) / 2.0;
    let mid = y + title_h / 2.0;
    out.icon(
        icons::ALERT,
        x,
        mid - ICON * s / 2.0,
        ICON * s,
        PANEL_ICON_STROKE * s,
        c.failure,
    );
    let tx = x + (ICON + ITEM_GAP) * s;
    let base = mid + font.centre_drop(title_px);
    out.text(font, &title, Weight::Medium, title_px, tx, base, c.text);
    y += title_h + PANEL_GAP * s;
    for line in &lines {
        let base = y + text_h / 2.0 + font.centre_drop(text_px);
        out.text(font, line, Weight::Regular, text_px, x, base, c.muted);
        y += text_h;
    }
    y += (PANEL_GAP + PANEL_ACTS) * s;
    let button = Rect {
        x,
        y,
        w: label_w + 2.0 * btn_pad * s,
        h: btn_h * s,
    };
    out.fill(button, btn_r * s, c.accent);
    let base = y + button.h / 2.0 + font.centre_drop(btn_px * s);
    let lx = x + btn_pad * s;
    out.text(
        font,
        label,
        Weight::Medium,
        btn_px * s,
        lx,
        base,
        c.on_accent,
    );
    button
}

/// `text` broken at spaces into lines at most `room` px wide at `px`; a
/// word wider than that keeps a line of its own.
pub(crate) fn wrap(font: &UiText, text: &str, px: f32, room: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        let joined = lines.last().map(|l| format!("{l} {word}"));
        match joined {
            Some(j) if font.width(&j, Weight::Regular, px) <= room => {
                if let Some(last) = lines.last_mut() {
                    *last = j;
                }
            }
            _ => lines.push(word.to_owned()),
        }
    }
    lines
}

fn rect(c: Cells) -> Rect {
    Rect {
        x: c.col as f32,
        y: c.row as f32,
        w: c.cols as f32,
        h: c.rows as f32,
    }
}

fn paint_block(
    out: &mut Pixmap,
    spots: &mut Vec<(Rect, Spot)>,
    block: &Shown,
    look: &Look,
    font: &mut UiText,
) {
    let (s, c) = (look.scale, &look.chrome);
    let r = rect(block.rect);
    let radius = RADIUS * s;
    let border = BORDER * s;
    let head = Rect {
        h: border + HEADER * s,
        ..r
    };
    out.fill(r, radius, look.bg);
    out.fill_part(r, radius, head, c.base);
    out.fill(
        Rect {
            x: r.x + border + 1.0,
            y: head.y + head.h,
            w: (r.w - 2.0 * border - 2.0).max(0.0),
            h: HAIRLINE * s,
        },
        0.0,
        c.line,
    );
    let (edge, width) = match block.focused {
        true => (focus_border(c), FOCUS_BORDER * s),
        false => (c.line, border),
    };
    out.border(r, radius, width, edge);
    paint_header(out, spots, head, block, look, font);
}

/// The focused border: the accent at 70% over the frame, or whole when that
/// would fall under 3:1 against it (D27, D47).
fn focus_border(c: &Chrome) -> u32 {
    let soft = blend(c.frame, c.accent, FOCUS_ALPHA);
    if contrast_ratio(soft, c.frame) >= FOCUS_MIN {
        soft
    } else {
        c.accent
    }
}

pub(crate) fn ink(c: &Chrome, ink: Ink) -> u32 {
    match ink {
        Ink::Text => c.text,
        Ink::Muted => c.muted,
        Ink::Accent => c.accent,
        Ink::Success => c.success,
        Ink::Failure => c.failure,
    }
}

/// A raised control's fill: pressed, hovered or at rest (D26).
fn control_fill(c: &Chrome, look: &Look, spot: Spot) -> u32 {
    if look.pressed == Some(spot) {
        c.press
    } else if look.hover == Some(spot) {
        c.hover
    } else {
        c.raised
    }
}

fn paint_header(
    out: &mut Pixmap,
    spots: &mut Vec<(Rect, Spot)>,
    head: Rect,
    block: &Shown,
    look: &Look,
    font: &mut UiText,
) {
    let (s, c) = (look.scale, &look.chrome);
    let header = &block.header;
    let border = BORDER * s;
    let mid = head.y + border + HEADER * s / 2.0;
    let icon_y = mid - ICON * s / 2.0;
    let base = mid + font.centre_drop(TEXT_PX * s);
    let small_base = mid + font.centre_drop(SMALL_PX * s);
    let left = head.x + border + HEADER_PAD.0 * s;
    let mut right = head.x + head.w - border - HEADER_PAD.1 * s;
    // The right group never shrinks: the zoom controls, then the lease
    // text with the eye before it.
    right = match block.zoomed {
        true => paint_zoomed_group(out, spots, right, mid, look, font),
        false => {
            let r = Rect {
                x: right - ZOOM_BTN.0 * s,
                y: mid - ZOOM_BTN.1 * s / 2.0,
                w: ZOOM_BTN.0 * s,
                h: ZOOM_BTN.1 * s,
            };
            let spot = Spot::Zoom(block.key);
            out.fill(r, BTN_R * s, control_fill(c, look, spot));
            let at = (r.x + (r.w - ICON * s) / 2.0, mid - ICON * s / 2.0);
            out.icon(icons::ZOOM, at.0, at.1, ICON * s, ICON_STROKE * s, c.muted);
            spots.push((r, spot));
            r.x
        }
    } - ITEM_GAP * s;
    if let Some(button) = header.take {
        let spot = Spot::Take(block.key, button);
        right = small_button(out, spots, (right, mid), button.label(), spot, look, font);
        right -= TAKE_GAP * s;
    }
    if let Some(lease) = &header.lease {
        let (glyph, tint) = match lease.alert {
            true => (icons::ALERT, c.failure),
            false => (icons::EYE, c.muted),
        };
        let w = font.width(&lease.text, Weight::Regular, SMALL_PX * s);
        right -= w;
        out.text(
            font,
            &lease.text,
            Weight::Regular,
            SMALL_PX * s,
            right,
            small_base,
            tint,
        );
        right -= (EYE_GAP + ICON) * s;
        out.icon(glyph, right, icon_y, ICON * s, ICON_STROKE * s, tint);
        right -= ITEM_GAP * s;
    }
    let texts = fit(&header.parts, right - left, s, font);
    let mut x = left;
    for (part, text) in header.parts.iter().zip(texts) {
        match part {
            Part::Icon(glyph, i) => {
                paint_glyph(out, *glyph, x, icon_y, s, ink(c, *i), c.line);
                x += ICON * s;
            }
            Part::Text { weight, ink: i, .. } => {
                x += out.text(font, &text, *weight, TEXT_PX * s, x, base, ink(c, *i));
            }
        }
        x += ITEM_GAP * s;
    }
}

/// A secondary small button (D62) labelled `label`, ending at `right` and
/// centred on `mid`, that does `spot`. Returns where it starts.
fn small_button(
    out: &mut Pixmap,
    spots: &mut Vec<(Rect, Spot)>,
    (right, mid): (f32, f32),
    label: &str,
    spot: Spot,
    look: &Look,
    font: &mut UiText,
) -> f32 {
    let (s, c) = (look.scale, &look.chrome);
    let (h, pad, radius) = EXIT;
    let w = font.width(label, Weight::Medium, SMALL_PX * s) + 2.0 * pad * s;
    let r = Rect {
        x: right - w,
        y: mid - h * s / 2.0,
        w,
        h: h * s,
    };
    out.fill(r, radius * s, control_fill(c, look, spot));
    out.border(r, radius * s, BORDER * s, c.line);
    let base = mid + font.centre_drop(SMALL_PX * s);
    out.text(
        font,
        label,
        Weight::Medium,
        SMALL_PX * s,
        r.x + pad * s,
        base,
        c.text,
    );
    spots.push((r, spot));
    r.x
}

/// The zoomed header's right group, ending at `right` (D49): the "Zoomed"
/// tag, the keys that unzoom, and "Exit zoom". Returns where it starts.
fn paint_zoomed_group(
    out: &mut Pixmap,
    spots: &mut Vec<(Rect, Spot)>,
    right: f32,
    mid: f32,
    look: &Look,
    font: &mut UiText,
) -> f32 {
    let (s, c) = (look.scale, &look.chrome);
    let gap = ITEM_GAP * s;
    let (exit_h, exit_pad, exit_r) = EXIT;
    let exit_w = font.width("Exit zoom", Weight::Medium, SMALL_PX * s) + 2.0 * exit_pad * s;
    let exit = Rect {
        x: right - exit_w,
        y: mid - exit_h * s / 2.0,
        w: exit_w,
        h: exit_h * s,
    };
    out.fill(exit, exit_r * s, control_fill(c, look, Spot::Unzoom));
    out.border(exit, exit_r * s, BORDER * s, c.line);
    let base = mid + font.centre_drop(SMALL_PX * s);
    out.text(
        font,
        "Exit zoom",
        Weight::Medium,
        SMALL_PX * s,
        exit.x + exit_pad * s,
        base,
        c.text,
    );
    spots.push((exit, Spot::Unzoom));
    let mut x = exit.x;
    for key in ["z", "Ctrl-]"] {
        x -= gap + keycap_width(key, s, font);
        keycap(out, font, key, (x, mid), Cap::Plain, (c, s));
    }
    let (tag_h, tag_pad, tag_r) = TAG;
    let tag_w = font.width("Zoomed", Weight::Regular, TAG_PX * s) + 2.0 * tag_pad * s;
    x -= gap + tag_w;
    let tag = Rect {
        x,
        y: mid - tag_h * s / 2.0,
        w: tag_w,
        h: tag_h * s,
    };
    out.fill(tag, tag_r * s, c.raised);
    out.border(tag, tag_r * s, BORDER * s, c.line);
    let tag_base = mid + font.centre_drop(TAG_PX * s);
    out.text(
        font,
        "Zoomed",
        Weight::Regular,
        TAG_PX * s,
        x + tag_pad * s,
        tag_base,
        c.muted,
    );
    x
}

/// A block's F2 badge (D49), centred under its header: its own pixmap,
/// since it sits over the grid, on the grid's background, and where it
/// goes in the body.
fn badge(block: &Shown, look: &Look, font: &mut UiText) -> Option<(Pixmap, (usize, usize))> {
    let n = block.badge?;
    let (s, c) = (look.scale, &look.chrome);
    let side = px(BADGE, s);
    let r = rect(block.rect);
    let below = r.y + (BORDER + HEADER + HAIRLINE) * s;
    let x = (r.x + (r.w - side as f32) / 2.0).max(0.0) as usize;
    let y = (below + (r.y + r.h - below - side as f32) / 2.0).max(0.0) as usize;
    let mut out = Pixmap::new(side, side, look.bg);
    let r = Rect {
        x: 0.0,
        y: 0.0,
        w: side as f32,
        h: side as f32,
    };
    let fg = if block.focused {
        out.fill(r, BADGE_R * s, c.accent);
        c.on_accent
    } else {
        out.fill(r, BADGE_R * s, c.raised);
        out.border(r, BADGE_R * s, BORDER * s, c.line);
        c.text
    };
    let digit = n.to_string();
    let w = font.width(&digit, Weight::Medium, BADGE_PX * s);
    let base = r.h / 2.0 + font.centre_drop(BADGE_PX * s);
    out.text(
        font,
        &digit,
        Weight::Medium,
        BADGE_PX * s,
        (r.w - w) / 2.0,
        base,
        fg,
    );
    Some((out, (x, y)))
}

/// A hidden block's rail chip: its lead glyph over its number (D49).
fn paint_chip(out: &mut Pixmap, chip: &Chip, look: &Look, font: &mut UiText) {
    let (s, c) = (look.scale, &look.chrome);
    let r = rect(chip.rect);
    out.fill(r, RADIUS * s, c.base);
    out.border(r, RADIUS * s, BORDER * s, c.line);
    let digit = chip.number.to_string();
    let cap = font.centre_drop(TEXT_PX * s) * 2.0;
    let glyph_h = chip.lead.map_or(0.0, |_| (ICON + CHIP_GAP) * s);
    let top = r.y + (r.h - glyph_h - cap) / 2.0;
    if let Some((glyph, i)) = chip.lead {
        let x = r.x + (r.w - ICON * s) / 2.0;
        paint_glyph(out, glyph, x, top, s, ink(c, i), c.line);
    }
    let w = font.width(&digit, Weight::Medium, TEXT_PX * s);
    out.text(
        font,
        &digit,
        Weight::Medium,
        TEXT_PX * s,
        r.x + (r.w - w) / 2.0,
        top + glyph_h + cap,
        c.text,
    );
}

/// The text each part shows in `room` px: whole when it fits, else the
/// last texts cut first, each down to [`TEXT_MIN`] before the one before
/// it is, then every one as far as it takes.
fn fit(parts: &[Part], room: f32, s: f32, font: &UiText) -> Vec<String> {
    let widths: Vec<f32> = parts
        .iter()
        .map(|p| match p {
            Part::Icon(..) => ICON * s,
            Part::Text { text, weight, .. } => font.width(text, *weight, TEXT_PX * s),
        })
        .collect();
    let gaps = parts.len().saturating_sub(1) as f32 * ITEM_GAP * s;
    let mut over = widths.iter().sum::<f32>() + gaps - room;
    let mut max = widths.clone();
    for floor in [TEXT_MIN * s, 0.0] {
        for (i, part) in parts.iter().enumerate().rev() {
            if over <= 0.0 {
                break;
            }
            if let Part::Text { .. } = part {
                let cut = (max[i] - floor).max(0.0).min(over);
                max[i] -= cut;
                over -= cut;
            }
        }
    }
    parts
        .iter()
        .zip(max)
        .map(|(p, max)| match p {
            Part::Icon(..) => String::new(),
            Part::Text {
                text, weight, keep, ..
            } => match keep {
                Keep::Start => font.fit(text, *weight, TEXT_PX * s, max),
                Keep::End => font.fit_tail(text, *weight, TEXT_PX * s, max),
            },
        })
        .collect()
}

pub(crate) fn paint_glyph(
    out: &mut Pixmap,
    glyph: Glyph,
    x: f32,
    y: f32,
    s: f32,
    color: u32,
    ring: u32,
) {
    let (size, stroke) = (ICON * s, ICON_STROKE * s);
    let path = match glyph {
        Glyph::Folder => icons::FOLDER,
        Glyph::Branch => icons::BRANCH,
        Glyph::Pause => icons::PAUSE,
        Glyph::Check => icons::CHECK,
        Glyph::Cross => icons::CROSS,
        Glyph::Spinner => {
            out.icon(icons::SPINNER_RING, x, y, size, stroke, ring);
            icons::SPINNER_ARC
        }
    };
    out.icon(path, x, y, size, stroke, color);
}

/// How long a folder's git branch is trusted before it is read again.
const BRANCH_TTL: Duration = Duration::from_secs(2);

/// An attached window's blocks: what the source says of each, and the last
/// painting, kept until the blocks, their headers or the look change.
pub(crate) struct BodyState {
    infos: HashMap<u64, BlockInfo>,
    /// Where taking each owner-only block over stands, by pane key, once
    /// its source can ask (gui-design D64).
    takes: HashMap<u64, Take>,
    /// The session runs on this machine, so its folders' branches can be
    /// read here.
    local: bool,
    /// What the window is connected to, for the ribbon.
    target: String,
    /// How it reaches its server (D62).
    link: Link,
    /// The shown session's name, once it ended while the link was down.
    ended: Option<String>,
    /// The info notice the banner shows while the link is up (D63).
    notice: Option<super::source::Notice>,
    home: Option<String>,
    branches: HashMap<String, (Option<String>, Instant)>,
    text: UiText,
    painted: Option<(Look, Scene, Body)>,
}

impl BodyState {
    /// No blocks known yet; `None` if the interface font is broken.
    pub(crate) fn new() -> Option<Self> {
        let home = std::env::var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).ok();
        Some(Self {
            infos: HashMap::new(),
            takes: HashMap::new(),
            local: false,
            target: String::new(),
            link: Link::Up,
            ended: None,
            notice: None,
            home,
            branches: HashMap::new(),
            text: UiText::embedded()?,
            painted: None,
        })
    }

    /// What the source now says of its blocks, by pane key.
    pub(crate) fn show(&mut self, blocks: Vec<(u64, BlockInfo)>) {
        self.infos = blocks.into_iter().collect();
    }

    /// Whether the session runs on this machine, and what the window is
    /// connected to.
    pub(crate) fn set_session(&mut self, local: bool, target: &str) {
        self.local = local;
        target.clone_into(&mut self.target);
    }

    /// The ribbon for a window of `blocks` blocks, `detached` more out
    /// of its layout, focused on `focus`.
    pub(crate) fn ribbon(
        &self,
        blocks: usize,
        detached: usize,
        focus: Option<u64>,
        armed: bool,
    ) -> Ribbon {
        Ribbon {
            target: self.target.clone(),
            blocks,
            detached,
            shell: focus
                .and_then(|k| self.infos.get(&k))
                .and_then(|i| i.shell.clone()),
            armed,
            link: self.link,
        }
    }

    /// How the window reaches its server now, for the ribbon.
    pub(crate) fn set_link(&mut self, link: Link) {
        self.link = link;
    }

    /// Show that the shown session, `name`, ended, or stop with `None`.
    pub(crate) fn set_ended(&mut self, name: Option<String>) {
        self.ended = name;
    }

    /// The shown session's name, once it ended while the link was down.
    pub(crate) fn ended(&self) -> Option<&str> {
        self.ended.as_deref()
    }

    /// Show `notice` while the link is up, or stop with `None` (D63).
    pub(crate) fn set_notice(&mut self, notice: Option<super::source::Notice>) {
        self.notice = notice;
    }

    /// The banner the link shows, if any (D62); while it is up, the
    /// notice, which comes back after a link banner it waited behind (D63).
    pub(crate) fn banner(&self) -> Option<Banner> {
        match self.link {
            Link::Up => self.notice.clone().map(Banner::Info),
            Link::Reconnecting { left, trying } => Some(Banner::Reconnecting { left, trying }),
            Link::Back { kept } => Some(Banner::Back { kept }),
            Link::Down { trying } => Some(Banner::Down { trying }),
        }
    }

    /// The link is down: blocks dim and take no input (D62).
    pub(crate) fn offline(&self) -> bool {
        matches!(self.link, Link::Reconnecting { .. } | Link::Down { .. })
    }

    /// The blocks whose program runs (D48's running rule), lit in the
    /// strip's thumbnails (D46, D60).
    pub(crate) fn running(&self) -> Vec<u64> {
        self.infos
            .iter()
            .filter(|(_, i)| matches!(lead(i), Some((Glyph::Spinner, _))))
            .map(|(&k, _)| k)
            .collect()
    }

    /// Block `key`'s lead glyph, for its rail chip.
    pub(crate) fn lead(&self, key: u64) -> Option<(Glyph, Ink)> {
        self.infos.get(&key).and_then(lead)
    }

    /// Block `key`'s [`state_word`]; "Idle" when it is not known.
    pub(crate) fn state(&self, key: u64) -> &'static str {
        self.infos.get(&key).map_or("Idle", state_word)
    }

    /// Block `key`'s header, its branch read by `read` while it is fresh.
    pub(crate) fn header(&mut self, key: u64, read: fn(&Path) -> Option<String>) -> Header {
        let info = self.infos.get(&key).cloned().unwrap_or_default();
        let branch = match (&info.cwd, self.local) {
            (Some(cwd), true) => self.branch(cwd, read),
            _ => None,
        };
        let take = self.takes.get(&key).copied();
        header(&info, take, branch.as_deref(), self.home.as_deref())
    }

    /// Note where taking block `key` over stands (gui-design D64).
    pub(crate) fn set_take(&mut self, key: u64, take: Take) {
        self.takes.insert(key, take);
    }

    /// Block `key`'s program and its owner's name, for the take-over
    /// confirm (D64 2); `None` unless its owner decides who types.
    pub(crate) fn owned(&self, key: u64) -> Option<(String, String)> {
        let info = self.infos.get(&key)?;
        match &info.gate {
            Some(Gate::Owned(owner)) => Some((program(info).to_owned(), owner.clone())),
            _ => None,
        }
    }

    fn branch(&mut self, cwd: &str, read: fn(&Path) -> Option<String>) -> Option<String> {
        if let Some((branch, at)) = self.branches.get(cwd)
            && at.elapsed() < BRANCH_TTL
        {
            return branch.clone();
        }
        let branch = read(Path::new(cwd));
        self.branches
            .insert(cwd.to_owned(), (branch.clone(), Instant::now()));
        branch
    }

    /// Paint `scene` on a `width × height` body for `look`, unless the
    /// last painting already is that.
    pub(crate) fn repaint(&mut self, width: usize, height: usize, scene: Scene, look: Look) {
        let fresh = matches!(&self.painted, Some((l, sc, b))
            if *l == look && *sc == scene && (b.pixmap.width, b.pixmap.height) == (width, height));
        if !fresh {
            let body = paint(width, height, &scene, &look, &mut self.text);
            self.painted = Some((look, scene, body));
        }
    }

    /// The last painting, if any.
    pub(crate) fn painted(&self) -> Option<&Body> {
        self.painted.as_ref().map(|(_, _, b)| b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::source::Status;

    fn text_of(h: &Header) -> Vec<String> {
        h.parts
            .iter()
            .map(|p| match p {
                Part::Icon(g, _) => format!("{g:?}"),
                Part::Text { text, .. } => text.clone(),
            })
            .collect()
    }

    fn shell() -> BlockInfo {
        BlockInfo {
            cwd: Some("/home/nano/src/rusty_mill".into()),
            shell: Some("bash".into()),
            ..BlockInfo::default()
        }
    }

    #[test]
    fn a_shell_shows_its_folder_and_branch() {
        let h = header(&shell(), None, Some("main"), Some("/home/nano"));
        assert_eq!(
            text_of(&h),
            ["Folder", "~/src/rusty_mill", "Branch", "main"]
        );
        assert_eq!(h.lease, None);
    }

    #[test]
    fn a_shell_without_a_folder_or_branch_shows_its_name() {
        let info = BlockInfo {
            cwd: None,
            ..shell()
        };
        assert_eq!(text_of(&header(&info, None, None, None)), ["bash"]);
    }

    #[test]
    fn a_foreground_program_is_named_with_a_spinner() {
        let info = BlockInfo {
            foreground: Some("cargo".into()),
            ..shell()
        };
        assert_eq!(
            text_of(&header(&info, None, Some("main"), None)),
            ["Spinner", "cargo"]
        );
    }

    #[test]
    fn a_status_says_what_the_program_does() {
        let with = |state, msg: Option<&str>| {
            let info = BlockInfo {
                foreground: Some("claude".into()),
                status: Some(Status {
                    state,
                    msg: msg.map(str::to_owned),
                }),
                ..shell()
            };
            text_of(&header(&info, None, None, None))
        };
        assert_eq!(
            with(State::Working, Some("Editing zoom.rs")),
            ["Spinner", "claude", "Editing zoom.rs"]
        );
        assert_eq!(with(State::Working, None), ["Spinner", "claude", "Working"]);
        assert_eq!(
            with(State::Blocked(Some(Wait::Permission)), None),
            ["Pause", "claude", "Needs permission"]
        );
        assert_eq!(
            with(State::Blocked(None), None),
            ["Pause", "claude", "Waiting for you"]
        );
        assert_eq!(with(State::Done, None), ["Check", "claude", "Done"]);
        assert_eq!(with(State::Idle, Some("ignored")), ["claude", "Idle"]);
        assert_eq!(
            with(State::Error, None),
            ["Cross", "claude", "Stopped: error"]
        );
    }

    #[test]
    fn a_status_without_a_foreground_names_the_shell() {
        let info = BlockInfo {
            status: Some(Status {
                state: State::Done,
                msg: None,
            }),
            ..shell()
        };
        assert_eq!(
            text_of(&header(&info, None, None, None)),
            ["Check", "bash", "Done"]
        );
    }

    #[test]
    fn another_holder_is_named() {
        let info = BlockInfo {
            holder: Some("laptop".into()),
            ..shell()
        };
        assert_eq!(
            header(&info, None, None, None)
                .lease
                .map(|l| l.text)
                .as_deref(),
            Some("laptop has input")
        );
    }

    fn agent(gate: Gate) -> BlockInfo {
        BlockInfo {
            foreground: Some("claude".into()),
            gate: Some(gate),
            ..shell()
        }
    }

    fn line(info: &BlockInfo, take: Option<Take>) -> (Option<(String, bool)>, Option<TakeButton>) {
        let h = header(info, take, None, None);
        (h.lease.map(|l| (l.text, l.alert)), h.take)
    }

    #[test]
    fn an_owner_only_header_says_where_taking_it_over_stands() {
        let info = agent(Gate::Owned("orch".into()));
        let said = |t: &str| Some((t.to_owned(), false));
        let alert = |t: &str| Some((t.to_owned(), true));
        use TakeButton::*;
        assert_eq!(
            line(&info, None),
            (None, None),
            "no button until the source can ask"
        );
        assert_eq!(
            line(&info, Some(Take::Rest)),
            (said("Agent has input"), Some(TakeOver))
        );
        let owner_types = BlockInfo {
            holder: Some("orch".into()),
            ..info.clone()
        };
        assert_eq!(
            line(&owner_types, Some(Take::Rest)).0,
            said("Agent has input")
        );
        let other = BlockInfo {
            holder: Some("laptop".into()),
            ..info.clone()
        };
        assert_eq!(line(&other, Some(Take::Rest)).0, said("laptop has input"));
        assert_eq!(
            line(&info, Some(Take::Asking)),
            (said("Asking orch\u{2026}"), Some(CancelAsk))
        );
        assert_eq!(
            line(&info, Some(Take::Held)),
            (said("You have input"), Some(GiveBack))
        );
        assert_eq!(
            line(&info, Some(Take::Refused(Refusal::Unreachable))),
            (alert("Can't reach orch right now"), Some(TakeOver))
        );
        assert_eq!(
            line(&info, Some(Take::Refused(Refusal::Other))),
            (alert("Couldn't ask orch"), Some(TakeOver))
        );
        assert_eq!(
            line(&info, Some(Take::Ended)),
            (said("Your input from orch ended"), Some(TakeOver))
        );
        assert_eq!(
            line(&agent(Gate::Orphaned), Some(Take::Rest)),
            (alert("claude has no owner now"), None)
        );
    }

    #[test]
    fn home_is_shortened_only_at_a_folder_boundary() {
        assert_eq!(tilde("/home/nano", Some("/home/nano")), "~");
        assert_eq!(tilde("/home/nanoX/a", Some("/home/nano")), "/home/nanoX/a");
        assert_eq!(tilde("C:\\Users\\n\\a", Some("C:\\Users\\n")), "~\\a");
    }

    #[test]
    fn the_grid_sits_under_the_header_inside_the_padding() {
        let g = grid_area(Cells::new(8, 52, 400, 300), 1.0);
        assert_eq!(g, Cells::new(25, 52 + 52, 400 - 34, 300 - 52 - 13));
        let g2 = grid_area(Cells::new(16, 104, 800, 600), 2.0);
        assert_eq!((g2.col, g2.row), (16 + 34, 104 + 104));
    }

    #[test]
    fn the_area_is_inset_but_at_the_top() {
        assert_eq!(area(1200, 800, 52, 0, 1.0), Cells::new(8, 52, 1184, 740));
        assert_eq!(area(10, 10, 52, 0, 1.0).rows, 0, "never negative");
    }

    fn look() -> Look {
        let chrome = Chrome::from_theme(&crate::core::Theme::default());
        Look {
            chrome,
            bg: 0x000000,
            scale: 1.0,
            hover: None,
            pressed: None,
        }
    }

    fn block(key: u64, rect: Cells) -> Shown {
        Shown {
            key,
            rect,
            focused: false,
            zoomed: false,
            header: header(&shell(), None, Some("main"), None),
            badge: None,
        }
    }

    fn scene(blocks: Vec<Shown>) -> Scene {
        Scene {
            blocks,
            ..Scene::default()
        }
    }

    #[test]
    fn a_block_is_framed_headed_and_sits_on_the_frame() {
        let look = look();
        let c = look.chrome;
        let mut font = UiText::embedded().expect("the embedded faces parse");
        let body = paint(
            320,
            250,
            &scene(vec![block(1, Cells::new(8, 0, 300, 200))]),
            &look,
            &mut font,
        );
        let p = &body.pixmap;
        let at = |x: usize, y: usize| p.pixels()[y * p.width + x];
        assert_eq!(at(2, 100), c.frame, "the gap");
        assert_eq!(at(8, 100), c.line, "the border");
        assert_eq!(at(150, 5), c.base, "the header");
        assert_eq!(at(150, 39), c.line, "the hairline");
        assert_eq!(at(150, 120), look.bg, "the grid's place");
        assert_eq!(at(8, 0), c.frame, "a rounded corner");
        assert_eq!(at(150, 249), c.sunk, "the ribbon along the bottom");
    }

    #[test]
    fn a_header_has_a_zoom_button_and_a_zoomed_one_exit_zoom() {
        let look = look();
        let mut font = UiText::embedded().expect("the embedded faces parse");
        let r = Cells::new(8, 0, 600, 200);
        let rest = paint(620, 250, &scene(vec![block(7, r)]), &look, &mut font);
        let zoom = rest.spots.iter().find(|(_, s)| *s == Spot::Zoom(7));
        let (button, _) = zoom.expect("the zoom button is a spot");
        assert_eq!((button.w, button.h), ZOOM_BTN);
        assert_eq!(button.x + button.w, 8.0 + 600.0 - BORDER - HEADER_PAD.1);
        let zoomed = Shown {
            zoomed: true,
            ..block(7, r)
        };
        let z = paint(620, 250, &scene(vec![zoomed]), &look, &mut font);
        assert_eq!(z.spots.len(), 1);
        assert_eq!(z.spots[0].1, Spot::Unzoom);
        let (x, y) = (z.spots[0].0.x + 2.0, z.spots[0].0.y + 2.0);
        assert_eq!(z.spot(x, y), Some(Spot::Unzoom));
    }

    #[test]
    fn a_take_over_button_sits_before_the_zoom_button() {
        let look = look();
        let mut font = UiText::embedded().expect("the embedded faces parse");
        let shown = Shown {
            header: header(
                &agent(Gate::Owned("orch".into())),
                Some(Take::Rest),
                None,
                None,
            ),
            ..block(7, Cells::new(8, 0, 600, 200))
        };
        let body = paint(620, 250, &scene(vec![shown]), &look, &mut font);
        let at = |spot: Spot| body.spots.iter().find(|(_, s)| *s == spot).map(|(r, _)| *r);
        let zoom = at(Spot::Zoom(7)).expect("the zoom button");
        let take = at(Spot::Take(7, TakeButton::TakeOver)).expect("Take over");
        assert_eq!(take.h, EXIT.0, "a small button");
        assert!(
            (zoom.x - ITEM_GAP - (take.x + take.w)).abs() < 0.01,
            "8 px before zoom"
        );
    }

    #[test]
    fn hovering_the_zoom_button_changes_only_its_pixels() {
        let mut font = UiText::embedded().expect("the embedded faces parse");
        let s = scene(vec![block(7, Cells::new(8, 0, 600, 200))]);
        let rest = paint(620, 250, &s, &look(), &mut font);
        let hovered = Look {
            hover: Some(Spot::Zoom(7)),
            ..look()
        };
        let hover = paint(620, 250, &s, &hovered, &mut font);
        assert_eq!(rest.spots, hover.spots);
        let (r, _) = rest.spots[0];
        let (px, py) = ((r.x + 2.0) as usize, (r.y + 2.0) as usize);
        let at = |b: &Body| b.pixmap.pixels()[py * 620 + px];
        assert_eq!(at(&rest), look().chrome.raised);
        assert_eq!(at(&hover), look().chrome.hover);
    }

    #[test]
    fn an_armed_prefix_badges_the_focused_block_in_the_accent() {
        let look = look();
        let c = look.chrome;
        let mut font = UiText::embedded().expect("the embedded faces parse");
        let focused = Shown {
            focused: true,
            badge: Some(1),
            ..block(1, Cells::new(8, 0, 300, 239))
        };
        let other = Shown {
            badge: Some(2),
            ..block(2, Cells::new(316, 0, 300, 239))
        };
        let body = paint(624, 290, &scene(vec![focused, other]), &look, &mut font);
        // Each badge sits over its grid, centred under the header: 40..239.
        let places: Vec<_> = body.over.iter().map(|(_, at)| *at).collect();
        assert_eq!(
            places,
            [(8 + 150 - 16, 139 - 16), (316 + 150 - 16, 139 - 16)]
        );
        let left_of_digit = |p: &Pixmap| p.pixels()[16 * p.width + 4];
        assert_eq!(left_of_digit(&body.over[0].0), c.accent);
        assert_eq!(left_of_digit(&body.over[1].0), c.raised);
        assert_eq!(
            body.over[0].0.pixels()[0],
            look.bg,
            "a corner shows the grid's ground"
        );
    }

    #[test]
    fn a_zoomed_window_leaves_a_rail_for_the_others() {
        let area = Cells::new(8, 52, 1000, 600);
        assert_eq!(zoomed(area, 0, 1.0), (area, None));
        let (block, rail) = zoomed(area, 2, 1.0);
        assert_eq!(block, Cells::new(8, 52, 1000 - 44, 600));
        let rail = rail.expect("a rail beside the block");
        assert_eq!(rail, Cells::new(8 + 1000 - 36, 52, 36, 600));
        let chips = chips(rail, 2, 1.0);
        assert_eq!(chips[1], Cells::new(rail.col, 52 + 64, 36, 56));
        assert_eq!(
            super::chips(Cells::new(0, 0, 36, 100), 5, 1.0).len(),
            1,
            "as many as fit"
        );
    }

    #[test]
    fn a_chip_is_a_spot_that_focuses_its_block() {
        let mut font = UiText::embedded().expect("the embedded faces parse");
        let s = Scene {
            rail: vec![Chip {
                key: 4,
                rect: Cells::new(500, 0, 36, 56),
                lead: Some((Glyph::Check, Ink::Success)),
                number: 2,
            }],
            ..Scene::default()
        };
        let body = paint(560, 120, &s, &look(), &mut font);
        assert_eq!(body.spot(510.0, 20.0), Some(Spot::Chip(4)));
        let check = body
            .pixmap
            .pixels()
            .iter()
            .filter(|&&p| p == look().chrome.success);
        assert!(check.count() > 0, "the chip shows its block's glyph");
    }

    #[test]
    fn the_lead_glyph_follows_the_status() {
        assert_eq!(lead(&shell()), None, "a shell at its prompt");
        let running = BlockInfo {
            foreground: Some("cargo".into()),
            ..shell()
        };
        assert_eq!(lead(&running), Some((Glyph::Spinner, Ink::Accent)));
        let failed = BlockInfo {
            status: Some(Status {
                state: State::Error,
                msg: None,
            }),
            ..shell()
        };
        assert_eq!(lead(&failed), Some((Glyph::Cross, Ink::Failure)));
    }

    #[test]
    fn the_focused_border_keeps_three_to_one() {
        let c = look().chrome;
        assert!(contrast_ratio(focus_border(&c), c.frame) >= FOCUS_MIN);
    }

    #[test]
    fn long_texts_are_cut_last_first_and_fit() {
        let font = UiText::embedded().expect("the embedded faces parse");
        let parts = header(
            &BlockInfo {
                cwd: Some("/a/rather/long/path/to/a/project/folder".into()),
                ..shell()
            },
            None,
            Some("feature/a-very-long-branch-name"),
            None,
        )
        .parts;
        let texts = fit(&parts, 260.0, 1.0, &font);
        assert!(
            texts[1].starts_with('…'),
            "the folder keeps its end: {}",
            texts[1]
        );
        assert!(
            texts[3].ends_with('…'),
            "the branch keeps its start: {}",
            texts[3]
        );
        let used: f32 = texts
            .iter()
            .zip(&parts)
            .map(|(t, p)| match p {
                Part::Icon(..) => ICON,
                Part::Text { weight, .. } => font.width(t, *weight, TEXT_PX),
            })
            .sum::<f32>()
            + 3.0 * ITEM_GAP;
        assert!(used <= 260.0, "{used}");
    }

    #[test]
    fn with_every_block_detached_the_body_says_so_in_the_middle() {
        let look = look();
        let mut font = UiText::embedded().expect("the embedded faces parse");
        let bare = paint(600, 300, &Scene::default(), &look, &mut font);
        let empty = Scene {
            empty: Some(2),
            ..Scene::default()
        };
        let said = paint(600, 300, &empty, &look, &mut font);
        let text = look.chrome.text;
        let lit = |b: &Body| b.pixmap.pixels().iter().filter(|&&p| p == text).count();
        assert!(lit(&said) > lit(&bare), "the title is drawn in bone");
        // Centred: as much ink left of the middle as right of it.
        let p = &said.pixmap;
        let (mut left, mut right) = (0usize, 0usize);
        for (i, &px) in p.pixels().iter().enumerate() {
            if px == text && (i / p.width) < 200 {
                if i % p.width < p.width / 2 {
                    left += 1;
                } else {
                    right += 1;
                }
            }
        }
        assert!(left.abs_diff(right) * 4 < left + right, "{left} vs {right}");
    }

    #[test]
    fn a_notice_shows_while_the_link_is_up_and_waits_behind_a_link_banner() {
        let Some(mut state) = BodyState::new() else {
            return;
        };
        let notice = crate::gui::source::Notice {
            title: "vm runs an older rusty_mux".into(),
            detail: "It lacks shared focus.".into(),
        };
        state.set_notice(Some(notice.clone()));
        assert_eq!(state.banner(), Some(Banner::Info(notice.clone())));
        state.set_link(Link::Down { trying: false });
        assert_eq!(state.banner(), Some(Banner::Down { trying: false }));
        state.set_link(Link::Up);
        assert_eq!(state.banner(), Some(Banner::Info(notice)));
        state.set_notice(None);
        assert_eq!(state.banner(), None);
    }

    #[test]
    fn a_down_link_dims_the_blocks_and_only_the_banner_takes_clicks() {
        let look = look();
        let mut font = UiText::embedded().expect("the embedded faces parse");
        let mut b = block(1, Cells::new(8, 0, 1000, 300));
        b.zoomed = true;
        let up = paint(1100, 350, &scene(vec![b.clone()]), &look, &mut font);
        assert!(!up.spots.is_empty(), "the zoom button");
        let down = Scene {
            banner: Some(Banner::Down { trying: false }),
            dim: true,
            ..scene(vec![b])
        };
        let down = paint(1100, 350, &down, &look, &mut font);
        let spots: Vec<Spot> = down.spots.iter().map(|&(_, s)| s).collect();
        assert_eq!(spots, [Spot::Banner(Action::Retry)]);
        let (veil, banner) = (&down.over[0].0, &down.over[1].0);
        assert_eq!((veil.opacity, veil.width), (DIM_VEIL, 1100));
        assert!(veil.height < 350, "the ribbon stays bright");
        assert_eq!(banner.height, banner::height(1.0));
    }

    #[test]
    fn an_ended_session_fills_the_body_with_its_panel_and_open_switcher() {
        let look = look();
        let mut font = UiText::embedded().expect("the embedded faces parse");
        let ended = Scene {
            ended: Some("rusty_mill".into()),
            ..Scene::default()
        };
        let body = paint(900, 500, &ended, &look, &mut font);
        let spots: Vec<Spot> = body.spots.iter().map(|&(_, s)| s).collect();
        assert_eq!(spots, [Spot::OpenSwitcher]);
        let (button, _) = body.spots[0];
        assert_eq!(button.h, 28.0);
        let p = &body.pixmap;
        let at = |x: f32, y: f32| p.pixels()[y as usize * p.width + x as usize];
        assert_eq!(at(button.x + 3.0, button.y + 14.0), look.chrome.accent);
        assert_eq!(at(100.0, 100.0), look.chrome.base, "inside the block");
        assert_eq!(at(2.0, 100.0), look.chrome.frame, "the gap");
        let failure = p
            .pixels()
            .iter()
            .filter(|&&px| px == look.chrome.failure)
            .count();
        assert!(failure > 0, "the alert");
    }

    #[test]
    fn the_panel_text_wraps_at_its_width() {
        let font = UiText::embedded().expect("the embedded faces parse");
        let lines = wrap(&font, ENDED_TEXT, 12.0, 420.0);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!(lines.join(" "), ENDED_TEXT);
        assert!(
            lines
                .iter()
                .all(|l| font.width(l, Weight::Regular, 12.0) <= 420.0)
        );
    }
}
