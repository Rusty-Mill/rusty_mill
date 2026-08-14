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
mod storage;
mod token;
mod tosql;
mod value;

pub use config::{DbConfig, Limit};
pub use connection::Connection;
pub use ddl::{parse_create_table, ColumnDef, CreateTable, ParseError};
pub use dml_insert::{parse_insert, Insert};
pub use dml_select::{parse_select, BinaryOp, Expr, Select, SelectColumns};
pub use engine::{execute_create_table, execute_insert, execute_select};
pub use error::{Error, Result};
pub use eval::{evaluate, evaluate_bool};
pub use fromsql::{FromSql, FromSqlError, FromSqlResult};
pub use hooks::{CheckpointMode, Wal};
pub use storage::{Database, Table};
pub use token::{tokenize, Token, TokenError};
pub use tosql::ToSql;
pub use value::{Type, Value, ValueRef};
