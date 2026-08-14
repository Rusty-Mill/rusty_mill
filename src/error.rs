use std::fmt;

/// Errors produced by this crate's engine and API layer.
#[derive(Debug, PartialEq)]
pub enum Error {
    /// The connection has already been closed.
    ConnectionClosed,
    /// `CREATE TABLE` named a table that already exists.
    TableAlreadyExists(String),
    /// An operation referenced a table that doesn't exist.
    TableNotFound(String),
    /// An `INSERT` row had a different number of values than the table
    /// has columns.
    ColumnCountMismatch { expected: usize, actual: usize },
    /// An expression referenced a column that doesn't exist in scope.
    UnknownColumn(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::ConnectionClosed => write!(f, "connection is already closed"),
            Error::TableAlreadyExists(name) => write!(f, "table {name:?} already exists"),
            Error::TableNotFound(name) => write!(f, "no such table: {name:?}"),
            Error::ColumnCountMismatch { expected, actual } => {
                write!(f, "{actual} values for {expected} columns")
            }
            Error::UnknownColumn(name) => write!(f, "no such column: {name:?}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
