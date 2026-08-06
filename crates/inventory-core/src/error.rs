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

    /// A version of this app encrypted this file differently (SQLCipher, up
    /// to and including 1.0.2) and it can no longer be decrypted — no
    /// OpenSSL is linked any more. Same recovery story as a lost key: the
    /// index is a derived artifact, not the source of truth.
    #[error("{path} was encrypted by an older version of Inventory and cannot be read by this one.\nMove it aside and re-index — your tools' own history is untouched and will be read again.")]
    LegacyIndexFormat { path: PathBuf },

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
