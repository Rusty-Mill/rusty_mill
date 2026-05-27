//! `SqliteStore` — the long-term memory graph (data-model §3). FTS5 lexical
//! recall + typed edges. Semantic (embedding) recall is the Phase-5 DuckDB
//! backend; the `embed` argument is accepted but unused here (lexical fallback).

use std::path::Path;
use std::sync::Mutex;

use async_trait::async_trait;
use rusqlite::Connection;

use super::{MemType, Memory, Store};
use crate::error::ToolError;

/// SQLite-backed [`Store`] over `store.db`.
pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    /// Open (creating if needed) the store DB at `path`.
    pub fn open(path: &Path) -> Result<Self, ToolError> {
        let conn = Connection::open(path)?;
        init_schema(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// In-memory store for tests.
    #[cfg(test)]
    pub fn in_memory() -> Result<Self, ToolError> {
        let conn = Connection::open_in_memory()?;
        init_schema(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
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
         CREATE TABLE IF NOT EXISTS memories (
             id           INTEGER PRIMARY KEY AUTOINCREMENT,
             title        TEXT NOT NULL UNIQUE,
             body         TEXT NOT NULL,
             mem_type     TEXT NOT NULL,
             importance   REAL NOT NULL DEFAULT 0.5,
             validated    INTEGER NOT NULL DEFAULT 0,
             created_ts   REAL NOT NULL,
             last_used_ts REAL NOT NULL,
             use_count    INTEGER NOT NULL DEFAULT 0,
             embedding    BLOB,
             source_ts_lo REAL,
             source_ts_hi REAL
         );
         CREATE INDEX IF NOT EXISTS idx_mem_type ON memories(mem_type);
         CREATE INDEX IF NOT EXISTS idx_mem_importance ON memories(importance);
         CREATE TABLE IF NOT EXISTS memory_edges (
             src_title TEXT NOT NULL,
             dst_title TEXT NOT NULL,
             rel       TEXT NOT NULL,
             PRIMARY KEY (src_title, dst_title, rel)
         );
         CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts
             USING fts5(title, body, content='memories', content_rowid='id');
         CREATE TRIGGER IF NOT EXISTS mem_ai AFTER INSERT ON memories BEGIN
             INSERT INTO memories_fts(rowid, title, body) VALUES (new.id, new.title, new.body);
         END;
         CREATE TRIGGER IF NOT EXISTS mem_ad AFTER DELETE ON memories BEGIN
             INSERT INTO memories_fts(memories_fts, rowid, title, body)
                 VALUES ('delete', old.id, old.title, old.body);
         END;
         CREATE TRIGGER IF NOT EXISTS mem_au AFTER UPDATE ON memories BEGIN
             INSERT INTO memories_fts(memories_fts, rowid, title, body)
                 VALUES ('delete', old.id, old.title, old.body);
             INSERT INTO memories_fts(rowid, title, body) VALUES (new.id, new.title, new.body);
         END;",
    )?;
    Ok(())
}

const COLS: &str =
    "memories.title, memories.body, memories.mem_type, memories.importance, memories.validated, \
     memories.created_ts, memories.last_used_ts, memories.use_count, memories.source_ts_lo, memories.source_ts_hi";

fn row_to_memory(row: &rusqlite::Row<'_>) -> rusqlite::Result<Memory> {
    let mem_type_str: String = row.get(2)?;
    let lo: Option<f64> = row.get(8)?;
    let hi: Option<f64> = row.get(9)?;
    Ok(Memory {
        title: row.get(0)?,
        body: row.get(1)?,
        mem_type: MemType::parse(&mem_type_str).unwrap_or(MemType::Fact),
        importance: row.get::<_, f64>(3)? as f32,
        validated: row.get::<_, i64>(4)? != 0,
        created_ts: row.get(5)?,
        last_used_ts: row.get(6)?,
        use_count: row.get::<_, i64>(7)? as u32,
        source_ts: lo.zip(hi),
        edges: Vec::new(),
    })
}

/// Build a safe FTS5 MATCH expression: each alphanumeric token quoted, OR-joined.
fn match_expr(query: &str) -> Option<String> {
    let tokens: Vec<String> = query
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{t}\""))
        .collect();
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" OR "))
    }
}

#[async_trait]
impl Store for SqliteStore {
    async fn upsert(&self, memory: &Memory) -> Result<(), ToolError> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO memories
                 (title, body, mem_type, importance, validated, created_ts, last_used_ts, use_count, source_ts_lo, source_ts_hi)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(title) DO UPDATE SET
                 body = excluded.body,
                 mem_type = excluded.mem_type,
                 importance = excluded.importance,
                 validated = excluded.validated,
                 last_used_ts = excluded.last_used_ts,
                 use_count = excluded.use_count,
                 source_ts_lo = excluded.source_ts_lo,
                 source_ts_hi = excluded.source_ts_hi",
            rusqlite::params![
                memory.title,
                memory.body,
                memory.mem_type.as_str(),
                memory.importance as f64,
                memory.validated as i64,
                memory.created_ts,
                memory.last_used_ts,
                memory.use_count as i64,
                memory.source_ts.map(|(lo, _)| lo),
                memory.source_ts.map(|(_, hi)| hi),
            ],
        )?;
        conn.execute(
            "DELETE FROM memory_edges WHERE src_title = ?1",
            [&memory.title],
        )?;
        for edge in &memory.edges {
            conn.execute(
                "INSERT OR IGNORE INTO memory_edges (src_title, dst_title, rel) VALUES (?1, ?2, ?3)",
                rusqlite::params![memory.title, edge.to, edge.rel],
            )?;
        }
        Ok(())
    }

    async fn candidates(
        &self,
        query: &str,
        _embed: Option<&[f32]>,
        k: usize,
    ) -> Result<Vec<(Memory, f32)>, ToolError> {
        let Some(expr) = match_expr(query) else {
            return Ok(Vec::new());
        };
        let conn = self.lock();
        let sql = format!(
            "SELECT {COLS}, bm25(memories_fts) AS score
             FROM memories_fts JOIN memories ON memories.id = memories_fts.rowid
             WHERE memories_fts MATCH ?1 ORDER BY score ASC LIMIT ?2"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params![expr, k as i64], |row| {
            // bm25 is lower-is-better; negate so higher = more relevant.
            let score: f64 = row.get(10)?;
            Ok((row_to_memory(row)?, -score as f32))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    async fn neighbors(&self, title: &str) -> Result<Vec<Memory>, ToolError> {
        let conn = self.lock();
        let sql = format!(
            "SELECT {COLS} FROM memories
             WHERE title IN (SELECT dst_title FROM memory_edges WHERE src_title = ?1)"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([title], row_to_memory)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    async fn set_validated(&self, title: &str, validated: bool) -> Result<(), ToolError> {
        let conn = self.lock();
        conn.execute(
            "UPDATE memories SET validated = ?2 WHERE title = ?1",
            rusqlite::params![title, validated as i64],
        )?;
        Ok(())
    }

    async fn skills(&self) -> Result<Vec<Memory>, ToolError> {
        let conn = self.lock();
        let sql = format!("SELECT {COLS} FROM memories WHERE mem_type = 'skill'");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], row_to_memory)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    async fn recent(&self, n: usize) -> Result<Vec<Memory>, ToolError> {
        let conn = self.lock();
        let sql = format!(
            "SELECT {COLS} FROM memories ORDER BY created_ts DESC, memories.id DESC LIMIT ?1"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([n as i64], row_to_memory)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    async fn prune(&self, older_than: f64, importance_below: f32) -> Result<usize, ToolError> {
        let conn = self.lock();
        // Validated skills are exempt (ADR-0011); candidate skills prune normally.
        let removed = conn.execute(
            "DELETE FROM memories
             WHERE NOT (mem_type = 'skill' AND validated = 1)
               AND last_used_ts < ?1 AND importance < ?2",
            rusqlite::params![older_than, importance_below as f64],
        )?;
        conn.execute(
            "DELETE FROM memory_edges
             WHERE src_title NOT IN (SELECT title FROM memories)
                OR dst_title NOT IN (SELECT title FROM memories)",
            [],
        )?;
        Ok(removed)
    }

    async fn remove(&self, title: &str) -> Result<(), ToolError> {
        let conn = self.lock();
        conn.execute("DELETE FROM memories WHERE title = ?1", [title])?;
        conn.execute(
            "DELETE FROM memory_edges WHERE src_title = ?1 OR dst_title = ?1",
            [title],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::Edge;

    fn mem(title: &str, body: &str) -> Memory {
        Memory::new(title, body, MemType::Fact, 100.0)
    }

    #[tokio::test]
    async fn upsert_then_lexical_recall() {
        let s = SqliteStore::in_memory().unwrap();
        s.upsert(&mem("auth module", "the login flow validates tokens"))
            .await
            .unwrap();
        s.upsert(&mem("parser", "tokenizes the grammar input"))
            .await
            .unwrap();

        let hits = s.candidates("login tokens", None, 5).await.unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0].0.title, "auth module");
    }

    #[tokio::test]
    async fn upsert_updates_in_place_and_keeps_edges() {
        let s = SqliteStore::in_memory().unwrap();
        let mut m = mem("a", "first");
        m.edges = vec![Edge {
            to: "b".into(),
            rel: "relates".into(),
        }];
        s.upsert(&m).await.unwrap();
        s.upsert(&mem("b", "neighbor body")).await.unwrap();

        // Update body; recall should find the new text, not the old.
        let mut m2 = mem("a", "completely different content xyzzy");
        m2.edges = m.edges.clone();
        s.upsert(&m2).await.unwrap();

        assert!(s.candidates("first", None, 5).await.unwrap().is_empty());
        assert!(!s.candidates("xyzzy", None, 5).await.unwrap().is_empty());

        let nbrs = s.neighbors("a").await.unwrap();
        assert_eq!(nbrs.len(), 1);
        assert_eq!(nbrs[0].title, "b");
    }

    #[tokio::test]
    async fn prune_exempts_validated_skills() {
        let s = SqliteStore::in_memory().unwrap();
        let mut skill = Memory::new("lesson", "always reproduce first", MemType::Skill, 1.0);
        skill.validated = true;
        skill.importance = 0.1;
        s.upsert(&skill).await.unwrap();
        let mut fact = mem("stale", "old fact");
        fact.importance = 0.1;
        fact.last_used_ts = 1.0;
        s.upsert(&fact).await.unwrap();

        let removed = s.prune(1000.0, 0.5).await.unwrap();
        assert_eq!(removed, 1); // the fact, not the validated skill
        assert!(!s.candidates("reproduce", None, 5).await.unwrap().is_empty());
    }
}
