//! `SqliteStream` — the short-term observation log (data-model §2). WAL mode,
//! scoped to one `session_id`.

use std::path::Path;
use std::sync::Mutex;

use async_trait::async_trait;
use rusqlite::Connection;

use super::{Observation, Stream};
use crate::error::ToolError;

/// SQLite-backed [`Stream`] over `stream.db`.
pub struct SqliteStream {
    conn: Mutex<Connection>,
    session_id: String,
}

impl SqliteStream {
    /// Open (creating if needed) the stream DB at `path` for `session_id`.
    pub fn open(path: &Path, session_id: impl Into<String>) -> Result<Self, ToolError> {
        let conn = Connection::open(path)?;
        init_schema(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            session_id: session_id.into(),
        })
    }

    /// In-memory stream for tests.
    #[cfg(test)]
    pub fn in_memory(session_id: impl Into<String>) -> Result<Self, ToolError> {
        let conn = Connection::open_in_memory()?;
        init_schema(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            session_id: session_id.into(),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }
}

fn init_schema(conn: &Connection) -> Result<(), ToolError> {
    conn.execute_batch(
        "PRAGMA user_version = 1;
         PRAGMA journal_mode = WAL;
         PRAGMA busy_timeout = 5000;
         CREATE TABLE IF NOT EXISTS observations (
             id          INTEGER PRIMARY KEY AUTOINCREMENT,
             session_id  TEXT NOT NULL,
             ts          REAL NOT NULL,
             role        TEXT NOT NULL,
             kind        TEXT NOT NULL,
             content     TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_obs_session_ts ON observations(session_id, ts);
         CREATE INDEX IF NOT EXISTS idx_obs_kind ON observations(kind);",
    )?;
    Ok(())
}

fn row_to_obs(row: &rusqlite::Row<'_>) -> rusqlite::Result<Observation> {
    Ok(Observation {
        ts: row.get(0)?,
        role: row.get(1)?,
        kind: row.get(2)?,
        content: row.get(3)?,
    })
}

#[async_trait]
impl Stream for SqliteStream {
    async fn append(&self, obs: &Observation) -> Result<(), ToolError> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO observations (session_id, ts, role, kind, content)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![self.session_id, obs.ts, obs.role, obs.kind, obs.content],
        )?;
        Ok(())
    }

    async fn recent(&self, n: usize) -> Result<Vec<Observation>, ToolError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT ts, role, kind, content FROM observations
             WHERE session_id = ?1 ORDER BY ts DESC, id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![self.session_id, n as i64], row_to_obs)?;
        let mut out: Vec<Observation> = rows.collect::<rusqlite::Result<_>>()?;
        out.reverse(); // oldest-first
        Ok(out)
    }

    async fn since(&self, ts: f64) -> Result<Vec<Observation>, ToolError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT ts, role, kind, content FROM observations
             WHERE session_id = ?1 AND ts >= ?2 ORDER BY ts ASC, id ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![self.session_id, ts], row_to_obs)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(ts: f64, content: &str) -> Observation {
        Observation {
            ts,
            role: "user".into(),
            kind: "message".into(),
            content: content.into(),
        }
    }

    #[tokio::test]
    async fn append_and_recent_oldest_first() {
        let s = SqliteStream::in_memory("s1").unwrap();
        s.append(&obs(1.0, "a")).await.unwrap();
        s.append(&obs(2.0, "b")).await.unwrap();
        s.append(&obs(3.0, "c")).await.unwrap();

        let recent = s.recent(2).await.unwrap();
        assert_eq!(
            recent
                .iter()
                .map(|o| o.content.as_str())
                .collect::<Vec<_>>(),
            ["b", "c"]
        );
    }

    #[tokio::test]
    async fn since_filters_by_ts_and_session() {
        let s = SqliteStream::in_memory("s1").unwrap();
        s.append(&obs(1.0, "old")).await.unwrap();
        s.append(&obs(5.0, "new")).await.unwrap();
        let got = s.since(2.0).await.unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].content, "new");
    }
}
