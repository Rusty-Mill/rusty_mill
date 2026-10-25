use async_trait::async_trait;
use rusty_stream::{BoxStream, StreamExt};

use crate::error::{Error, Result};
use crate::pool::PoolStats;
use crate::row::Row;
use crate::schema::TableSchema;
use crate::value::Value;

/// The lowest-level operation set every driver must provide: run SQL text
/// with positional parameters, get rows back. Everything above this
/// (the query builder, `Engine`) is built entirely in terms of `Executor`,
/// so a new backend only has to implement this trait plus `Driver`.
#[async_trait]
pub trait Executor: Send {
    /// Run a statement that doesn't return rows (INSERT/UPDATE/DELETE/DDL).
    /// Returns the number of rows affected.
    async fn execute(&mut self, sql: &str, params: &[Value]) -> Result<u64>;

    /// Run a query and collect every row.
    async fn fetch_all(&mut self, sql: &str, params: &[Value]) -> Result<Vec<Row>>;

    /// Run a query expected to return at most one row.
    async fn fetch_optional(&mut self, sql: &str, params: &[Value]) -> Result<Option<Row>>;

    /// Run a query expected to return exactly one row.
    async fn fetch_one(&mut self, sql: &str, params: &[Value]) -> Result<Row> {
        match self.fetch_optional(sql, params).await? {
            Some(row) => Ok(row),
            None => Err(crate::error::Error::RowNotFound),
        }
    }

    /// Like `execute` with no parameters, but explicitly bypassing
    /// whatever prepared-statement machinery the driver would otherwise
    /// use — some administrative statements (MySQL/MariaDB's `XA ...`
    /// transaction-control commands, specifically) don't reliably resolve
    /// a transaction prepared on a different connection when sent through
    /// a driver's binary/prepared-statement protocol, even though the
    /// exact same statement text works when sent as plain text SQL.
    ///
    /// Defaults to `execute`, which is correct for every driver that
    /// doesn't have this quirk; MySQL overrides it.
    async fn execute_unprepared(&mut self, sql: &str) -> Result<u64> {
        self.execute(sql, &[]).await
    }
}

/// A single, live connection to a database. Driver crates implement this
/// over their underlying client (e.g. an `sqlx::PoolConnection`).
#[async_trait]
pub trait Connection: Executor {
    /// Closes this connection outright instead of letting it be reused
    /// (e.g. returned to a driver's connection pool on drop). Needed after
    /// an operation that can leave the underlying session unusable for
    /// anything else — currently: MySQL/MariaDB's `XA PREPARE`, after
    /// which that session refuses any further statement at all (even an
    /// unrelated one, from an unrelated caller who happens to be handed
    /// the same connection back out of the pool) until it resolves its
    /// own prepared transaction itself — exactly the resolution
    /// two-phase commit defers to a possibly-different connection or
    /// process. Defaults to a plain no-op (just drop the connection as
    /// usual), which is correct for every driver without this quirk.
    async fn close(self: Box<Self>) -> Result<()> {
        Ok(())
    }

    /// How many distinct prepared statements are currently cached on this
    /// specific physical connection (see
    /// `PoolConfig::with_statement_cache_capacity`) — a per-connection
    /// number, not a pool-wide one (unlike `Engine::pool_stats()`), since
    /// each connection keeps its own statement cache. Defaults to `0` for
    /// a driver/test double with no statement cache to report.
    fn cached_statement_count(&self) -> usize {
        0
    }

    /// Like `fetch_all`, but yields rows one at a time as they become
    /// available instead of collecting the entire result set first — the
    /// escape hatch for large exports/reports where materializing a full
    /// `Vec<Row>` would be a real memory ceiling. Consumes the connection
    /// (rather than borrowing `&mut self`, like every other method here)
    /// since the returned stream has to keep it alive across the whole
    /// iteration — the same reason `close` above needs `self: Box<Self>`.
    ///
    /// The default implementation here still collects everything up
    /// front (via `fetch_all`) and streams from that `Vec` afterward —
    /// correct, but no different from `fetch_all` memory-wise. The real
    /// driver crates (SQLite/Postgres/MySQL) override this with a
    /// genuine row-at-a-time stream via sqlx's own `Executor::fetch`.
    fn fetch_stream(
        self: Box<Self>,
        sql: String,
        params: Vec<Value>,
    ) -> BoxStream<'static, Result<Row>>
    where
        Self: 'static,
    {
        let mut this = self;
        rusty_stream::try_stream! {
            for row in this.fetch_all(&sql, &params).await? {
                yield row;
            }
        }
        .boxed()
    }
}

/// A source of `Connection`s for one database plus the `Dialect` needed to
/// render portable SQL for it. This is the trait object that lets rusty_db
/// swap backends: an `Engine` just holds an `Arc<dyn Driver>`.
#[async_trait]
pub trait Driver: Send + Sync {
    async fn connect(&self) -> Result<Box<dyn Connection>>;

    fn dialect(&self) -> &dyn crate::dialect::Dialect;

    /// List every user table in the database's default schema/database,
    /// in name order.
    ///
    /// The default implementation reports no schema introspection
    /// support at all; the real driver crates (SQLite/Postgres/MySQL)
    /// override this using their own catalog. A driver that doesn't
    /// override it (e.g. a test double) simply doesn't support
    /// reflection, rather than needing to implement it just to satisfy
    /// the trait.
    async fn list_tables(&self) -> Result<Vec<String>> {
        Err(Error::Unsupported(
            "schema introspection is not implemented for this driver".to_string(),
        ))
    }

    /// Look up one table's columns; `Ok(None)` if no such table exists.
    async fn table_schema(&self, table: &str) -> Result<Option<TableSchema>> {
        let _ = table;
        Err(Error::Unsupported(
            "schema introspection is not implemented for this driver".to_string(),
        ))
    }

    /// A snapshot of this driver's connection pool (see `PoolStats`).
    ///
    /// The default implementation reports all zeros, for a driver with no
    /// real pool concept (e.g. a test double) rather than requiring every
    /// `Driver` to implement it just to satisfy the trait. The real driver
    /// crates (SQLite/Postgres/MySQL) override this using their own
    /// `sqlx::Pool` plus a small amount of their own bookkeeping.
    fn pool_stats(&self) -> PoolStats {
        PoolStats::default()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    /// A connection whose `fetch_all` returns a fixed result, to exercise
    /// the default `fetch_stream`.
    struct Fixed(Option<Result<Vec<Row>>>);

    #[async_trait]
    impl Executor for Fixed {
        async fn execute(&mut self, _sql: &str, _params: &[Value]) -> Result<u64> {
            Ok(0)
        }

        async fn fetch_all(&mut self, _sql: &str, _params: &[Value]) -> Result<Vec<Row>> {
            self.0.take().unwrap_or(Ok(Vec::new()))
        }

        async fn fetch_optional(&mut self, _sql: &str, _params: &[Value]) -> Result<Option<Row>> {
            Ok(None)
        }
    }

    impl Connection for Fixed {}

    fn row(n: i64) -> Row {
        Row::new(Arc::from(["n".to_string()]), vec![Value::I64(n)])
    }

    async fn drain(result: Result<Vec<Row>>) -> Vec<Result<i64>> {
        let stream = Box::new(Fixed(Some(result))).fetch_stream(String::new(), Vec::new());
        stream
            .map(|row| row.and_then(|row| row.get::<i64>(0)))
            .collect()
            .await
    }

    #[rusty_tokio::test]
    async fn default_fetch_stream_yields_every_row_then_ends() {
        let got = drain(Ok(vec![row(1), row(2)])).await;
        assert!(matches!(got.as_slice(), [Ok(1), Ok(2)]), "{got:?}");
    }

    #[rusty_tokio::test]
    async fn default_fetch_stream_yields_the_error_once() {
        let got = drain(Err(Error::RowNotFound)).await;
        assert!(
            matches!(got.as_slice(), [Err(Error::RowNotFound)]),
            "{got:?}"
        );
    }
}
