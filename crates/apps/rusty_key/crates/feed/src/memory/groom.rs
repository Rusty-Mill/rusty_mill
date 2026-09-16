//! Skill grooming (PRD 03; ADR-0031). The optimizer step of the self-improvement
//! loop: the model proposes `refine`/`merge`/`split` over the skill set. Like
//! consolidation, the model call is injected by the caller; this module owns the
//! prompt + tolerant parse + apply.
//!
//! Non-regression-conservative (ADR-0031): a groomed skill is written as a
//! *candidate* (`validated=false`), so grooming can never silently inherit a
//! proven lesson's durability — the new body must re-earn promotion.

use serde::Deserialize;
use serde_json::Value;

use super::{ConsolidationStats, MemType, Memory, Store};
use crate::error::ToolError;

#[derive(Debug, Deserialize)]
struct GroomResult {
    title: String,
    body: String,
}

#[derive(Debug, Deserialize)]
struct GroomOp {
    op: String,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    sources: Vec<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    results: Vec<GroomResult>,
}

#[derive(Debug, Deserialize)]
struct GroomEnvelope {
    #[serde(default)]
    ops: Vec<Value>,
}

fn parse_ops(json: &str) -> Vec<GroomOp> {
    let Ok(env) = serde_json::from_str::<GroomEnvelope>(json) else {
        return Vec::new();
    };
    env.ops
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect()
}

/// Build the grooming prompt over the current `skills`.
pub fn build_prompt(skills: &[Memory]) -> String {
    let mut p = String::from(
        "You groom an AI agent's skill set. Propose refine/merge/split operations to \
         remove redundancy and sharpen lessons. Do NOT drop a skill's failing-condition \
         coverage.\n\nSkills:\n",
    );
    for s in skills {
        p.push_str(&format!("- {}: {}\n", s.title, s.body));
    }
    p.push_str(
        "\nReturn ONLY valid JSON:\n\
         {\"ops\":[\
         {\"op\":\"refine\",\"target\":\"...\",\"body\":\"...\"},\
         {\"op\":\"merge\",\"sources\":[\"...\"],\"title\":\"...\",\"body\":\"...\"},\
         {\"op\":\"split\",\"source\":\"...\",\"results\":[{\"title\":\"...\",\"body\":\"...\"}]}]}\n",
    );
    p
}

async fn put_candidate(
    store: &dyn Store,
    title: &str,
    body: &str,
    now: f64,
) -> Result<(), ToolError> {
    let m = Memory::new(title, body, MemType::Skill, now); // validated=false by default
    store.upsert(&m).await
}

/// Apply the model's grooming JSON. Each applied op increments `groomed`.
pub async fn apply(
    store: &dyn Store,
    groom_json: &str,
    now: f64,
) -> Result<ConsolidationStats, ToolError> {
    let mut stats = ConsolidationStats::default();
    for op in parse_ops(groom_json) {
        match op.op.as_str() {
            "refine" => {
                if let (Some(target), Some(body)) = (op.target.as_deref(), op.body.as_deref()) {
                    put_candidate(store, target, body, now).await?;
                    stats.groomed += 1;
                }
            }
            "merge" => {
                if let (Some(title), Some(body)) = (op.title.as_deref(), op.body.as_deref()) {
                    if !op.sources.is_empty() {
                        put_candidate(store, title, body, now).await?;
                        for s in &op.sources {
                            if s != title {
                                store.remove(s).await?;
                            }
                        }
                        stats.groomed += 1;
                    }
                }
            }
            "split" => {
                if let Some(source) = op.source.as_deref() {
                    if !op.results.is_empty() {
                        let keep: Vec<&str> = op.results.iter().map(|r| r.title.as_str()).collect();
                        for r in &op.results {
                            put_candidate(store, &r.title, &r.body, now).await?;
                        }
                        if !keep.contains(&source) {
                            store.remove(source).await?;
                        }
                        stats.groomed += 1;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SqliteStore;

    #[tokio::test]
    async fn merge_removes_sources_and_writes_candidate() {
        let store = SqliteStore::in_memory().unwrap();
        let mut a = Memory::new(
            "retry on timeout",
            "retry idempotent ops on timeout",
            MemType::Skill,
            1.0,
        );
        a.validated = true;
        let mut b = Memory::new(
            "retry on error",
            "retry idempotent ops on error",
            MemType::Skill,
            1.0,
        );
        b.validated = true;
        store.upsert(&a).await.unwrap();
        store.upsert(&b).await.unwrap();

        let json = r#"{"ops":[{"op":"merge","sources":["retry on timeout","retry on error"],
            "title":"retry idempotently","body":"retry idempotent ops on timeout or error"}]}"#;
        let stats = apply(&store, json, 2.0).await.unwrap();
        assert_eq!(stats.groomed, 1);

        let skills = store.skills().await.unwrap();
        let titles: Vec<&str> = skills.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["retry idempotently"]);
        // The merged skill is a candidate (must re-earn promotion).
        assert!(!skills[0].validated);
    }

    #[tokio::test]
    async fn refine_updates_body_as_candidate() {
        let store = SqliteStore::in_memory().unwrap();
        let mut s = Memory::new("lesson", "old body", MemType::Skill, 1.0);
        s.validated = true;
        store.upsert(&s).await.unwrap();

        let json = r#"{"ops":[{"op":"refine","target":"lesson","body":"sharper lesson xyzzy"}]}"#;
        apply(&store, json, 2.0).await.unwrap();

        assert!(!store.candidates("xyzzy", None, 5).await.unwrap().is_empty());
        let skills = store.skills().await.unwrap();
        assert!(!skills[0].validated); // body changed ⇒ re-candidate
    }

    #[tokio::test]
    async fn malformed_groom_is_non_fatal() {
        let store = SqliteStore::in_memory().unwrap();
        let stats = apply(&store, "garbage", 1.0).await.unwrap();
        assert_eq!(stats.groomed, 0);
    }
}
