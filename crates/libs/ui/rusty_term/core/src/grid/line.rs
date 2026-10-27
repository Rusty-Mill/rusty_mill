//! One grid row, and the "already blank" flag that makes repeat clears cheap.
//!
//! An alt-screen switch blanks the reused buffer and the app's `CSI 2J` then
//! blanks it again, so most clears land on rows that are already blank. A row
//! remembers that it is uniformly `fill` and skips the rewrite. The cells are
//! private to this module so every write goes through [`Line::cells_mut`],
//! which drops the flag: a missed write site is a compile error, not stale text.

use std::borrow::Cow;

use super::{Cell, LineAttr};
use crate::cell::PackedCell;
use crate::checkpoint::{Error, Reader, Result, Wire, Writer};

/// One row of cells, on the live screen or in scrollback. The same value moves
/// from the screen into history when it scrolls off the top, and an evicted
/// history line is recycled as the new bottom row, so a scroll moves no cells.
#[derive(Clone, Debug, Default)]
pub(crate) struct Line {
    /// The row's cells: exactly `cols` on the live screen; a history line keeps
    /// the width it had when it left the screen until the next reflow. Their
    /// style ids belong to the owning grid's table.
    cells: Vec<PackedCell>,
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
    /// How many more times the last stored cell repeats past
    /// [`Line::cells`]: the trailing padding a trimmed history line no
    /// longer stores (see [`Line::trim_tail`]). A count, not a cell, so a
    /// line stays as small as before; a row is at most 4096 cells wide.
    repeat: u16,
}

impl Line {
    /// A row holding `cells`, not known to be blank.
    pub(crate) fn new(cells: Vec<PackedCell>, wrapped: bool, attr: LineAttr) -> Self {
        Line {
            cells,
            blank: false,
            wrapped,
            attr,
            repeat: 0,
        }
    }

    /// A row of `cols` copies of `fill`, unwrapped and single-width.
    pub(crate) fn blank(cols: usize, fill: PackedCell) -> Self {
        Line {
            cells: vec![fill; cols],
            blank: true,
            wrapped: false,
            attr: LineAttr::Single,
            repeat: 0,
        }
    }

    /// The row's cell buffer, for reuse; its contents are not meaningful.
    pub(crate) fn into_cells(self) -> Vec<PackedCell> {
        self.cells
    }

    /// The row's stored cells: all of them, unless [`Line::trim_tail`]
    /// left the last one standing for a repeated run.
    pub(crate) fn cells(&self) -> &[PackedCell] {
        &self.cells
    }

    /// The row's width before any trim: the stored cells and the repeats.
    pub(crate) fn width(&self) -> usize {
        self.cells.len() + usize::from(self.repeat)
    }

    /// The cell at `col`, repeats included; `None` past [`Line::width`].
    pub(crate) fn cell(&self, col: usize) -> Option<PackedCell> {
        match self.cells.get(col) {
            Some(&c) => Some(c),
            None if col < self.width() => self.cells.last().copied(),
            None => None,
        }
    }

    /// Every cell, repeats included, borrowed when there are none.
    pub(crate) fn padded(&self) -> Cow<'_, [PackedCell]> {
        match self.cells.last() {
            Some(&fill) if self.repeat > 0 => {
                let mut cells = Vec::with_capacity(self.width());
                cells.extend_from_slice(&self.cells);
                cells.resize(self.width(), fill);
                Cow::Owned(cells)
            }
            _ => Cow::Borrowed(&self.cells),
        }
    }

    /// Store `cells[keep]` once in place of the run from `keep` to the end,
    /// which must all equal it, releasing the rest's memory. A line already
    /// trimmed, or with at most one cell from `keep` on, is left as it is.
    pub(crate) fn trim_tail(&mut self, keep: usize) {
        let Some(repeat) = self.cells.len().checked_sub(keep + 1) else {
            return;
        };
        let Ok(repeat) = u16::try_from(repeat) else {
            return;
        };
        if repeat == 0 || self.repeat > 0 {
            return;
        }
        debug_assert!(
            self.cells[keep..].iter().all(|c| *c == self.cells[keep]),
            "trimmed run is not uniform"
        );
        self.repeat = repeat;
        self.cells.truncate(keep + 1);
        self.cells.shrink_to_fit();
    }

    /// Store the repeats again, so the cells span the whole width. Only
    /// history lines have any, so writers pay just the check.
    #[inline]
    fn untrim(&mut self) {
        if self.repeat > 0 {
            self.store_repeats();
        }
    }

    #[cold]
    #[inline(never)]
    fn store_repeats(&mut self) {
        if let Some(&fill) = self.cells.last() {
            self.cells.resize(self.width(), fill);
        }
        self.repeat = 0;
    }

    /// The row's cells for writing, the repeats stored again first. Drops
    /// the blank flag, so the next clear rewrites the row.
    pub(crate) fn cells_mut(&mut self) -> &mut [PackedCell] {
        self.untrim();
        self.blank = false;
        &mut self.cells
    }

    /// The stored cells for a style remap, keeping the trim: the repeats
    /// follow the last cell. A remap maps equal cells to equal cells, so
    /// the blank flag stays true.
    pub(crate) fn cells_restyle(&mut self) -> &mut [PackedCell] {
        &mut self.cells
    }

    /// Reset in place to a copy of `blank`, a uniform row (see
    /// `Grid::blank_row`), reusing the cell allocation. A row already blank
    /// in that fill and width is left as it is.
    pub(crate) fn clear(&mut self, blank: &[PackedCell]) {
        let hit = match blank.first() {
            Some(&fill) => self.is_blank_with(blank.len(), fill),
            None => self.cells.is_empty(),
        };
        if !hit {
            if self.cells.len() == blank.len() {
                self.cells.copy_from_slice(blank);
            } else {
                self.cells.clear();
                self.cells.extend_from_slice(blank);
            }
            self.blank = true;
        }
        self.repeat = 0;
        self.wrapped = false;
        self.attr = LineAttr::Single;
    }

    /// Set every cell to `fill` and drop the soft wrap, keeping the width and
    /// the line size (`ED 2` semantics).
    pub(crate) fn fill(&mut self, fill: PackedCell) {
        self.untrim();
        if !self.is_blank_with(self.cells.len(), fill) {
            self.cells.fill(fill);
            self.blank = true;
        }
        self.wrapped = false;
    }

    /// Whether the row is already `len` copies of `fill`, so a clear can skip it.
    fn is_blank_with(&self, len: usize, fill: PackedCell) -> bool {
        let skip =
            self.blank && self.cells.len() == len && self.cells.first().is_none_or(|c| *c == fill);
        debug_assert!(
            !skip || self.cells.iter().all(|c| *c == fill),
            "row flagged blank holds a written cell"
        );
        skip
    }
}

/// Widest row a checkpoint may hold, matching the grid's column limit.
const MAX_WIRE_CELLS: usize = 4096;
/// Shortest stretch of one repeated char worth a repeat run.
const MIN_REPEAT: usize = 4;

/// Checkpoint wire form: the cell count, then runs that cover exactly that
/// many cells, then wrap and attr. A run is a style reference
/// ([`Writer::style`]), `len << 1 | repeat`, then one char repeated `len`
/// times or `len` literal chars. The blank flag is a cache and is not
/// written; a restored row starts unflagged, which is always safe. Writing
/// resolves each run's style through the table the writer borrowed;
/// reading interns each wire style once into the reader's table.
impl Wire for Line {
    fn write(&self, w: &mut Writer) -> Result<()> {
        let cells = self.padded();
        w.charge(cells.len().saturating_mul(size_of::<Cell>()))?;
        w.varint(cells.len() as u64)?;
        let mut at = 0;
        while at < cells.len() {
            let (len, repeat) = run_len(&cells[at..]);
            write_run(w, &cells[at..at + len], repeat)?;
            at += len;
        }
        self.wrapped.write(w)?;
        self.attr.write(w)
    }

    fn read(r: &mut Reader<'_>) -> Result<Self> {
        let n = usize::try_from(r.varint()?)
            .ok()
            .filter(|&n| n <= MAX_WIRE_CELLS)
            .ok_or(Error("line width"))?;
        r.charge(n * size_of::<Cell>())?;
        let mut cells = Vec::new();
        cells
            .try_reserve_exact(n)
            .map_err(|_| Error("allocation failed"))?;
        while cells.len() < n {
            read_run(r, &mut cells, n)?;
        }
        let wrapped = Wire::read(r)?;
        let attr = Wire::read(r)?;
        Ok(Line::new(cells, wrapped, attr))
    }
}

/// Length of the run that starts `cells`, and whether it repeats: a repeat
/// run if the first cell repeats at least [`MIN_REPEAT`] times, else a
/// literal run up to the next style change or repeat. One pass; the length is
/// never zero for non-empty `cells`.
fn run_len(cells: &[PackedCell]) -> (usize, bool) {
    let repeat = repeat_len(cells);
    if repeat >= MIN_REPEAT {
        return (repeat, true);
    }
    // `streak` counts the equal cells ending at `len - 1`.
    let (mut len, mut streak) = (repeat, repeat);
    while let Some(c) = cells.get(len) {
        let prev = &cells[len - 1];
        if !prev.same_run(*c) {
            break;
        }
        streak = if c.ch() == prev.ch() { streak + 1 } else { 1 };
        if streak == MIN_REPEAT {
            return (len + 1 - MIN_REPEAT, false);
        }
        len += 1;
    }
    (len, false)
}

/// How many leading cells equal `cells[0]`.
fn repeat_len(cells: &[PackedCell]) -> usize {
    cells.iter().take_while(|c| **c == cells[0]).count()
}

fn write_run(w: &mut Writer, run: &[PackedCell], repeat: bool) -> Result<()> {
    let style = w.resolve(run[0])?;
    w.style(style)?;
    w.varint((run.len() as u64) << 1 | u64::from(repeat))?;
    if repeat {
        return w.utf8(run[0].ch());
    }
    w.text(run.iter().map(|c| c.ch()))
}

/// Append one run to `cells`, which must end up no longer than `n`.
fn read_run(r: &mut Reader<'_>, cells: &mut Vec<PackedCell>, n: usize) -> Result<()> {
    let (style, id) = r.style()?;
    let head = r.varint()?;
    let len = usize::try_from(head >> 1)
        .ok()
        .filter(|&len| len > 0 && len <= n - cells.len())
        .ok_or(Error("line run length"))?;
    if head & 1 == 1 {
        let ch = r.utf8()?;
        cells.resize(
            cells.len() + len,
            PackedCell::new(&Cell { ch, ..style }, id),
        );
        return Ok(());
    }
    for _ in 0..len {
        let ch = r.utf8()?;
        cells.push(PackedCell::new(&Cell { ch, ..style }, id));
    }
    Ok(())
}
