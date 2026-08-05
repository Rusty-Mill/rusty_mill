use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The index key could not be read. This is deliberately fatal: the
    /// reviewed product stops rather than starting over, so a transient
    /// keychain failure can never be mistaken for "no index yet" and wipe
    /// history that is still perfectly good on disk.
    #[error("index key unavailable: {0}\nInventory stopped rather than rebuilding the index. Nothing was deleted.")]
    KeyUnavailable(String),

    #[error("the index exists but the key does not open it: {0}")]
    KeyMismatch(String),

    /// A source failed to parse. Carried rather than propagated: the indexer
    /// freezes that one source and leaves every other source working.
    // `source` is a reserved field name for thiserror, hence `which`.
    #[error("source `{which}` could not be read: {detail}")]
    SourceUnreadable { which: String, detail: String },

    #[error("{0} is not a file this version knows how to read")]
    UnknownFormat(PathBuf),

    #[error("no conversation with id {0}")]
    NoSuchConversation(i64),

    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn other(msg: impl Into<String>) -> Self {
        Error::Other(msg.into())
    }
}
