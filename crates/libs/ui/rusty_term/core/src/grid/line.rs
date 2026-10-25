//! One grid row, and the "already blank" flag that makes repeat clears cheap.
//!
//! An alt-screen switch blanks the reused buffer and the app's `CSI 2J` then
//! blanks it again, so most clears land on rows that are already blank. A row
//! remembers that it is uniformly `fill` and skips the rewrite. The cells are
//! private to this module so every write goes through [`Line::cells_mut`],
//! which drops the flag: a missed write site is a compile error, not stale text.

use super::{Cell, LineAttr};
use crate::checkpoint::{Error, Reader, Result, Wire, Writer};

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

/// Widest row a checkpoint may hold, matching the grid's column limit.
const MAX_WIRE_CELLS: usize = 4096;
/// Shortest stretch of one repeated char worth a repeat run.
const MIN_REPEAT: usize = 4;

/// Checkpoint wire form: the cell count, then runs that cover exactly that
/// many cells, then wrap and attr. A run is a style reference
/// ([`Writer::style`]), `len << 1 | repeat`, then one char repeated `len`
/// times or `len` literal chars. The blank flag is a cache and is not
/// written; a restored row starts unflagged, which is always safe.
impl Wire for Line {
    fn write(&self, w: &mut Writer) -> Result<()> {
        w.charge(self.cells.len().saturating_mul(size_of::<Cell>()))?;
        w.varint(self.cells.len() as u64)?;
        let mut at = 0;
        while at < self.cells.len() {
            let (len, repeat) = run_len(&self.cells[at..]);
            write_run(w, &self.cells[at..at + len], repeat)?;
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
fn run_len(cells: &[Cell]) -> (usize, bool) {
    let repeat = repeat_len(cells);
    if repeat >= MIN_REPEAT {
        return (repeat, true);
    }
    // `streak` counts the equal cells ending at `len - 1`.
    let (mut len, mut streak) = (repeat, repeat);
    while let Some(c) = cells.get(len) {
        let prev = &cells[len - 1];
        if !same_style(prev, c) {
            break;
        }
        streak = if c.ch == prev.ch { streak + 1 } else { 1 };
        if streak == MIN_REPEAT {
            return (len + 1 - MIN_REPEAT, false);
        }
        len += 1;
    }
    (len, false)
}

/// How many leading cells equal `cells[0]`.
fn repeat_len(cells: &[Cell]) -> usize {
    cells.iter().take_while(|c| **c == cells[0]).count()
}

/// Equal in every field but `ch`.
fn same_style(a: &Cell, b: &Cell) -> bool {
    a.fg == b.fg
        && a.bg == b.bg
        && a.flags == b.flags
        && a.link == b.link
        && a.cluster == b.cluster
        && a.underline_color == b.underline_color
}

fn write_run(w: &mut Writer, run: &[Cell], repeat: bool) -> Result<()> {
    w.style(run[0])?;
    w.varint((run.len() as u64) << 1 | u64::from(repeat))?;
    if repeat {
        return w.utf8(run[0].ch);
    }
    w.text(run.iter().map(|c| c.ch))
}

/// Append one run to `cells`, which must end up no longer than `n`.
fn read_run(r: &mut Reader<'_>, cells: &mut Vec<Cell>, n: usize) -> Result<()> {
    let style = r.style()?;
    let head = r.varint()?;
    let len = usize::try_from(head >> 1)
        .ok()
        .filter(|&len| len > 0 && len <= n - cells.len())
        .ok_or(Error("line run length"))?;
    if head & 1 == 1 {
        let ch = r.utf8()?;
        cells.resize(cells.len() + len, Cell { ch, ..style });
        return Ok(());
    }
    for _ in 0..len {
        cells.push(Cell {
            ch: r.utf8()?,
            ..style
        });
    }
    Ok(())
}
