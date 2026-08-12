//! Mirrors the `SearchError` example from
//! <https://github.com/baileyrd/rusty_err/issues/1>: a `#[derive(Error)]`
//! enum with a `#[from]` field for a foreign (`core::error::Error`) type,
//! plus a `BoxError`-backed catch-all variant for boxing arbitrary backend
//! errors the way the issue's `backend_err()` helpers do.

use rusty_err::{BoxError, Error};

#[derive(Debug)]
struct JsonLikeError(&'static str);

impl core::fmt::Display for JsonLikeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "invalid json: {}", self.0)
    }
}

impl core::error::Error for JsonLikeError {}

#[derive(Debug)]
struct SqliteLikeError(&'static str);

impl core::fmt::Display for SqliteLikeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "sqlite failure: {}", self.0)
    }
}

impl core::error::Error for SqliteLikeError {}

#[derive(Debug, Error)]
enum SearchError {
    #[error("index `{0}` not found")]
    IndexNotFound(String),
    #[error("serialization error: {0}")]
    Serialization(#[from] JsonLikeError),
    #[error("backend error: {0}")]
    Backend(BoxError),
}

impl SearchError {
    /// Analogous to the issue's per-backend `backend_err()` helpers: boxes
    /// any sovereign error into the catch-all `Backend` variant.
    fn backend(err: impl Error + 'static) -> Self {
        SearchError::Backend(BoxError::new(err))
    }
}

#[test]
fn index_not_found_display() {
    let err = SearchError::IndexNotFound("main".into());
    assert_eq!(err.to_string(), "index `main` not found");
}

#[test]
fn from_bridges_foreign_error_and_chains_source() {
    let err: SearchError = JsonLikeError("eof").into();
    assert_eq!(err.to_string(), "serialization error: invalid json: eof");
    let source = err.source().expect("source should be present");
    assert_eq!(source.to_string(), "invalid json: eof");
}

#[test]
fn backend_helper_boxes_any_error() {
    let err = SearchError::backend(SqliteLikeError("locked"));
    assert_eq!(err.to_string(), "backend error: sqlite failure: locked");
}

#[test]
fn box_error_downcasts_back_to_backend_type() {
    let err = SearchError::backend(SqliteLikeError("locked"));
    let SearchError::Backend(boxed) = err else {
        panic!("expected Backend variant");
    };
    let original = boxed.downcast::<SqliteLikeError>().expect("downcast");
    assert_eq!(original.0, "locked");
}
