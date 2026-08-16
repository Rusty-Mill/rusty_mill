//! Pure-Rust, from-scratch SQLite reimplementation aiming for `rusqlite`
//! API parity. See `ARCHITECTURE.md` for the engine/API boundary and
//! `gap-analysis.md` for what's tracked toward that parity target.

/// The name of the always-present default database (Part B gap row
/// "Top-level: params_from_iter, version, version_number, MAIN_DB/TEMP_DB
/// constants" — the constants slice; see [`crate::params_from_iter`] and
/// [`version`]/[`version_number`] for the rest of issue #43).
pub const MAIN_DB: &str = "main";

/// The name SQLite's temporary-table database would use. This crate has
/// no temporary-table support (no `ATTACH`, no `CREATE TEMP TABLE`), so
/// nothing produces or accepts this name yet — provided for API-shape
/// parity, same status as [`crate::hooks::Wal`]/[`crate::hooks::CheckpointMode`].
pub const TEMP_DB: &str = "temp";

/// This crate's own Cargo package version (e.g. `"0.0.1"`) — **not** a
/// SQLite library version, since this crate wraps no real SQLite build
/// to report one from. A deliberate human decision (issue #43), not a
/// silently invented number: real `rusqlite::version()` reports the
/// linked SQLite C library's version string, and the closest honest
/// equivalent for a from-scratch engine is this crate's own version.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// [`version`], encoded the way SQLite encodes its own version number —
/// `major * 1_000_000 + minor * 1_000 + patch` (e.g. real SQLite's
/// `3.40.1` becomes `3040001`) — applied here to this crate's own
/// major/minor/patch instead.
pub fn version_number() -> i32 {
    let major: i32 = env!("CARGO_PKG_VERSION_MAJOR").parse().unwrap();
    let minor: i32 = env!("CARGO_PKG_VERSION_MINOR").parse().unwrap();
    let patch: i32 = env!("CARGO_PKG_VERSION_PATCH").parse().unwrap();
    major * 1_000_000 + minor * 1_000 + patch
}

mod aggregate;
mod blob;
mod config;
mod connection;
mod ddl;
mod dml_insert;
mod dml_select;
mod engine;
mod error;
mod eval;
mod fromsql;
mod hooks;
mod macros;
mod params;
mod row;
mod rows;
mod serialize;
mod statement;
mod storage;
mod token;
mod tosql;
mod trace;
mod transaction;
mod value;
mod vtab;
mod vtab_array;
mod vtab_csvtab;
mod vtab_series;

pub use aggregate::Aggregate;
pub use blob::{Blob, ZeroBlob};
pub use config::{DbConfig, Limit, OpenFlags};
pub use connection::{ColumnMetadata, Connection};
pub use ddl::{
    parse_create_table, parse_create_virtual_table, ColumnDef, CreateTable, CreateVirtualTable,
    ParseError,
};
pub use dml_insert::{parse_insert, Insert};
pub use dml_select::{
    parse_select, AggregateArg, AggregateCall, BinaryOp, Expr, ParamMarker, Select, SelectColumns,
    WindowCall,
};
pub use engine::{
    execute_create_table, execute_insert, execute_insert_into_virtual_table, execute_select,
    execute_select_with_aggregates, execute_select_with_functions, execute_select_with_window,
};
pub use error::{Error, OptionalExtension, Result};
pub use eval::{
    evaluate, evaluate_bool, evaluate_bool_with_functions, evaluate_with_functions, ScalarFn,
};
pub use fromsql::{FromSql, FromSqlError, FromSqlResult};
pub use hooks::{Action, AuthContext, Authorization, CheckpointMode, TransactionOperation, Wal};
pub use params::{params_from_iter, BindIndex, Name, NamedParams, Params, ParamsFromIter};
pub use row::{Row, RowIndex};
pub use rows::{AndThenRows, MappedRows, Rows};
pub use serialize::{deserialize as deserialize_database, serialize as serialize_database};
pub use statement::{Statement, StatementStatus};
pub use storage::{Database, Table, TableSource};
pub use token::{tokenize, Token, TokenError};
pub use tosql::ToSql;
pub use trace::{ConnRef, StmtRef, TraceEvent, TraceEventCodes};
pub use transaction::{
    DropBehavior, Savepoint, Transaction, TransactionBehavior, TransactionState,
};
pub use value::{Type, Value, ValueRef};
pub use vtab::{
    dequote, escape_double_quote, parameter, parse_boolean, Context, CreateVTab, TransactionVTab,
    UpdateVTab, VTab, VTabCursor, VTabTableSource,
};
pub use vtab_array::{ArrayCursor, ArrayTab};
pub use vtab_csvtab::{CsvCursor, CsvTab};
pub use vtab_series::{SeriesCursor, SeriesTab};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_db_and_temp_db_have_the_expected_names() {
        assert_eq!(MAIN_DB, "main");
        assert_eq!(TEMP_DB, "temp");
    }

    #[test]
    fn main_db_matches_connection_db_name() {
        let conn = Connection::open_in_memory().unwrap();
        assert_eq!(conn.db_name(0).unwrap(), MAIN_DB);
        assert!(!conn.is_readonly(MAIN_DB).unwrap());
    }

    #[test]
    fn version_matches_this_crates_own_cargo_version() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn version_number_encodes_major_minor_patch() {
        let major: i32 = env!("CARGO_PKG_VERSION_MAJOR").parse().unwrap();
        let minor: i32 = env!("CARGO_PKG_VERSION_MINOR").parse().unwrap();
        let patch: i32 = env!("CARGO_PKG_VERSION_PATCH").parse().unwrap();
        assert_eq!(version_number(), major * 1_000_000 + minor * 1_000 + patch);
    }
}
