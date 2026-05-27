//! Consolidation — distill short-term observations into long-term memories
//! (PRD 03). The model call itself is injected by the caller (the post-turn join
//! in `app`); this module owns the *contract*: the prompt, the tolerant emit
//! parser, and applying the result to the [`Store`]. That keeps `feed`
//! model-agnostic and the contract fully testable offline.

use serde::Deserialize;

use super::{Edge, MemType, Memory, Observation, Store};
use crate::error::ToolError;

/// Consolidation tempo (PRD 03). Idle is cheap/additive; sleep adds decay/prune;
/// explicit is user-triggered (`/reflect`, `/sleep`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsolidationScope {
    /// After a turn once enough observations accrue: extract only NEW durables.
    Idle,
    /// Session end: idle pass + merge/decay/prune + grooming.
    Sleep,
    /// User-triggered.
    Explicit,
}

impl ConsolidationScope {
    /// snake_case label for the evidence `improvement` record.
    pub fn as_str(self) -> &'static str {
        match self {
            ConsolidationScope::Idle => "idle",
            ConsolidationScope::Sleep => "sleep",
            ConsolidationScope::Explicit => "explicit",
        }
    }
}

/// The structured attribution handed from compose into consolidation — the
/// middle link of the self-improvement loop (PRD 03/05). Carried as plain
/// strings so `feed` need not depend on `compose`'s `FailureType`.
#[derive(Debug, Clone)]
pub struct AttributionContext {
    /// Fixed failure type (e.g. `f_tool`), snake_case.
    pub failure_type: String,
    /// Layer (e.g. `feed/tools`).
    pub layer: String,
    /// Free-text evidence.
    pub evidence: String,
}

/// What a consolidation did (→ the evidence `improvement` record, data-model §4.1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConsolidationStats {
    /// New memories created.
    pub created: usize,
    /// Existing memories updated.
    pub updated: usize,
    /// Memories pruned (sleep tempo).
    pub pruned: usize,
    /// Skills groomed.
    pub groomed: usize,
}

/// One emitted record (PRD 03 emit contract). Field names match the model JSON.
#[derive(Debug, Clone, Deserialize)]
struct Emitted {
    op: String,
    #[serde(rename = "type")]
    mem_type: MemType,
    title: String,
    body: String,
    #[serde(default = "default_importance")]
    importance: f32,
    #[serde(default)]
    edges: Vec<Edge>,
    #[serde(default)]
    source_ts_range: Option<[f64; 2]>,
}

fn default_importance() -> f32 {
    0.5
}

#[derive(Debug, Deserialize)]
struct Envelope {
    #[serde(default)]
    memories: Vec<serde_json::Value>,
}

/// Parse the model's emit JSON, skipping records that fail to deserialize
/// (graceful degradation — a bad record never fails the whole pass).
fn parse_emit(json: &str) -> Vec<Emitted> {
    let Ok(env) = serde_json::from_str::<Envelope>(json) else {
        return Vec::new();
    };
    env.memories
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect()
}

/// Build the consolidation prompt for `scope` over `observations`, optionally
/// carrying the prior turn's `attribution` (the close-the-loop feed). The model
/// must reply with the emit-contract JSON.
pub fn build_prompt(
    observations: &[Observation],
    scope: ConsolidationScope,
    attribution: Option<&AttributionContext>,
) -> String {
    let mut p = String::new();
    p.push_str(
        "You distill an AI agent's recent observations into durable long-term memories.\n\n",
    );

    match scope {
        ConsolidationScope::Idle | ConsolidationScope::Explicit => p.push_str(
            "Extract only NEW durable facts/skills from the observations below. Do not \
             restate memories that already exist.\n",
        ),
        ConsolidationScope::Sleep => p.push_str(
            "Extract new durables AND merge near-duplicates (op=update), decay stale \
             importances, and propose grooming.\n",
        ),
    }

    p.push_str(
        "\nImportance rubric (importance ∈ [0,1]): an UNVERIFIED outcome ⇒ one `skill` \
         capturing the lesson AND its failing condition (≥0.8); a VERIFIED outcome ⇒ \
         update the skill used (≥0.7); stable project fact 0.4–0.6; recap `summary` \
         0.3–0.5; named `entity` 0.3–0.5. Prefer op=update when a title matches an \
         existing memory.\n",
    );

    if let Some(a) = attribution {
        p.push_str(&format!(
            "\nLast turn FAILED verification:\n  failure_type: {}\n  layer: {}\n  evidence: {}\n\
             Emit a `skill` capturing the lesson AND the failing condition above, so the next \
             similar turn recalls it before repeating the mistake.\n",
            a.failure_type, a.layer, a.evidence,
        ));
    }

    p.push_str("\nObservations:\n");
    for o in observations {
        p.push_str(&format!("- ({}/{}) {}\n", o.role, o.kind, o.content));
    }

    p.push_str(
        "\nReturn ONLY valid JSON:\n\
         {\"memories\":[{\"op\":\"create|update\",\"type\":\"fact|summary|skill|entity\",\
         \"title\":\"...\",\"body\":\"...\",\"importance\":0.0,\
         \"edges\":[{\"to\":\"...\",\"rel\":\"relates|causes|supersedes|part_of\"}],\
         \"source_ts_range\":[t0,t1]}]}\n",
    );
    p
}

/// Apply the model's emit JSON to the store. Records are upserted by title
/// (create/update both map to upsert; ON CONFLICT preserves `created_ts`).
/// A failure-born skill is minted as a *candidate* (`validated=false`, ADR-0031).
pub async fn apply(
    store: &dyn Store,
    emit_json: &str,
    now: f64,
) -> Result<ConsolidationStats, ToolError> {
    let mut stats = ConsolidationStats::default();
    for e in parse_emit(emit_json) {
        let mut m = Memory::new(e.title, e.body, e.mem_type, now);
        m.importance = e.importance.clamp(0.0, 1.0);
        m.edges = e.edges;
        m.source_ts = e.source_ts_range.map(|r| (r[0], r[1]));
        store.upsert(&m).await?;
        if e.op == "update" {
            stats.updated += 1;
        } else {
            stats.created += 1;
        }
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SqliteStore;

    #[test]
    fn prompt_includes_attribution_feed_when_present() {
        let attr = AttributionContext {
            failure_type: "f_tool".into(),
            layer: "feed/tools".into(),
            evidence: "bash exited 1".into(),
        };
        let p = build_prompt(&[], ConsolidationScope::Idle, Some(&attr));
        assert!(p.contains("failure_type: f_tool"));
        assert!(p.contains("Emit a `skill`"));
    }

    #[test]
    fn parse_skips_malformed_records() {
        let json = r#"{"memories":[
            {"op":"create","type":"fact","title":"good","body":"ok"},
            {"op":"create","type":"nonsense","title":"bad","body":"x"},
            {"nope":true}
        ]}"#;
        assert_eq!(parse_emit(json).len(), 1);
    }

    #[tokio::test]
    async fn apply_creates_and_updates() {
        let store = SqliteStore::in_memory().unwrap();
        let json = r#"{"memories":[
            {"op":"create","type":"skill","title":"reproduce","body":"reproduce before editing","importance":0.9},
            {"op":"create","type":"fact","title":"paths","body":"workspace is the root","importance":0.5}
        ]}"#;
        let stats = apply(&store, json, 100.0).await.unwrap();
        assert_eq!(stats.created, 2);
        assert!(!store
            .candidates("reproduce", None, 5)
            .await
            .unwrap()
            .is_empty());

        // An update by the same title overwrites the body in place.
        let upd = r#"{"memories":[{"op":"update","type":"fact","title":"paths","body":"the workspace boundary changed token","importance":0.6}]}"#;
        let s2 = apply(&store, upd, 200.0).await.unwrap();
        assert_eq!(s2.updated, 1);
        assert!(!store.candidates("token", None, 5).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn wholesale_parse_failure_is_empty_not_fatal() {
        let store = SqliteStore::in_memory().unwrap();
        let stats = apply(&store, "not json at all", 1.0).await.unwrap();
        assert_eq!(stats, ConsolidationStats::default());
    }
}
