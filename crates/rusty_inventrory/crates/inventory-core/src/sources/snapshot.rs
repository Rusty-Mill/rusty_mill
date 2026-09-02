//! Private snapshots of live SQLite stores.
//!
//! "It reads live databases through a private snapshot so indexing can never
//! interfere." Editors keep these databases open in WAL mode; attaching to
//! one directly risks lock contention with a running app and, on a read-only
//! open, SQLite may refuse to recover the WAL at all — which would silently
//! hide the most recent conversations, the ones the user most wants.
//!
//! Copying the file set and opening the copy sidesteps both. The original is
//! only ever read.

use crate::Result;
use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};

pub struct Snapshot {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

impl Snapshot {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Copy `db` and its sidecar files into a temporary directory.
    pub fn take(db: &Path) -> Result<Snapshot> {
        let dir = tempfile::Builder::new()
            .prefix("inventory-snap")
            .tempdir()?;
        let name = db
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("store.db"));
        let dest = dir.path().join(name);

        std::fs::copy(db, &dest)?;
        // -wal holds commits not yet checkpointed; without it a snapshot can
        // be arbitrarily stale. -shm is index-only but keeps SQLite happy.
        for suffix in ["-wal", "-shm"] {
            let side = with_suffix(db, suffix);
            if side.exists() {
                let _ = std::fs::copy(&side, with_suffix(&dest, suffix));
            }
        }

        Ok(Snapshot {
            _dir: dir,
            path: dest,
        })
    }

    /// Open the snapshot. Writable because SQLite needs to replay the copied
    /// WAL — into our throwaway copy, never the original.
    pub fn open(&self) -> Result<Connection> {
        Ok(Connection::open(&self.path)?)
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

/// Open a database directly, read-only. Used for stores that are not held
/// open by a running app; callers fall back to a snapshot on failure.
pub fn open_read_only(db: &Path) -> Result<Connection> {
    Ok(Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?)
}

/// Does this table exist in the connected database?
pub fn has_table(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type IN ('table','view') AND name = ?1",
        [table],
        |_| Ok(()),
    )
    .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_reads_data_without_touching_the_original() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.vscdb");
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE ItemTable(key TEXT PRIMARY KEY, value BLOB);
                 INSERT INTO ItemTable VALUES ('a', 'b');",
            )
            .unwrap();
        }
        let before = std::fs::metadata(&db).unwrap().len();

        let snap = Snapshot::take(&db).unwrap();
        let conn = snap.open().unwrap();
        assert!(has_table(&conn, "ItemTable"));
        let v: String = conn
            .query_row("SELECT value FROM ItemTable WHERE key='a'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(v, "b");

        assert_ne!(snap.path(), db.as_path());
        assert_eq!(std::fs::metadata(&db).unwrap().len(), before);
    }

    #[test]
    fn snapshot_captures_uncheckpointed_wal_content() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("wal.db");
        let conn = Connection::open(&db).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.execute_batch("CREATE TABLE t(x TEXT); INSERT INTO t VALUES ('recent');")
            .unwrap();
        // Deliberately do not checkpoint or close: this is the live-editor case.

        let snap = Snapshot::take(&db).unwrap();
        let read = snap.open().unwrap();
        let x: String = read.query_row("SELECT x FROM t", [], |r| r.get(0)).unwrap();
        assert_eq!(x, "recent", "snapshot missed data still in the WAL");
    }
}
