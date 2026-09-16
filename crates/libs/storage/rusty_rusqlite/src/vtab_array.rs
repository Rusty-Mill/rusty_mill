//! `ArrayTab` (issue #96) — binds a `Vec<Value>` as a query-able
//! one-column table, the same use case as real `rusqlite`'s
//! `rarray!`/`vtab::array` (`Array`, `array::load_module`).
//!
//! **Scope deviation from real `rusqlite`'s `array` module, stated
//! plainly:** real `rarray!` binds an `Rc<Vec<Value>>` as a *query
//! parameter* (`SELECT * FROM rarray(?1)`), using SQLite's
//! `sqlite3_bind_pointer` to smuggle the `Rc` through the C FFI
//! boundary as an opaque pointer — a value can't cross that boundary
//! any other way. This crate's engine has no FFI boundary and no
//! per-query pointer-bound parameters to mirror that with, so the
//! honest equivalent is simpler: register a fresh table per `Vec` via
//! [`crate::Connection::create_module`] (the eponymous path, issue
//! #92) — the table is queryable immediately, no separate bind step.
//! Also means there's no single shared `rarray` name every array binds
//! to; each `Vec` gets whatever table name the caller registers it
//! under.
//!
//! **Also not reproducible yet:** real `rarray!`'s headline use case —
//! `WHERE x IN rarray(?1)` — needs `IN` and/or `JOIN` support this
//! engine's `WHERE`-clause grammar doesn't have yet (a separate,
//! pre-existing gap). [`ArrayTab`] itself works standalone today —
//! `SELECT * FROM name`, with the usual `WHERE value = ...` filtering
//! any [`crate::TableSource`] supports — and will compose with `IN`/
//! `JOIN` once those land.

use crate::dml_select::Expr;
use crate::error::Result;
use crate::value::Value;
use crate::vtab::{Context, VTab, VTabCursor};

/// A read-only, single-column (`value`) virtual table backed by an
/// in-memory `Vec<Value>` snapshot, taken once at construction —
/// mutating the `Vec` afterward has no effect on an already-registered
/// [`ArrayTab`], the same "column names/shape fixed at registration"
/// rule every other `VTab` in this crate follows.
pub struct ArrayTab {
    values: Vec<Value>,
}

impl ArrayTab {
    /// Wraps `values` for querying, one row per element.
    pub fn new(values: Vec<Value>) -> ArrayTab {
        ArrayTab { values }
    }
}

impl VTab for ArrayTab {
    type Cursor = ArrayCursor;

    fn column_names(&self) -> Vec<String> {
        vec!["value".to_string()]
    }

    fn open(&self) -> Result<ArrayCursor> {
        Ok(ArrayCursor {
            values: self.values.clone(),
            pos: 0,
        })
    }
}

pub struct ArrayCursor {
    values: Vec<Value>,
    pos: usize,
}

impl VTabCursor for ArrayCursor {
    fn filter(&mut self, _filter: Option<&Expr>) -> Result<()> {
        Ok(())
    }

    fn next(&mut self) -> Result<()> {
        self.pos += 1;
        Ok(())
    }

    fn eof(&self) -> bool {
        self.pos >= self.values.len()
    }

    fn column(&self, ctx: &mut Context, i: usize) -> Result<()> {
        assert_eq!(i, 0, "ArrayTab only has one column");
        ctx.set_result(&self.values[self.pos])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::Connection;
    use crate::error::Error;

    #[test]
    fn array_tab_scans_every_element_in_order() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.create_module(
            "ids",
            ArrayTab::new(vec![
                Value::Integer(10),
                Value::Integer(20),
                Value::Integer(30),
            ]),
        )
        .unwrap();

        let values: Vec<i64> = conn
            .query_map("SELECT * FROM ids", |row| row.get(0))
            .unwrap();
        assert_eq!(values, vec![10, 20, 30]);
    }

    #[test]
    fn array_tab_supports_where_filtering_on_its_only_column() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.create_module(
            "ids",
            ArrayTab::new(vec![
                Value::Integer(1),
                Value::Integer(2),
                Value::Integer(3),
            ]),
        )
        .unwrap();

        let values: Vec<i64> = conn
            .query_map("SELECT * FROM ids WHERE value > 1", |row| row.get(0))
            .unwrap();
        assert_eq!(values, vec![2, 3]);
    }

    #[test]
    fn array_tab_holds_a_snapshot_not_a_live_view() {
        let mut conn = Connection::open_in_memory().unwrap();
        let source = vec![Value::Integer(1)];
        conn.create_module("ids", ArrayTab::new(source.clone()))
            .unwrap();
        drop(source);

        let values: Vec<i64> = conn
            .query_map("SELECT * FROM ids", |row| row.get(0))
            .unwrap();
        assert_eq!(values, vec![1]);
    }

    #[test]
    fn array_tab_of_text_values_round_trips() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.create_module(
            "names",
            ArrayTab::new(vec![
                Value::Text("alice".to_string()),
                Value::Text("bob".to_string()),
            ]),
        )
        .unwrap();

        let values: Vec<String> = conn
            .query_map("SELECT * FROM names", |row| row.get(0))
            .unwrap();
        assert_eq!(values, vec!["alice", "bob"]);
    }

    #[test]
    fn empty_array_tab_scans_zero_rows() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.create_module("empty", ArrayTab::new(Vec::new()))
            .unwrap();

        let values: Vec<i64> = conn
            .query_map("SELECT * FROM empty", |row| row.get(0))
            .unwrap();
        assert!(values.is_empty());
    }

    #[test]
    fn array_tab_is_read_only() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.create_module("ids", ArrayTab::new(vec![Value::Integer(1)]))
            .unwrap();

        assert_eq!(
            conn.execute("INSERT INTO ids VALUES (2)"),
            Err(Error::ReadOnlyVirtualTable)
        );
    }
}
