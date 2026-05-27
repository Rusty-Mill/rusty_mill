//! Recall — the Orient layer (PRD 03). Scores long-term candidates by
//! relevance + recency + importance, takes top-k, expands 1-hop neighbors on the
//! top-3, and renders the `## Relevant memory` block. Also emits a structured
//! [`ContextEntry`] per selected memory for the episode package's `context_trace`
//! (ADR-0036).
//!
//! v1 weights/τ are PRD 03 starting points, not a frozen contract. The
//! failure-born skill boost (which needs the turn's attribution context) lands
//! with the close-the-loop increment; the validated-skill floor is applied here.

use std::cmp::Ordering;
use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::{MemType, Memory, Store};
use crate::error::ToolError;

/// Default top-k (`RUSTYKEYS_RECALL_K`).
pub const DEFAULT_RECALL_K: usize = 6;

const TAU_DAYS: f64 = 14.0;
const W_REL: f64 = 0.55;
const W_RECENCY: f64 = 0.25;
const W_IMPORTANCE: f64 = 0.20;
/// Validated skills are floored here at recall time (ADR-0011/0031).
const SKILL_FLOOR: f64 = 0.6;
const BODY_CAP: usize = 200;

/// One `context_trace` element (data-model §5.1; ADR-0036). `influenced_decision`
/// is backfilled after the turn — recall sets it `false`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextEntry {
    /// The memory title (or a read file / static artifact).
    pub artifact: String,
    /// `primary` (top-k) | `supporting` (neighbor) | `unused`.
    pub contribution: String,
    /// Did this artifact change what the agent did? (backfilled post-turn).
    pub influenced_decision: bool,
}

/// The two recall outputs (PRD 03): the rendered prompt block and the structured
/// entries for the episode package.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecallOutput {
    /// The `## Relevant memory` string → `extra_context`.
    pub block: String,
    /// One entry per selected candidate/neighbor → episode `context_trace`.
    pub entries: Vec<ContextEntry>,
}

fn type_rank(t: MemType) -> u8 {
    match t {
        MemType::Skill => 0,
        MemType::Summary => 1,
        MemType::Fact => 2,
        MemType::Entity => 3,
    }
}

fn effective_importance(m: &Memory) -> f64 {
    let imp = m.importance as f64;
    if m.mem_type == MemType::Skill && m.validated && imp < SKILL_FLOOR {
        SKILL_FLOOR
    } else {
        imp
    }
}

fn truncate(body: &str) -> String {
    if body.chars().count() <= BODY_CAP {
        body.to_string()
    } else {
        let cut: String = body.chars().take(BODY_CAP).collect();
        format!("{cut}…")
    }
}

/// Best-effort `(why: matched "…")` fragment: the first query token that appears
/// in the title or body.
fn why_fragment(query: &str, m: &Memory) -> Option<String> {
    let hay = format!("{} {}", m.title, m.body).to_ascii_lowercase();
    query
        .split(|c: char| !c.is_ascii_alphanumeric())
        .find(|tok| tok.len() >= 3 && hay.contains(&tok.to_ascii_lowercase()))
        .map(str::to_string)
}

/// Assemble the Orient block + context entries for `query`. Lexical via the
/// store's `candidates` (semantic when a Phase-5 backend is configured).
pub async fn recall(
    store: &dyn Store,
    query: &str,
    k: usize,
    now: f64,
) -> Result<RecallOutput, ToolError> {
    let fetch = (k * 4).max(16);
    let batch = store.candidates(query, None, fetch).await?;
    if batch.is_empty() {
        return Ok(RecallOutput::default());
    }

    // Batch min-max of raw relevance so it shares a [0,1] domain with the other
    // two terms (PRD 03). Degenerate batch ⇒ rel_norm = 1.0.
    let min = batch.iter().map(|(_, r)| *r).fold(f32::INFINITY, f32::min);
    let max = batch
        .iter()
        .map(|(_, r)| *r)
        .fold(f32::NEG_INFINITY, f32::max);
    let span = (max - min) as f64;

    let mut scored: Vec<(f64, Memory)> = batch
        .into_iter()
        .map(|(m, rel)| {
            let rel_norm = if span.abs() < f64::EPSILON {
                1.0
            } else {
                ((rel - min) as f64) / span
            };
            let dz = ((now - m.last_used_ts) / 86_400.0).max(0.0);
            let recency = (-dz / TAU_DAYS).exp();
            let score =
                W_REL * rel_norm + W_RECENCY * recency + W_IMPORTANCE * effective_importance(&m);
            (score, m)
        })
        .collect();

    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(Ordering::Equal)
            .then_with(|| type_rank(a.1.mem_type).cmp(&type_rank(b.1.mem_type)))
            .then_with(|| {
                b.1.last_used_ts
                    .partial_cmp(&a.1.last_used_ts)
                    .unwrap_or(Ordering::Equal)
            })
    });

    let top: Vec<Memory> = scored.into_iter().take(k).map(|(_, m)| m).collect();

    // 1-hop neighbor expansion on the top-3 only (caps added tokens). Neighbors
    // are de-duplicated against the top-k and not re-scored.
    let mut seen: HashSet<String> = top.iter().map(|m| m.title.clone()).collect();
    let mut neighbors: Vec<Vec<Memory>> = vec![Vec::new(); top.len()];
    for (i, m) in top.iter().enumerate().take(3) {
        for n in store.neighbors(&m.title).await? {
            if seen.insert(n.title.clone()) {
                neighbors[i].push(n);
            }
        }
    }

    let mut block = String::from("## Relevant memory\n");
    let mut entries = Vec::new();
    for (i, m) in top.iter().enumerate() {
        let why = why_fragment(query, m)
            .map(|f| format!("  (why: matched \"{f}\")"))
            .unwrap_or_default();
        block.push_str(&format!(
            "- [{}] {}: {}{}\n",
            m.mem_type.as_str(),
            m.title,
            truncate(&m.body),
            why
        ));
        entries.push(ContextEntry {
            artifact: m.title.clone(),
            contribution: "primary".to_string(),
            influenced_decision: false,
        });
        for n in &neighbors[i] {
            block.push_str(&format!(
                "  ↳ related: {}: {}\n",
                n.title,
                truncate(&n.body)
            ));
            entries.push(ContextEntry {
                artifact: n.title.clone(),
                contribution: "supporting".to_string(),
                influenced_decision: false,
            });
        }
    }

    Ok(RecallOutput {
        block: block.trim_end().to_string(),
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{Edge, SqliteStore};

    async fn store_with(mems: &[Memory]) -> SqliteStore {
        let s = SqliteStore::in_memory().unwrap();
        for m in mems {
            s.upsert(m).await.unwrap();
        }
        s
    }

    #[tokio::test]
    async fn ranks_relevant_memory_first_and_renders_block() {
        let now = 1_000_000.0;
        let mut auth = Memory::new(
            "auth flow",
            "login validates the session token",
            MemType::Fact,
            now,
        );
        auth.last_used_ts = now;
        let mut other = Memory::new(
            "ui theme",
            "dark mode palette colors",
            MemType::Fact,
            now - 30.0 * 86_400.0,
        );
        other.last_used_ts = now - 30.0 * 86_400.0;
        let store = store_with(&[auth, other]).await;

        let out = recall(&store, "login token", DEFAULT_RECALL_K, now)
            .await
            .unwrap();
        assert!(out.block.starts_with("## Relevant memory"));
        assert_eq!(out.entries[0].artifact, "auth flow");
        assert_eq!(out.entries[0].contribution, "primary");
    }

    #[tokio::test]
    async fn empty_corpus_yields_empty_block() {
        let store = SqliteStore::in_memory().unwrap();
        let out = recall(&store, "anything", 6, 1.0).await.unwrap();
        assert!(out.block.is_empty());
        assert!(out.entries.is_empty());
    }

    #[tokio::test]
    async fn validated_skill_floor_outranks_equal_relevance_fact() {
        let now = 1_000_000.0;
        let old = now - 90.0 * 86_400.0;
        // Identical body (⇒ equal bm25 relevance) and identical recency; titles
        // carry no query token. The only differentiator is importance: the
        // validated skill is floored to 0.6, the fact stays at 0.1 → skill first.
        let mut skill = Memory::new("alpha", "the token matters here", MemType::Skill, old);
        skill.validated = true;
        skill.importance = 0.1;
        skill.last_used_ts = old;
        let mut fact = Memory::new("beta", "the token matters here", MemType::Fact, old);
        fact.importance = 0.1;
        fact.last_used_ts = old;
        let store = store_with(&[skill, fact]).await;

        let out = recall(&store, "token", DEFAULT_RECALL_K, now)
            .await
            .unwrap();
        assert_eq!(out.entries[0].artifact, "alpha");
    }

    #[tokio::test]
    async fn expands_one_hop_neighbor_as_supporting() {
        let now = 1_000_000.0;
        let mut a = Memory::new(
            "anchor",
            "matches the query keyword zzz",
            MemType::Fact,
            now,
        );
        a.edges = vec![Edge {
            to: "nbr".into(),
            rel: "relates".into(),
        }];
        let nbr = Memory::new("nbr", "neighbor body", MemType::Fact, now);
        let store = store_with(&[a, nbr]).await;

        let out = recall(&store, "zzz", DEFAULT_RECALL_K, now).await.unwrap();
        assert!(out.block.contains("↳ related: nbr"));
        let supporting = out.entries.iter().find(|e| e.artifact == "nbr").unwrap();
        assert_eq!(supporting.contribution, "supporting");
    }
}
