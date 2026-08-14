//! Pure-Rust, from-scratch SQLite reimplementation aiming for `rusqlite`
//! API parity. See `ARCHITECTURE.md` for the engine/API boundary and
//! `gap-analysis.md` for what's tracked toward that parity target.

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
mod row;
mod rows;
mod serialize;
mod storage;
mod token;
mod tosql;
mod transaction;
mod value;

pub use config::{DbConfig, Limit};
pub use connection::{ColumnMetadata, Connection};
pub use ddl::{parse_create_table, ColumnDef, CreateTable, ParseError};
pub use dml_insert::{parse_insert, Insert};
pub use dml_select::{parse_select, BinaryOp, Expr, Select, SelectColumns};
pub use engine::{
    execute_create_table, execute_insert, execute_select, execute_select_with_functions,
};
pub use error::{Error, OptionalExtension, Result};
pub use eval::{
    evaluate, evaluate_bool, evaluate_bool_with_functions, evaluate_with_functions, ScalarFn,
};
pub use fromsql::{FromSql, FromSqlError, FromSqlResult};
pub use hooks::{CheckpointMode, Wal};
pub use row::{Row, RowIndex};
pub use rows::{AndThenRows, MappedRows, Rows};
pub use serialize::{deserialize as deserialize_database, serialize as serialize_database};
pub use storage::{Database, Table};
pub use token::{tokenize, Token, TokenError};
pub use tosql::ToSql;
pub use transaction::{DropBehavior, Savepoint, Transaction, TransactionBehavior};
pub use value::{Type, Value, ValueRef};
