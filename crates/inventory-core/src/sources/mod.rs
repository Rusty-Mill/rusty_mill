//! Readers for the six tools.
//!
//! Two rules hold for every source in here:
//!
//! 1. **Read-only, always.** A source never writes to, moves, or locks a file
//!    a tool owns. Live SQLite databases are copied to a private snapshot
//!    first (see [`snapshot`]), so indexing cannot interfere with a running
//!    editor and cannot be blamed for a corrupt store.
//! 2. **Tolerant parsing.** These are undocumented formats that change without
//!    notice. Parsers skip what they do not recognise rather than failing the
//!    file, and the indexer freezes a source rather than deleting its history
//!    when a format moves out from under it.

pub mod antigravity;
pub mod claude_code;
pub mod codex;
pub mod cursor;
pub mod kiro;
pub mod snapshot;
pub mod vscdb;
pub mod zed;

use crate::model::{ParsedConversation, SourceId};
use crate::Result;
use std::path::{Path, PathBuf};

/// Tracks which files have already been read so a re-index reads each file
/// once — "After that it stays live as you work, reading each file once."
pub struct ScanContext {
    /// Only conversations updated at or after this are worth keeping.
    pub since: Option<i64>,
    /// (source, path) -> (mtime, size) from the previous pass.
    seen: std::collections::HashMap<(SourceId, String), (i64, i64)>,
    /// Files visited this pass, to be written back on success.
    touched: Vec<(SourceId, String, i64, i64)>,
    /// Ignore the seen-file cache and re-read everything.
    pub force_full: bool,
}

impl ScanContext {
    pub fn new(since: Option<i64>, force_full: bool) -> Self {
        ScanContext {
            since,
            seen: std::collections::HashMap::new(),
            touched: Vec::new(),
            force_full,
        }
    }

    pub fn preload_seen(
        &mut self,
        entries: impl IntoIterator<Item = (SourceId, String, i64, i64)>,
    ) {
        for (src, path, mtime, size) in entries {
            self.seen.insert((src, path), (mtime, size));
        }
    }

    /// Should this file be parsed? Records the visit either way.
    pub fn should_read(&mut self, source: SourceId, path: &Path) -> bool {
        let key = path.to_string_lossy().to_string();
        let (mtime, size) = match std::fs::metadata(path) {
            Ok(m) => (
                m.modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0),
                m.len() as i64,
            ),
            Err(_) => return false,
        };
        self.touched.push((source, key.clone(), mtime, size));
        if self.force_full {
            return true;
        }
        match self.seen.get(&(source, key)) {
            Some(&(prev_mtime, prev_size)) => prev_mtime != mtime || prev_size != size,
            None => true,
        }
    }

    pub fn take_touched(&mut self) -> Vec<(SourceId, String, i64, i64)> {
        std::mem::take(&mut self.touched)
    }
}

pub trait Source: Send + Sync {
    fn id(&self) -> SourceId;

    /// Roots that exist on this machine. Empty means the tool is not installed.
    fn roots(&self) -> Vec<PathBuf> {
        crate::paths::existing_roots(self.id())
    }

    fn is_installed(&self) -> bool {
        !self.roots().is_empty()
    }

    /// Read everything this source can offer. Errors freeze the source; they
    /// never delete what it previously contributed.
    fn scan(&self, ctx: &mut ScanContext) -> Result<Vec<ParsedConversation>>;
}

pub fn all() -> Vec<Box<dyn Source>> {
    vec![
        Box::new(claude_code::ClaudeCode),
        Box::new(codex::Codex),
        Box::new(cursor::Cursor),
        Box::new(zed::Zed),
        Box::new(kiro::Kiro),
        Box::new(antigravity::Antigravity),
    ]
}

pub fn by_id(id: SourceId) -> Box<dyn Source> {
    match id {
        SourceId::ClaudeCode => Box::new(claude_code::ClaudeCode),
        SourceId::Codex => Box::new(codex::Codex),
        SourceId::Cursor => Box::new(cursor::Cursor),
        SourceId::Zed => Box::new(zed::Zed),
        SourceId::Kiro => Box::new(kiro::Kiro),
        SourceId::Antigravity => Box::new(antigravity::Antigravity),
    }
}

// --- shared parsing helpers -------------------------------------------------

/// Parse the timestamp formats these tools actually emit: RFC3339 strings,
/// unix seconds, and unix milliseconds, as either JSON strings or numbers.
pub fn parse_timestamp(value: &serde_json::Value) -> Option<i64> {
    match value {
        serde_json::Value::Number(n) => {
            let raw = n.as_f64()?;
            Some(normalize_epoch(raw))
        }
        serde_json::Value::String(s) => {
            if let Ok(n) = s.parse::<f64>() {
                return Some(normalize_epoch(n));
            }
            parse_rfc3339(s)
        }
        _ => None,
    }
}

fn normalize_epoch(raw: f64) -> i64 {
    // Anything past ~1e12 is milliseconds; past ~1e15 is microseconds.
    if raw > 1e14 {
        (raw / 1_000_000.0) as i64
    } else if raw > 1e11 {
        (raw / 1000.0) as i64
    } else {
        raw as i64
    }
}

/// Minimal RFC3339 -> unix seconds. Only the shapes these tools emit.
fn parse_rfc3339(s: &str) -> Option<i64> {
    let bytes = s.as_bytes();
    if bytes.len() < 19 {
        return None;
    }
    let num = |a: usize, b: usize| -> Option<i64> { s.get(a..b)?.parse::<i64>().ok() };
    let (year, month, day) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (hour, min, sec) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    let days = days_from_civil(year, month, day);
    let mut secs = days * 86_400 + hour * 3600 + min * 60 + sec;

    // Trailing offset, e.g. +01:00 / -05:00. 'Z' or absent means UTC.
    if let Some(idx) = s.rfind(['+', '-']) {
        if idx > 10 {
            if let (Some(oh), Some(om)) = (
                s.get(idx + 1..idx + 3).and_then(|v| v.parse::<i64>().ok()),
                s.get(idx + 4..idx + 6).and_then(|v| v.parse::<i64>().ok()),
            ) {
                let offset = oh * 3600 + om * 60;
                secs += if s.as_bytes()[idx] == b'+' {
                    -offset
                } else {
                    offset
                };
            }
        }
    }
    Some(secs)
}

/// Days since 1970-01-01 (Howard Hinnant's civil calendar algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Collapse the several shapes a "content" field takes across these tools:
/// a bare string, an array of typed blocks, or a nested object.
pub fn extract_text(value: &serde_json::Value) -> String {
    let mut out = String::new();
    collect_text(value, &mut out, 0);
    out.trim().to_string()
}

fn collect_text(value: &serde_json::Value, out: &mut String, depth: usize) {
    if depth > 12 {
        return;
    }
    match value {
        serde_json::Value::String(s) => {
            if !s.trim().is_empty() {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(s);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_text(item, out, depth + 1);
            }
        }
        serde_json::Value::Object(map) => {
            // Prefer the conventional text carriers; fall back to a shallow
            // sweep so an unfamiliar block still contributes something.
            for key in [
                "text", "content", "value", "message", "summary", "input", "output",
            ] {
                if let Some(v) = map.get(key) {
                    collect_text(v, out, depth + 1);
                }
            }
        }
        _ => {}
    }
}

/// First non-empty line, trimmed to something that reads as a title.
pub fn derive_title(messages: &[crate::model::Message]) -> String {
    for m in messages {
        if m.role != crate::model::Role::User {
            continue;
        }
        if let Some(line) = first_meaningful_line(&m.text) {
            return line;
        }
    }
    for m in messages {
        if let Some(line) = first_meaningful_line(&m.text) {
            return line;
        }
    }
    "Untitled conversation".to_string()
}

fn first_meaningful_line(text: &str) -> Option<String> {
    let mut in_fence = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        // A title taken from inside a code block is never the thing the user
        // would recognise the conversation by.
        if in_fence {
            continue;
        }
        let line = line
            .trim()
            .trim_start_matches(['#', '>', '-', '*', ' '])
            .trim();
        if line.len() < 3 {
            continue;
        }
        let mut title: String = line.chars().take(120).collect();
        if line.chars().count() > 120 {
            title.push('…');
        }
        return Some(title);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_the_timestamp_shapes_these_tools_emit() {
        // 2026-08-05T12:00:00Z
        let expect = 1_785_931_200;
        assert_eq!(
            parse_timestamp(&json!("2026-08-05T12:00:00Z")),
            Some(expect)
        );
        assert_eq!(
            parse_timestamp(&json!("2026-08-05T12:00:00.123Z")),
            Some(expect)
        );
        assert_eq!(parse_timestamp(&json!(expect)), Some(expect));
        assert_eq!(parse_timestamp(&json!(expect * 1000)), Some(expect));
        assert_eq!(parse_timestamp(&json!(expect.to_string())), Some(expect));
        assert_eq!(parse_timestamp(&json!("not a date")), None);
    }

    #[test]
    fn honours_utc_offsets() {
        let utc = parse_timestamp(&json!("2026-08-05T12:00:00Z")).unwrap();
        let plus_one = parse_timestamp(&json!("2026-08-05T13:00:00+01:00")).unwrap();
        assert_eq!(utc, plus_one);
    }

    #[test]
    fn extracts_text_from_every_content_shape() {
        assert_eq!(extract_text(&json!("hello")), "hello");
        assert_eq!(
            extract_text(&json!([{"type":"text","text":"a"},{"type":"text","text":"b"}])),
            "a\nb"
        );
        assert_eq!(extract_text(&json!({"content":{"text":"deep"}})), "deep");
        assert_eq!(extract_text(&json!(42)), "");
    }

    #[test]
    fn title_prefers_the_first_real_user_line() {
        let msgs = vec![
            crate::model::Message {
                seq: 0,
                role: crate::model::Role::System,
                text: "system preamble".into(),
                created_at: 0,
            },
            crate::model::Message {
                seq: 1,
                role: crate::model::Role::User,
                text: "```\ncode\n```\nRefactor the auth middleware".into(),
                created_at: 0,
            },
        ];
        assert_eq!(derive_title(&msgs), "Refactor the auth middleware");
    }

    #[test]
    fn scan_context_skips_unchanged_files() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.jsonl");
        std::fs::write(&f, b"{}").unwrap();
        let meta = std::fs::metadata(&f).unwrap();
        let mtime = meta
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let mut ctx = ScanContext::new(None, false);
        ctx.preload_seen([(
            SourceId::ClaudeCode,
            f.to_string_lossy().to_string(),
            mtime,
            meta.len() as i64,
        )]);
        assert!(
            !ctx.should_read(SourceId::ClaudeCode, &f),
            "unchanged file re-read"
        );

        let mut forced = ScanContext::new(None, true);
        forced.preload_seen([(
            SourceId::ClaudeCode,
            f.to_string_lossy().to_string(),
            mtime,
            meta.len() as i64,
        )]);
        assert!(
            forced.should_read(SourceId::ClaudeCode, &f),
            "--full ignored the cache"
        );
    }
}
