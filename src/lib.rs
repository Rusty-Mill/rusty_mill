//! Pure-Rust, from-scratch SQLite reimplementation aiming for `rusqlite`
//! API parity. See `ARCHITECTURE.md` for the engine/API boundary and
//! `gap-analysis.md` for what's tracked toward that parity target.

/// The name of the always-present default database (Part B gap row
/// "Top-level: params_from_iter, version, version_number, MAIN_DB/TEMP_DB
/// constants" — the constants slice; `version`/`version_number` need a
/// human decision on versioning semantics, so neither is implemented
/// here. `params_from_iter` was blocked on the parameter-binding design
/// decision issue #25 made (see `docs/adr/0002-parameter-markers.md`
/// and [`crate::Params`]) — no longer blocked, but not yet implemented
/// as its own function; tracked as the rest of issue #43).
pub const MAIN_DB: &str = "main";

/// The name SQLite's temporary-table database would use. This crate has
/// no temporary-table support (no `ATTACH`, no `CREATE TEMP TABLE`), so
/// nothing produces or accepts this name yet — provided for API-shape
/// parity, same status as [`crate::hooks::Wal`]/[`crate::hooks::CheckpointMode`].
pub const TEMP_DB: &str = "temp";

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

pub use aggregate::Aggregate;
pub use blob::{Blob, ZeroBlob};
pub use config::{DbConfig, Limit, OpenFlags};
pub use connection::{ColumnMetadata, Connection};
pub use ddl::{parse_create_table, ColumnDef, CreateTable, ParseError};
pub use dml_insert::{parse_insert, Insert};
pub use dml_select::{
    parse_select, AggregateArg, AggregateCall, BinaryOp, Expr, ParamMarker, Select, SelectColumns,
    WindowCall,
};
pub use engine::{
    execute_create_table, execute_insert, execute_select, execute_select_with_aggregates,
    execute_select_with_functions, execute_select_with_window,
};
pub use error::{Error, OptionalExtension, Result};
pub use eval::{
    evaluate, evaluate_bool, evaluate_bool_with_functions, evaluate_with_functions, ScalarFn,
};
pub use fromsql::{FromSql, FromSqlError, FromSqlResult};
pub use hooks::{Action, AuthContext, Authorization, CheckpointMode, TransactionOperation, Wal};
pub use params::{BindIndex, Name, NamedParams, Params};
pub use row::{Row, RowIndex};
pub use rows::{AndThenRows, MappedRows, Rows};
pub use serialize::{deserialize as deserialize_database, serialize as serialize_database};
pub use statement::{Statement, StatementStatus};
pub use storage::{Database, Table};
pub use token::{tokenize, Token, TokenError};
pub use tosql::ToSql;
pub use trace::{ConnRef, StmtRef, TraceEvent, TraceEventCodes};
pub use transaction::{
    DropBehavior, Savepoint, Transaction, TransactionBehavior, TransactionState,
};
pub use value::{Type, Value, ValueRef};

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
}
