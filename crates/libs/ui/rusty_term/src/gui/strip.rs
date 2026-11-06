//! An attached window's top strip (gui-design F1; geometry D46, scope D45):
//! the session chip, a tab per server window with its layout thumbnail, a
//! "+" for a new window, the detached-blocks chip (D61), the
//! attached-clients count and the caption buttons.
//!
//! Pure: [`paint`] turns a [`Session`] and a [`Look`] into a [`Strip`], the
//! pixels both renderers present plus the spots a click can hit. Sizes are
//! the pack's logical px times the DPI scale, never the terminal zoom.

use super::chrome::Chrome;
use super::layout::Dir;
use super::paint::{Pixmap, Rect, icons};
use super::source::{Session, Tree, WindowTab};
use super::text::{UiText, Weight};

/// Strip height, logical px.
const HEIGHT: f32 = 52.0;
/// Left padding before the session chip.
const LEFT: f32 = 14.0;
/// Chips' height and corner radius.
const CHIP_H: f32 = 34.0;
const CHIP_R: f32 = 7.0;
/// Gap between the parts of a chip, and the session chip's side padding.
const GAP: f32 = 8.0;
const ICON: f32 = 16.0;
const ICON_STROKE: f32 = 1.5;
/// Space each side of the divider after the session chip; its height.
const DIVIDER_SPACE: f32 = 16.0;
const DIVIDER_H: f32 = 24.0;
/// Tab padding (left, right), thumbnail-to-label gap and gap between tabs.
const TAB_PAD: (f32, f32) = (12.0, 14.0);
const THUMB_GAP: f32 = 10.0;
const TAB_GAP: f32 = 6.0;
/// The widest a tab's label gets before it is cut short (D45 d).
const LABEL_MAX: f32 = 160.0;
const THUMB: (f32, f32) = (26.0, 16.0);
/// The "+" button's box, its radius and where its icon sits inside.
const ADD_BOX: f32 = 24.0;
const ADD_R: f32 = 5.0;
/// Caption buttons' width and the space before them.
const CAPTION_W: f32 = 46.0;
const CAPTION_SPACE: f32 = 16.0;
/// Text sizes: chips and tabs, and the clients line.
const TEXT_PX: f32 = 13.0;
const SMALL_PX: f32 = 12.0;
/// The caption close button's hover, native red as on Windows 11.
const CLOSE_HOVER: (u32, u32) = (0xFFFFFF, 0xC42B1C);
/// The detached chip's height and side padding (D61).
const DETACHED_H: f32 = 28.0;
const DETACHED_PAD: f32 = 12.0;

/// What a spot on the strip does when clicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Spot {
    /// The session chip: its menu (D45 b).
    Session,
    /// Show window `key`.
    Tab(u64),
    /// A new window.
    NewWindow,
    /// The detached chip: its popover (D61).
    Detached,
    Minimize,
    Maximize,
    Close,
}

/// How to draw the strip: colours, DPI scale and pointer state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Look {
    pub chrome: Chrome,
    pub scale: f32,
    pub hover: Option<Spot>,
    pub pressed: Option<Spot>,
    /// Whether the detached popover is open (D61: raised, accent border).
    pub detached_open: bool,
}

/// A painted strip: its pixels and the spots on them. A click anywhere
/// else on it drags the window.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Strip {
    pub pixmap: Pixmap,
    pub spots: Vec<(Rect, Spot)>,
}

impl Strip {
    /// The spot under `(x, y)`, if any.
    pub(crate) fn spot(&self, x: f32, y: f32) -> Option<Spot> {
        self.spots
            .iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|&(_, s)| s)
    }
}

/// An attached window's strip: the session it shows and its last
/// painting, kept until the session, the look or the width changes.
pub(crate) struct StripState {
    session: Session,
    /// Blocks whose program runs, lit in their thumbnails (D46, D60).
    running: Vec<u64>,
    text: UiText,
    painted: Option<(Look, Strip)>,
}

impl StripState {
    /// A strip showing nothing yet; `None` if the interface font is broken.
    pub(crate) fn new() -> Option<Self> {
        Some(Self {
            session: Session::default(),
            running: Vec::new(),
            text: UiText::embedded()?,
            painted: None,
        })
    }

    /// Show `session` from the next painting on.
    pub(crate) fn show(&mut self, session: Session) {
        if session != self.session {
            self.session = session;
            self.painted = None;
        }
    }

    /// The session shown.
    pub(crate) fn session(&self) -> &Session {
        &self.session
    }

    /// Light `running` blocks' thumbnail cells from the next painting on.
    pub(crate) fn set_running(&mut self, mut running: Vec<u64>) {
        running.sort_unstable();
        if running != self.running {
            self.running = running;
            self.painted = None;
        }
    }

    /// Paint the strip for `look` across `width` px, unless the last
    /// painting already is that.
    pub(crate) fn repaint(&mut self, look: Look, width: usize) {
        let fresh = matches!(&self.painted, Some((l, s)) if *l == look && s.pixmap.width == width);
        if !fresh {
            let strip = paint(&self.session, &self.running, &look, width, &mut self.text);
            self.painted = Some((look, strip));
        }
    }

    /// The last painting, if any.
    pub(crate) fn painted(&self) -> Option<&Strip> {
        self.painted.as_ref().map(|(_, s)| s)
    }
}

/// The strip's height in px at `scale`.
pub(crate) fn height(scale: f32) -> usize {
    (HEIGHT * scale).round() as usize
}

/// One tab's label and its place.
struct TabPlan<'a> {
    tab: &'a WindowTab,
    label: String,
    weight: Weight,
    w: f32,
}

/// Paint `session` across a strip `width` px wide, the `running` blocks
/// lit in their thumbnails.
pub(crate) fn paint(
    session: &Session,
    running: &[u64],
    look: &Look,
    width: usize,
    font: &mut UiText,
) -> Strip {
    let s = look.scale;
    let c = &look.chrome;
    let mut p = Pixmap::new(width, height(s), c.frame);
    let mut spots = Vec::new();
    let mid = HEIGHT * s / 2.0;
    let base = mid + font.centre_drop(TEXT_PX * s);
    let chip = |x: f32, w: f32| Rect {
        x,
        y: mid - CHIP_H * s / 2.0,
        w,
        h: CHIP_H * s,
    };

    // The right group first: what the tabs must leave room for.
    let caption_x = width as f32 - 3.0 * CAPTION_W * s;
    caption(&mut p, &mut spots, look, caption_x);
    let clients = clients_label(session.clients);
    let clients_w = font.width(&clients, Weight::Regular, SMALL_PX * s);
    let text_x = caption_x - CAPTION_SPACE * s - clients_w;
    let small_base = mid + font.centre_drop(SMALL_PX * s);
    p.text(
        font,
        &clients,
        Weight::Regular,
        SMALL_PX * s,
        text_x,
        small_base,
        c.muted,
    );
    let icon_x = text_x - (GAP + ICON) * s;
    p.icon(
        icons::CLIENTS,
        icon_x,
        mid - ICON * s / 2.0,
        ICON * s,
        ICON_STROKE * s,
        c.muted,
    );
    let mut right = icon_x - CAPTION_SPACE * s;
    if !session.detached.is_empty() {
        let r = detached_chip(&mut p, look, font, session.detached.len(), right);
        spots.push((r, Spot::Detached));
        right = r.x - CAPTION_SPACE * s;
    }

    // The session chip.
    let mut x = LEFT * s;
    let host = format!("on {}", session.host);
    let name_w = font.width(&session.name, Weight::Medium, TEXT_PX * s);
    let host_w = font.width(&host, Weight::Regular, TEXT_PX * s);
    let chip_w = (GAP + ICON + GAP) * s + name_w + GAP * s + host_w + (GAP + ICON + GAP) * s;
    let r = chip(x, chip_w);
    surface(&mut p, look, Spot::Session, r, None);
    spots.push((r, Spot::Session));
    let icon_y = mid - ICON * s / 2.0;
    x += GAP * s;
    p.icon(icons::SERVER, x, icon_y, ICON * s, ICON_STROKE * s, c.muted);
    x += (ICON + GAP) * s;
    x += p.text(
        font,
        &session.name,
        Weight::Medium,
        TEXT_PX * s,
        x,
        base,
        c.text,
    );
    x += GAP * s;
    x += p.text(font, &host, Weight::Regular, TEXT_PX * s, x, base, c.muted);
    x += GAP * s;
    p.icon(
        icons::CHEVRON_DOWN,
        x,
        icon_y,
        ICON * s,
        ICON_STROKE * s,
        c.muted,
    );
    x = r.x + r.w + DIVIDER_SPACE * s;
    p.fill(
        Rect {
            x: x.round(),
            y: mid - DIVIDER_H * s / 2.0,
            w: s.max(1.0).round(),
            h: DIVIDER_H * s,
        },
        0.0,
        c.line,
    );
    x += DIVIDER_SPACE * s + 1.0;

    // The tabs, then "+".
    let room = right - x - (TAB_GAP + ADD_BOX) * s;
    for plan in plan_tabs(&session.windows, font, s, room) {
        if x + plan.w > right {
            break;
        }
        let r = chip(x, plan.w);
        let selected = plan.tab.shown;
        let spot = Spot::Tab(plan.tab.key);
        surface(&mut p, look, spot, r, selected.then_some(c.base));
        if selected {
            p.border(r, CHIP_R * s, 1.0, c.line);
        }
        spots.push((r, spot));
        let tx = x + TAB_PAD.0 * s;
        // On a lit tab the thumbnail takes the selected fill, which holds
        // against `raised` and `line` as the unselected one can't.
        let lit = selected || look.hover == Some(spot) || look.pressed == Some(spot);
        let fill = |key: u64| {
            if running.contains(&key) {
                c.accent
            } else {
                c.thumb[usize::from(lit)]
            }
        };
        thumbnail(&mut p, plan.tab, fill, tx, mid - THUMB.1 * s / 2.0, s);
        let fg = if selected || look.pressed == Some(spot) {
            c.text
        } else {
            c.muted
        };
        let lx = tx + (THUMB.0 + THUMB_GAP) * s;
        p.text(font, &plan.label, plan.weight, TEXT_PX * s, lx, base, fg);
        x += plan.w + TAB_GAP * s;
    }
    if x + ADD_BOX * s <= right {
        let r = Rect {
            x,
            y: mid - ADD_BOX * s / 2.0,
            w: ADD_BOX * s,
            h: ADD_BOX * s,
        };
        let fg = if look.hover == Some(Spot::NewWindow) || look.pressed == Some(Spot::NewWindow) {
            c.text
        } else {
            c.control
        };
        surface_r(&mut p, look, Spot::NewWindow, r, ADD_R * s, None);
        let inset = (ADD_BOX - ICON) * s / 2.0;
        p.icon(
            icons::PLUS,
            r.x + inset,
            r.y + inset,
            ICON * s,
            ICON_STROKE * s,
            fg,
        );
        spots.push((r, Spot::NewWindow));
    }
    Strip { pixmap: p, spots }
}

/// The "{n} detached" chip ending at `right` (D61): a zoom icon, the
/// count and a chevron; raised with an accent border while its popover
/// is open.
fn detached_chip(p: &mut Pixmap, look: &Look, font: &mut UiText, n: usize, right: f32) -> Rect {
    let s = look.scale;
    let c = &look.chrome;
    let mid = HEIGHT * s / 2.0;
    let label = format!("{n} detached");
    let text_w = font.width(&label, Weight::Medium, SMALL_PX * s);
    let w = (DETACHED_PAD + ICON + GAP) * s + text_w + (GAP + ICON + DETACHED_PAD) * s;
    let r = Rect {
        x: right - w,
        y: mid - DETACHED_H * s / 2.0,
        w,
        h: DETACHED_H * s,
    };
    let lit = look.detached_open || look.hover == Some(Spot::Detached);
    let rest = if lit { c.raised } else { c.base };
    surface(p, look, Spot::Detached, r, Some(rest));
    let edge = if look.detached_open { c.accent } else { c.line };
    p.border(r, CHIP_R * s, s.max(1.0).round(), edge);
    let icon_y = mid - ICON * s / 2.0;
    let mut x = r.x + DETACHED_PAD * s;
    p.icon(icons::ZOOM, x, icon_y, ICON * s, ICON_STROKE * s, c.muted);
    x += (ICON + GAP) * s;
    let base = mid + font.centre_drop(SMALL_PX * s);
    x += p.text(font, &label, Weight::Medium, SMALL_PX * s, x, base, c.text);
    x += GAP * s;
    p.icon(
        icons::CHEVRON_DOWN,
        x,
        icon_y,
        ICON * s,
        ICON_STROKE * s,
        c.muted,
    );
    r
}

/// "{n} clients attached", singular for one.
fn clients_label(n: usize) -> String {
    match n {
        1 => "1 client attached".to_owned(),
        n => format!("{n} clients attached"),
    }
}

/// Each tab's label and width. Labels are cut at [`LABEL_MAX`]; when the
/// tabs still don't fit `room`, unselected ones keep only their number
/// (D45 d).
fn plan_tabs<'a>(tabs: &'a [WindowTab], font: &UiText, s: f32, room: f32) -> Vec<TabPlan<'a>> {
    let plan = |short: bool| -> Vec<TabPlan<'a>> {
        tabs.iter()
            .enumerate()
            .map(|(i, tab)| {
                let weight = if tab.shown {
                    Weight::Medium
                } else {
                    Weight::Regular
                };
                let label = if (short && !tab.shown) || tab.name.is_empty() {
                    format!("{}", i + 1)
                } else {
                    let full = format!("{}  {}", i + 1, tab.name);
                    font.fit(&full, weight, TEXT_PX * s, LABEL_MAX * s)
                };
                let text = font.width(&label, weight, TEXT_PX * s);
                let w = (TAB_PAD.0 + THUMB.0 + THUMB_GAP + TAB_PAD.1) * s + text;
                TabPlan {
                    tab,
                    label,
                    weight,
                    w,
                }
            })
            .collect()
    };
    let total = |p: &[TabPlan]| p.iter().map(|t| t.w + TAB_GAP * s).sum::<f32>();
    let full = plan(false);
    if total(&full) <= room {
        return full;
    }
    plan(true)
}

/// A chip's fill: pressed `line`, hover `raised`, else `rest` (D43).
fn surface(p: &mut Pixmap, look: &Look, spot: Spot, r: Rect, rest: Option<u32>) {
    surface_r(p, look, spot, r, CHIP_R * look.scale, rest);
}

fn surface_r(p: &mut Pixmap, look: &Look, spot: Spot, r: Rect, radius: f32, rest: Option<u32>) {
    let fill = if look.pressed == Some(spot) {
        Some(look.chrome.line)
    } else if look.hover == Some(spot) && rest.is_none() {
        Some(look.chrome.raised)
    } else {
        rest
    };
    if let Some(fill) = fill {
        p.fill(r, radius, fill);
    }
}

/// The minimize, maximize and close buttons from `x` to the right edge,
/// full height, with today's states (D45 a).
fn caption(p: &mut Pixmap, spots: &mut Vec<(Rect, Spot)>, look: &Look, x: f32) {
    let s = look.scale;
    let c = &look.chrome;
    let h = p.height as f32;
    let buttons = [
        (Spot::Minimize, icons::MINIMIZE),
        (Spot::Maximize, icons::MAXIMIZE),
        (Spot::Close, icons::CLOSE),
    ];
    for (i, (spot, path)) in buttons.into_iter().enumerate() {
        let last = i == buttons.len() - 1;
        let bx = x + i as f32 * CAPTION_W * s;
        // The last button runs to the edge: the corner always closes.
        let w = if last {
            p.width as f32 - bx
        } else {
            CAPTION_W * s
        };
        let r = Rect {
            x: bx,
            y: 0.0,
            w,
            h,
        };
        let (hover, pressed) = (look.hover == Some(spot), look.pressed == Some(spot));
        let (fg, bg) = match (spot, hover, pressed) {
            (Spot::Close, true, _) => (CLOSE_HOVER.0, Some(CLOSE_HOVER.1)),
            (_, _, true) => (c.text, Some(c.line)),
            (_, true, _) => (c.text, Some(c.raised)),
            _ => (c.muted, None),
        };
        if let Some(bg) = bg {
            p.fill(r, 0.0, bg);
        }
        let size = 12.0 * s;
        let ix = bx + (CAPTION_W * s - size) / 2.0;
        p.icon(path, ix, (h - size) / 2.0, size, s.max(1.0), fg);
        spots.push((r, spot));
    }
}

/// A tab's 26×16 thumbnail of its window's blocks (D10, D46): a rounded
/// cell per block with 1.5 px gaps, each `fill(key)`.
fn thumbnail(p: &mut Pixmap, tab: &WindowTab, fill: impl Fn(u64) -> u32, x: f32, y: f32, s: f32) {
    // A window with no blocks shows an empty slot.
    let Some(tree) = &tab.tree else {
        return;
    };
    for (key, (fx, fy, fw, fh)) in cells(tree) {
        let r = Rect {
            x: x + (fx * THUMB.0 + 0.5) * s,
            y: y + (fy * THUMB.1 + 0.5) * s,
            w: (fw * THUMB.0 - 1.5) * s,
            h: (fh * THUMB.1 - 1.5) * s,
        };
        p.fill(r, 1.5 * s, fill(key));
    }
}

/// A place in the unit square, as fractions `(x, y, w, h)`.
type Unit = (f32, f32, f32, f32);

/// Each block's key and place in the unit square.
fn cells(tree: &Tree) -> Vec<(u64, Unit)> {
    let mut out = Vec::new();
    place(tree, (0.0, 0.0, 1.0, 1.0), &mut out);
    out
}

fn place(tree: &Tree, at: Unit, out: &mut Vec<(u64, Unit)>) {
    let (x, y, w, h) = at;
    match tree {
        Tree::Pane(key) => out.push((*key, at)),
        Tree::Split { dir, ratio, a, b } => {
            let r = ratio.clamp(0.0, 1.0);
            let (first, second) = match dir {
                Dir::Vertical => ((x, y, w * r, h), (x + w * r, y, w * (1.0 - r), h)),
                Dir::Horizontal => ((x, y, w, h * r), (x, y + h * r, w, h * (1.0 - r))),
            };
            place(a, first, out);
            place(b, second, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Look, Session, Spot, Tree, WindowTab, cells, clients_label, height, paint};
    use crate::core::Theme;
    use crate::gui::chrome::Chrome;
    use crate::gui::layout::Dir;
    use crate::gui::source::Detached;
    use crate::gui::text::UiText;

    fn session(windows: usize) -> Session {
        Session {
            name: "rusty_mill".into(),
            host: "baileyai".into(),
            windows: (0..windows)
                .map(|i| WindowTab {
                    key: i as u64 + 10,
                    name: format!("window {i}"),
                    tree: Some(Tree::Split {
                        dir: Dir::Vertical,
                        ratio: 0.6,
                        a: Box::new(Tree::Pane(1)),
                        b: Box::new(Tree::Pane(2)),
                    }),
                    shown: i == 0,
                })
                .collect(),
            clients: 2,
            local: true,
            sessions: Vec::new(),
            detached: Vec::new(),
        }
    }

    fn look(scale: f32) -> Look {
        Look {
            chrome: Chrome::from_theme(&Theme::default()),
            scale,
            hover: None,
            pressed: None,
            detached_open: false,
        }
    }

    fn font() -> UiText {
        UiText::embedded().expect("embedded faces")
    }

    #[test]
    fn the_strip_is_52_logical_px_scaled_by_dpi() {
        assert_eq!(height(1.0), 52);
        assert_eq!(height(1.5), 78);
        let strip = paint(&session(1), &[], &look(2.0), 1200, &mut font());
        assert_eq!((strip.pixmap.width, strip.pixmap.height), (1200, 104));
    }

    #[test]
    fn every_window_gets_a_tab_in_order_after_the_session_chip() {
        let strip = paint(&session(3), &[], &look(1.0), 1400, &mut font());
        let xs: Vec<(f32, Spot)> = strip.spots.iter().map(|(r, s)| (r.x, *s)).collect();
        let order: Vec<Spot> = {
            let mut v = xs.clone();
            v.sort_by(|a, b| a.0.total_cmp(&b.0));
            v.into_iter().map(|(_, s)| s).collect()
        };
        assert_eq!(
            order,
            [
                Spot::Session,
                Spot::Tab(10),
                Spot::Tab(11),
                Spot::Tab(12),
                Spot::NewWindow,
                Spot::Minimize,
                Spot::Maximize,
                Spot::Close,
            ]
        );
    }

    #[test]
    fn the_top_right_corner_closes() {
        let strip = paint(&session(1), &[], &look(1.0), 900, &mut font());
        assert_eq!(strip.spot(899.0, 0.0), Some(Spot::Close));
        assert_eq!(strip.spot(899.0, 51.0), Some(Spot::Close));
    }

    #[test]
    fn empty_strip_is_no_spot_so_it_drags() {
        let strip = paint(&session(1), &[], &look(1.0), 1400, &mut font());
        assert_eq!(strip.spot(700.0, 26.0), None);
        assert_eq!(strip.spot(2.0, 26.0), None, "the left padding");
    }

    #[test]
    fn crowded_tabs_drop_unselected_names_then_stop_before_the_right_group() {
        let roomy = paint(&session(3), &[], &look(1.0), 1400, &mut font());
        let tab_w = |strip: &super::Strip, key| {
            strip
                .spots
                .iter()
                .find(|(_, s)| *s == Spot::Tab(key))
                .map(|(r, _)| r.w)
        };
        let tight = paint(&session(3), &[], &look(1.0), 760, &mut font());
        let (a, b) = (
            tab_w(&roomy, 11).expect("tab"),
            tab_w(&tight, 11).expect("tab"),
        );
        assert!(b < a, "unselected tab shrinks: {a} -> {b}");
        assert_eq!(
            tab_w(&tight, 10),
            tab_w(&roomy, 10),
            "the selected one keeps its name"
        );
        let crowded = paint(&session(30), &[], &look(1.0), 760, &mut font());
        let clients_x = crowded
            .spots
            .iter()
            .find(|(_, s)| *s == Spot::Minimize)
            .map(|(r, _)| r.x)
            .expect("caption");
        assert!(
            crowded
                .spots
                .iter()
                .filter(|(_, s)| matches!(s, Spot::Tab(_) | Spot::NewWindow))
                .all(|(r, _)| r.x + r.w < clients_x - 100.0),
            "tabs stop before the clients line"
        );
    }

    #[test]
    fn hover_and_press_change_only_the_pixels() {
        let mut f = font();
        let rest = paint(&session(2), &[], &look(1.0), 1200, &mut f);
        let mut hovered = look(1.0);
        hovered.hover = Some(Spot::Tab(11));
        let hover = paint(&session(2), &[], &hovered, 1200, &mut f);
        assert_eq!(rest.spots, hover.spots);
        assert_ne!(rest.pixmap, hover.pixmap);
    }

    #[test]
    fn the_selected_tab_has_the_base_surface() {
        let strip = paint(&session(2), &[], &look(1.0), 1200, &mut font());
        let (r, _) = strip
            .spots
            .iter()
            .find(|(_, s)| *s == Spot::Tab(10))
            .expect("tab");
        let c = look(1.0).chrome;
        let px = |x: f32, y: f32| strip.pixmap.pixels()[y as usize * 1200 + x as usize];
        assert_eq!(px(r.x + r.w - 4.0, r.y + 4.0), c.base);
        assert_eq!(px(r.x + r.w + 2.0, r.y + 4.0), c.frame);
    }

    #[test]
    fn thumbnails_split_as_the_tree_does() {
        let tree = Tree::Split {
            dir: Dir::Vertical,
            ratio: 0.6,
            a: Box::new(Tree::Pane(1)),
            b: Box::new(Tree::Split {
                dir: Dir::Horizontal,
                ratio: 0.5,
                a: Box::new(Tree::Pane(2)),
                b: Box::new(Tree::Pane(3)),
            }),
        };
        let got = cells(&tree);
        let keys: Vec<u64> = got.iter().map(|c| c.0).collect();
        assert_eq!(keys, [1, 2, 3]);
        let got: Vec<_> = got.into_iter().map(|c| c.1).collect();
        assert_eq!(got[0], (0.0, 0.0, 0.6, 1.0));
        assert!((got[1].0 - 0.6).abs() < 1e-6 && got[1].3 == 0.5);
        assert!((got[2].1 - 0.5).abs() < 1e-6);
    }

    #[test]
    fn one_client_is_singular() {
        assert_eq!(clients_label(1), "1 client attached");
        assert_eq!(clients_label(3), "3 clients attached");
    }

    #[test]
    fn a_running_block_lights_its_thumbnail_cell() {
        let mut f = font();
        let c = look(1.0).chrome;
        let accent = |strip: &super::Strip| {
            strip
                .pixmap
                .pixels()
                .iter()
                .filter(|&&px| px == c.accent)
                .count()
        };
        let idle = paint(&session(1), &[], &look(1.0), 1200, &mut f);
        let running = paint(&session(1), &[2], &look(1.0), 1200, &mut f);
        assert_eq!(accent(&idle), 0);
        assert!(accent(&running) > 0, "block 2's cell is the accent");
    }

    fn with_detached(n: usize) -> Session {
        let mut s = session(2);
        s.detached = (0..n)
            .map(|i| Detached {
                key: 30 + i as u64,
                name: format!("block {i}"),
                since: None,
            })
            .collect();
        s
    }

    #[test]
    fn the_detached_chip_shows_only_with_detached_blocks() {
        let mut f = font();
        let none = paint(&session(2), &[], &look(1.0), 1200, &mut f);
        assert!(!none.spots.iter().any(|(_, s)| *s == Spot::Detached));
        let some = paint(&with_detached(2), &[], &look(1.0), 1200, &mut f);
        let (chip, _) = some
            .spots
            .iter()
            .find(|(_, s)| *s == Spot::Detached)
            .expect("a detached chip");
        assert_eq!(chip.h, 28.0);
        let add = some
            .spots
            .iter()
            .find(|(_, s)| *s == Spot::NewWindow)
            .map(|(r, _)| r.x + r.w)
            .expect("+");
        assert!(add < chip.x, "the chip sits right of the tabs");
    }

    #[test]
    fn an_open_detached_chip_has_an_accent_border() {
        let mut f = font();
        let c = look(1.0).chrome;
        let accent = |strip: &super::Strip| {
            strip
                .pixmap
                .pixels()
                .iter()
                .filter(|&&px| px == c.accent)
                .count()
        };
        let shut = paint(&with_detached(1), &[], &look(1.0), 1200, &mut f);
        let open = Look {
            detached_open: true,
            ..look(1.0)
        };
        let open = paint(&with_detached(1), &[], &open, 1200, &mut f);
        assert_eq!(accent(&shut), 0);
        assert!(accent(&open) > 0);
    }
}
