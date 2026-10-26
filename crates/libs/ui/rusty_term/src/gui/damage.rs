//! Damage tracking for the CPU renderer: which pane rows a frame must
//! repaint over the pixels already in the presentation buffer.
//!
//! Each presented frame leaves a [`Frame`]: the window-wide inputs that
//! paint outside the pane rows ([`Layout`]) and, per pane row, everything
//! that row's pixels depend on ([`RowState`]). A buffer the backend hands
//! back `age` frames old holds the pixels of the frame captured that many
//! presents ago, so only rows whose state differs from it are repainted.
//! A row's glyphs may reach a neighbor's band (see `cpu::draw_grid`), so a
//! changed row repaints its neighbors too.

use std::collections::VecDeque;

use crate::core::{BidiRow, Cell, CursorShape, RowKey};

use super::render::PaneFrame;

/// Presented frames remembered. softbuffer reports a buffer age of 1 on
/// X11 and Win32 and 2 on Wayland (double buffered), and 0 (unknown) on
/// macOS; 3 covers a triple-buffered backend.
const HISTORY: usize = 3;

/// The window-wide inputs to a frame. Any difference repaints everything.
#[derive(PartialEq)]
struct Layout {
    size: (usize, usize),
    pad: usize,
    font: u64,
    colors: [u32; 3],
    chrome: Vec<Cell>,
    status: Vec<Cell>,
    panes: Vec<PaneLayout>,
}

/// One pane's inputs that are not per row.
#[derive(PartialEq)]
struct PaneLayout {
    origin: (usize, usize),
    size: (usize, usize),
    scrollbar: Option<(usize, usize, u32)>,
    marks: Vec<(usize, u32)>,
    status: Option<Vec<Cell>>,
    cursor_color: u32,
    min_contrast: u32,
}

/// Everything one pane row's pixels depend on besides [`Layout`]. The GPU
/// renderer keys its per-row instance cache on it too.
#[derive(PartialEq)]
pub(super) struct RowState {
    /// The row's cells, as of their last change; `None` has no key.
    key: Option<RowKey>,
    highlight: Box<[u8]>,
    /// The cursor's column and shape when it is drawn on this row.
    cursor: Option<(usize, CursorShape)>,
    /// The IME preedit drawn at this row's cursor column.
    ime: Option<(usize, String)>,
    /// The hovered link's column span on this row.
    hover: Option<(usize, usize)>,
    bidi: Option<BidiRow>,
    /// Trail ghosts on this row: column and alpha bits.
    trail: Vec<(usize, u32)>,
    /// Pixel images or placeholders cover the row; their pixels have no key.
    overlay: bool,
}

impl RowState {
    /// Whether a row painted with `old` already shows this state.
    pub(super) fn same(&self, old: &RowState) -> bool {
        self.key.is_some() && !self.overlay && self == old
    }
}

/// What one presented frame painted.
pub(super) struct Frame {
    layout: Layout,
    /// Per pane, per viewport row.
    rows: Vec<Vec<RowState>>,
}

impl Frame {
    /// Capture the inputs of one [`Renderer::render`] call.
    ///
    /// [`Renderer::render`]: super::render::Renderer::render
    #[allow(clippy::too_many_arguments)]
    pub(super) fn capture(
        panes: &[PaneFrame],
        chrome: &[Cell],
        status: &[Cell],
        font: u64,
        size: (usize, usize),
        colors: [u32; 3],
        pad: usize,
    ) -> Frame {
        let layout = Layout {
            size,
            pad,
            font,
            colors,
            chrome: chrome.to_vec(),
            status: status.to_vec(),
            panes: panes.iter().map(pane_layout).collect(),
        };
        let rows = panes
            .iter()
            .map(|p| (0..p.grid.rows).map(|r| row_state(p, r)).collect())
            .collect();
        Frame { layout, rows }
    }
}

fn pane_layout(p: &PaneFrame) -> PaneLayout {
    let g = p.grid;
    PaneLayout {
        origin: (p.col0, p.row0),
        size: (g.cols, g.rows),
        scrollbar: g.scrollbar(),
        marks: p.marks.clone(),
        status: g.status_row().map(<[Cell]>::to_vec),
        cursor_color: g.cursor_color,
        min_contrast: g.min_contrast.to_bits(),
    }
}

/// The state of `p`'s viewport `row`.
pub(super) fn row_state(p: &PaneFrame, row: usize) -> RowState {
    let g = p.grid;
    let live = g.view_offset == 0 && g.cursor.1 == row;
    let cursor = (live && p.focused && g.cursor_visible && p.cursor_on)
        .then_some((g.cursor.0, g.cursor_shape));
    let ime = (live && p.focused && !g.ime_preedit.is_empty())
        .then(|| (g.cursor.0, g.ime_preedit.clone()));
    let images = g.images().iter().any(|im| {
        let r = row as isize;
        im.top <= r && r < im.top + im.rows as isize
    });
    let placeholders = g.placeholder_map().is_some_and(|m| {
        m[row * g.cols..(row + 1) * g.cols]
            .iter()
            .any(Option::is_some)
    });
    RowState {
        key: g.row_key(row),
        highlight: g.row_highlight(row).into(),
        cursor,
        ime,
        hover: p.hover_link.filter(|h| h.0 == row).map(|h| (h.1, h.2)),
        bidi: g.bidi_row(row).cloned(),
        trail: p
            .trail
            .iter()
            .filter(|t| t.1 == row)
            .map(|t| (t.0, t.2.to_bits()))
            .collect(),
        overlay: images || placeholders,
    }
}

/// Which rows of each pane a frame repaints.
#[derive(Debug, PartialEq)]
pub(super) enum Plan {
    /// Every pixel.
    Full,
    /// Per pane, per viewport row; everything else is left as it is.
    Rows(Vec<Vec<bool>>),
}

impl Plan {
    /// The rows to pass to `render::composite`: `None` repaints all.
    pub(super) fn rows(&self) -> Option<&[Vec<bool>]> {
        match self {
            Plan::Full => None,
            Plan::Rows(rows) => Some(rows),
        }
    }
}

/// The last few presented frames, newest first.
#[derive(Default)]
pub(super) struct Damage {
    history: VecDeque<Frame>,
}

impl Damage {
    /// The repaint for `frame` over a buffer that holds the frame presented
    /// `age` presents ago (`0`: unknown contents).
    pub(super) fn plan(&self, frame: &Frame, age: u8) -> Plan {
        let old = (age as usize)
            .checked_sub(1)
            .and_then(|i| self.history.get(i));
        match old.and_then(|old| changed(frame, old)) {
            Some(rows) => Plan::Rows(rows),
            None => Plan::Full,
        }
    }

    /// The rows whose pixels differ from the last present once `plan` is
    /// painted, for the compositor's damage. `None`: the whole window.
    pub(super) fn shown(&self, frame: &Frame, plan: &Plan) -> Option<Vec<Vec<bool>>> {
        let Plan::Rows(repaint) = plan else {
            return None;
        };
        let last = changed(frame, self.history.front()?)?;
        Some(
            repaint
                .iter()
                .zip(last)
                .map(|(a, b)| a.iter().zip(b).map(|(&a, b)| a || b).collect())
                .collect(),
        )
    }

    /// Record `frame` as presented.
    pub(super) fn presented(&mut self, frame: Frame) {
        self.history.push_front(frame);
        self.history.truncate(HISTORY);
    }

    /// Forget every frame: the buffer contents are no longer known.
    pub(super) fn forget(&mut self) {
        self.history.clear();
    }
}

/// The rows of `new` to repaint over pixels painted for `old`, each changed
/// row widened to its neighbors; `None` when the layouts differ.
fn changed(new: &Frame, old: &Frame) -> Option<Vec<Vec<bool>>> {
    if new.layout != old.layout {
        return None;
    }
    let rows = new
        .rows
        .iter()
        .zip(&old.rows)
        .map(|(new, old)| {
            let diff: Vec<bool> = new.iter().zip(old).map(|(n, o)| !n.same(o)).collect();
            widen(&diff)
        })
        .collect();
    Some(rows)
}

/// Mark each row next to a marked one.
fn widen(rows: &[bool]) -> Vec<bool> {
    (0..rows.len())
        .map(|r| rows[r.saturating_sub(1)..(r + 2).min(rows.len())].contains(&true))
        .collect()
}

/// Shared with the GPU renderer's cache tests.
#[cfg(test)]
pub(super) mod tests;
