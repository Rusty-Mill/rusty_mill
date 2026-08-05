//! Shared reader for the VS Code forks: Cursor, Kiro and Antigravity.
//!
//! All three inherit Code's storage layout — a `state.vscdb` SQLite file with
//! a `key`/`value` table, one global and one per workspace — and all three
//! stuff their chat history into JSON blobs under keys they rename freely
//! between releases.
//!
//! So the extractor here is structural rather than schema-bound: it walks the
//! JSON looking for anything shaped like a conversation (an array of things
//! that have a speaker and some text) instead of reaching for known key
//! paths. That is what lets a rename ship without taking the source down, and
//! it is the mechanism behind "survives a tool changing its storage format".

use super::{derive_title, snapshot, ScanContext};
use crate::model::{Conversation, Message, ParsedConversation, Role, SourceId};
use crate::Result;
use serde_json::Value;
use std::path::PathBuf;

/// Arrays with these names are candidate message lists.
const MESSAGE_ARRAY_KEYS: &[&str] = &[
    "bubbles",
    "messages",
    "conversation",
    "turns",
    "exchanges",
    "requests",
    "history",
    "items",
];

const TITLE_KEYS: &[&str] = &["chatTitle", "title", "name", "summary", "customTitle"];
const ID_KEYS: &[&str] = &[
    "tabId",
    "composerId",
    "sessionId",
    "id",
    "chatId",
    "threadId",
];
const TIME_KEYS: &[&str] = &[
    "lastUpdatedAt",
    "updatedAt",
    "createdAt",
    "timestamp",
    "lastSendTime",
    "creationDate",
];

pub fn scan_fork(
    source: SourceId,
    roots: Vec<PathBuf>,
    ctx: &mut ScanContext,
) -> Result<Vec<ParsedConversation>> {
    let mut out = Vec::new();
    for db in store_files(&roots) {
        if !ctx.should_read(source, &db) {
            continue;
        }
        // A store we cannot open at all is worth surfacing: it freezes the
        // source rather than silently returning nothing.
        let snap = snapshot::Snapshot::take(&db)?;
        let conn = snap.open()?;

        // A store that exists but whose schema cannot be read is the
        // format-changed/corrupted case. Reporting it as an error freezes the
        // source; returning an empty list would look like "you have no
        // conversations", which is the outcome the freeze exists to prevent.
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
            r.get::<_, i64>(0)
        })
        .map_err(|e| crate::Error::SourceUnreadable {
            which: source.slug().to_string(),
            detail: format!("{} could not be opened: {e}", db.display()),
        })?;

        for table in ["ItemTable", "cursorDiskKV"] {
            if !snapshot::has_table(&conn, table) {
                continue;
            }
            let sql = format!("SELECT key, value FROM {table}");
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map([], |row| {
                let key: String = row.get(0)?;
                // Values are TEXT in some versions and BLOB in others.
                let value = match row.get::<_, String>(1) {
                    Ok(s) => s,
                    Err(_) => row
                        .get::<_, Vec<u8>>(1)
                        .map(|b| String::from_utf8_lossy(&b).into_owned())
                        .unwrap_or_default(),
                };
                Ok((key, value))
            })?;

            for row in rows.flatten() {
                let (key, value) = row;
                if value.len() < 32 {
                    continue;
                }
                let Ok(json) = serde_json::from_str::<Value>(&value) else {
                    continue;
                };
                out.extend(extract_conversations(source, &key, &json));
            }
        }
    }

    // Later stores can repeat a conversation the global store already had.
    out.sort_by(|a, b| {
        a.conversation
            .external_id
            .cmp(&b.conversation.external_id)
            .then(
                b.conversation
                    .message_count
                    .cmp(&a.conversation.message_count),
            )
    });
    out.dedup_by(|a, b| a.conversation.external_id == b.conversation.external_id);

    if let Some(since) = ctx.since {
        out.retain(|c| c.conversation.updated_at >= since);
    }
    Ok(out)
}

pub(crate) fn store_files(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in roots {
        let global = root.join("globalStorage").join("state.vscdb");
        if global.exists() {
            out.push(global);
        }
        let ws = root.join("workspaceStorage");
        if let Ok(entries) = std::fs::read_dir(&ws) {
            for e in entries.flatten() {
                let candidate = e.path().join("state.vscdb");
                if candidate.exists() {
                    out.push(candidate);
                }
            }
        }
    }
    out
}

/// Walk a JSON value and pull out everything conversation-shaped.
pub fn extract_conversations(source: SourceId, key: &str, root: &Value) -> Vec<ParsedConversation> {
    let mut found = Vec::new();
    walk(source, key, root, &mut found, 0);
    found
}

fn walk(
    source: SourceId,
    key: &str,
    value: &Value,
    out: &mut Vec<ParsedConversation>,
    depth: usize,
) {
    if depth > 8 {
        return;
    }
    match value {
        Value::Array(items) => {
            for item in items {
                walk(source, key, item, out, depth + 1);
            }
        }
        Value::Object(map) => {
            for name in MESSAGE_ARRAY_KEYS {
                let Some(Value::Array(items)) = map.get(*name) else {
                    continue;
                };
                if let Some(conv) = build_conversation(source, key, map, items, out.len()) {
                    out.push(conv);
                }
            }
            for (_, v) in map {
                walk(source, key, v, out, depth + 1);
            }
        }
        _ => {}
    }
}

fn build_conversation(
    source: SourceId,
    key: &str,
    container: &serde_json::Map<String, Value>,
    items: &[Value],
    ordinal: usize,
) -> Option<ParsedConversation> {
    let mut messages = Vec::new();
    for item in items {
        extract_messages(item, &mut messages);
    }
    messages.retain(|m: &Message| !m.text.trim().is_empty());
    if messages.is_empty() {
        return None;
    }
    for (i, m) in messages.iter_mut().enumerate() {
        m.seq = i as i64;
    }

    let external_id = ID_KEYS
        .iter()
        .find_map(|k| container.get(*k).and_then(value_as_id))
        .unwrap_or_else(|| format!("{key}#{ordinal}"));

    let timestamp = TIME_KEYS
        .iter()
        .find_map(|k| container.get(*k).and_then(super::parse_timestamp))
        .unwrap_or(0);

    let title = TITLE_KEYS
        .iter()
        .find_map(|k| {
            container
                .get(*k)
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| derive_title(&messages));

    let message_times: Vec<i64> = messages
        .iter()
        .map(|m| m.created_at)
        .filter(|t| *t > 0)
        .collect();
    let started_at = message_times.iter().copied().min().unwrap_or(timestamp);
    let updated_at = message_times
        .iter()
        .copied()
        .max()
        .unwrap_or(timestamp)
        .max(timestamp);
    for m in messages.iter_mut() {
        if m.created_at == 0 {
            m.created_at = started_at;
        }
    }

    let count = messages.len() as i64;
    Some(ParsedConversation {
        conversation: Conversation {
            id: 0,
            source,
            external_id,
            title,
            project_path: container
                .get("workspaceFolder")
                .or_else(|| container.get("cwd"))
                .or_else(|| container.get("folder"))
                .and_then(|v| v.as_str())
                .map(str::to_string),
            git_branch: None,
            started_at,
            updated_at,
            message_count: count,
        },
        messages,
    })
}

fn value_as_id(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// One element of a message array. Handles both the "one entry per message"
/// shape and Code's "request carries its own response" shape.
fn extract_messages(item: &Value, out: &mut Vec<Message>) {
    let Some(map) = item.as_object() else {
        if let Value::String(s) = item {
            if !s.trim().is_empty() {
                out.push(Message {
                    seq: 0,
                    role: Role::User,
                    text: s.clone(),
                    created_at: 0,
                });
            }
        }
        return;
    };

    let created_at = TIME_KEYS
        .iter()
        .find_map(|k| map.get(*k).and_then(super::parse_timestamp))
        .unwrap_or(0);

    // Request/response pair: emit two messages.
    if let (Some(req), Some(resp)) = (map.get("message"), map.get("response")) {
        let q = super::extract_text(req);
        let a = super::extract_text(resp);
        if !q.is_empty() {
            out.push(Message {
                seq: 0,
                role: Role::User,
                text: q,
                created_at,
            });
        }
        if !a.is_empty() {
            out.push(Message {
                seq: 0,
                role: Role::Assistant,
                text: a,
                created_at,
            });
        }
        return;
    }

    let role = role_of(map);
    let text = ["text", "content", "richText", "message", "value", "parts"]
        .iter()
        .map(|k| map.get(*k).map(super::extract_text).unwrap_or_default())
        .find(|t| !t.trim().is_empty())
        .unwrap_or_default();

    if !text.trim().is_empty() {
        out.push(Message {
            seq: 0,
            role,
            text,
            created_at,
        });
    }
}

/// Speaker attribution — "knows who said what". Each fork encodes it
/// differently: a string role, Cursor's numeric bubble `type` (1 = user,
/// 2 = assistant), or a boolean flag.
fn role_of(map: &serde_json::Map<String, Value>) -> Role {
    for k in ["role", "author", "speaker", "sender"] {
        if let Some(s) = map.get(k).and_then(|v| v.as_str()) {
            return Role::parse(s);
        }
    }
    if let Some(t) = map.get("type") {
        match t {
            Value::String(s) => return Role::parse(s),
            Value::Number(n) => {
                return match n.as_i64() {
                    Some(1) => Role::User,
                    Some(2) => Role::Assistant,
                    _ => Role::System,
                }
            }
            _ => {}
        }
    }
    for k in ["isUser", "fromUser", "isUserMessage"] {
        if let Some(b) = map.get(k).and_then(|v| v.as_bool()) {
            return if b { Role::User } else { Role::Assistant };
        }
    }
    Role::User
}

/// Build a `state.vscdb` for tests.
#[cfg(test)]
pub fn write_test_store(dir: &std::path::Path, rows: &[(&str, &str)]) -> PathBuf {
    let global = dir.join("globalStorage");
    std::fs::create_dir_all(&global).unwrap();
    let db = global.join("state.vscdb");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch("CREATE TABLE ItemTable(key TEXT PRIMARY KEY, value BLOB)")
        .unwrap();
    for (k, v) in rows {
        conn.execute(
            "INSERT INTO ItemTable VALUES (?1, ?2)",
            rusqlite::params![k, v],
        )
        .unwrap();
    }
    db
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_cursor_style_bubbles_with_numeric_roles() {
        let blob = json!({
            "tabs": [{
                "tabId": "tab-1",
                "chatTitle": "Git remote setup and waitlist",
                "lastUpdatedAt": 1785931200000i64,
                "bubbles": [
                    {"type": 1, "text": "how do I add a git remote"},
                    {"type": 2, "text": "Use git remote add origin <url>."}
                ]
            }]
        });
        let convs = extract_conversations(SourceId::Cursor, "workbench.panel.aichat", &blob);
        assert_eq!(convs.len(), 1);
        let c = &convs[0];
        assert_eq!(c.conversation.external_id, "tab-1");
        assert_eq!(c.conversation.title, "Git remote setup and waitlist");
        assert_eq!(c.conversation.updated_at, 1_785_931_200);
        assert_eq!(c.messages[0].role, Role::User);
        assert_eq!(c.messages[1].role, Role::Assistant);
    }

    #[test]
    fn extracts_the_request_response_shape() {
        let blob = json!({
            "sessionId": "s-7",
            "requests": [
                {"message": {"text": "why is the build red"},
                 "response": [{"value": "A feature flag is missing."}],
                 "timestamp": 1785931200000i64}
            ]
        });
        let convs = extract_conversations(SourceId::Kiro, "chat.sessions", &blob);
        assert_eq!(convs.len(), 1);
        assert_eq!(convs[0].messages.len(), 2);
        assert_eq!(convs[0].messages[0].role, Role::User);
        assert_eq!(convs[0].messages[0].text, "why is the build red");
        assert_eq!(convs[0].messages[1].role, Role::Assistant);
        assert_eq!(convs[0].messages[1].text, "A feature flag is missing.");
    }

    #[test]
    fn extracts_string_roles_and_derives_a_title() {
        let blob = json!({
            "id": "c9",
            "messages": [
                {"role": "user", "content": "Antigravity truncates tool arguments at 256 bytes"},
                {"role": "assistant", "content": "That is a known limit."}
            ]
        });
        let convs = extract_conversations(SourceId::Antigravity, "agent.store", &blob);
        assert_eq!(convs.len(), 1);
        assert_eq!(
            convs[0].conversation.title,
            "Antigravity truncates tool arguments at 256 bytes"
        );
    }

    /// A renamed container key must not stop extraction: the walker finds the
    /// conversation by shape, wherever it is nested.
    #[test]
    fn finds_conversations_under_unfamiliar_keys() {
        let blob = json!({
            "someFutureWrapper": {
                "v2": {
                    "entries": [{
                        "id": "deep-1",
                        "messages": [{"role": "user", "content": "still findable"}]
                    }]
                }
            }
        });
        let convs = extract_conversations(SourceId::Cursor, "unknown.key", &blob);
        assert_eq!(convs.len(), 1);
        assert_eq!(convs[0].conversation.external_id, "deep-1");
    }

    #[test]
    fn ignores_arrays_that_are_not_conversations() {
        let blob = json!({"messages": [1, 2, 3], "items": [{"colour": "red"}]});
        assert!(extract_conversations(SourceId::Cursor, "k", &blob).is_empty());
    }

    #[test]
    fn reads_an_end_to_end_store_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("User");
        std::fs::create_dir_all(&user).unwrap();
        write_test_store(
            &user,
            &[(
                "workbench.panel.aichat.view.aichat.chatdata",
                &json!({"tabs":[{"tabId":"t1","chatTitle":"Refactor auth",
                    "bubbles":[{"type":1,"text":"refactor the auth middleware"}]}]})
                .to_string(),
            )],
        );

        let mut ctx = ScanContext::new(None, true);
        let convs = scan_fork(SourceId::Cursor, vec![user], &mut ctx).unwrap();
        assert_eq!(convs.len(), 1);
        assert_eq!(convs[0].conversation.title, "Refactor auth");
        assert_eq!(convs[0].conversation.source, SourceId::Cursor);
    }
}
