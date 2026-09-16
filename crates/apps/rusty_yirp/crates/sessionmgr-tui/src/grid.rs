//! Grid layout math: turning N panes into a rows-by-columns arrangement
//! of `ratatui::Rect`s, with keyboard-driven resize.
//!
//! Pure and unit-testable without a real terminal -- no `ratatui::Frame`,
//! no crossterm, just `Rect` in and `Vec<Rect>` out. That is deliberate:
//! this is the one part of the TUI worth the same fast, deterministic
//! coverage `sessionmgr-core`'s state machine gets, for the same reason
//! (PLAN.md's testing strategy).

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// A grid's shape and each row/column's share of the available space.
///
/// Weights are percentages (0-100) rather than fixed cells, so a resize
/// is "nudge this row/column's share" rather than "recompute everything
/// from a pixel count" -- the same reasoning `ratatui::layout::Constraint`
/// itself is built on.
#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    row_weights: Vec<u16>,
    col_weights: Vec<u16>,
}

/// How far one resize nudge moves a row/column's share, in percentage
/// points.
const RESIZE_STEP: u16 = 5;

/// A weight is never pushed below this floor. Below a few percent a pane
/// stops being usable at all, and a floor of zero would let one resize
/// key silently swallow a whole row/column.
const MIN_WEIGHT: u16 = 10;

impl Grid {
    /// A grid sized to fit `count` panes with no manual resize applied
    /// yet: as close to square as `count` allows, filling row-major so
    /// the last row may be short rather than every row being sparse.
    ///
    /// Mirrors CAPABILITIES.md's "add all" behavior -- a grid that grows
    /// to fit whatever is open rather than a layout the user has to
    /// configure before it shows anything.
    pub fn for_pane_count(count: usize) -> Self {
        if count == 0 {
            return Grid {
                row_weights: vec![],
                col_weights: vec![],
            };
        }
        let cols = (count as f64).sqrt().ceil() as usize;
        let rows = count.div_ceil(cols);
        Grid {
            row_weights: even_weights(rows),
            col_weights: even_weights(cols),
        }
    }

    pub fn rows(&self) -> usize {
        self.row_weights.len()
    }

    pub fn cols(&self) -> usize {
        self.col_weights.len()
    }

    /// Splits `area` into `rows() * cols()` rects, row-major, using the
    /// current weights. The caller is responsible for not asking for more
    /// panes than `rows() * cols()` provides -- see [`Self::cell_for`].
    pub fn split(&self, area: Rect) -> Vec<Rect> {
        if self.row_weights.is_empty() || self.col_weights.is_empty() {
            return vec![];
        }
        let row_areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                self.row_weights
                    .iter()
                    .map(|w| Constraint::Percentage(*w))
                    .collect::<Vec<_>>(),
            )
            .split(area);
        row_areas
            .iter()
            .flat_map(|row| {
                Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints(
                        self.col_weights
                            .iter()
                            .map(|w| Constraint::Percentage(*w))
                            .collect::<Vec<_>>(),
                    )
                    .split(*row)
                    .to_vec()
            })
            .collect()
    }

    /// The rect for pane `index` (row-major) within `area`, if the grid
    /// has that many cells.
    pub fn cell_for(&self, area: Rect, index: usize) -> Option<Rect> {
        self.split(area).into_iter().nth(index)
    }

    /// Grows row `row`'s share by [`RESIZE_STEP`], taking it evenly from
    /// every other row so the total stays 100. A no-op if there is only
    /// one row, or the row is already at the ceiling the others' floors
    /// impose.
    pub fn grow_row(&mut self, row: usize) {
        grow(&mut self.row_weights, row);
    }

    pub fn shrink_row(&mut self, row: usize) {
        shrink(&mut self.row_weights, row);
    }

    pub fn grow_col(&mut self, col: usize) {
        grow(&mut self.col_weights, col);
    }

    pub fn shrink_col(&mut self, col: usize) {
        shrink(&mut self.col_weights, col);
    }
}

fn even_weights(n: usize) -> Vec<u16> {
    if n == 0 {
        return vec![];
    }
    // Integer-divide, then hand the remainder to the last share, so the
    // weights always sum to exactly 100 rather than drifting under from
    // truncation (three rows would otherwise be 33/33/33 = 99).
    let base = 100 / n as u16;
    let mut weights = vec![base; n];
    let remainder = 100 - base * n as u16;
    if let Some(last) = weights.last_mut() {
        *last += remainder;
    }
    weights
}

/// Grows `weights[i]` by [`RESIZE_STEP`], taking it evenly from the
/// others (down to [`MIN_WEIGHT`] each), so the sum stays 100.
fn grow(weights: &mut [u16], i: usize) {
    if weights.len() < 2 || i >= weights.len() {
        return;
    }
    let others: Vec<usize> = (0..weights.len()).filter(|&j| j != i).collect();
    let mut remaining = RESIZE_STEP;
    for &j in &others {
        if remaining == 0 {
            break;
        }
        let take = remaining.min(weights[j].saturating_sub(MIN_WEIGHT));
        weights[j] -= take;
        weights[i] += take;
        remaining -= take;
    }
}

/// The inverse of [`grow`]: shrinks `weights[i]` down to [`MIN_WEIGHT`]
/// and hands what it gave up to the others evenly.
fn shrink(weights: &mut [u16], i: usize) {
    if weights.len() < 2 || i >= weights.len() {
        return;
    }
    let give = RESIZE_STEP.min(weights[i].saturating_sub(MIN_WEIGHT));
    if give == 0 {
        return;
    }
    weights[i] -= give;
    let others: Vec<usize> = (0..weights.len()).filter(|&j| j != i).collect();
    let each = give / others.len() as u16;
    let mut remainder = give - each * others.len() as u16;
    for &j in &others {
        let mut share = each;
        if remainder > 0 {
            share += 1;
            remainder -= 1;
        }
        weights[j] += share;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_panes_produces_an_empty_grid() {
        let grid = Grid::for_pane_count(0);
        assert_eq!(grid.rows(), 0);
        assert_eq!(grid.cols(), 0);
        assert_eq!(grid.split(Rect::new(0, 0, 100, 100)), vec![]);
    }

    #[test]
    fn one_pane_is_a_single_cell() {
        let grid = Grid::for_pane_count(1);
        assert_eq!((grid.rows(), grid.cols()), (1, 1));
        let cells = grid.split(Rect::new(0, 0, 80, 24));
        assert_eq!(cells, vec![Rect::new(0, 0, 80, 24)]);
    }

    #[test]
    fn four_panes_are_a_two_by_two_grid() {
        let grid = Grid::for_pane_count(4);
        assert_eq!((grid.rows(), grid.cols()), (2, 2));
        assert_eq!(grid.split(Rect::new(0, 0, 80, 24)).len(), 4);
    }

    #[test]
    fn three_panes_fill_row_major_with_a_short_last_row() {
        // sqrt(3).ceil() = 2 columns, ceil(3/2) = 2 rows -> a 2x2 grid
        // with one empty cell, not three sparse rows.
        let grid = Grid::for_pane_count(3);
        assert_eq!((grid.rows(), grid.cols()), (2, 2));
    }

    #[test]
    fn row_weights_always_sum_to_100() {
        for n in 1..=7 {
            let grid = Grid::for_pane_count(n);
            let row_sum: u16 = grid.row_weights.iter().sum();
            let col_sum: u16 = grid.col_weights.iter().sum();
            assert_eq!(row_sum, 100, "n={n}");
            assert_eq!(col_sum, 100, "n={n}");
        }
    }

    #[test]
    fn growing_a_column_shrinks_the_others_and_preserves_the_total() {
        let mut grid = Grid::for_pane_count(4); // 2x2, cols start at 50/50
        grid.grow_col(0);
        assert_eq!(grid.col_weights, vec![55, 45]);
        let sum: u16 = grid.col_weights.iter().sum();
        assert_eq!(sum, 100);
    }

    #[test]
    fn growing_repeatedly_stops_at_the_others_floor() {
        let mut grid = Grid::for_pane_count(4);
        for _ in 0..20 {
            grid.grow_col(0);
        }
        // The other column floors at MIN_WEIGHT; growth then stalls.
        assert_eq!(grid.col_weights[1], MIN_WEIGHT);
        assert_eq!(grid.col_weights[0], 100 - MIN_WEIGHT);
    }

    #[test]
    fn shrink_undoes_grow() {
        let mut grid = Grid::for_pane_count(4);
        grid.grow_col(0);
        grid.shrink_col(0);
        assert_eq!(grid.col_weights, vec![50, 50]);
    }

    #[test]
    fn single_row_or_column_resize_is_a_no_op() {
        let mut grid = Grid::for_pane_count(1);
        grid.grow_row(0);
        grid.grow_col(0);
        assert_eq!(grid.row_weights, vec![100]);
        assert_eq!(grid.col_weights, vec![100]);
    }

    #[test]
    fn cell_for_out_of_range_index_is_none() {
        let grid = Grid::for_pane_count(1);
        assert!(grid.cell_for(Rect::new(0, 0, 10, 10), 5).is_none());
    }
}
