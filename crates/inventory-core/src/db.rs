//! The single encrypted file everything lives in.

use crate::keychain::KeyProvider;
use crate::{Error, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

pub const SCHEMA_VERSION: i32 = 1;

/// Open (creating if needed) the encrypted index at `path`.
///
/// SQLCipher takes the key before any other statement runs. We then touch a
/// real page to force a decrypt, which is what turns a wrong key into a clear
/// error instead of a confusing "file is not a database" much later.
pub fn open(path: &Path, key: &dyn KeyProvider) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let key_hex = key.get_or_create()?;
    let existed = path.exists();

    let conn = Connection::open(path)?;
    apply_key(&conn, &key_hex)?;

    // Force a decrypt of page 1.
    if let Err(e) = conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
        r.get::<_, i64>(0)
    }) {
        return Err(if existed {
            Error::KeyMismatch(format!(
                "{} exists but the key from {} does not open it: {e}",
                path.display(),
                key.describe()
            ))
        } else {
            Error::Sqlite(e)
        });
    }

    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    migrate(&conn)?;
    Ok(conn)
}

fn apply_key(conn: &Connection, key_hex: &str) -> Result<()> {
    // Raw-key form: SQLCipher takes the 32 bytes verbatim rather than running
    // a KDF over an ASCII passphrase. The key is already full-entropy random.
    let quoted = if key_hex.len() == 64 && key_hex.chars().all(|c| c.is_ascii_hexdigit()) {
        format!("\"x'{key_hex}'\"")
    } else {
        format!("'{}'", key_hex.replace('\'', "''"))
    };
    conn.execute_batch(&format!("PRAGMA key = {quoted};"))?;
    Ok(())
}

/// Is this file encrypted? Used by the conversion path and by `inv doctor`.
/// A plaintext SQLite file starts with the ASCII header `SQLite format 3\0`;
/// an encrypted one starts with random bytes.
pub fn looks_encrypted(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let bytes = std::fs::read(path)?;
    if bytes.len() < 16 {
        return Ok(false);
    }
    Ok(&bytes[..16] != b"SQLite format 3\0")
}

/// Shannon entropy of the file in bits per byte. The security page invites
/// verification of exactly this number, so `inv doctor` reports it.
pub fn shannon_entropy(path: &Path) -> Result<f64> {
    let bytes = std::fs::read(path)?;
    if bytes.is_empty() {
        return Ok(0.0);
    }
    let mut counts = [0u64; 256];
    for b in &bytes {
        counts[*b as usize] += 1;
    }
    let len = bytes.len() as f64;
    let mut h = 0.0;
    for c in counts {
        if c > 0 {
            let p = c as f64 / len;
            h -= p * p.log2();
        }
    }
    Ok(h)
}

/// Convert a plaintext index to an encrypted one.
///
/// "Existing indexes are converted automatically, without touching the
/// original until the new one is proven." The original is only renamed to
/// `.plaintext.bak` after the replacement has been reopened and verified, so
/// an interruption at any point leaves a working index behind.
pub fn convert_plaintext_to_encrypted(
    path: &Path,
    key: &dyn KeyProvider,
) -> Result<Option<PathBuf>> {
    if !path.exists() || looks_encrypted(path)? {
        return Ok(None);
    }
    let key_hex = key.get_or_create()?;
    let staging = path.with_extension("converting");
    let _ = std::fs::remove_file(&staging);

    {
        let conn = Connection::open(path)?;
        let expected: i64 = conn
            .query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))
            .unwrap_or(0);

        conn.execute(
            "ATTACH DATABASE ?1 AS encrypted KEY ?2",
            rusqlite::params![staging.to_string_lossy(), format!("x'{key_hex}'")],
        )?;
        conn.query_row("SELECT sqlcipher_export('encrypted')", [], |_| Ok(()))?;
        conn.execute_batch("DETACH DATABASE encrypted")?;
        drop(conn);

        // Prove the new file before the old one is disturbed.
        let check = Connection::open(&staging)?;
        apply_key(&check, &key_hex)?;
        let got: i64 = check
            .query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))
            .map_err(|e| Error::other(format!("converted index failed verification: {e}")))?;
        if got < expected {
            return Err(Error::other(format!(
                "converted index is missing objects ({got} of {expected}); original left untouched"
            )));
        }
    }

    let backup = path.with_extension("plaintext.bak");
    std::fs::rename(path, &backup)?;
    std::fs::rename(&staging, path)?;
    Ok(Some(backup))
}

fn migrate(conn: &Connection) -> Result<()> {
    let current: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current >= SCHEMA_VERSION {
        return Ok(());
    }

    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS conversations (
            id            INTEGER PRIMARY KEY,
            source        TEXT    NOT NULL,
            external_id   TEXT    NOT NULL,
            title         TEXT    NOT NULL,
            project_path  TEXT,
            git_branch    TEXT,
            started_at    INTEGER NOT NULL,
            updated_at    INTEGER NOT NULL,
            message_count INTEGER NOT NULL DEFAULT 0,
            UNIQUE(source, external_id)
        );
        CREATE INDEX IF NOT EXISTS conversations_updated  ON conversations(updated_at DESC);
        CREATE INDEX IF NOT EXISTS conversations_source   ON conversations(source, updated_at DESC);

        CREATE TABLE IF NOT EXISTS messages (
            id              INTEGER PRIMARY KEY,
            conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
            seq             INTEGER NOT NULL,
            role            TEXT    NOT NULL,
            text            TEXT    NOT NULL,
            created_at      INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS messages_conv ON messages(conversation_id, seq);

        -- Standalone (not external-content) so snippet() and highlight() work
        -- directly. rowid is always the conversation id.
        CREATE VIRTUAL TABLE IF NOT EXISTS conversations_fts USING fts5(
            title,
            body,
            tokenize = 'porter unicode61 remove_diacritics 2'
        );

        -- One row per source file already read, so a re-index reads each file
        -- once: unchanged mtime+size+digest means skip.
        CREATE TABLE IF NOT EXISTS seen_files (
            source TEXT    NOT NULL,
            path   TEXT    NOT NULL,
            mtime  INTEGER NOT NULL,
            size   INTEGER NOT NULL,
            digest TEXT    NOT NULL,
            PRIMARY KEY (source, path)
        );

        CREATE TABLE IF NOT EXISTS source_status (
            source      TEXT PRIMARY KEY,
            state       TEXT    NOT NULL,
            last_ok_at  INTEGER,
            last_error  TEXT,
            frozen_at   INTEGER
        );

        -- Quick capture (⌘⇧N).
        CREATE TABLE IF NOT EXISTS notes (
            id         INTEGER PRIMARY KEY,
            text       TEXT    NOT NULL,
            created_at INTEGER NOT NULL
        );

        -- Clipboard scratchpad (⌘⇧V). Off by default; see settings.
        CREATE TABLE IF NOT EXISTS clips (
            id         INTEGER PRIMARY KEY,
            text       TEXT    NOT NULL,
            app        TEXT,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS clips_recent ON clips(created_at DESC);

        -- Dense vectors for the semantic half of search.
        CREATE TABLE IF NOT EXISTS embeddings (
            conversation_id INTEGER PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,
            model           TEXT NOT NULL,
            vec             BLOB NOT NULL
        );

        -- Trained embedding model state (vocabulary + term vectors).
        CREATE TABLE IF NOT EXISTS embedding_model (
            id         INTEGER PRIMARY KEY CHECK (id = 1),
            kind       TEXT    NOT NULL,
            dim        INTEGER NOT NULL,
            trained_at INTEGER NOT NULL,
            doc_count  INTEGER NOT NULL,
            payload    BLOB    NOT NULL
        );

        CREATE TABLE IF NOT EXISTS settings (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        "#,
    )?;

    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}

pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>> {
    let mut stmt = conn.prepare("SELECT value FROM settings WHERE key = ?1")?;
    let mut rows = stmt.query([key])?;
    Ok(match rows.next()? {
        Some(row) => Some(row.get(0)?),
        None => None,
    })
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO settings(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keychain::StaticKey;

    #[test]
    fn round_trips_through_encryption() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("inventory.sqlite3");
        let key = StaticKey::new("a".repeat(64));

        {
            let conn = open(&path, &key).unwrap();
            conn.execute(
                "INSERT INTO settings(key,value) VALUES ('hello','world')",
                [],
            )
            .unwrap();
        }

        assert!(
            looks_encrypted(&path).unwrap(),
            "index should be encrypted at rest"
        );
        assert!(
            shannon_entropy(&path).unwrap() > 7.5,
            "encrypted file should look random"
        );

        let conn = open(&path, &key).unwrap();
        assert_eq!(
            get_setting(&conn, "hello").unwrap().as_deref(),
            Some("world")
        );
    }

    #[test]
    fn wrong_key_is_an_error_not_a_fresh_index() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("inventory.sqlite3");
        {
            let _ = open(&path, &StaticKey::new("a".repeat(64))).unwrap();
        }
        let err = open(&path, &StaticKey::new("b".repeat(64))).unwrap_err();
        assert!(matches!(err, Error::KeyMismatch(_)), "got {err:?}");
    }

    #[test]
    fn plaintext_index_converts_and_keeps_a_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("inventory.sqlite3");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO settings VALUES ('kept','yes');",
            )
            .unwrap();
        }
        assert!(!looks_encrypted(&path).unwrap());

        let key = StaticKey::new("c".repeat(64));
        let backup = convert_plaintext_to_encrypted(&path, &key)
            .unwrap()
            .unwrap();

        assert!(backup.exists(), "original should be preserved");
        assert!(looks_encrypted(&path).unwrap());
        let conn = open(&path, &key).unwrap();
        assert_eq!(get_setting(&conn, "kept").unwrap().as_deref(), Some("yes"));
    }
}
