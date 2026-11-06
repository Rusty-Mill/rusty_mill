//! An attached window's blocks (gui-design F1; geometry D47, header D48):
//! the server's split tree laid out in pixels with gaps of the frame colour,
//! each block framed and headed by its folder and branch, or by its program
//! and status, with who else has input on the right.
//!
//! Pure: [`grid_area`] says where a block's terminal sits, [`header`] what
//! its header says, and [`paint`] draws every frame and header into one
//! pixmap under the strip, which both renderers present behind the grids.
//! Sizes are the pack's logical px times the DPI scale, never the terminal
//! zoom.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use super::chrome::Chrome;
use super::layout::Rect as Cells;
use super::paint::{Pixmap, Rect, icons};
use super::render::blend;
use super::source::{BlockInfo, State, Wait};
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

/// What a block's header says (D48): its left group, and the lease text on
/// the right when another client has input.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Header {
    pub parts: Vec<Part>,
    pub lease: Option<String>,
}

fn text(text: impl Into<String>, weight: Weight, ink: Ink, keep: Keep) -> Part {
    Part::Text {
        text: text.into(),
        weight,
        ink,
        keep,
    }
}

/// The header for `info` (D48). `branch` is the git branch of its folder,
/// when known; `home` shortens the folder to "~".
pub(crate) fn header(info: &BlockInfo, branch: Option<&str>, home: Option<&str>) -> Header {
    let lease = info.holder.as_ref().map(|h| format!("{h} has input"));
    let parts = if info.foreground.is_none() && info.status.is_none() {
        shell_parts(info, branch, home)
    } else {
        program_parts(info)
    };
    Header { parts, lease }
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

/// The program's status glyph, its name and its status text.
fn program_parts(info: &BlockInfo) -> Vec<Part> {
    let name = info
        .foreground
        .as_deref()
        .or(info.shell.as_deref())
        .unwrap_or("program");
    let msg = info.status.as_ref().and_then(|s| s.msg.clone());
    let (lead, said) = match info.status.as_ref().map(|s| s.state) {
        None => (Some((Glyph::Spinner, Ink::Accent)), None),
        Some(State::Working) => (
            Some((Glyph::Spinner, Ink::Accent)),
            Some((msg.unwrap_or_else(|| "Working".into()), Ink::Muted)),
        ),
        Some(State::Blocked(wait)) => (
            Some((Glyph::Pause, Ink::Muted)),
            Some((msg.unwrap_or_else(|| waiting(wait).into()), Ink::Text)),
        ),
        Some(State::Done) => (
            Some((Glyph::Check, Ink::Success)),
            Some((msg.unwrap_or_else(|| "Done".into()), Ink::Muted)),
        ),
        Some(State::Idle) => (None, Some(("Idle".into(), Ink::Muted))),
        Some(State::Error) => (
            Some((Glyph::Cross, Ink::Failure)),
            Some((msg.unwrap_or_else(|| "Stopped: error".into()), Ink::Failure)),
        ),
    };
    let mut parts: Vec<Part> = lead.map(|(g, i)| Part::Icon(g, i)).into_iter().collect();
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

/// How to draw the blocks: colours and DPI scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Look {
    pub chrome: Chrome,
    /// The terminal background, which the grids sit on (D47).
    pub bg: u32,
    pub scale: f32,
}

/// A block to draw: its box in the body's px, whether it has the focus,
/// and its header.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Shown {
    pub rect: Cells,
    pub focused: bool,
    pub header: Header,
}

/// The body under the strip, `width × height` px: the frame colour, and
/// each of `blocks` framed and headed.
pub(crate) fn paint(
    width: usize,
    height: usize,
    blocks: &[Shown],
    look: &Look,
    font: &mut UiText,
) -> Pixmap {
    let mut out = Pixmap::new(width, height, look.chrome.frame);
    for block in blocks {
        paint_block(&mut out, block, look, font);
    }
    out
}

fn paint_block(out: &mut Pixmap, block: &Shown, look: &Look, font: &mut UiText) {
    let (s, c) = (look.scale, &look.chrome);
    let r = Rect {
        x: block.rect.col as f32,
        y: block.rect.row as f32,
        w: block.rect.cols as f32,
        h: block.rect.rows as f32,
    };
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
    paint_header(out, head, &block.header, look, font);
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

fn ink(c: &Chrome, ink: Ink) -> u32 {
    match ink {
        Ink::Text => c.text,
        Ink::Muted => c.muted,
        Ink::Accent => c.accent,
        Ink::Success => c.success,
        Ink::Failure => c.failure,
    }
}

fn paint_header(out: &mut Pixmap, head: Rect, header: &Header, look: &Look, font: &mut UiText) {
    let (s, c) = (look.scale, &look.chrome);
    let border = BORDER * s;
    let mid = head.y + border + HEADER * s / 2.0;
    let icon_y = mid - ICON * s / 2.0;
    let base = mid + font.centre_drop(TEXT_PX * s);
    let small_base = mid + font.centre_drop(SMALL_PX * s);
    let left = head.x + border + HEADER_PAD.0 * s;
    let mut right = head.x + head.w - border - HEADER_PAD.1 * s;
    // The right group never shrinks: the lease text, the eye before it.
    if let Some(lease) = &header.lease {
        let w = font.width(lease, Weight::Regular, SMALL_PX * s);
        right -= w;
        out.text(
            font,
            lease,
            Weight::Regular,
            SMALL_PX * s,
            right,
            small_base,
            c.muted,
        );
        right -= (EYE_GAP + ICON) * s;
        out.icon(
            icons::EYE,
            right,
            icon_y,
            ICON * s,
            ICON_STROKE * s,
            c.muted,
        );
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

fn paint_glyph(out: &mut Pixmap, glyph: Glyph, x: f32, y: f32, s: f32, color: u32, ring: u32) {
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
    /// The session runs on this machine, so its folders' branches can be
    /// read here.
    local: bool,
    home: Option<String>,
    branches: HashMap<String, (Option<String>, Instant)>,
    text: UiText,
    painted: Option<(Look, Vec<Shown>, Pixmap)>,
}

impl BodyState {
    /// No blocks known yet; `None` if the interface font is broken.
    pub(crate) fn new() -> Option<Self> {
        let home = std::env::var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).ok();
        Some(Self {
            infos: HashMap::new(),
            local: false,
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

    /// Whether the session runs on this machine.
    pub(crate) fn set_local(&mut self, local: bool) {
        self.local = local;
    }

    /// Block `key`'s header, its branch read by `read` while it is fresh.
    pub(crate) fn header(&mut self, key: u64, read: fn(&Path) -> Option<String>) -> Header {
        let info = self.infos.get(&key).cloned().unwrap_or_default();
        let branch = match (&info.cwd, self.local) {
            (Some(cwd), true) => self.branch(cwd, read),
            _ => None,
        };
        header(&info, branch.as_deref(), self.home.as_deref())
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

    /// Paint `blocks` on a `width × height` body for `look`, unless the
    /// last painting already is that.
    pub(crate) fn repaint(&mut self, width: usize, height: usize, blocks: Vec<Shown>, look: Look) {
        let fresh = matches!(&self.painted, Some((l, b, p))
            if *l == look && *b == blocks && (p.width, p.height) == (width, height));
        if !fresh {
            let pixmap = paint(width, height, &blocks, &look, &mut self.text);
            self.painted = Some((look, blocks, pixmap));
        }
    }

    /// The last painting, if any.
    pub(crate) fn painted(&self) -> Option<&Pixmap> {
        self.painted.as_ref().map(|(_, _, p)| p)
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
        let h = header(&shell(), Some("main"), Some("/home/nano"));
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
        assert_eq!(text_of(&header(&info, None, None)), ["bash"]);
    }

    #[test]
    fn a_foreground_program_is_named_with_a_spinner() {
        let info = BlockInfo {
            foreground: Some("cargo".into()),
            ..shell()
        };
        assert_eq!(
            text_of(&header(&info, Some("main"), None)),
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
            text_of(&header(&info, None, None))
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
            text_of(&header(&info, None, None)),
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
            header(&info, None, None).lease.as_deref(),
            Some("laptop has input")
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
        }
    }

    #[test]
    fn a_block_is_framed_headed_and_sits_on_the_frame() {
        let look = look();
        let c = look.chrome;
        let mut font = UiText::embedded().expect("the embedded faces parse");
        let block = Shown {
            rect: Cells::new(8, 0, 300, 200),
            focused: false,
            header: header(&shell(), Some("main"), None),
        };
        let p = paint(320, 210, &[block], &look, &mut font);
        let at = |x: usize, y: usize| p.pixels()[y * p.width + x];
        assert_eq!(at(2, 100), c.frame, "the gap");
        assert_eq!(at(8, 100), c.line, "the border");
        assert_eq!(at(200, 5), c.base, "the header");
        assert_eq!(at(200, 39), c.line, "the hairline");
        assert_eq!(at(200, 120), look.bg, "the grid's place");
        assert_eq!(at(8, 0), c.frame, "a rounded corner");
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
}
