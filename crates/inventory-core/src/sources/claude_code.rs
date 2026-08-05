//! Claude Code: one JSONL transcript per session under
//! `~/.claude/projects/<project-slug>/<session-id>.jsonl`.
//!
//! Each line is a self-contained JSON record. Lines carry `cwd`, `gitBranch`
//! and `sessionId`, which is where the project path and branch shown next to a
//! result come from — and what makes resumption work after a project folder
//! has moved.

use super::{derive_title, extract_text, parse_timestamp, ScanContext, Source};
use crate::model::{Conversation, Message, ParsedConversation, Role, SourceId};
use crate::Result;
use std::path::Path;

pub struct ClaudeCode;

impl Source for ClaudeCode {
    fn id(&self) -> SourceId {
        SourceId::ClaudeCode
    }

    fn scan(&self, ctx: &mut ScanContext) -> Result<Vec<ParsedConversation>> {
        let mut out = Vec::new();
        for root in self.roots() {
            for entry in walkdir::WalkDir::new(&root)
                .max_depth(4)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                    continue;
                }
                if !ctx.should_read(SourceId::ClaudeCode, path) {
                    continue;
                }
                if let Some(conv) = parse_transcript(path)? {
                    if ctx.since.is_none_or(|s| conv.conversation.updated_at >= s) {
                        out.push(conv);
                    }
                }
            }
        }
        Ok(out)
    }
}

/// Parse one transcript. Unrecognised lines are skipped rather than failing
/// the file — a single new record type must not cost the user the session.
pub fn parse_transcript(path: &Path) -> Result<Option<ParsedConversation>> {
    let text = std::fs::read_to_string(path)?;

    let mut messages: Vec<Message> = Vec::new();
    let mut cwd: Option<String> = None;
    let mut branch: Option<String> = None;
    let mut session_id: Option<String> = None;
    let mut summary: Option<String> = None;
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

        if let Some(s) = v.get("cwd").and_then(|c| c.as_str()) {
            cwd.get_or_insert_with(|| s.to_string());
        }
        if let Some(s) = v.get("gitBranch").and_then(|c| c.as_str()) {
            if !s.is_empty() {
                branch = Some(s.to_string());
            }
        }
        if let Some(s) = v.get("sessionId").and_then(|c| c.as_str()) {
            session_id.get_or_insert_with(|| s.to_string());
        }

        let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if kind == "summary" {
            if let Some(s) = v.get("summary").and_then(|c| c.as_str()) {
                summary = Some(s.to_string());
            }
            continue;
        }

        let ts = v.get("timestamp").and_then(parse_timestamp);
        if let Some(t) = ts {
            first_ts = Some(first_ts.map_or(t, |f: i64| f.min(t)));
            last_ts = Some(last_ts.map_or(t, |l: i64| l.max(t)));
        }

        // The payload sits under `message` for user/assistant records.
        let payload = v.get("message").unwrap_or(&v);
        let role = payload
            .get("role")
            .and_then(|r| r.as_str())
            .map(Role::parse)
            .unwrap_or_else(|| Role::parse(kind));

        let body = payload.get("content").map(extract_text).unwrap_or_default();
        if body.is_empty() {
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

    let external_id = session_id
        .or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| path.to_string_lossy().to_string());

    let fallback_ts = file_mtime(path);
    let started_at = first_ts.unwrap_or(fallback_ts);
    let updated_at = last_ts.unwrap_or(fallback_ts);
    for m in messages.iter_mut() {
        if m.created_at == 0 {
            m.created_at = started_at;
        }
    }

    let title = summary.unwrap_or_else(|| derive_title(&messages));

    Ok(Some(ParsedConversation {
        conversation: Conversation {
            id: 0,
            source: SourceId::ClaudeCode,
            external_id,
            title,
            // Fall back to decoding the project slug in the directory name,
            // which Claude Code derives from the cwd by replacing separators.
            project_path: cwd.or_else(|| project_from_slug(path)),
            git_branch: branch,
            started_at,
            updated_at,
            message_count: messages.len() as i64,
        },
        messages,
    }))
}

fn project_from_slug(path: &Path) -> Option<String> {
    let slug = path.parent()?.file_name()?.to_str()?;
    if !slug.starts_with('-') {
        return None;
    }
    Some(slug.replace('-', "/"))
}

pub(crate) fn file_mtime(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn parses_a_transcript_with_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(
            dir.path(),
            "session.jsonl",
            r#"
{"type":"user","sessionId":"abc-123","cwd":"/Users/x/proj","gitBranch":"feat/auth","timestamp":"2026-08-05T12:00:00Z","message":{"role":"user","content":"Refactor the auth middleware into a shared hook"}}
{"type":"assistant","timestamp":"2026-08-05T12:01:00Z","message":{"role":"assistant","content":[{"type":"text","text":"Here is the refactor."}]}}
"#,
        );

        let parsed = parse_transcript(&p).unwrap().unwrap();
        let c = &parsed.conversation;
        assert_eq!(c.source, SourceId::ClaudeCode);
        assert_eq!(c.external_id, "abc-123");
        assert_eq!(c.project_path.as_deref(), Some("/Users/x/proj"));
        assert_eq!(c.git_branch.as_deref(), Some("feat/auth"));
        assert_eq!(c.title, "Refactor the auth middleware into a shared hook");
        assert_eq!(c.message_count, 2);
        assert_eq!(c.started_at, 1_785_931_200);
        assert_eq!(c.updated_at, 1_785_931_260);
        assert_eq!(parsed.messages[0].role, Role::User);
        assert_eq!(parsed.messages[1].role, Role::Assistant);
        assert_eq!(parsed.messages[1].text, "Here is the refactor.");
    }

    #[test]
    fn a_summary_record_becomes_the_title() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(
            dir.path(),
            "s.jsonl",
            r#"{"type":"summary","summary":"Postgres index tuning"}
{"type":"user","timestamp":"2026-08-05T12:00:00Z","message":{"role":"user","content":"why is this slow"}}"#,
        );
        assert_eq!(
            parse_transcript(&p).unwrap().unwrap().conversation.title,
            "Postgres index tuning"
        );
    }

    /// The capability this protects is "survives a tool changing its storage
    /// format": one unreadable record must not cost the whole session.
    #[test]
    fn unknown_and_malformed_records_are_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(
            dir.path(),
            "s.jsonl",
            r#"{"type":"user","timestamp":"2026-08-05T12:00:00Z","message":{"role":"user","content":"first"}}
this is not json at all
{"type":"some_future_record_type","payload":{"nested":true}}
{"type":"assistant","timestamp":"2026-08-05T12:05:00Z","message":{"role":"assistant","content":"second"}}"#,
        );
        let parsed = parse_transcript(&p).unwrap().unwrap();
        assert_eq!(parsed.conversation.message_count, 2);
        assert_eq!(parsed.messages[0].text, "first");
        assert_eq!(parsed.messages[1].text, "second");
    }

    #[test]
    fn an_empty_transcript_yields_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "s.jsonl", "\n\n");
        assert!(parse_transcript(&p).unwrap().is_none());
    }

    #[test]
    fn project_path_falls_back_to_the_directory_slug() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("-Users-x-work-api");
        std::fs::create_dir_all(&proj).unwrap();
        let p = write(
            &proj,
            "s.jsonl",
            r#"{"type":"user","timestamp":"2026-08-05T12:00:00Z","message":{"role":"user","content":"hello there"}}"#,
        );
        let parsed = parse_transcript(&p).unwrap().unwrap();
        assert_eq!(
            parsed.conversation.project_path.as_deref(),
            Some("/Users/x/work/api")
        );
    }
}
