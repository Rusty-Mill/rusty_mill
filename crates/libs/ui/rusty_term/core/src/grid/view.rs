//! The fold-aware viewport, resolved once per frame.
//!
//! A viewport row shows a live screen row, a scrollback line, or a folded
//! block's one-line summary. Resolving one row walks the visible fold blocks
//! (`O(blocks)`), so a renderer that asked per cell paid that walk several
//! times per cell. [`Grid::view`] resolves every row in a single walk
//! (`O(blocks + rows)`) and answers the per-cell questions from that map.

use super::{BidiRow, Cell, Grid, HistLine};

/// What one viewport row shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ViewLine {
    /// Scrollback line, by absolute index.
    Hist(usize),
    /// The summary of the folded block at this `fold_blocks` index.
    Summary(usize),
    /// Live screen row.
    Live(usize),
}

/// A frame's view of a [`Grid`]: every viewport row resolved up front, so
/// per-cell lookups cost no fold walk. Borrows the grid, so it cannot outlive
/// a change to it.
pub struct View<'g> {
    grid: &'g Grid,
    lines: Vec<ViewLine>,
    /// Summary text per row (empty for non-summary rows).
    summaries: Vec<Vec<char>>,
    selection: Option<((usize, usize), (usize, usize))>,
}

impl<'g> View<'g> {
    /// The cell at viewport `(col, row)`, as [`Grid::viewport_cell`].
    pub fn cell(&self, col: usize, row: usize) -> Cell {
        match self.lines[row] {
            ViewLine::Summary(_) => self
                .grid
                .summary_cell(self.summaries[row].get(col).copied().unwrap_or(' ')),
            line => self.grid.cell_of(line, col),
        }
    }

    /// The absolute row viewport `row` shows, as [`Grid::abs_of_view_row`].
    pub fn abs(&self, row: usize) -> usize {
        self.grid.abs_of_line(self.lines[row])
    }

    /// Whether `(col, row)` is selected, as [`Grid::is_selected`].
    pub fn is_selected(&self, col: usize, row: usize) -> bool {
        let Some((start, end)) = self.selection else {
            return false;
        };
        let lin = |c: usize, r: usize| r * self.grid.cols + c;
        (lin(start.0, start.1)..=lin(end.0, end.1)).contains(&lin(col, self.abs(row)))
    }

    /// Search highlight at `(col, row)`, as [`Grid::search_highlight`].
    pub fn search_highlight(&self, col: usize, row: usize) -> Option<bool> {
        self.grid.search_highlight_abs(col, self.abs(row))
    }

    /// The bidi view of viewport `row`, as [`Grid::bidi_row`].
    pub fn bidi_row(&self, row: usize) -> Option<BidiRow> {
        self.grid.bidi_row_of(row, |col| self.cell(col, row))
    }

    /// Viewport `row`'s cells appended to `out`: the bulk form of
    /// [`View::cell`], resolving each run of one style once.
    pub(super) fn push_row(&self, row: usize, out: &mut Vec<Cell>) {
        let cols = self.grid.cols;
        let line = match self.lines[row] {
            ViewLine::Hist(l) => &self.grid.scrollback[l],
            ViewLine::Live(r) => &self.grid.screen[r],
            ViewLine::Summary(_) => return out.extend((0..cols).map(|col| self.cell(col, row))),
        };
        self.grid.push_resolved(line, cols, out);
    }

    /// The resolved rows, top to bottom.
    pub(super) fn lines(&self) -> &[ViewLine] {
        &self.lines
    }
}

impl Grid {
    /// Resolve the whole viewport once, for a frame's per-cell lookups.
    pub fn view(&self) -> View<'_> {
        let lines = self.view_lines();
        let summaries = lines
            .iter()
            .map(|l| match *l {
                ViewLine::Summary(i) => self.summary_text(i).chars().collect(),
                _ => Vec::new(),
            })
            .collect();
        View {
            grid: self,
            lines,
            summaries,
            selection: self.selection_bounds(),
        }
    }

    /// What viewport row `vr` shows, resolved alone (one fold walk).
    pub(super) fn view_line(&self, vr: usize) -> ViewLine {
        let dhl = self.display_history_len();
        let off = self.view_offset.min(dhl);
        if vr >= off {
            return ViewLine::Live(vr - off);
        }
        match self.history_line(dhl - off + vr) {
            HistLine::Abs(l) => ViewLine::Hist(l),
            HistLine::Summary(i) => ViewLine::Summary(i),
        }
    }

    /// Every viewport row, in one walk over the visible folds. Equal to
    /// `(0..rows).map(view_line)`.
    pub(super) fn view_lines(&self) -> Vec<ViewLine> {
        let dhl = self.display_history_len();
        let off = self.view_offset.min(dhl);
        // History display indices shown: `lo..hi`, at most `rows` of them.
        let lo = dhl - off;
        let hi = lo + off.min(self.rows);
        let mut out = Vec::with_capacity(self.rows);
        // Display indices `d..d + n` show absolute lines `abs..abs + n`.
        let plain = |out: &mut Vec<ViewLine>, d: usize, n: usize, abs: usize| {
            out.extend((d.max(lo)..(d + n).min(hi)).map(|k| ViewLine::Hist(abs + k - d)));
        };
        let (mut d, mut abs) = (0usize, 0usize);
        for (i, b) in self.visible_folds() {
            if d >= hi {
                break;
            }
            let n = b.start - abs;
            plain(&mut out, d, n, abs);
            d += n;
            if (lo..hi).contains(&d) {
                out.push(ViewLine::Summary(i));
            }
            d += 1;
            abs = b.end;
        }
        if d < hi {
            plain(&mut out, d, hi - d, abs);
        }
        out.extend((0..self.rows.saturating_sub(off)).map(ViewLine::Live));
        out
    }

    /// The cell at `col` of a resolved row. A blank fills past a short
    /// history line's width.
    pub(super) fn cell_of(&self, line: ViewLine, col: usize) -> Cell {
        match line {
            ViewLine::Hist(l) => self.line_cell(&self.scrollback[l], col),
            ViewLine::Summary(i) => {
                self.summary_cell(self.summary_text(i).chars().nth(col).unwrap_or(' '))
            }
            ViewLine::Live(r) => self.line_cell(&self.screen[r], col),
        }
    }

    /// The absolute row (scrollback + live) a resolved row shows. A summary
    /// maps to its block's first line.
    pub(super) fn abs_of_line(&self, line: ViewLine) -> usize {
        match line {
            ViewLine::Hist(l) => l,
            ViewLine::Summary(i) => self.fold_blocks[i].start,
            ViewLine::Live(r) => self.scrollback.len() + r,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Selection;
    use super::*;
    use crate::AnsiParser;

    /// 40 commands with 1-3 output lines; the 3-line blocks folded, 1- and
    /// 2-line ones left open, plus a search and a selection.
    fn folded_grid(rows: usize) -> Grid {
        let mut g = Grid::new(30, rows);
        let mut p = AnsiParser::new();
        for i in 0..40 {
            p.advance(&mut g, format!("$ cmd{i}\r\n\x1b]133;C\x07").as_bytes());
            for k in 0..=(i % 3) {
                p.advance(&mut g, format!("out {i}.{k}\r\n").as_bytes());
            }
            p.advance(&mut g, b"\x1b]133;D;0\x07");
        }
        // Search before folding: a jump to a hidden match unfolds its block.
        assert!(g.search_with("cmd3", false) > 0);
        for b in g.fold_blocks.iter_mut().skip(2).step_by(3) {
            b.folded = true;
        }
        g.selection = Some(Selection {
            anchor: (4, 7),
            head: (2, 60),
        });
        g
    }

    /// The frame view agrees with the per-row lookups at every offset.
    fn assert_view_matches(g: &mut Grid) {
        let top = g.display_history_len();
        assert!(g.visible_folds().count() > 1, "fixture folds blocks");
        for off in (0..=top + 3).chain([usize::MAX]) {
            g.view_offset = off;
            let lines = g.view_lines();
            let single: Vec<ViewLine> = (0..g.rows).map(|r| g.view_line(r)).collect();
            assert_eq!(lines, single, "offset {off}");
            let v = g.view();
            for row in 0..g.rows {
                assert_eq!(v.abs(row), g.abs_of_view_row(row), "offset {off} row {row}");
                assert_eq!(v.bidi_row(row).is_some(), g.bidi_row(row).is_some());
                for col in 0..g.cols {
                    assert_eq!(v.cell(col, row), g.viewport_cell(col, row), "({col},{row})");
                    assert_eq!(v.is_selected(col, row), g.is_selected(col, row));
                    assert_eq!(v.search_highlight(col, row), g.search_highlight(col, row));
                }
            }
        }
    }

    #[test]
    fn view_matches_per_row_lookups_across_folds_and_offsets() {
        // Shorter and taller than one block, so offsets cross pages.
        for rows in [2, 4, 9] {
            assert_view_matches(&mut folded_grid(rows));
        }
    }

    #[test]
    fn view_matches_with_bidi_and_after_eviction() {
        let mut g = folded_grid(6);
        g.bidi = true;
        g.set_scrollback_max(50);
        let mut p = AnsiParser::new();
        p.advance(&mut g, "\u{5d0}\u{5d1} rtl\r\n".repeat(20).as_bytes());
        assert_view_matches(&mut g);
    }

    #[test]
    fn view_shows_summary_text_and_live_rows_unscrolled() {
        let mut g = folded_grid(4);
        g.view_offset = 0;
        let v = g.view();
        assert!(v.lines.iter().all(|l| matches!(l, ViewLine::Live(_))));
        // Put the first folded block's summary on the top row.
        let (_, b) = g.visible_folds().next().expect("a folded block");
        g.view_offset = g.display_history_len() - g.display_index_of_abs(b.start);
        let v = g.view();
        assert!(matches!(v.lines[0], ViewLine::Summary(_)), "{:?}", v.lines);
        let text: String = (0..g.cols).map(|c| v.cell(c, 0).ch).collect();
        assert!(text.starts_with("\u{25B7} 3 lines hidden"), "{text:?}");
    }
}
