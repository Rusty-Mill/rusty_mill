//! `SeriesTab` (issue #97) — a `generate_series`-style row generator,
//! the same spirit as real `rusqlite::vtab::series::Series`. An
//! example/utility module, not core `vtab` functionality (see
//! `src/vtab.rs`'s doc comment for that boundary).
//!
//! **Scope deviation from real `rusqlite`'s `series` module, stated
//! plainly:** real SQLite's `generate_series` is a table-valued
//! function — `start`/`stop`/`step` are bound *per query* (e.g.
//! `SELECT * FROM generate_series(1, 10)` or `... WHERE start = 1 AND
//! stop = 10`), negotiated through `best_index`/hidden columns. This
//! crate has no such per-query constraint binding (issue #94's
//! resolution — there's no query planner to negotiate a plan with), so
//! `start`/`stop`/`step` are instead fixed at `CREATE VIRTUAL TABLE`
//! time: `CREATE VIRTUAL TABLE s USING series(1, 10)` or `USING
//! series(1, 10, 2)`. Less flexible, but an honest reflection of what
//! this engine can actually negotiate — not a faithful port.

use crate::dml_select::Expr;
use crate::error::{Error, Result};
use crate::vtab::{Context, CreateVTab, VTab, VTabCursor};

/// Generates the inclusive integer sequence `start..=stop` (or
/// `start..=stop` counting down, if `step` is negative), one column
/// named `value`. Register via [`crate::Connection::register_module`]
/// under whatever module name the caller chooses (this crate ships no
/// built-in modules — nothing is auto-registered on a new connection).
pub struct SeriesTab {
    start: i64,
    stop: i64,
    step: i64,
}

impl VTab for SeriesTab {
    type Cursor = SeriesCursor;

    fn column_names(&self) -> Vec<String> {
        vec!["value".to_string()]
    }

    fn open(&self) -> Result<SeriesCursor> {
        Ok(SeriesCursor {
            current: self.start,
            stop: self.stop,
            step: self.step,
        })
    }
}

impl CreateVTab for SeriesTab {
    /// `args`: `start, stop` or `start, stop, step` (`step` defaults to
    /// `1`). Errors if there aren't 2 or 3 args, any of them isn't a
    /// plain integer, or `step` is `0` (an infinite/empty series either
    /// way — SQLite silently treats a `0` step as `1`, but silently
    /// picking a different number the caller didn't ask for is worse
    /// than telling them).
    fn connect(args: &[String]) -> Result<Self> {
        // Module-argument text is reconstructed from its source tokens
        // joined with single spaces (see `CreateVirtualTable`'s doc
        // comment), so a negative literal like `-1` round-trips as
        // `"- 1"` — the `-` and the digits are separate tokens. Strip
        // internal whitespace before parsing; a plain integer argument
        // never has any of its own.
        let parse = |s: &str| {
            s.trim()
                .replace(' ', "")
                .parse::<i64>()
                .map_err(|_| Error::UnrecognizedStatement(format!("not an integer: {s:?}")))
        };
        let (start, stop, step) = match args {
            [start, stop] => (parse(start)?, parse(stop)?, 1),
            [start, stop, step] => (parse(start)?, parse(stop)?, parse(step)?),
            _ => {
                return Err(Error::UnrecognizedStatement(
                    "series needs 2 or 3 args: start, stop[, step]".to_string(),
                ))
            }
        };
        if step == 0 {
            return Err(Error::UnrecognizedStatement(
                "series step must not be 0".to_string(),
            ));
        }
        Ok(SeriesTab { start, stop, step })
    }
}

pub struct SeriesCursor {
    current: i64,
    stop: i64,
    step: i64,
}

impl VTabCursor for SeriesCursor {
    fn filter(&mut self, _filter: Option<&Expr>) -> Result<()> {
        Ok(())
    }

    fn next(&mut self) -> Result<()> {
        self.current += self.step;
        Ok(())
    }

    fn eof(&self) -> bool {
        if self.step > 0 {
            self.current > self.stop
        } else {
            self.current < self.stop
        }
    }

    fn column(&self, ctx: &mut Context, i: usize) -> Result<()> {
        assert_eq!(i, 0, "SeriesTab only has one column");
        ctx.set_result(&self.current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::Connection;

    #[test]
    fn series_generates_an_ascending_inclusive_range() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.register_module::<SeriesTab>("series").unwrap();
        conn.execute("CREATE VIRTUAL TABLE s USING series(1, 5)")
            .unwrap();

        let values: Vec<i64> = conn.query_map("SELECT * FROM s", |row| row.get(0)).unwrap();
        assert_eq!(values, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn series_honors_an_explicit_step() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.register_module::<SeriesTab>("series").unwrap();
        conn.execute("CREATE VIRTUAL TABLE s USING series(0, 10, 2)")
            .unwrap();

        let values: Vec<i64> = conn.query_map("SELECT * FROM s", |row| row.get(0)).unwrap();
        assert_eq!(values, vec![0, 2, 4, 6, 8, 10]);
    }

    #[test]
    fn series_counts_down_with_a_negative_step() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.register_module::<SeriesTab>("series").unwrap();
        conn.execute("CREATE VIRTUAL TABLE s USING series(5, 1, -1)")
            .unwrap();

        let values: Vec<i64> = conn.query_map("SELECT * FROM s", |row| row.get(0)).unwrap();
        assert_eq!(values, vec![5, 4, 3, 2, 1]);
    }

    #[test]
    fn series_start_after_stop_with_a_positive_step_is_empty() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.register_module::<SeriesTab>("series").unwrap();
        conn.execute("CREATE VIRTUAL TABLE s USING series(5, 1)")
            .unwrap();

        let values: Vec<i64> = conn.query_map("SELECT * FROM s", |row| row.get(0)).unwrap();
        assert!(values.is_empty());
    }

    #[test]
    fn series_rejects_a_zero_step() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.register_module::<SeriesTab>("series").unwrap();
        assert!(conn
            .execute("CREATE VIRTUAL TABLE s USING series(1, 5, 0)")
            .is_err());
    }

    #[test]
    fn series_rejects_the_wrong_number_of_args() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.register_module::<SeriesTab>("series").unwrap();
        assert!(conn
            .execute("CREATE VIRTUAL TABLE s USING series(1)")
            .is_err());
        assert!(conn
            .execute("CREATE VIRTUAL TABLE s USING series(1, 2, 3, 4)")
            .is_err());
    }

    #[test]
    fn series_rejects_a_non_integer_arg() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.register_module::<SeriesTab>("series").unwrap();
        assert!(conn
            .execute("CREATE VIRTUAL TABLE s USING series(abc, 5)")
            .is_err());
    }
}
