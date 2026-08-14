//! Pure-Rust, from-scratch SQLite reimplementation aiming for `rusqlite`
//! API parity. See `ARCHITECTURE.md` for the engine/API boundary and
//! `gap-analysis.md` for what's tracked toward that parity target.

mod connection;
mod ddl;
mod dml_insert;
mod dml_select;
mod error;
mod storage;
mod token;
mod value;

pub use connection::Connection;
pub use ddl::{parse_create_table, ColumnDef, CreateTable, ParseError};
pub use dml_insert::{parse_insert, Insert};
pub use dml_select::{parse_select, BinaryOp, Expr, Select, SelectColumns};
pub use error::{Error, Result};
pub use storage::{Database, Table};
pub use token::{tokenize, Token, TokenError};
pub use value::{Type, Value};
