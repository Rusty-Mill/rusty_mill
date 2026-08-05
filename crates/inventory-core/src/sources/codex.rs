//! Codex: rollout JSONL files under `~/.codex/sessions/YYYY/MM/DD/`.
//!
//! Codex has used two record shapes: flat (`{"type":"message","role":...}`)
//! and wrapped (`{"type":"response_item","payload":{...}}`). Both are read,
//! because a user's history spans whichever versions they have run.

use super::{derive_title, extract_text, parse_timestamp, ScanContext, Source};
use crate::model::{Conversation, Message, ParsedConversation, Role, SourceId};
use crate::Result;
use std::path::Path;

pub struct Codex;

impl Source for Codex {
    fn id(&self) -> SourceId {
        SourceId::Codex
    }

    fn scan(&self, ctx: &mut ScanContext) -> Result<Vec<ParsedConversation>> {
        let mut out = Vec::new();
        for root in self.roots() {
            for entry in walkdir::WalkDir::new(&root)
                .max_depth(5)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                    continue;
                }
                if !ctx.should_read(SourceId::Codex, path) {
                    continue;
                }
                if let Some(conv) = parse_rollout(path)? {
                    if ctx.since.is_none_or(|s| conv.conversation.updated_at >= s) {
                        out.push(conv);
                    }
                }
            }
        }
        Ok(out)
    }
}

pub fn parse_rollout(path: &Path) -> Result<Option<ParsedConversation>> {
    let text = std::fs::read_to_string(path)?;

    let mut messages: Vec<Message> = Vec::new();
    let mut cwd: Option<String> = None;
    let mut id: Option<String> = None;
    let mut instructions_seen = false;
    let mut first_ts: Option<i64> = None;
    let mut last_ts: Option<i64> = None;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };

        for key in ["cwd", "workdir", "cwd_path"] {
            if let Some(s) = v.get(key).and_then(|c| c.as_str()) {
                cwd.get_or_insert_with(|| s.to_string());
            }
        }
        for key in ["id", "session_id", "conversation_id"] {
            if let Some(s) = v.get(key).and_then(|c| c.as_str()) {
                id.get_or_insert_with(|| s.to_string());
            }
        }

        let ts = v
            .get("timestamp")
            .or_else(|| v.get("ts"))
            .and_then(parse_timestamp);
        if let Some(t) = ts {
            first_ts = Some(first_ts.map_or(t, |f: i64| f.min(t)));
            last_ts = Some(last_ts.map_or(t, |l: i64| l.max(t)));
        }

        // Unwrap the `response_item` envelope when present.
        let item = v.get("payload").unwrap_or(&v);
        let kind = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if !matches!(kind, "message" | "user_message" | "agent_message" | "") {
            continue;
        }
        let Some(role_raw) = item.get("role").and_then(|r| r.as_str()) else {
            continue;
        };
        let role = Role::parse(role_raw);

        let body = item.get("content").map(extract_text).unwrap_or_default();
        if body.is_empty() {
            continue;
        }

        // The first system turn is Codex's own boilerplate instruction block;
        // indexing it would put identical text on every single conversation.
        if role == Role::System && !instructions_seen {
            instructions_seen = true;
            continue;
        }

        messages.push(Message {
            seq: messages.len() as i64,
            role,
            text: body,
            created_at: ts.unwrap_or(0),
        });
    }

    if messages.is_empty() {
        return Ok(None);
    }

    let external_id = id
        .or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| path.to_string_lossy().to_string());

    let fallback = super::claude_code::file_mtime(path);
    let started_at = first_ts.unwrap_or(fallback);
    let updated_at = last_ts.unwrap_or(fallback);
    for m in messages.iter_mut() {
        if m.created_at == 0 {
            m.created_at = started_at;
        }
    }

    let title = derive_title(&messages);
    let count = messages.len() as i64;

    Ok(Some(ParsedConversation {
        conversation: Conversation {
            id: 0,
            source: SourceId::Codex,
            external_id,
            title,
            project_path: cwd,
            git_branch: None,
            started_at,
            updated_at,
            message_count: count,
        },
        messages,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_flat_record_shape() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("rollout-1.jsonl");
        std::fs::write(
            &p,
            r#"{"id":"sess-9","timestamp":"2026-08-05T12:00:00Z","cwd":"/srv/api"}
{"type":"message","role":"user","timestamp":"2026-08-05T12:00:10Z","content":[{"type":"input_text","text":"Postgres index tuning for the search table"}]}
{"type":"message","role":"assistant","timestamp":"2026-08-05T12:02:00Z","content":[{"type":"output_text","text":"Add a partial index."}]}"#,
        )
        .unwrap();

        let parsed = parse_rollout(&p).unwrap().unwrap();
        assert_eq!(parsed.conversation.external_id, "sess-9");
        assert_eq!(
            parsed.conversation.project_path.as_deref(),
            Some("/srv/api")
        );
        assert_eq!(
            parsed.conversation.title,
            "Postgres index tuning for the search table"
        );
        assert_eq!(parsed.messages.len(), 2);
        assert_eq!(parsed.messages[1].text, "Add a partial index.");
    }

    #[test]
    fn parses_the_wrapped_response_item_shape() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("rollout-2.jsonl");
        std::fs::write(
            &p,
            r#"{"timestamp":"2026-08-05T09:00:00Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"why is the build failing"}]}}
{"timestamp":"2026-08-05T09:00:30Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Missing feature flag."}]}}"#,
        )
        .unwrap();

        let parsed = parse_rollout(&p).unwrap().unwrap();
        assert_eq!(parsed.messages.len(), 2);
        assert_eq!(parsed.messages[0].role, Role::User);
        assert_eq!(parsed.messages[1].text, "Missing feature flag.");
    }

    #[test]
    fn the_leading_instruction_block_is_not_indexed() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("rollout-3.jsonl");
        std::fs::write(
            &p,
            r#"{"type":"message","role":"system","content":"You are Codex, a coding agent. Follow these rules."}
{"type":"message","role":"user","timestamp":"2026-08-05T09:00:00Z","content":"actual question here"}"#,
        )
        .unwrap();

        let parsed = parse_rollout(&p).unwrap().unwrap();
        assert_eq!(parsed.messages.len(), 1, "boilerplate should be dropped");
        assert_eq!(parsed.messages[0].text, "actual question here");
    }
}
