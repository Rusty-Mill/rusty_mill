use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// The six sources the reviewed product reads. Closed set on purpose —
/// adding a seventh is a code change, not configuration, because each one
/// needs a parser for a storage format its vendor can change at any time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceId {
    ClaudeCode,
    Codex,
    Cursor,
    Zed,
    Kiro,
    Antigravity,
}

impl SourceId {
    pub const ALL: [SourceId; 6] = [
        SourceId::ClaudeCode,
        SourceId::Codex,
        SourceId::Cursor,
        SourceId::Zed,
        SourceId::Kiro,
        SourceId::Antigravity,
    ];

    pub fn slug(self) -> &'static str {
        match self {
            SourceId::ClaudeCode => "claude-code",
            SourceId::Codex => "codex",
            SourceId::Cursor => "cursor",
            SourceId::Zed => "zed",
            SourceId::Kiro => "kiro",
            SourceId::Antigravity => "antigravity",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            SourceId::ClaudeCode => "Claude Code",
            SourceId::Codex => "Codex",
            SourceId::Cursor => "Cursor",
            SourceId::Zed => "Zed",
            SourceId::Kiro => "Kiro",
            SourceId::Antigravity => "Antigravity",
        }
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.slug())
    }
}

impl FromStr for SourceId {
    type Err = crate::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let norm = s.trim().to_ascii_lowercase().replace([' ', '_'], "-");
        SourceId::ALL
            .into_iter()
            .find(|id| id.slug() == norm)
            .ok_or_else(|| crate::Error::other(format!("unknown source `{s}`")))
    }
}

/// Who produced a message. The compare matrix calls this out as its own
/// capability ("knows who said what"), so it is modelled rather than folded
/// into the message text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    System,
    Tool,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::System => "system",
            Role::Tool => "tool",
        }
    }

    pub fn parse(s: &str) -> Role {
        match s.trim().to_ascii_lowercase().as_str() {
            "user" | "human" => Role::User,
            "assistant" | "ai" | "model" | "agent" => Role::Assistant,
            "tool" | "tool_result" | "function" | "tool_use" => Role::Tool,
            _ => Role::System,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub seq: i64,
    pub role: Role,
    pub text: String,
    /// Unix seconds. Sources vary in what they record; falls back to the
    /// conversation's own timestamps when a message carries none.
    pub created_at: i64,
}

/// A conversation as it exists in the index, independent of which tool
/// produced it. `external_id` is the source's own identifier and is what
/// makes re-indexing idempotent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    #[serde(default)]
    pub id: i64,
    pub source: SourceId,
    pub external_id: String,
    pub title: String,
    pub project_path: Option<String>,
    pub git_branch: Option<String>,
    pub started_at: i64,
    pub updated_at: i64,
    pub message_count: i64,
}

/// What a source hands the indexer.
#[derive(Debug, Clone)]
pub struct ParsedConversation {
    pub conversation: Conversation,
    pub messages: Vec<Message>,
}

impl ParsedConversation {
    /// The text the FTS index and the embedder both see.
    pub fn body(&self) -> String {
        let mut out = String::new();
        for m in &self.messages {
            out.push_str(m.role.as_str());
            out.push_str(": ");
            out.push_str(&m.text);
            out.push('\n');
        }
        out
    }
}

/// Per-source health, surfaced by `inv sources`. The reviewed product shows
/// the last successful read when a source goes unreadable, so the timestamp
/// is part of the state rather than a log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceState {
    /// Read cleanly on the last pass.
    Ok,
    /// Installed, but the last read failed. Everything already indexed from
    /// it is retained and still searchable; retried on the next pass.
    Frozen,
    /// No local store found — the tool is not installed, or never used.
    Absent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceStatus {
    pub source: SourceId,
    pub state: SourceState,
    pub last_ok_at: Option<i64>,
    pub last_error: Option<String>,
    pub frozen_at: Option<i64>,
    pub conversation_count: i64,
    pub message_count: i64,
}

/// How far back to index. The command palette shows the on-disk cost of each
/// choice, so the enum is fixed rather than free-form days.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Retention {
    Days7,
    Days30,
    Days90,
    Days365,
    All,
}

impl Retention {
    pub const ALL: [Retention; 5] = [
        Retention::Days7,
        Retention::Days30,
        Retention::Days90,
        Retention::Days365,
        Retention::All,
    ];

    pub fn days(self) -> Option<i64> {
        match self {
            Retention::Days7 => Some(7),
            Retention::Days30 => Some(30),
            Retention::Days90 => Some(90),
            Retention::Days365 => Some(365),
            Retention::All => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Retention::Days7 => "7 days",
            Retention::Days30 => "30 days",
            Retention::Days90 => "90 days",
            Retention::Days365 => "365 days",
            Retention::All => "everything",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Retention::Days7 => "7",
            Retention::Days30 => "30",
            Retention::Days90 => "90",
            Retention::Days365 => "365",
            Retention::All => "all",
        }
    }

    /// Unix-seconds cutoff, or None for everything.
    pub fn cutoff(self, now: i64) -> Option<i64> {
        self.days().map(|d| now - d * 86_400)
    }
}

impl FromStr for Retention {
    type Err = crate::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "7" | "7d" => Ok(Retention::Days7),
            "30" | "30d" => Ok(Retention::Days30),
            "90" | "90d" => Ok(Retention::Days90),
            "365" | "365d" => Ok(Retention::Days365),
            "all" | "everything" | "0" => Ok(Retention::All),
            other => Err(crate::Error::other(format!(
                "retention must be one of 7, 30, 90, 365, all (got `{other}`)"
            ))),
        }
    }
}

/// Quick capture note (⌘⇧N).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: i64,
    pub text: String,
    pub created_at: i64,
}

/// Clipboard scratchpad entry (⌘⇧V), tagged with the app it came from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Clip {
    pub id: i64,
    pub text: String,
    pub app: Option<String>,
    pub created_at: i64,
}

pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
