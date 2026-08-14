//! Top-level parameter-binding traits: `BindIndex` (resolve a parameter
//! reference — position or name — to a bound index) and `Params` (bind a
//! whole set of values into a [`Statement`] at once). Part B gap row
//! "Top-level traits: BindIndex, Params, RowIndex, Name,
//! OptionalExtension" — `RowIndex` lives in `row.rs`, `OptionalExtension`
//! in `error.rs` (both already implemented before this module existed;
//! see the parameter-binding ADR `docs/adr/0002-parameter-markers.md`
//! for why parameter binding itself only became possible once this
//! module could exist at all).

use crate::error::{Error, Result};
use crate::statement::Statement;
use crate::tosql::ToSql;

/// Resolves to a 1-based bound-parameter index — either directly
/// (`usize`) or by name (`&str`, via [`Statement::parameter_index`]).
/// Powers [`Statement::bind_parameter`].
pub trait BindIndex {
    fn idx(&self, stmt: &Statement<'_>) -> Result<usize>;
}

impl BindIndex for usize {
    fn idx(&self, _stmt: &Statement<'_>) -> Result<usize> {
        Ok(*self)
    }
}

impl BindIndex for &str {
    fn idx(&self, stmt: &Statement<'_>) -> Result<usize> {
        stmt.parameter_index(self)?
            .ok_or_else(|| Error::UnknownColumn(self.to_string()))
    }
}

/// The name half of a `:name`/`@name`/`$name`-style bound parameter.
///
/// **Provenance caveat:** the original `docs.rs` scan behind this
/// crate's `gap-analysis.md` listed a top-level `Name` trait alongside
/// `BindIndex`/`Params`/`RowIndex`/`OptionalExtension`, without further
/// detail, and no such trait could be confirmed in real `rusqlite`'s
/// current public API while implementing this. This is a best-effort
/// interpretation — "the name half of a named-parameter pair," matching
/// [`BindIndex`]'s `&str` case and useful to the still-unimplemented
/// `named_params!` macro (issue #42) — rather than a byte-for-byte port
/// of something unverified. Revisit if a more precise reference turns up.
pub trait Name {
    fn param_name(&self) -> &str;
}

impl Name for &str {
    fn param_name(&self) -> &str {
        self
    }
}

impl Name for String {
    fn param_name(&self) -> &str {
        self.as_str()
    }
}

/// Binds a whole set of positional (`?`/`?N`) values into a [`Statement`]
/// at once — the counterpart to real `rusqlite::Params`, consumed by
/// [`Statement::execute_with_params`]/[`Statement::query_map_with_params`]/
/// [`crate::Connection::execute_with_params`]/
/// [`crate::Connection::query_map_with_params`].
pub trait Params {
    fn bind_all(self, stmt: &mut Statement<'_>) -> Result<()>;
}

/// No parameters to bind — for a parameter-free statement run through a
/// `_with_params` method (matching real `rusqlite`'s `[]`/`()` no-params
/// convention).
impl Params for () {
    fn bind_all(self, _stmt: &mut Statement<'_>) -> Result<()> {
        Ok(())
    }
}

impl<T: ToSql> Params for &[T] {
    fn bind_all(self, stmt: &mut Statement<'_>) -> Result<()> {
        for (i, value) in self.iter().enumerate() {
            stmt.raw_bind_parameter(i + 1, value.to_sql())?;
        }
        Ok(())
    }
}

impl<T: ToSql, const N: usize> Params for [T; N] {
    fn bind_all(self, stmt: &mut Statement<'_>) -> Result<()> {
        for (i, value) in self.into_iter().enumerate() {
            stmt.raw_bind_parameter(i + 1, value.to_sql())?;
        }
        Ok(())
    }
}

/// Tuples up to 4 elements — an honest, documented cap rather than the
/// arbitrary-arity support a proc macro could give; extend if a real
/// call site needs more.
macro_rules! impl_params_tuple {
    ($($T:ident : $idx:tt),+) => {
        impl<$($T: ToSql),+> Params for ($($T,)+) {
            fn bind_all(self, stmt: &mut Statement<'_>) -> Result<()> {
                $(
                    stmt.raw_bind_parameter($idx + 1, self.$idx.to_sql())?;
                )+
                Ok(())
            }
        }
    };
}

impl_params_tuple!(A: 0);
impl_params_tuple!(A: 0, B: 1);
impl_params_tuple!(A: 0, B: 1, C: 2);
impl_params_tuple!(A: 0, B: 1, C: 2, D: 3);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::Connection;
    use crate::value::Value;

    #[test]
    fn bind_index_resolves_usize_directly() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        let stmt = conn.prepare("SELECT * FROM t WHERE a = ?").unwrap();
        assert_eq!(BindIndex::idx(&1usize, &stmt).unwrap(), 1);
    }

    #[test]
    fn bind_index_resolves_name_via_parameter_index() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        let stmt = conn.prepare("SELECT * FROM t WHERE a = :x").unwrap();
        assert_eq!(BindIndex::idx(&":x", &stmt).unwrap(), 1);
        assert!(BindIndex::idx(&":missing", &stmt).is_err());
    }

    #[test]
    fn name_trait_returns_the_name_text() {
        assert_eq!(Name::param_name(&":foo"), ":foo");
        assert_eq!(Name::param_name(&":bar".to_string()), ":bar");
    }

    #[test]
    fn params_array_binds_positionally() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER, b TEXT)").unwrap();
        let mut stmt = conn.prepare("INSERT INTO t VALUES (?, ?)").unwrap();
        [Value::Integer(1), Value::Text("x".into())]
            .bind_all(&mut stmt)
            .unwrap();
        stmt.execute().unwrap();

        let row = conn.query_row("SELECT * FROM t").unwrap();
        assert_eq!(row, vec![Value::Integer(1), Value::Text("x".into())]);
    }

    #[test]
    fn params_slice_binds_positionally() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        let mut stmt = conn.prepare("INSERT INTO t VALUES (?)").unwrap();
        let values = [7i64];
        (&values[..]).bind_all(&mut stmt).unwrap();
        stmt.execute().unwrap();

        let row = conn.query_row("SELECT * FROM t").unwrap();
        assert_eq!(row, vec![Value::Integer(7)]);
    }

    #[test]
    fn params_tuple_binds_positionally() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER, b TEXT, c INTEGER)")
            .unwrap();
        let mut stmt = conn.prepare("INSERT INTO t VALUES (?, ?, ?)").unwrap();
        (1i64, "x", 3i64).bind_all(&mut stmt).unwrap();
        stmt.execute().unwrap();

        let row = conn.query_row("SELECT * FROM t").unwrap();
        assert_eq!(
            row,
            vec![
                Value::Integer(1),
                Value::Text("x".into()),
                Value::Integer(3)
            ]
        );
    }

    #[test]
    fn empty_params_binds_nothing() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        let mut stmt = conn.prepare("INSERT INTO t VALUES (?)").unwrap();
        ().bind_all(&mut stmt).unwrap();
        stmt.execute().unwrap();

        let row = conn.query_row("SELECT * FROM t").unwrap();
        assert_eq!(row, vec![Value::Null]);
    }
}
