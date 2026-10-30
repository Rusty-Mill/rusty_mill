//! Drawing a window's blocks side by side: compose a [`Frame`] from each
//! block's grid at its place in the layout, then [`render`] what changed
//! since the last frame.

use std::fmt::Write as _;

use rusty_mux_core::{BlockId, Rect, Size};
use rusty_mux_server::style::Style;
use rusty_term_core::{ATTR_UNDERLINE, Cell, Grid, WIDE_TRAILER};

use crate::keys::Dir;
use crate::predict::Overlay;

/// The colour of the dividers around the focused block.
const FOCUS: u32 = 0x5f_d7_5f;

/// One screen cell as drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Glyph {
    ch: char,
    /// A grapheme's continuation after `ch`; empty, so unallocated, for
    /// most cells.
    extra: String,
    style: Style,
    /// The second cell of a wide glyph: drawn by the first.
    trailer: bool,
}

impl Glyph {
    fn blank() -> Self {
        Self::plain(' ', Style::of(&Cell::blank()))
    }

    fn plain(ch: char, style: Style) -> Self {
        Self {
            ch,
            extra: String::new(),
            style,
            trailer: false,
        }
    }
}

/// A whole screen, ready to diff against the next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    area: Size,
    cells: Vec<Glyph>,
    /// Where the focused block's cursor is, if it shows one.
    cursor: Option<(u16, u16)>,
}

/// One block to draw: its place and, once attached, its grid, with the
/// echo predicted over it, if any.
pub struct View<'a> {
    pub block: BlockId,
    pub rect: Rect,
    pub grid: Option<&'a Grid>,
    pub overlay: Option<Overlay>,
}

impl Frame {
    fn index(&self, x: u16, y: u16) -> usize {
        usize::from(y) * usize::from(self.area.cols()) + usize::from(x)
    }

    fn row(&self, y: u16) -> &[Glyph] {
        let start = self.index(0, y);
        &self.cells[start..start + usize::from(self.area.cols())]
    }
}

/// The screen of `area` with each view's grid clipped to its rect, the
/// space between them drawn as dividers, those around `focused` in colour.
pub fn compose(area: Size, views: &[View<'_>], focused: BlockId) -> Frame {
    let mut frame = Frame {
        area,
        cells: vec![Glyph::blank(); usize::from(area.cols()) * usize::from(area.rows())],
        cursor: None,
    };
    let mut covered = vec![false; frame.cells.len()];
    for view in views {
        let Rect { x, y, size } = view.rect;
        for dy in 0..size.rows() {
            for dx in 0..size.cols() {
                covered[frame.index(x + dx, y + dy)] = true;
            }
        }
        if let Some(grid) = view.grid {
            draw(&mut frame, view.rect, grid);
            let ahead =
                (view.overlay.as_ref()).and_then(|o| predict(&mut frame, view.rect, grid, o));
            if view.block == focused {
                // Shown only where the grid shows it, after the predictions.
                let shown = cursor(view.rect, grid);
                frame.cursor = shown.and(ahead).or(shown);
            }
        }
    }
    let focus = views.iter().find(|v| v.block == focused).map(|v| v.rect);
    dividers(&mut frame, &covered, focus);
    frame
}

/// `grid`'s screen into `rect`, clipped; a wide glyph cut by the right
/// edge becomes a blank.
fn draw(frame: &mut Frame, rect: Rect, grid: &Grid) {
    let (cols, rows) = (usize::from(rect.size.cols()), usize::from(rect.size.rows()));
    for y in 0..rows.min(grid.rows) {
        let cells: Vec<Cell> = grid.row_cells(y).take(cols).collect();
        for (x, cell) in cells.iter().enumerate() {
            let wide_cut = x + 1 == cols
                && cells.len() == cols
                && grid
                    .row_cells(y)
                    .nth(cols)
                    .is_some_and(|c| c.flags & WIDE_TRAILER != 0);
            let glyph = match wide_cut {
                true => Glyph::plain(' ', Style::of(cell)),
                false => Glyph {
                    ch: cell.ch,
                    extra: usize::from(cell.cluster)
                        .checked_sub(1)
                        .and_then(|i| grid.clusters.get(i))
                        .cloned()
                        .unwrap_or_default(),
                    style: Style::of(cell),
                    trailer: cell.flags & WIDE_TRAILER != 0,
                },
            };
            // Both fit in u16: they are below the rect's size.
            let i = frame.index(rect.x + x as u16, rect.y + y as u16);
            frame.cells[i] = glyph;
        }
    }
}

/// Draws `overlay`'s cells, characters underlined, into `rect` where they fit on a
/// single-width cell of `grid`; returns the predicted cursor, if inside
/// `rect`.
fn predict(frame: &mut Frame, rect: Rect, grid: &Grid, overlay: &Overlay) -> Option<(u16, u16)> {
    let (cols, rows) = (usize::from(rect.size.cols()), usize::from(rect.size.rows()));
    if overlay.row >= rows {
        return None;
    }
    let cells: Vec<Cell> = grid.row_cells(overlay.row).collect();
    let trailer = |x: usize| cells.get(x).is_some_and(|c| c.flags & WIDE_TRAILER != 0);
    // Both fit in u16: they are below the rect's size.
    let y = rect.y + overlay.row as u16;
    for &(x, ch) in &overlay.cells {
        if x >= cols || trailer(x) || trailer(x + 1) {
            continue;
        }
        let i = frame.index(rect.x + x as u16, y);
        let style = frame.cells[i].style;
        frame.cells[i] = match ch {
            Some(ch) => Glyph::plain(
                ch,
                Style {
                    attrs: style.attrs | ATTR_UNDERLINE,
                    ..style
                },
            ),
            None => Glyph::plain(' ', style),
        };
    }
    (overlay.cursor < cols).then(|| (rect.x + overlay.cursor as u16, y))
}

/// The grid's cursor on the screen, if it is visible and inside `rect`.
fn cursor(rect: Rect, grid: &Grid) -> Option<(u16, u16)> {
    let (cx, cy) = grid.cursor;
    let inside = cx < usize::from(rect.size.cols()) && cy < usize::from(rect.size.rows());
    // Both fit in u16: they are below the rect's size.
    (grid.cursor_visible && inside).then(|| (rect.x + cx as u16, rect.y + cy as u16))
}

/// Box-drawing lines over every cell no view covers, joined where they
/// meet; those next to `focus` in [`FOCUS`].
fn dividers(frame: &mut Frame, covered: &[bool], focus: Option<Rect>) {
    let (cols, rows) = (frame.area.cols(), frame.area.rows());
    let open = |x: Option<u16>, y: Option<u16>| match (x, y) {
        (Some(x), Some(y)) if x < cols && y < rows => {
            !covered[usize::from(y) * usize::from(cols) + usize::from(x)]
        }
        _ => false,
    };
    let plain = Style::of(&Cell::blank());
    for y in 0..rows {
        for x in 0..cols {
            if !open(Some(x), Some(y)) {
                continue;
            }
            let up = open(Some(x), y.checked_sub(1));
            let down = open(Some(x), y.checked_add(1));
            let left = open(x.checked_sub(1), Some(y));
            let right = open(x.checked_add(1), Some(y));
            let ch = match (up || down, left || right) {
                (true, false) => '│',
                (false, true) => '─',
                (false, false) => '│',
                (true, true) => match (up, down, left, right) {
                    (true, true, false, true) => '├',
                    (true, true, true, false) => '┤',
                    (false, true, true, true) => '┬',
                    (true, false, true, true) => '┴',
                    _ => '┼',
                },
            };
            let near = focus.is_some_and(|r| touches(r, x, y));
            let style = match near {
                true => Style { fg: FOCUS, ..plain },
                false => plain,
            };
            let i = frame.index(x, y);
            frame.cells[i] = Glyph::plain(ch, style);
        }
    }
}

/// Whether cell `(x, y)` borders `rect`, corners included.
fn touches(rect: Rect, x: u16, y: u16) -> bool {
    let (x, y) = (u32::from(x), u32::from(y));
    let (left, top) = (u32::from(rect.x), u32::from(rect.y));
    let right = left + u32::from(rect.size.cols());
    let bottom = top + u32::from(rect.size.rows());
    x + 1 >= left && x <= right && y + 1 >= top && y <= bottom
}

/// The bytes that turn the screen `prev` showed into `next`: every row
/// that changed, whole, then the cursor. Everything when there is no
/// `prev` or its size differs; nothing when nothing changed.
pub fn render(prev: Option<&Frame>, next: &Frame) -> Vec<u8> {
    let fresh = prev.filter(|p| p.area == next.area);
    let mut out = String::new();
    if fresh.is_none() {
        out.push_str("\x1b[0m\x1b[2J");
    }
    let mut pen = None;
    for y in 0..next.area.rows() {
        let row = next.row(y);
        if fresh.is_some_and(|p| p.row(y) == row) {
            continue;
        }
        let _ = write!(out, "\x1b[{};1H", y + 1);
        for glyph in row.iter().filter(|g| !g.trailer) {
            if pen != Some(glyph.style) {
                glyph.style.sgr(&mut out);
                pen = Some(glyph.style);
            }
            out.push(glyph.ch);
            out.push_str(&glyph.extra);
        }
    }
    if fresh.is_some_and(|p| p.cursor == next.cursor) && out.is_empty() {
        return Vec::new();
    }
    // Drawing moved the cursor; hide it while it is elsewhere.
    let mut framed = String::from("\x1b[?25l");
    framed.push_str(&out);
    framed.push_str("\x1b[0m");
    if let Some((x, y)) = next.cursor {
        let _ = write!(framed, "\x1b[{};{}H\x1b[?25h", y + 1, x + 1);
    }
    framed.into_bytes()
}

/// The block whose rect lies nearest `from`'s in direction `dir` and
/// overlaps it across that direction; the first of equals.
pub fn neighbor(rects: &[(BlockId, Rect)], from: BlockId, dir: Dir) -> Option<BlockId> {
    let (_, f) = rects.iter().find(|(b, _)| *b == from)?;
    let span = |r: &Rect| {
        let (x, y) = (u32::from(r.x), u32::from(r.y));
        (
            x,
            x + u32::from(r.size.cols()),
            y,
            y + u32::from(r.size.rows()),
        )
    };
    let (fl, fr, ft, fb) = span(f);
    rects
        .iter()
        .filter(|(b, _)| *b != from)
        .filter_map(|(b, r)| {
            let (l, r, t, bottom) = span(r);
            let across_rows = t < fb && ft < bottom;
            let across_cols = l < fr && fl < r;
            let gap = match dir {
                Dir::Right if across_rows && l >= fr => l - fr,
                Dir::Left if across_rows && r <= fl => fl - r,
                Dir::Down if across_cols && t >= fb => t - fb,
                Dir::Up if across_cols && bottom <= ft => ft - bottom,
                _ => return None,
            };
            Some((gap, *b))
        })
        .min_by_key(|(gap, _)| *gap)
        .map(|(_, b)| b)
}

#[cfg(test)]
mod tests {
    use rusty_mux_core::Layout;
    use rusty_term_core::AnsiParser;
    use rusty_uuid::Uuid;

    use super::*;

    fn id(n: u8) -> BlockId {
        BlockId::from_uuid(Uuid::from_bytes([n; 16]))
    }

    fn size(cols: u16, rows: u16) -> Size {
        Size::new(cols, rows).unwrap()
    }

    fn grid(cols: usize, rows: usize, input: &str) -> Grid {
        let mut grid = Grid::new(cols, rows);
        AnsiParser::new().advance(&mut grid, input.as_bytes());
        grid
    }

    fn text(frame: &Frame, y: u16) -> String {
        frame
            .row(y)
            .iter()
            .filter(|g| !g.trailer)
            .map(|g| format!("{}{}", g.ch, g.extra))
            .collect()
    }

    /// `1` left of `2` over `3`, on a 9x5 screen.
    fn three() -> Vec<(BlockId, Rect)> {
        let layout: Layout = format!("cols50({},rows50({},{}))", id(1), id(2), id(3))
            .parse()
            .unwrap();
        layout.rects(size(9, 5))
    }

    #[test]
    fn blocks_are_clipped_into_their_rects_between_dividers() {
        let (a, b, c) = (grid(6, 2, "abcdef"), grid(4, 2, "xy"), grid(4, 2, "z"));
        let grids = [&a, &b, &c];
        let views: Vec<View<'_>> = three()
            .into_iter()
            .zip(grids)
            .map(|((block, rect), grid)| View {
                block,
                rect,
                grid: Some(grid),
                overlay: None,
            })
            .collect();
        let frame = compose(size(9, 5), &views, id(2));
        assert_eq!(text(&frame, 0), "abcd│xy  ");
        assert_eq!(text(&frame, 1), "    │    ");
        assert_eq!(text(&frame, 2), "    ├────");
        assert_eq!(text(&frame, 3), "    │z   ");
        // `xy` leaves the cursor after it, in block 2's rect.
        assert_eq!(frame.cursor, Some((7, 0)));
        let focused = frame.row(1)[4].style.fg;
        assert_eq!(focused, FOCUS);
        assert_ne!(frame.row(4)[4].style.fg, FOCUS);
    }

    #[test]
    fn a_wide_glyph_cut_by_the_edge_is_blank() {
        let wide = grid(4, 1, "a日");
        let rect = Rect {
            x: 0,
            y: 0,
            size: size(2, 1),
        };
        let views = [View {
            block: id(1),
            rect,
            grid: Some(&wide),
            overlay: None,
        }];
        assert_eq!(text(&compose(size(2, 1), &views, id(1)), 0), "a ");
        let whole = Rect {
            size: size(3, 1),
            ..rect
        };
        let views = [View {
            block: id(1),
            rect: whole,
            grid: Some(&wide),
            overlay: None,
        }];
        assert_eq!(text(&compose(size(3, 1), &views, id(1)), 0), "a日");
    }

    #[test]
    fn render_redraws_only_changed_rows() {
        let one = grid(4, 2, "ab");
        let two = grid(4, 2, "ab\r\ncd");
        let rect = Rect {
            x: 0,
            y: 0,
            size: size(4, 2),
        };
        let frame = |g| {
            compose(
                size(4, 2),
                &[View {
                    block: id(1),
                    rect,
                    grid: Some(g),
                    overlay: None,
                }],
                id(1),
            )
        };
        let (first, second) = (frame(&one), frame(&two));
        let full = String::from_utf8(render(None, &first)).unwrap();
        assert!(full.contains("\x1b[2J") && full.contains("ab"), "{full:?}");
        let delta = String::from_utf8(render(Some(&first), &second)).unwrap();
        assert!(!delta.contains("\x1b[1;1H"), "{delta:?}");
        assert!(
            delta.contains("\x1b[2;1H") && delta.contains("cd"),
            "{delta:?}"
        );
        assert!(delta.ends_with("\x1b[2;3H\x1b[?25h"), "{delta:?}");
        assert!(render(Some(&second), &second).is_empty());
    }

    #[test]
    fn predictions_are_drawn_underlined_with_the_cursor_after_them() {
        let typed = grid(4, 2, "$ ");
        let rect = Rect {
            x: 0,
            y: 0,
            size: size(4, 2),
        };
        let views = [View {
            block: id(1),
            rect,
            grid: Some(&typed),
            overlay: Some(Overlay {
                row: 0,
                cells: vec![(2, Some('l')), (3, Some('s'))],
                cursor: 4,
            }),
        }];
        let frame = compose(size(4, 2), &views, id(1));
        assert_eq!(text(&frame, 0), "$ ls");
        let underlined = |x: usize| frame.row(0)[x].style.attrs & ATTR_UNDERLINE != 0;
        assert!(!underlined(1) && underlined(2) && underlined(3));
        // Past the margin: the cursor stays where the grid has it.
        assert_eq!(frame.cursor, Some((2, 0)));
        let views = [View {
            overlay: Some(Overlay {
                row: 0,
                cells: vec![(2, Some('l')), (3, None)],
                cursor: 3,
            }),
            ..views.into_iter().next().unwrap()
        }];
        assert_eq!(compose(size(4, 2), &views, id(1)).cursor, Some((3, 0)));
    }

    #[test]
    fn a_prediction_never_covers_a_wide_glyph_or_leaves_its_rect() {
        let wide = grid(6, 1, "$ 日");
        let rect = Rect {
            x: 0,
            y: 0,
            size: size(3, 1),
        };
        let views = [View {
            block: id(1),
            rect,
            grid: Some(&wide),
            overlay: Some(Overlay {
                row: 0,
                cells: vec![(2, Some('x')), (3, Some('y')), (5, Some('z'))],
                cursor: 6,
            }),
        }];
        let frame = compose(size(3, 1), &views, id(1));
        // `日` is cut to a blank by the edge, and nothing lands on it.
        assert_eq!(text(&frame, 0), "$  ");
    }

    #[test]
    fn neighbors_are_found_across_the_split() {
        let rects = three();
        assert_eq!(neighbor(&rects, id(1), Dir::Right), Some(id(2)));
        assert_eq!(neighbor(&rects, id(2), Dir::Down), Some(id(3)));
        assert_eq!(neighbor(&rects, id(3), Dir::Up), Some(id(2)));
        assert_eq!(neighbor(&rects, id(3), Dir::Left), Some(id(1)));
        assert_eq!(neighbor(&rects, id(1), Dir::Left), None);
        assert_eq!(neighbor(&rects, id(9), Dir::Left), None);
    }
}
