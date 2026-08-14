//! `Row`: a single query-result row, exposing typed column access via
//! [`FromSql`]. Part B gap row "Row accessors (get, get_unwrap, get_ref,
//! get_ref_unwrap, get_pointer)". `get_pointer` is a raw-FFI-handle
//! accessor in `rusqlite` — not applicable here (no C backend to expose a
//! pointer into), so it's intentionally omitted rather than stubbed.

use crate::error::{Error, Result};
use crate::fromsql::FromSql;
use crate::value::{Value, ValueRef};

/// A borrowed view over one result row's column names and values.
#[derive(Debug, Clone, Copy)]
pub struct Row<'a> {
    column_names: &'a [String],
    values: &'a [Value],
}

impl<'a> Row<'a> {
    /// Builds a `Row` over the given column names and values (same
    /// length, same order). Used by the query-execution path to wrap a
    /// single result row.
    pub fn new(column_names: &'a [String], values: &'a [Value]) -> Row<'a> {
        Row {
            column_names,
            values,
        }
    }

    /// Reads column `idx` (by 0-based position) and converts it to `T`.
    pub fn get<T: FromSql>(&self, idx: usize) -> Result<T> {
        let value = self
            .values
            .get(idx)
            .ok_or_else(|| Error::UnknownColumn(format!("column index {idx} out of range")))?;
        Ok(T::column_result(value)?)
    }

    /// Like [`Row::get`], but panics on error instead of returning one —
    /// an ergonomic convenience for tests and quick scripts, mirroring
    /// `rusqlite::Row::get_unwrap`'s documented panicking contract rather
    /// than an accidental `unwrap()` in library logic.
    pub fn get_unwrap<T: FromSql>(&self, idx: usize) -> T {
        self.get(idx).expect("Row::get_unwrap failed")
    }

    /// Reads column `idx` as a borrowed [`ValueRef`], avoiding a clone of
    /// `Text`/`Blob` payloads.
    pub fn get_ref(&self, idx: usize) -> Result<ValueRef<'a>> {
        let value = self
            .values
            .get(idx)
            .ok_or_else(|| Error::UnknownColumn(format!("column index {idx} out of range")))?;
        Ok(value.as_ref())
    }

    /// Like [`Row::get_ref`], but panics on error instead of returning
    /// one.
    pub fn get_ref_unwrap(&self, idx: usize) -> ValueRef<'a> {
        self.get_ref(idx).expect("Row::get_ref_unwrap failed")
    }

    /// Returns the 0-based index of the named column, if present.
    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.column_names.iter().position(|c| c == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row_data() -> (Vec<String>, Vec<Value>) {
        (
            vec!["a".into(), "b".into()],
            vec![Value::Integer(42), Value::Text("hi".into())],
        )
    }

    #[test]
    fn get_converts_typed_column() {
        let (cols, vals) = row_data();
        let row = Row::new(&cols, &vals);
        assert_eq!(row.get::<i64>(0).unwrap(), 42);
        assert_eq!(row.get::<String>(1).unwrap(), "hi");
    }

    #[test]
    fn get_out_of_range_is_an_error() {
        let (cols, vals) = row_data();
        let row = Row::new(&cols, &vals);
        assert!(row.get::<i64>(5).is_err());
    }

    #[test]
    fn get_wrong_type_is_an_error() {
        let (cols, vals) = row_data();
        let row = Row::new(&cols, &vals);
        assert!(row.get::<i64>(1).is_err());
    }

    #[test]
    fn get_unwrap_returns_value() {
        let (cols, vals) = row_data();
        let row = Row::new(&cols, &vals);
        assert_eq!(row.get_unwrap::<i64>(0), 42);
    }

    #[test]
    fn get_ref_borrows_without_cloning() {
        let (cols, vals) = row_data();
        let row = Row::new(&cols, &vals);
        assert_eq!(row.get_ref(1).unwrap(), ValueRef::Text("hi"));
    }

    #[test]
    fn column_index_finds_named_column() {
        let (cols, vals) = row_data();
        let row = Row::new(&cols, &vals);
        assert_eq!(row.column_index("b"), Some(1));
        assert_eq!(row.column_index("z"), None);
    }
}
