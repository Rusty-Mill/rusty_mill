use crate::ddl::ParseError;
use crate::fromsql::FromSqlError;
use crate::token::TokenError;
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
    /// SQL text failed to tokenize.
    Token(TokenError),
    /// SQL text failed to parse.
    Parse(ParseError),
    /// `Connection::execute` was given SQL that isn't a statement it
    /// currently recognizes (only `CREATE TABLE`/`INSERT` are wired up so
    /// far — see `A8` in `gap-analysis.md`).
    UnrecognizedStatement(String),
    /// `Connection::query_row` found no rows.
    QueryReturnedNoRows,
    /// A [`crate::FromSql`] conversion failed while reading a column.
    FromSql(FromSqlError),
    /// A database-name/index lookup referenced something other than the
    /// single implicit `"main"` database — this crate has no `ATTACH`
    /// support, so no other database ever exists.
    NoSuchDatabase(String),
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
            Error::Token(e) => write!(f, "tokenize error: {e:?}"),
            Error::Parse(e) => write!(f, "parse error: {e:?}"),
            Error::UnrecognizedStatement(sql) => {
                write!(f, "statement not recognized by this connection: {sql:?}")
            }
            Error::QueryReturnedNoRows => write!(f, "query returned no rows"),
            Error::FromSql(e) => write!(f, "column conversion error: {e:?}"),
            Error::NoSuchDatabase(name) => write!(f, "no such database: {name:?}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<TokenError> for Error {
    fn from(e: TokenError) -> Error {
        Error::Token(e)
    }
}

impl From<ParseError> for Error {
    fn from(e: ParseError) -> Error {
        Error::Parse(e)
    }
}

impl From<FromSqlError> for Error {
    fn from(e: FromSqlError) -> Error {
        Error::FromSql(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Turns [`Error::QueryReturnedNoRows`] into `Ok(None)` instead of an
/// error, for callers that treat "no matching row" as a normal outcome
/// rather than a failure. Part B gap row "Top-level traits: BindIndex,
/// Params, RowIndex, Name, OptionalExtension" (the `OptionalExtension`
/// slice — see `row.rs` for `RowIndex`; `BindIndex`/`Params`/`Name` are
/// parameter-binding traits blocked on the same `?`-marker design
/// decision as issue #25).
pub trait OptionalExtension<T> {
    fn optional(self) -> Result<Option<T>>;
}

impl<T> OptionalExtension<T> for Result<T> {
    fn optional(self) -> Result<Option<T>> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod optional_extension_tests {
    use super::*;

    #[test]
    fn ok_becomes_some() {
        let result: Result<i64> = Ok(42);
        assert_eq!(result.optional(), Ok(Some(42)));
    }

    #[test]
    fn no_rows_becomes_none() {
        let result: Result<i64> = Err(Error::QueryReturnedNoRows);
        assert_eq!(result.optional(), Ok(None));
    }

    #[test]
    fn other_errors_pass_through() {
        let result: Result<i64> = Err(Error::ConnectionClosed);
        assert_eq!(result.optional(), Err(Error::ConnectionClosed));
    }
}
