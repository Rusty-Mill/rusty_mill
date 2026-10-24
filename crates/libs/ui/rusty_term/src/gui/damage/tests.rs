//! A partial repaint is only correct if it leaves the same pixels a full
//! repaint would. These tests drive a generated session through both paths,
//! with a backend that hands out buffers 1 to 3 frames old, and compare
//! every pixel after every frame. The test font's glyphs overhang their cell
//! on all four sides, so a row's paint reaching its neighbors is exercised.
//! The session generator is shared with the GPU renderer's cache tests.

use std::rc::Rc;

use crate::core::{AnsiParser, Cell, Grid};

use super::super::font::{Glyph, GlyphSource, Style};
use super::super::render::{PaneFrame, composite, damage_rects, grid_top};
use super::{Damage, Frame, Plan};

const CELL: (usize, usize) = (6, 10);
const PAD: usize = 3;
const COLORS: [u32; 3] = [0x303030, 0x101010, 0x202040];

/// Glyphs 8×14 at (-1, -2) from the cell, partly transparent, so blending
/// twice or missing an overhang shows.
struct OverhangFont;

impl GlyphSource for OverhangFont {
    fn cell_size(&self) -> (usize, usize) {
        CELL
    }
    fn baseline(&self) -> i32 {
        8
    }
    fn glyph(&mut self, ch: char, _style: Style) -> Rc<Glyph> {
        let (width, height) = if ch == ' ' { (0, 0) } else { (8, 14) };
        let coverage = (0..width * height)
            .map(|i| ((ch as usize * 37 + i * 11) % 256) as u8)
            .collect();
        Rc::new(Glyph {
            width,
            height,
            left: -1,
            top: -10,
            coverage,
            color: None,
        })
    }
}

/// Deterministic xorshift, so a failure replays.
pub(in crate::gui) struct Rng(pub(in crate::gui) u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const PIECES: &[&str] = &[
    "\x1b[H",
    "\x1b[3;5H",
    "\x1b[2J",
    "\x1b[K",
    "\x1b[2K",
    "\x1b[J",
    "\x1b[2L",
    "\x1b[M",
    "\x1b[3@",
    "\x1b[2P",
    "\x1b[S",
    "\x1b[T",
    "\x1b[2;5r",
    "\x1b[r",
    "\x1bM",
    "\x1b[?1049h",
    "\x1b[?1049l",
    "\x1b#8",
    "\x1b#6",
    "\x1b[1;31m",
    "\x1b[4m",
    "\x1b[9m",
    "\x1b[4:3m",
    "\x1b[7m",
    "\x1b[0m",
    "\x1b[42m",
    "\r\n",
    "\n",
    "\r",
    "\x08",
    "abc",
    "x",
    "-> != ",
    "\u{4e2d}\u{6587}",
    "e\u{301}",
    "\u{5d0}\u{5d1}\u{5d2}",
    "\x1b[?25l",
    "\x1b[?25h",
    "\x1b[2 q",
    "\x1b[6 q",
    "\x1b[4 q",
    "\x1b]8;;http://x\x07link\x1b]8;;\x07",
    "\x1bc",
    // A sixel image: an overlay with no row key.
    "\x1bPq#0;2;100;0;0#0~~~~~~-~~~~~~\x1b\\",
];

/// Per-frame inputs that live outside the grid.
#[derive(Clone, Copy)]
pub(in crate::gui) struct Extras {
    pub(in crate::gui) cursor_on: bool,
    pub(in crate::gui) focused: bool,
    pub(in crate::gui) hover: Option<(usize, usize, usize)>,
    pub(in crate::gui) trail: Option<(usize, usize, f32)>,
}

/// Mutate the session one step: grid writes, view moves, or window state.
pub(in crate::gui) fn step(rng: &mut Rng, grids: &mut [(Grid, AnsiParser)], x: &mut Extras) {
    let i = rng.below(grids.len());
    let (g, p) = &mut grids[i];
    match rng.below(12) {
        0..=4 => {
            for _ in 0..1 + rng.below(4) {
                p.advance(g, PIECES[rng.below(PIECES.len())].as_bytes());
            }
        }
        5 => x.cursor_on = !x.cursor_on,
        6 => x.focused = !x.focused,
        7 => x.hover = (rng.below(2) == 0).then(|| (rng.below(g.rows), 1, 3)),
        8 => {
            x.trail = (rng.below(2) == 0).then(|| {
                (
                    rng.below(g.cols),
                    rng.below(g.rows),
                    rng.below(9) as f32 / 8.0,
                )
            });
        }
        9 => {
            if rng.below(2) == 0 {
                g.scroll_view_up(1 + rng.below(3));
            } else {
                g.scroll_view_down(1 + rng.below(3));
            }
        }
        10 => g.select_word_at(rng.below(g.cols), rng.below(g.rows)),
        _ => {} // a frame with nothing new
    }
}

pub(in crate::gui) fn panes<'a>(
    views: &'a [crate::core::FrameView],
    x: Extras,
) -> Vec<PaneFrame<'a>> {
    let mut col0 = 0;
    views
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let p = PaneFrame {
                grid: v,
                col0,
                row0: 1,
                focused: x.focused == (i == 0),
                cursor_on: x.cursor_on,
                trail: x
                    .trail
                    .into_iter()
                    .filter(|t| t.0 < v.cols && t.1 < v.rows)
                    .collect(),
                hover_link: x.hover.filter(|h| h.0 < v.rows),
                marks: vec![(0, 0x00ff00)],
            };
            col0 += v.cols + 1;
            p
        })
        .collect()
}

fn bar(text: &str, bg: u32) -> Vec<Cell> {
    text.chars()
        .map(|ch| Cell {
            ch,
            fg: 0xeeeeee,
            bg,
            ..Cell::blank()
        })
        .collect()
}

/// Run `frames` steps on panes of `sizes`, presenting through a chain of
/// `chain` buffers, and check every partial frame against a full repaint
/// and every present's damage against the pixels that changed. Returns the
/// share of pane rows repainted.
fn assert_partial_matches_full(
    sizes: &[(usize, usize)],
    chain: usize,
    frames: usize,
    seed: u64,
) -> f64 {
    let mut rng = Rng(seed);
    let mut grids: Vec<(Grid, AnsiParser)> = sizes
        .iter()
        .map(|&(c, r)| (Grid::new(c, r), AnsiParser::new()))
        .collect();
    let cols: usize = sizes.iter().map(|s| s.0 + 1).sum();
    let rows = sizes.iter().map(|s| s.1).max().unwrap_or(0);
    let (w, h) = (PAD * 2 + cols * CELL.0, PAD * 2 + (rows + 2) * CELL.1);
    let (chrome, status) = (bar("tab", 0x404080), bar("status", 0x204020));
    let mut font = OverhangFont;
    let mut x = Extras {
        cursor_on: true,
        focused: true,
        hover: None,
        trail: None,
    };
    let mut damage = Damage::default();
    let mut bufs = vec![vec![0u32; w * h]; chain];
    let mut painted_at: Vec<Option<usize>> = vec![None; chain];
    let mut shown_full = vec![0u32; w * h];
    let (mut repainted, mut total) = (0usize, 0usize);
    for n in 0..frames {
        step(&mut rng, &mut grids, &mut x);
        let views: Vec<_> = grids.iter().map(|(g, _)| g.frame_view()).collect();
        let panes = panes(&views, x);
        let mut full = vec![0xDEAD_BEEF; w * h];
        composite(
            &mut full, &panes, &chrome, &status, &mut font, w, h, COLORS[0], COLORS[1], COLORS[2],
            PAD, None,
        );

        let slot = n % chain;
        let age = painted_at[slot].map_or(0, |f| (n - f) as u8);
        let frame = Frame::capture(&panes, &chrome, &status, 7, (w, h), COLORS, PAD);
        let plan = damage.plan(&frame, age);
        let shown = damage.shown(&frame, &plan);
        let buf = &mut bufs[slot];
        composite(
            buf,
            &panes,
            &chrome,
            &status,
            &mut font,
            w,
            h,
            COLORS[0],
            COLORS[1],
            COLORS[2],
            PAD,
            plan.rows(),
        );
        if let Some(i) = buf.iter().zip(&full).position(|(a, b)| a != b) {
            panic!(
                "frame {n} (age {age}, {plan:?}): pixel ({}, {}) differs from a full repaint",
                i % w,
                i / w
            );
        }
        if let (Some(rows), true) = (&shown, n > 0) {
            let top = grid_top(&chrome, PAD, CELL.1);
            let rects = damage_rects(&panes, rows, CELL, (PAD, top), (w, h));
            let inside = |i: usize| {
                let (px, py) = ((i % w) as u32, (i / w) as u32);
                rects.iter().any(|r| {
                    (r.x..r.x + r.width.get()).contains(&px)
                        && (r.y..r.y + r.height.get()).contains(&py)
                })
            };
            if let Some(i) = (0..w * h).find(|&i| full[i] != shown_full[i] && !inside(i)) {
                panic!(
                    "chain {chain} frame {n} age {age} {plan:?}: pixel ({}, {}) changed outside the damage {:x} -> {:x} cursor {:?} shape {:?}",
                    i % w,
                    i / w,
                    shown_full[i],
                    full[i],
                    views[0].cursor,
                    views[0].cursor_shape
                );
            }
        }
        if let Plan::Rows(r) = &plan {
            repainted += r.iter().flatten().filter(|&&b| b).count();
            total += r.iter().map(Vec::len).sum::<usize>();
        }
        damage.presented(frame);
        painted_at[slot] = Some(n);
        shown_full = full;
    }
    repainted as f64 / total.max(1) as f64
}

#[test]
fn partial_repaint_matches_full_on_one_pane() {
    for chain in 1..=3 {
        assert_partial_matches_full(
            &[(12, 6)],
            chain,
            1500,
            0x9E37_79B9_7F4A_7C15 + chain as u64,
        );
    }
}

#[test]
fn partial_repaint_matches_full_on_split_panes() {
    for chain in 1..=3 {
        assert_partial_matches_full(
            &[(9, 5), (7, 4)],
            chain,
            1500,
            0xC2B2_AE3D_27D4_EB4F + chain as u64,
        );
    }
}

#[test]
fn partial_repaint_skips_most_rows() {
    let share = assert_partial_matches_full(&[(40, 20)], 1, 800, 0x1656_67B1_9E37_79F9);
    assert!(share < 0.5, "repainted {:.0}% of rows", share * 100.0);
}

/// Rows repainted for `bytes` written to a settled 20×8 screen, one buffer.
fn repaint_after(bytes: &[u8]) -> Vec<bool> {
    let mut g = Grid::new(20, 8);
    let mut p = AnsiParser::new();
    p.advance(&mut g, b"one\r\ntwo\r\nthree\x1b[5;1H");
    let capture = |g: &Grid| {
        let v = g.frame_view();
        let pane = PaneFrame {
            grid: &v,
            col0: 0,
            row0: 0,
            focused: true,
            cursor_on: true,
            trail: Vec::new(),
            hover_link: None,
            marks: Vec::new(),
        };
        Frame::capture(&[pane], &[], &[], 1, (120, 80), COLORS, 0)
    };
    let mut damage = Damage::default();
    damage.presented(capture(&g));
    p.advance(&mut g, bytes);
    match damage.plan(&capture(&g), 1) {
        Plan::Rows(mut r) => r.remove(0),
        Plan::Full => panic!("same layout must repaint by rows"),
    }
}

#[test]
fn only_changed_rows_and_their_neighbors_repaint() {
    let rows = |marked: &[usize]| (0..8).map(|r| marked.contains(&r)).collect::<Vec<_>>();
    assert_eq!(repaint_after(b""), rows(&[]), "nothing changed");
    // The cursor moves along row 4 as `x` lands there.
    assert_eq!(repaint_after(b"x"), rows(&[3, 4, 5]));
    // Writing row 1 and coming back leaves the cursor's row as it was.
    assert_eq!(repaint_after(b"\x1b[2;1Hy\x1b[5;1H"), rows(&[0, 1, 2]));
}

#[test]
fn unknown_or_stale_buffers_repaint_fully() {
    let mut g = Grid::new(4, 2);
    let v = g.frame_view();
    let pane = |v| PaneFrame {
        grid: v,
        col0: 0,
        row0: 0,
        focused: true,
        cursor_on: true,
        trail: Vec::new(),
        hover_link: None,
        marks: Vec::new(),
    };
    let frame = |v, font| Frame::capture(&[pane(v)], &[], &[], font, (24, 20), COLORS, 0);
    let mut damage = Damage::default();
    assert_eq!(damage.plan(&frame(&v, 1), 1), Plan::Full, "no history");
    damage.presented(frame(&v, 1));
    assert_eq!(
        damage.plan(&frame(&v, 1), 0),
        Plan::Full,
        "age 0: unknown contents"
    );
    assert_eq!(
        damage.plan(&frame(&v, 1), 2),
        Plan::Full,
        "older than the history"
    );
    assert_eq!(damage.plan(&frame(&v, 2), 1), Plan::Full, "another font");
    assert_eq!(
        damage.plan(&frame(&v, 1), 1),
        Plan::Rows(vec![vec![false; 2]])
    );
    damage.forget();
    g.resize(5, 2);
    assert_eq!(
        damage.plan(&frame(&g.frame_view(), 1), 1),
        Plan::Full,
        "forgotten"
    );
}
