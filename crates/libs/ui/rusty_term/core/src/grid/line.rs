//! One grid row, and the "already blank" flag that makes repeat clears cheap.
//!
//! An alt-screen switch blanks the reused buffer and the app's `CSI 2J` then
//! blanks it again, so most clears land on rows that are already blank. A row
//! remembers that it is uniformly `fill` and skips the rewrite. The cells are
//! private to this module so every write goes through [`Line::cells_mut`],
//! which drops the flag: a missed write site is a compile error, not stale text.

use super::{Cell, LineAttr};

/// One row of cells, on the live screen or in scrollback. The same value moves
/// from the screen into history when it scrolls off the top, and an evicted
/// history line is recycled as the new bottom row, so a scroll moves no cells.
#[derive(Clone, Debug, Default)]
pub(crate) struct Line {
    /// The row's cells: exactly `cols` on the live screen; a history line keeps
    /// the width it had when it left the screen until the next reflow.
    cells: Vec<Cell>,
    /// Every cell equals `cells[0]`, as the last clear or fill left it. Set
    /// only here, dropped by any mutable access.
    blank: bool,
    /// Soft wrap: set when DECAWM autowrap overflowed this row into the next,
    /// marking the two as one logical line. A *hard* line break (LF, NEL, IND)
    /// leaves it clear. The reflow uses it to rejoin wrapped runs on resize.
    pub wrapped: bool,
    /// Line size (DECDWL/DECDHL). The renderer relays it to the host. Always
    /// [`LineAttr::Single`] in scrollback: a double-width line is meaningless
    /// once scrolled off.
    pub attr: LineAttr,
}

/// Content equality: the `blank` flag is a cache and never makes two rows differ.
impl PartialEq for Line {
    fn eq(&self, other: &Self) -> bool {
        self.cells == other.cells && self.wrapped == other.wrapped && self.attr == other.attr
    }
}

impl Line {
    /// A row holding `cells`, not known to be blank.
    pub(crate) fn new(cells: Vec<Cell>, wrapped: bool, attr: LineAttr) -> Self {
        Line {
            cells,
            blank: false,
            wrapped,
            attr,
        }
    }

    /// A row of `cols` copies of `fill`, unwrapped and single-width.
    pub(crate) fn blank(cols: usize, fill: Cell) -> Self {
        Line {
            cells: vec![fill; cols],
            blank: true,
            wrapped: false,
            attr: LineAttr::Single,
        }
    }

    /// The row's cells.
    pub(crate) fn cells(&self) -> &[Cell] {
        &self.cells
    }

    /// The row's cells for writing. Drops the blank flag, so the next clear
    /// rewrites the row.
    pub(crate) fn cells_mut(&mut self) -> &mut [Cell] {
        self.blank = false;
        &mut self.cells
    }

    /// Reset to [`Line::blank`] in place, reusing the cell allocation.
    pub(crate) fn clear(&mut self, cols: usize, fill: Cell) {
        if !self.is_blank_with(cols, fill) {
            self.cells.clear();
            self.cells.resize(cols, fill);
            self.blank = true;
        }
        self.wrapped = false;
        self.attr = LineAttr::Single;
    }

    /// Set every cell to `fill` and drop the soft wrap, keeping the width and
    /// the line size (`ED 2` semantics).
    pub(crate) fn fill(&mut self, fill: Cell) {
        if !self.is_blank_with(self.cells.len(), fill) {
            self.cells.fill(fill);
            self.blank = true;
        }
        self.wrapped = false;
    }

    /// Whether the row is already `len` copies of `fill`, so a clear can skip it.
    fn is_blank_with(&self, len: usize, fill: Cell) -> bool {
        let skip =
            self.blank && self.cells.len() == len && self.cells.first().is_none_or(|c| *c == fill);
        debug_assert!(
            !skip || self.cells.iter().all(|c| *c == fill),
            "row flagged blank holds a written cell"
        );
        skip
    }
}
