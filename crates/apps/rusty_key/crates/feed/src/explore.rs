//! Divergent → converge exploration (ADR-0032 / PRD 03, opt-in). For hard,
//! open-ended design, breadth of independent ideation beats a single linear
//! plan. We fan out `N` isolated child sessions under distinct **cognitive
//! frames** (the divergent pass), then run a **mechanical** score → cluster →
//! top-K converge, and finish with **one** critic synthesis call. Built entirely
//! on the existing `agent` tool + [`SessionFactory`] (no new infra); cost-gated
//! (≈N+1 model calls) so it is never the default path.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use rk_observe::ToolOutcome;
use serde_json::Value;
use tokio::task::JoinSet;

use crate::agent::SessionFactory;
use crate::error::ToolError;
use crate::tool::{ToolFn, ToolRegistry};

/// Cognitive-frame preambles (ADR-0032). Each seeds a child's layer-1 identity
/// so the branches diverge; the list is a product call, not a frozen contract.
pub const FRAMES: &[(&str, &str)] = &[
    (
        "regulator",
        "You are a risk-averse regulator. Favour safety, correctness, and \
         auditability over speed. Call out failure modes others miss.",
    ),
    (
        "speedrunner",
        "You are a speedrunner. Find the shortest path to a working result; \
         cut every step that is not strictly necessary.",
    ),
    (
        "zero_budget",
        "You operate on a $0 budget. Reuse what exists, avoid new dependencies \
         and services, and prefer the simplest possible mechanism.",
    ),
    (
        "infinite_budget",
        "You have an unlimited budget. Propose the most robust, scalable design \
         even if it is expensive or ambitious.",
    ),
    (
        "on_call_3am",
        "You are the on-call engineer at 3am. Optimise for operability, clear \
         signals, and the ability to debug under pressure.",
    ),
];

/// The frame used for the convergence/critic synthesis pass.
pub const CRITIC_FRAME: &str =
    "You are a decisive critic. Given several candidate plans, pick the best \
     elements, discard traps, and synthesise a single concrete recommendation.";

/// One explored branch.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// The cognitive frame that produced it.
    pub frame: String,
    /// The proposed plan text.
    pub plan: String,
    /// Mechanical score (higher is better).
    pub score: f64,
}

/// The result of an exploration: the surviving candidates plus a synthesis.
#[derive(Debug, Clone)]
pub struct ExploreReport {
    /// Top-K candidates after score + cluster.
    pub candidates: Vec<Candidate>,
    /// The critic's synthesised recommendation.
    pub recommendation: String,
    /// How many branches were fanned out.
    pub branches: usize,
}

/// Mechanical, deterministic plausibility score for a candidate plan: rewards
/// concrete, structured, non-trivial proposals; penalises empty or one-liner
/// output. No LLM call.
pub fn score(plan: &str) -> f64 {
    let trimmed = plan.trim();
    if trimmed.is_empty() {
        return 0.0;
    }
    let lines = trimmed
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count()
        .min(20) as f64;
    let words = trimmed.split_whitespace().count().min(400) as f64;
    // Concrete steps (numbered/bulleted) and distinct vocabulary signal substance.
    let steps = trimmed
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            t.starts_with("- ")
                || t.starts_with("* ")
                || t.chars().next().is_some_and(|c| c.is_ascii_digit())
        })
        .count()
        .min(20) as f64;
    let vocab = trimmed
        .split_whitespace()
        .map(|w| w.to_ascii_lowercase())
        .collect::<HashSet<_>>()
        .len()
        .min(200) as f64;
    // Weighted, normalised into a rough [0, 1] band.
    (0.35 * (steps / 20.0))
        + (0.30 * (lines / 20.0))
        + (0.20 * (vocab / 200.0))
        + (0.15 * (words / 400.0))
}

/// Word-set Jaccard similarity (for near-duplicate clustering).
fn jaccard(a: &str, b: &str) -> f64 {
    let set = |s: &str| {
        s.split_whitespace()
            .map(|w| w.to_ascii_lowercase())
            .collect::<HashSet<_>>()
    };
    let (sa, sb) = (set(a), set(b));
    if sa.is_empty() && sb.is_empty() {
        return 1.0;
    }
    let inter = sa.intersection(&sb).count() as f64;
    let union = sa.union(&sb).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

/// Mechanical converge: score every candidate, drop near-duplicates (keeping the
/// higher-scored of a cluster), and return the top-`k` by score. Deterministic.
pub fn converge(mut candidates: Vec<Candidate>, k: usize, dup_threshold: f64) -> Vec<Candidate> {
    for c in &mut candidates {
        c.score = score(&c.plan);
    }
    // Highest score first; ties broken by frame name for determinism.
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.frame.cmp(&b.frame))
    });
    let mut kept: Vec<Candidate> = Vec::new();
    for cand in candidates {
        let dup = kept
            .iter()
            .any(|k| jaccard(&k.plan, &cand.plan) >= dup_threshold);
        if !dup {
            kept.push(cand);
        }
        if kept.len() == k {
            break;
        }
    }
    kept
}

/// Render an [`ExploreReport`] as the tool's text result.
pub fn report_text(report: &ExploreReport) -> String {
    let mut s = format!(
        "Explored {} branches → {} candidate plan(s):\n",
        report.branches,
        report.candidates.len()
    );
    for (i, c) in report.candidates.iter().enumerate() {
        s.push_str(&format!(
            "\n[{}] frame={} score={:.2}\n{}\n",
            i + 1,
            c.frame,
            c.score,
            c.plan.trim()
        ));
    }
    s.push_str(&format!(
        "\nRecommendation:\n{}",
        report.recommendation.trim()
    ));
    s
}

/// Orchestrates a divergent→converge exploration over a [`SessionFactory`].
pub struct ExploreStrategy {
    factory: Arc<dyn SessionFactory>,
    branches: usize,
    top_k: usize,
}

impl ExploreStrategy {
    /// Build a strategy fanning out `branches` children and keeping `top_k`.
    pub fn new(factory: Arc<dyn SessionFactory>, branches: usize, top_k: usize) -> Self {
        Self {
            factory,
            branches: branches.max(1),
            top_k: top_k.max(1),
        }
    }

    /// Run the full divergent → converge → synthesise pass for `task`.
    pub async fn run(&self, task: &str) -> Result<ExploreReport, ToolError> {
        // Divergent pass: fan out N framed children in parallel.
        let mut set: JoinSet<(String, Result<String, ToolError>)> = JoinSet::new();
        for i in 0..self.branches {
            let (name, preamble) = FRAMES[i % FRAMES.len()];
            let factory = self.factory.clone();
            let prompt = format!(
                "Propose a concrete plan for the following, as a short numbered \
                 list of steps.\n\n{task}"
            );
            set.spawn(async move {
                let r = factory.spawn_framed(&prompt, preamble).await;
                (name.to_string(), r)
            });
        }
        let mut candidates = Vec::new();
        while let Some(joined) = set.join_next().await {
            let (frame, result) = joined.map_err(|e| ToolError::Other(e.to_string()))?;
            if let Ok(plan) = result {
                candidates.push(Candidate {
                    frame,
                    plan,
                    score: 0.0,
                });
            }
        }
        if candidates.is_empty() {
            return Err(ToolError::Other("explore: all branches failed".into()));
        }

        // Mechanical converge.
        let top = converge(candidates, self.top_k, 0.85);

        // One critic synthesis call over the survivors.
        let mut critic_prompt =
            format!("Synthesise a single concrete recommendation for: {task}\n\nCandidates:\n");
        for (i, c) in top.iter().enumerate() {
            critic_prompt.push_str(&format!("\n[{}] ({})\n{}\n", i + 1, c.frame, c.plan.trim()));
        }
        let recommendation = self
            .factory
            .spawn_framed(&critic_prompt, CRITIC_FRAME)
            .await
            .unwrap_or_else(|e| format!("(critic unavailable: {e})"));

        Ok(ExploreReport {
            candidates: top,
            recommendation,
            branches: self.branches,
        })
    }
}

struct ExploreTool {
    strategy: Arc<ExploreStrategy>,
}

#[async_trait]
impl ToolFn for ExploreTool {
    fn name(&self) -> &str {
        "explore"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {"task": {"type": "string"}},
            "required": ["task"]
        })
    }

    async fn call(&self, args: Value) -> ToolOutcome {
        let Some(task) = args.get("task").and_then(Value::as_str) else {
            return ToolOutcome::error("explore: missing 'task'");
        };
        match self.strategy.run(task).await {
            Ok(report) => ToolOutcome::ok(report_text(&report)),
            Err(e) => crate::error::outcome_from_error(e),
        }
    }
}

/// Register the opt-in `explore` tool backed by `strategy` (ADR-0032).
pub fn register_explore_tool(registry: &mut ToolRegistry, strategy: Arc<ExploreStrategy>) {
    registry.insert(Box::new(ExploreTool { strategy }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn score_rewards_structure_over_emptiness() {
        assert_eq!(score(""), 0.0);
        let thin = score("maybe do something");
        let rich = score("1. read the file\n2. parse it\n3. write output\n4. verify");
        assert!(rich > thin);
    }

    #[test]
    fn converge_dedups_and_takes_top_k() {
        let cands = vec![
            Candidate {
                frame: "a".into(),
                plan: "1. step one\n2. step two\n3. step three".into(),
                score: 0.0,
            },
            // Near-identical to the first → clustered out.
            Candidate {
                frame: "b".into(),
                plan: "1. step one\n2. step two\n3. step three".into(),
                score: 0.0,
            },
            Candidate {
                frame: "c".into(),
                plan: "- alpha\n- beta\n- gamma\n- delta\n- epsilon".into(),
                score: 0.0,
            },
        ];
        let top = converge(cands, 2, 0.85);
        // The two identical plans collapse to one; two distinct survive.
        assert_eq!(top.len(), 2);
        let plans: HashSet<_> = top.iter().map(|c| c.plan.clone()).collect();
        assert_eq!(plans.len(), 2);
    }

    /// A deterministic factory: returns a frame-stamped plan, counting calls.
    struct MockFactory {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl SessionFactory for MockFactory {
        async fn spawn(
            &self,
            _task: &str,
            _tools: Option<Vec<String>>,
        ) -> Result<String, ToolError> {
            Ok("default".into())
        }
        async fn spawn_framed(&self, _task: &str, frame: &str) -> Result<String, ToolError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            // Distinct, structured plan per frame so they don't all cluster.
            Ok(format!(
                "1. {frame} analyses\n2. {frame} proposes\n3. {frame} validates"
            ))
        }
    }

    #[tokio::test]
    async fn explore_fans_out_and_synthesises() {
        let calls = Arc::new(AtomicUsize::new(0));
        let factory = Arc::new(MockFactory {
            calls: calls.clone(),
        });
        let strategy = ExploreStrategy::new(factory, 4, 2);
        let report = strategy.run("design a cache").await.unwrap();

        assert_eq!(report.branches, 4);
        assert_eq!(report.candidates.len(), 2); // top-K
        assert!(report.recommendation.contains(CRITIC_FRAME) || !report.recommendation.is_empty());
        // 4 divergent spawns + 1 critic synthesis.
        assert_eq!(calls.load(Ordering::SeqCst), 5);
    }
}
