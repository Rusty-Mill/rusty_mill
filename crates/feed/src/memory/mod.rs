//! Memory — the three-tier cognitive architecture (PRD 03): short-term
//! [`Stream`], long-term [`Store`], and (later) Task State. Phase-3 increment 1:
//! the trait seams and their SQLite backends. Recall scoring, consolidation,
//! and validation-gated skills land in later increments.

mod consolidate;
mod groom;
mod recall;
mod store;
mod stream;

pub use consolidate::{
    apply as consolidate_apply, build_prompt as consolidation_prompt, AttributionContext,
    ConsolidationScope, ConsolidationStats,
};
pub use groom::{apply as groom_apply, build_prompt as groom_prompt};
pub use recall::{recall, ContextEntry, RecallOutput, DEFAULT_RECALL_K};
pub use store::SqliteStore;
pub use stream::SqliteStream;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::ToolError;

/// One short-term observation (PRD 03; data-model §2). `session_id` is carried
/// by the backend, not this value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    /// Epoch seconds.
    pub ts: f64,
    /// `user | assistant | system | tool`.
    pub role: String,
    /// `message | tool_event | verification | task_change | consolidation`.
    pub kind: String,
    /// The observation payload.
    pub content: String,
}

/// Kind of a long-term memory (data-model §3). snake_case on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemType {
    /// A stable project fact.
    Fact,
    /// An episode/period recap.
    Summary,
    /// A reusable lesson (validation-gated; ADR-0031).
    Skill,
    /// A named person/system/component.
    Entity,
}

impl MemType {
    /// snake_case wire token.
    pub fn as_str(self) -> &'static str {
        match self {
            MemType::Fact => "fact",
            MemType::Summary => "summary",
            MemType::Skill => "skill",
            MemType::Entity => "entity",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "fact" => Some(MemType::Fact),
            "summary" => Some(MemType::Summary),
            "skill" => Some(MemType::Skill),
            "entity" => Some(MemType::Entity),
            _ => None,
        }
    }
}

/// A typed edge between memories (data-model §3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    /// Destination memory title.
    pub to: String,
    /// `relates | causes | supersedes | part_of`.
    pub rel: String,
}

/// A consolidated long-term memory (data-model §3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Memory {
    /// Stable identity; edges and updates reference this.
    pub title: String,
    /// The durable content.
    pub body: String,
    /// Kind of memory.
    pub mem_type: MemType,
    /// 0.0..1.0 (validated skills floored at recall; ADR-0011).
    pub importance: f32,
    /// Skill candidate→promoted lifecycle (ADR-0031); 0/1.
    pub validated: bool,
    /// Epoch seconds first created.
    pub created_ts: f64,
    /// Recall recency input.
    pub last_used_ts: f64,
    /// Reinforcement count.
    pub use_count: u32,
    /// Provenance: the observation window distilled from, if any.
    pub source_ts: Option<(f64, f64)>,
    /// Outgoing typed edges.
    pub edges: Vec<Edge>,
}

impl Memory {
    /// A minimal fact/skill with sensible defaults; `created`/`last_used` set to `ts`.
    pub fn new(
        title: impl Into<String>,
        body: impl Into<String>,
        mem_type: MemType,
        ts: f64,
    ) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            mem_type,
            importance: 0.5,
            validated: false,
            created_ts: ts,
            last_used_ts: ts,
            use_count: 0,
            source_ts: None,
            edges: Vec::new(),
        }
    }
}

/// Short-term observation log (PRD 03). Append-optimized; backed by `stream.db`.
#[async_trait]
pub trait Stream: Send + Sync {
    /// Append one observation.
    async fn append(&self, obs: &Observation) -> Result<(), ToolError>;
    /// The most recent `n` observations, oldest-first.
    async fn recent(&self, n: usize) -> Result<Vec<Observation>, ToolError>;
    /// All observations since epoch `ts`, oldest-first.
    async fn since(&self, ts: f64) -> Result<Vec<Observation>, ToolError>;
}

/// Long-term memory graph (PRD 03). Backed by `store.db` (FTS5 lexical recall).
#[async_trait]
pub trait Store: Send + Sync {
    /// Insert or update a memory (keyed by `title`) and its edges.
    async fn upsert(&self, memory: &Memory) -> Result<(), ToolError>;
    /// Top-`k` candidates for `query` with raw relevance (bm25 lexical in v1;
    /// `embed` reserved for the Phase-5 semantic backend).
    async fn candidates(
        &self,
        query: &str,
        embed: Option<&[f32]>,
        k: usize,
    ) -> Result<Vec<(Memory, f32)>, ToolError>;
    /// 1-hop neighbors of `title` via edges.
    async fn neighbors(&self, title: &str) -> Result<Vec<Memory>, ToolError>;
    /// Set a memory's `validated` flag — the skill candidate→promoted lifecycle
    /// (ADR-0031): promote on a matching VERIFIED turn; un-validate on `direct_edit`.
    async fn set_validated(&self, title: &str, validated: bool) -> Result<(), ToolError>;
    /// All `skill` memories (for grooming).
    async fn skills(&self) -> Result<Vec<Memory>, ToolError>;
    /// The most-recently-created `n` memories (for `/memory`).
    async fn recent(&self, n: usize) -> Result<Vec<Memory>, ToolError>;
    /// Prune non-skill memories older than `older_than` below `importance_below`;
    /// returns how many were removed. Validated skills are exempt (ADR-0011).
    async fn prune(&self, older_than: f64, importance_below: f32) -> Result<usize, ToolError>;
    /// Remove a memory and its edges by `title`.
    async fn remove(&self, title: &str) -> Result<(), ToolError>;
}
