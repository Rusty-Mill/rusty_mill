//! When to compact a pane's history: once its output has been quiet for
//! [`QUIET`], in steps of [`BUDGET`] lines, so the parse path never pays
//! for it and no single step holds the grid lock for long. The TUI reactor
//! and the GUI window each keep one [`Compactor`] per pane and wake at
//! [`Compactor::due`].

use std::time::{Duration, Instant};

use crate::core::Grid;

/// Quiet time after the last output before compaction starts.
pub(crate) const QUIET: Duration = Duration::from_millis(100);
/// Lines looked at per step.
pub(crate) const BUDGET: usize = 1000;

/// A pane's compaction schedule.
#[derive(Debug, Default)]
pub(crate) struct Compactor {
    due: Option<Instant>,
}

impl Compactor {
    /// Output arrived or the grid was resized at `now`: history may have
    /// grown or been stored at full width again, so compact once it has
    /// been quiet for [`QUIET`].
    pub(crate) fn on_change(&mut self, now: Instant) {
        self.due = Some(now + QUIET);
    }

    /// Resize `grid` to `cols` x `rows` at `now`, and whether its size
    /// changed. A reflow stores every history line at full width again, so
    /// a real change schedules a pass even if no output follows.
    pub(crate) fn resize(
        &mut self,
        grid: &mut Grid,
        cols: usize,
        rows: usize,
        now: Instant,
    ) -> bool {
        let before = (grid.cols, grid.rows);
        grid.resize(cols, rows);
        let changed = (grid.cols, grid.rows) != before;
        if changed {
            self.on_change(now);
        }
        changed
    }

    /// When the next step should run, or `None` when there is nothing to do.
    pub(crate) fn due(&self) -> Option<Instant> {
        self.due
    }

    /// Whether a step is due at `now`.
    pub(crate) fn is_due(&self, now: Instant) -> bool {
        self.due.is_some_and(|at| at <= now)
    }

    /// Run one step if it is due at `now`. Another step is due at once while
    /// lines remain, and none after the pass that finds no more.
    pub(crate) fn run(&mut self, grid: &mut Grid, now: Instant) {
        if !self.is_due(now) {
            return;
        }
        self.due = grid.compact_history(BUDGET).then_some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::AnsiParser;

    /// A grid with `lines` hard-broken short lines in its history.
    fn grid_with_history(lines: usize) -> Grid {
        let mut g = Grid::new(80, 4);
        g.set_scrollback_max(lines + 10);
        AnsiParser::new().advance(&mut g, "x\r\n".repeat(lines + 4).as_bytes());
        g
    }

    #[test]
    fn nothing_is_due_before_any_output() {
        let mut c = Compactor::default();
        let mut g = grid_with_history(10);
        assert_eq!(c.due(), None);
        c.run(&mut g, Instant::now() + QUIET * 10);
        assert_eq!(c.due(), None);
        assert!(g.compact_history(1), "the history was left untouched");
    }

    #[test]
    fn output_defers_compaction_until_quiet() {
        let (mut c, mut g, t0) = (Compactor::default(), grid_with_history(10), Instant::now());
        c.on_change(t0);
        c.run(&mut g, t0 + QUIET / 2);
        assert_eq!(c.due(), Some(t0 + QUIET), "not yet quiet: nothing ran");
        c.on_change(t0 + QUIET / 2);
        assert_eq!(
            c.due(),
            Some(t0 + QUIET / 2 + QUIET),
            "more output pushes it back"
        );
    }

    #[test]
    fn steps_run_back_to_back_until_a_pass_finds_nothing() {
        let (mut c, mut g, t0) = (
            Compactor::default(),
            grid_with_history(2 * BUDGET + 5),
            Instant::now(),
        );
        c.on_change(t0);
        let mut steps = 0;
        let mut now = t0 + QUIET;
        while let Some(at) = c.due() {
            assert!(at <= now, "each further step is due at once");
            c.run(&mut g, now);
            steps += 1;
            now += Duration::from_millis(1);
            assert!(steps <= 3, "three steps cover two budgets and five lines");
        }
        assert_eq!(steps, 3);
        assert!(!g.compact_history(BUDGET), "the history is fully compacted");
    }

    #[test]
    fn a_resize_after_a_finished_pass_schedules_another_without_output() {
        let (mut c, mut g, t0) = (Compactor::default(), grid_with_history(50), Instant::now());
        c.on_change(t0);
        c.run(&mut g, t0 + QUIET);
        assert_eq!(c.due(), None, "one pass covered the history");
        let t1 = t0 + QUIET * 2;
        assert!(!c.resize(&mut g, 80, 4, t1), "same size");
        assert_eq!(c.due(), None, "nothing to schedule");
        assert!(c.resize(&mut g, 60, 4, t1));
        assert_eq!(c.due(), Some(t1 + QUIET), "scheduled with no output");
        assert!(g.compact_history(0), "the reflow left lines to look at");
        c.run(&mut g, t1 + QUIET);
        assert_eq!(c.due(), None);
        assert!(!g.compact_history(0), "the scheduled pass covered them");
    }
}
