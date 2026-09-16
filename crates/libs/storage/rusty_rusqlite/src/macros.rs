//! Ergonomic parameter-binding macros (Part B gap row "Ergonomic macros:
//! params!, named_params!, prepare_and_bind!, prepare_cached_and_bind!").
//! Built on [`crate::Params`]/[`crate::BindIndex`] (issue #44) and
//! [`crate::Statement::raw_bind_parameter`] (issue #25) — see
//! `docs/adr/0002-parameter-markers.md`.

/// Builds a [`crate::Params`] value from a positional argument list, for
/// `stmt.execute_with_params(params![1, "x"])`-style call sites.
/// `params![]` (no arguments) uses the `Params for ()` impl.
#[macro_export]
macro_rules! params {
    () => {
        ()
    };
    ($($arg:expr),+ $(,)?) => {
        [$($crate::ToSql::to_sql(&$arg)),+]
    };
}

/// Builds a [`crate::NamedParams`] value from `name => value` pairs, for
/// named-parameter statements
/// (`stmt.execute_with_params(named_params![":x" => 1])`).
///
/// **Syntax deviation, stated plainly:** real `rusqlite::named_params!`
/// separates each pair with `:` (`named_params!{":x": 1}`); this crate
/// uses `=>`. `macro_rules!` only allows a fixed set of tokens (`=>`,
/// `,`, `;`) to follow an `expr` fragment in a matcher, and `:` isn't
/// one of them — `=>` sidesteps that restriction directly rather than
/// routing the parameter name through a narrower fragment specifier to
/// try to make `:` work.
#[macro_export]
macro_rules! named_params {
    () => {
        $crate::NamedParams(&[])
    };
    ($($name:expr => $value:expr),+ $(,)?) => {
        $crate::NamedParams(&[$(($name, $crate::ToSql::to_sql(&$value))),+])
    };
}

/// Prepares `sql` on `conn`, binds `$($arg),*` positionally (via
/// [`params!`]), and evaluates to the bound [`crate::Statement`].
/// Expands to a plain block (not a closure) so the `Statement` it
/// produces — which borrows from `conn` — can escape into the calling
/// scope; the `?` inside it propagates through whatever function the
/// macro is invoked in, which must therefore return a `Result`
/// compatible with [`crate::Error`].
#[macro_export]
macro_rules! prepare_and_bind {
    ($conn:expr, $sql:expr $(, $arg:expr)* $(,)?) => {{
        let mut stmt = $conn.prepare($sql)?;
        $crate::Params::bind_all($crate::params![$($arg),*], &mut stmt)?;
        stmt
    }};
}

/// Like [`prepare_and_bind!`], named for parity with real
/// `rusqlite::prepare_cached_and_bind!`. **No actual caching happens:**
/// this crate has no prepared-statement cache to consult (see
/// [`crate::Connection::set_prepared_statement_cache_capacity`]'s doc
/// comment) — identical to [`prepare_and_bind!`] until one exists.
#[macro_export]
macro_rules! prepare_cached_and_bind {
    ($conn:expr, $sql:expr $(, $arg:expr)* $(,)?) => {
        $crate::prepare_and_bind!($conn, $sql $(, $arg)*)
    };
}

#[cfg(test)]
mod tests {
    use crate::connection::Connection;
    use crate::error::Result;
    use crate::value::Value;

    #[test]
    fn params_macro_builds_a_positional_array() -> Result<()> {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER, b TEXT)").unwrap();
        let mut stmt = conn.prepare("INSERT INTO t VALUES (?, ?)").unwrap();
        stmt.execute_with_params(params![1i64, "x"])?;

        let row = conn.query_row("SELECT * FROM t").unwrap();
        assert_eq!(row, vec![Value::Integer(1), Value::Text("x".into())]);
        Ok(())
    }

    #[test]
    fn params_macro_with_no_arguments() -> Result<()> {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        let mut stmt = conn.prepare("INSERT INTO t VALUES (1)").unwrap();
        stmt.execute_with_params(params![])?;

        let row = conn.query_row("SELECT * FROM t").unwrap();
        assert_eq!(row, vec![Value::Integer(1)]);
        Ok(())
    }

    #[test]
    fn named_params_macro_binds_by_name() -> Result<()> {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1), (2), (3)").unwrap();

        let mut stmt = conn.prepare("SELECT * FROM t WHERE a = :target").unwrap();
        let values: Vec<i64> =
            stmt.query_map_with_params(named_params![":target" => 2i64], |row| row.get(0))?;
        assert_eq!(values, vec![2]);
        Ok(())
    }

    #[test]
    fn prepare_and_bind_macro_prepares_and_binds_in_one_call() -> Result<()> {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER, b TEXT)").unwrap();

        let mut stmt = prepare_and_bind!(conn, "INSERT INTO t VALUES (?, ?)", 1i64, "x");
        stmt.execute()?;

        let row = conn.query_row("SELECT * FROM t").unwrap();
        assert_eq!(row, vec![Value::Integer(1), Value::Text("x".into())]);
        Ok(())
    }

    #[test]
    fn prepare_and_bind_macro_with_no_extra_args() -> Result<()> {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();

        let mut stmt = prepare_and_bind!(conn, "CREATE TABLE t2 (a INTEGER)");
        stmt.execute()?;
        assert!(conn.table_exists("t2"));
        Ok(())
    }

    #[test]
    fn prepare_cached_and_bind_macro_behaves_like_prepare_and_bind() -> Result<()> {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();

        let mut stmt = prepare_cached_and_bind!(conn, "INSERT INTO t VALUES (?)", 5i64);
        stmt.execute()?;

        let row = conn.query_row("SELECT * FROM t").unwrap();
        assert_eq!(row, vec![Value::Integer(5)]);
        Ok(())
    }
}
