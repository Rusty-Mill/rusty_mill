//! An owned copy of what one frame of a [`Grid`] draws.
//!
//! The window used to hold every visible pane's grid lock while it
//! composited and presented, so the parser waited out the whole frame,
//! vsync included. [`Grid::frame_view`] copies what a frame needs (resolved
//! cells, highlights, overlays, images by shared reference) in one pass, so
//! the caller can release the lock and paint from the copy.

use std::sync::Arc;

use super::view::View;
use super::{BidiRow, BlockMark, Cell, CursorShape, Grid};

/// Per-cell highlight bits.
const HL_SELECTED: u8 = 1;
const HL_MATCH: u8 = 2;
const HL_CURRENT: u8 = 4;

/// A resolved placeholder cell: `(image id, placement row, placement col)`.
type Placeholder = (u32, u32, u32);

/// A placed pixel image, resolved for one frame.
pub struct FrameImage {
    /// Placement serial (stable while the image stays placed).
    pub serial: usize,
    pub col: usize,
    pub cols: usize,
    pub rows: usize,
    pub pw: usize,
    pub ph: usize,
    /// Viewport row of the image's top cell (negative when scrolled partly
    /// above the view).
    pub top: isize,
    /// The pixels to draw: an animated image's current frame, else its own.
    pub pixels: Arc<[Option<u32>]>,
    /// `(kitty id, frame index)` when `pixels` is an animation frame.
    pub anim: Option<(u32, usize)>,
}

/// A stored Kitty image that placeholder cells in this frame reference.
struct FrameKitty {
    id: u32,
    w: usize,
    h: usize,
    current: usize,
    pixels: Arc<[Option<u32>]>,
    /// Virtual placement `(cols, rows)`; 0 = derive from the cell size.
    geometry: (usize, usize),
}

/// One frame of a [`Grid`], detached from it. Answers the same questions
/// the renderers used to ask the grid, so painting needs no lock.
pub struct FrameView {
    pub cols: usize,
    pub rows: usize,
    pub cursor: (usize, usize),
    pub cursor_visible: bool,
    pub cursor_shape: CursorShape,
    pub cursor_color: u32,
    pub view_offset: usize,
    pub min_contrast: f32,
    pub ime_preedit: String,
    cells: Vec<Cell>,
    highlight: Vec<u8>,
    bidi: Vec<Option<BidiRow>>,
    status: Option<Vec<Cell>>,
    scrollbar: Option<(usize, usize, u32)>,
    marks: Vec<Option<BlockMark>>,
    placeholders: Option<Vec<Option<Placeholder>>>,
    images: Vec<FrameImage>,
    kitty: Vec<FrameKitty>,
}

impl FrameView {
    /// The cell at viewport `(col, row)`, as [`Grid::viewport_cell`].
    pub fn cell(&self, col: usize, row: usize) -> Cell {
        self.cells[row * self.cols + col]
    }

    /// Whether `(col, row)` is selected, as [`Grid::is_selected`].
    pub fn is_selected(&self, col: usize, row: usize) -> bool {
        self.highlight[row * self.cols + col] & HL_SELECTED != 0
    }

    /// Search highlight at `(col, row)`, as [`Grid::search_highlight`].
    pub fn search_highlight(&self, col: usize, row: usize) -> Option<bool> {
        let h = self.highlight[row * self.cols + col];
        (h & HL_MATCH != 0).then_some(h & HL_CURRENT != 0)
    }

    /// The bidi view of viewport `row`, as [`Grid::bidi_row`]. `None` for
    /// the status-line row.
    pub fn bidi_row(&self, row: usize) -> Option<&BidiRow> {
        self.bidi.get(row)?.as_ref()
    }

    /// The status-line overlay, as [`Grid::status_row`].
    pub fn status_row(&self) -> Option<&[Cell]> {
        self.status.as_deref()
    }

    /// The scrollbar thumb, as [`Grid::scrollbar`].
    pub fn scrollbar(&self) -> Option<(usize, usize, u32)> {
        self.scrollbar
    }

    /// Command gutter marks per viewport row, as
    /// [`Grid::viewport_block_marks`].
    pub fn block_marks(&self) -> &[Option<BlockMark>] {
        &self.marks
    }

    /// Placeholder cells, as [`Grid::placeholder_map`].
    pub fn placeholder_map(&self) -> Option<&[Option<Placeholder>]> {
        self.placeholders.as_deref()
    }

    /// Placed pixel images, top row resolved.
    pub fn images(&self) -> &[FrameImage] {
        &self.images
    }

    /// The current frame of placeholder image `id`, as [`Grid::kitty_frame`].
    pub fn kitty_frame(&self, id: u32) -> Option<(usize, usize, &[Option<u32>])> {
        let k = self.kitty(id)?;
        Some((k.w, k.h, &k.pixels))
    }

    /// The current frame index of placeholder image `id`.
    pub fn kitty_current(&self, id: u32) -> Option<usize> {
        Some(self.kitty(id)?.current)
    }

    /// The placement grid of placeholder image `id`, as
    /// [`Grid::placeholder_grid`].
    pub fn placeholder_grid(&self, id: u32, cw: usize, ch: usize) -> Option<(usize, usize)> {
        let k = self.kitty(id)?;
        let (cols, rows) = k.geometry;
        let cols = if cols == 0 {
            k.w.div_ceil(cw.max(1)).max(1)
        } else {
            cols
        };
        let rows = if rows == 0 {
            k.h.div_ceil(ch.max(1)).max(1)
        } else {
            rows
        };
        Some((cols, rows))
    }

    fn kitty(&self, id: u32) -> Option<&FrameKitty> {
        self.kitty.iter().find(|k| k.id == id)
    }
}

impl Grid {
    /// Copy what one frame draws, so the caller can release the grid's
    /// lock before painting. Images are shared, not copied.
    pub fn frame_view(&self) -> FrameView {
        let view = self.view();
        let status = self.status_row().map(<[Cell]>::to_vec);
        let n = self.cols * self.rows;
        let mut cells = Vec::with_capacity(n);
        let mut highlight = Vec::with_capacity(n);
        for row in 0..self.rows {
            for col in 0..self.cols {
                cells.push(view.cell(col, row));
                highlight.push(highlight_bits(&view, col, row));
            }
        }
        let last = self.rows.saturating_sub(1);
        let bidi = (0..self.rows)
            .map(|r| {
                if status.is_some() && r == last {
                    None
                } else {
                    view.bidi_row(r)
                }
            })
            .collect();
        let placeholders = self.placeholders_of(&view);
        let kitty = self.frame_kitty(placeholders.as_deref().unwrap_or(&[]));
        FrameView {
            cols: self.cols,
            rows: self.rows,
            cursor: self.cursor,
            cursor_visible: self.cursor_visible,
            cursor_shape: self.cursor_shape,
            cursor_color: self.cursor_color,
            view_offset: self.view_offset,
            min_contrast: self.min_contrast,
            ime_preedit: self.ime_preedit.clone(),
            cells,
            highlight,
            bidi,
            status,
            scrollbar: self.scrollbar(),
            marks: self.block_marks_of(view.lines()),
            placeholders,
            images: self.frame_images(),
            kitty,
        }
    }

    /// Placed images with their top row and drawable pixels resolved: an
    /// animated image's current frame when the store still has it at the
    /// image's size, else the image's own pixels.
    fn frame_images(&self) -> Vec<FrameImage> {
        self.images
            .iter()
            .map(|im| {
                let anim = im.anim.and_then(|id| {
                    let k = self.kitty_images.iter().find(|k| k.id == id)?;
                    (k.w == im.pw && k.h == im.ph).then(|| (id, k.current, &k.frames[k.current]))
                });
                let (pixels, anim) = match anim {
                    Some((id, cur, f)) => (Arc::clone(&f.pixels), Some((id, cur))),
                    None => (Arc::clone(&im.pixels), None),
                };
                FrameImage {
                    serial: im.serial,
                    col: im.col,
                    cols: im.cols,
                    rows: im.rows,
                    pw: im.pw,
                    ph: im.ph,
                    top: self.image_top_row(im),
                    pixels,
                    anim,
                }
            })
            .collect()
    }

    /// The stored images the frame's placeholder cells reference.
    fn frame_kitty(&self, placeholders: &[Option<Placeholder>]) -> Vec<FrameKitty> {
        let mut out: Vec<FrameKitty> = Vec::new();
        for &(id, _, _) in placeholders.iter().flatten() {
            if out.iter().any(|k| k.id == id) {
                continue;
            }
            let Some(img) = self.kitty_images.iter().find(|i| i.id == id) else {
                continue;
            };
            out.push(FrameKitty {
                id,
                w: img.w,
                h: img.h,
                current: img.current,
                pixels: Arc::clone(&img.frames[img.current].pixels),
                geometry: self.kitty_virtual_geometry(id).unwrap_or((0, 0)),
            });
        }
        out
    }
}

fn highlight_bits(view: &View<'_>, col: usize, row: usize) -> u8 {
    let mut bits = 0;
    if view.is_selected(col, row) {
        bits |= HL_SELECTED;
    }
    match view.search_highlight(col, row) {
        Some(true) => bits |= HL_MATCH | HL_CURRENT,
        Some(false) => bits |= HL_MATCH,
        None => {}
    }
    bits
}

#[cfg(test)]
mod tests {
    use super::super::Selection;
    use super::*;
    use crate::AnsiParser;

    /// Folded commands, RTL text, a search, a selection and (with `l13`) a
    /// status line.
    fn busy_grid() -> Grid {
        let mut g = Grid::new(24, 6);
        g.bidi = true;
        let mut p = AnsiParser::new();
        for i in 0..12 {
            p.advance(&mut g, format!("$ cmd{i}\r\n\x1b]133;C\x07").as_bytes());
            for k in 0..=(i % 3) {
                p.advance(&mut g, format!("out {i}.{k} \u{5d0}\u{5d1}\r\n").as_bytes());
            }
            p.advance(&mut g, format!("\x1b]133;D;{}\x07", i % 2).as_bytes());
        }
        assert!(g.search_with("cmd1", false) > 0);
        for b in g.fold_blocks.iter_mut().skip(2).step_by(3) {
            b.folded = true;
        }
        g.selection = Some(Selection {
            anchor: (3, 5),
            head: (1, 20),
        });
        #[cfg(feature = "l13")]
        g.set_status_line("status".into(), None, None);
        g
    }

    #[test]
    fn frame_view_is_unchanged_by_later_grid_writes() {
        let mut g = busy_grid();
        g.view_offset = 0;
        let f = g.frame_view();
        let before: Vec<Cell> = (0..g.rows)
            .flat_map(|r| (0..g.cols).map(move |c| (c, r)))
            .map(|(c, r)| f.cell(c, r))
            .collect();
        AnsiParser::new().advance(&mut g, b"\x1b[2J\x1b[Hoverwritten\r\n".repeat(8).as_slice());
        g.selection = None;
        let after: Vec<Cell> = (0..g.rows)
            .flat_map(|r| (0..g.cols).map(move |c| (c, r)))
            .map(|(c, r)| f.cell(c, r))
            .collect();
        assert_eq!(before, after);
        let now: Vec<Cell> = (0..g.rows)
            .flat_map(|r| (0..g.cols).map(move |c| (c, r)))
            .map(|(c, r)| g.viewport_cell(c, r))
            .collect();
        assert_ne!(before, now, "the grid did change");
    }

    #[test]
    fn frame_view_matches_the_grid_at_every_offset() {
        let mut g = busy_grid();
        let (mut marks, mut selected, mut matched) = (false, false, false);
        for off in 0..=g.display_history_len() + 1 {
            g.view_offset = off;
            let f = g.frame_view();
            assert_eq!((f.cols, f.rows, f.view_offset), (g.cols, g.rows, off));
            assert_eq!(f.status_row(), g.status_row());
            assert_eq!(f.scrollbar(), g.scrollbar());
            assert_eq!(f.block_marks(), g.viewport_block_marks().as_slice());
            assert_eq!(f.placeholder_map(), g.placeholder_map().as_deref());
            marks |= f.block_marks().iter().any(Option::is_some);
            for row in 0..g.rows {
                let on_status = g.status_row().is_some() && row + 1 == g.rows;
                let want = (!on_status).then(|| g.bidi_row(row)).flatten();
                assert_eq!(
                    f.bidi_row(row).map(|b| &b.log2vis),
                    want.as_ref().map(|b| &b.log2vis),
                    "offset {off} row {row}"
                );
                for col in 0..g.cols {
                    assert_eq!(f.cell(col, row), g.viewport_cell(col, row), "({col},{row})");
                    assert_eq!(f.is_selected(col, row), g.is_selected(col, row));
                    assert_eq!(f.search_highlight(col, row), g.search_highlight(col, row));
                    selected |= f.is_selected(col, row);
                    matched |= f.search_highlight(col, row).is_some();
                }
            }
        }
        assert!(
            g.bidi_active() && marks && selected && matched,
            "fixture covers each"
        );
    }
}
