//! Zed: agent threads in their own SQLite database under `threads/threads.db`.
//!
//! Each row carries a JSON `data` blob holding the messages, plus a `summary`
//! Zed generates itself — which is a better title than anything derivable from
//! the first user line, so it wins when present.
//!
//! Zed has also stored that blob compressed. A row we cannot decode is skipped
//! and the rest of the table is still indexed; it does not fail the source.

use super::{derive_title, extract_text, snapshot, ScanContext, Source};
use crate::model::{Conversation, Message, ParsedConversation, Role, SourceId};
use crate::Result;
use serde_json::Value;
use std::path::PathBuf;

pub struct Zed;

impl Source for Zed {
    fn id(&self) -> SourceId {
        SourceId::Zed
    }

    fn scan(&self, ctx: &mut ScanContext) -> Result<Vec<ParsedConversation>> {
        scan_roots(self.roots(), ctx)
    }
}

pub fn scan_roots(roots: Vec<PathBuf>, ctx: &mut ScanContext) -> Result<Vec<ParsedConversation>> {
    {
        let mut out = Vec::new();
        for db in thread_dbs(roots) {
            if !ctx.should_read(SourceId::Zed, &db) {
                continue;
            }
            let snap = snapshot::Snapshot::take(&db)?;
            let conn = snap.open()?;
            if !snapshot::has_table(&conn, "threads") {
                continue;
            }

            let columns = table_columns(&conn, "threads")?;
            let id_col = pick(&columns, &["id", "thread_id"]).unwrap_or_else(|| "rowid".into());
            let summary_col = pick(&columns, &["summary", "title"]);
            let updated_col = pick(&columns, &["updated_at", "modified_at", "created_at"]);
            let data_col = pick(&columns, &["data", "state", "body"])
                .ok_or_else(|| crate::Error::other("zed threads table has no data column"))?;

            let sql = format!(
                "SELECT {id_col}, {}, {}, {data_col} FROM threads",
                summary_col.clone().unwrap_or_else(|| "NULL".into()),
                updated_col.clone().unwrap_or_else(|| "NULL".into()),
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map([], |row| {
                let id: String = row
                    .get::<_, String>(0)
                    .or_else(|_| row.get::<_, i64>(0).map(|n| n.to_string()))
                    .unwrap_or_default();
                let summary: Option<String> = row.get(1).ok();
                let updated: Option<Value> = row
                    .get::<_, String>(2)
                    .map(Value::String)
                    .or_else(|_| row.get::<_, i64>(2).map(Value::from))
                    .ok();
                let data = match row.get::<_, String>(3) {
                    Ok(s) => s,
                    Err(_) => row
                        .get::<_, Vec<u8>>(3)
                        .map(|b| String::from_utf8_lossy(&b).into_owned())
                        .unwrap_or_default(),
                };
                Ok((id, summary, updated, data))
            })?;

            for (id, summary, updated, data) in rows.flatten() {
                let Ok(json) = serde_json::from_str::<Value>(&data) else {
                    // Compressed or a shape this version does not know.
                    continue;
                };
                let updated_at = updated
                    .as_ref()
                    .and_then(super::parse_timestamp)
                    .unwrap_or(0);
                if let Some(conv) = build(id, summary, updated_at, &json) {
                    if ctx.since.is_none_or(|s| conv.conversation.updated_at >= s) {
                        out.push(conv);
                    }
                }
            }
        }
        Ok(out)
    }
}

fn thread_dbs(roots: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in roots {
        for name in ["threads.db", "threads.sqlite", "threads.db3"] {
            let p = root.join(name);
            if p.exists() {
                out.push(p);
            }
        }
    }
    out
}

fn table_columns(conn: &rusqlite::Connection, table: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
    Ok(rows.flatten().collect())
}

fn pick(columns: &[String], candidates: &[&str]) -> Option<String> {
    candidates
        .iter()
        .find(|c| columns.iter().any(|col| col.eq_ignore_ascii_case(c)))
        .map(|c| (*c).to_string())
}

fn build(
    id: String,
    summary: Option<String>,
    updated_at: i64,
    data: &Value,
) -> Option<ParsedConversation> {
    let raw = data.get("messages")?.as_array()?;

    let mut messages = Vec::new();
    for (i, m) in raw.iter().enumerate() {
        let role = m
            .get("role")
            .and_then(|r| r.as_str())
            .map(Role::parse)
            .unwrap_or(Role::User);
        // Zed splits a message into typed segments.
        let text = m
            .get("segments")
            .or_else(|| m.get("content"))
            .or_else(|| m.get("text"))
            .map(extract_text)
            .unwrap_or_default();
        if text.trim().is_empty() {
            continue;
        }
        messages.push(Message {
            seq: i as i64,
            role,
            text,
            created_at: m
                .get("timestamp")
                .or_else(|| m.get("created_at"))
                .and_then(super::parse_timestamp)
                .unwrap_or(updated_at),
        });
    }
    if messages.is_empty() {
        return None;
    }
    for (i, m) in messages.iter_mut().enumerate() {
        m.seq = i as i64;
    }

    let title = summary
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| derive_title(&messages));
    let started_at = messages
        .iter()
        .map(|m| m.created_at)
        .min()
        .unwrap_or(updated_at);
    let count = messages.len() as i64;

    Some(ParsedConversation {
        conversation: Conversation {
            id: 0,
            source: SourceId::Zed,
            external_id: id,
            title,
            project_path: data
                .get("project_path")
                .or_else(|| data.get("cwd"))
                .and_then(|v| v.as_str())
                .map(str::to_string),
            git_branch: data
                .get("git_branch")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            started_at,
            updated_at: updated_at.max(started_at),
            message_count: count,
        },
        messages,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_db(dir: &std::path::Path, rows: &[(&str, &str, &str, String)]) -> PathBuf {
        let db = dir.join("threads.db");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE threads(id TEXT PRIMARY KEY, summary TEXT, updated_at TEXT, data BLOB)",
        )
        .unwrap();
        for (id, summary, updated, data) in rows {
            conn.execute(
                "INSERT INTO threads VALUES (?1,?2,?3,?4)",
                rusqlite::params![id, summary, updated, data],
            )
            .unwrap();
        }
        db
    }

    #[test]
    fn reads_threads_with_segments_and_summary() {
        let dir = tempfile::tempdir().unwrap();
        make_db(
            dir.path(),
            &[(
                "th-1",
                "Refactor the auth middleware into a shared hook",
                "2026-08-05T12:00:00Z",
                json!({
                    "project_path": "/work/app",
                    "git_branch": "feat/auth",
                    "messages": [
                        {"role":"user","segments":[{"type":"text","text":"refactor this"}]},
                        {"role":"assistant","segments":[{"type":"text","text":"done"}]}
                    ]
                })
                .to_string(),
            )],
        );

        let mut ctx = ScanContext::new(None, true);
        let convs = scan_roots(vec![dir.path().to_path_buf()], &mut ctx).unwrap();
        assert_eq!(convs.len(), 1);
        let c = &convs[0];
        assert_eq!(c.conversation.external_id, "th-1");
        assert_eq!(
            c.conversation.title,
            "Refactor the auth middleware into a shared hook"
        );
        assert_eq!(c.conversation.git_branch.as_deref(), Some("feat/auth"));
        assert_eq!(c.conversation.updated_at, 1_785_931_200);
        assert_eq!(c.messages.len(), 2);
        assert_eq!(c.messages[1].role, Role::Assistant);
    }

    /// A row Zed wrote compressed (or in a future shape) is skipped, and the
    /// readable rows around it still index.
    #[test]
    fn an_undecodable_row_does_not_lose_the_others() {
        let dir = tempfile::tempdir().unwrap();
        make_db(
            dir.path(),
            &[
                ("bad", "", "2026-08-05T12:00:00Z", "\u{1}\u{2}not json".to_string()),
                (
                    "good",
                    "Kept",
                    "2026-08-05T12:00:00Z",
                    json!({"messages":[{"role":"user","segments":[{"type":"text","text":"hello there"}]}]})
                        .to_string(),
                ),
            ],
        );
        let mut ctx = ScanContext::new(None, true);
        let convs = scan_roots(vec![dir.path().to_path_buf()], &mut ctx).unwrap();
        assert_eq!(convs.len(), 1);
        assert_eq!(convs[0].conversation.external_id, "good");
    }
}
