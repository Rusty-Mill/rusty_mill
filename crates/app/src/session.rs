//! `Session` — the centre of the harness (ARCHITECTURE §6). Phase-3: the full
//! OODA loop. Each [`Session::send`] orients (recall → context), runs the turn
//! through the policy-vetted traced registry, verifies, journals, captures the
//! turn into the short-term stream, promotes recalled candidate skills on a
//! verified turn, and (past an idle threshold) consolidates into long-term
//! memory. `/reflect`, `/sleep`, `/groom`, `/memory` drive memory explicitly.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use aisdk::core::capabilities::{TextInputSupport, ToolCallSupport};
use aisdk::core::language_model::LanguageModel;
use rk_compose::{
    judge_prompt, parse_judge, EvidenceJournal, JudgeResult, VerificationReport, Verifier,
};
use rk_config::Config;
use rk_constrain::{
    BashGuard, ModePolicy, PermissionMode, PolicyChain, SecurityLog, ToolDispatch, WorkspacePolicy,
};
use rk_feed::{
    consolidate_apply, consolidation_prompt, groom_apply, groom_prompt, recall,
    register_agent_tool, register_builtins, register_task_management_tools, register_task_tools,
    register_web_tools, system_prompt, AttributionContext, BackgroundTaskStore, ConsolidationScope,
    ConsolidationStats, Embedder, Memory, Observation, SessionFactory, SqliteStore, SqliteStream,
    Store, Stream, TaskState, TaskStore, ToolError, ToolRegistry, DEFAULT_RECALL_K,
};
use rk_kernel::{complete, run_turn};
use rk_observe::{InterventionKind, InterventionLogger, MhirReport, Tracer};

const CONSOLIDATE_SYSTEM: &str =
    "You are a memory consolidation engine for an AI agent. Output ONLY the requested JSON.";
const JUDGE_SYSTEM: &str =
    "You are a strict success-criteria judge. Output ONLY the requested JSON.";
const CONSOLIDATE_WINDOW: usize = 20;

/// The result of one turn: the reply plus its verification verdict.
pub struct TurnOutcome {
    /// The model's final reply.
    pub reply: String,
    /// The deterministic verification report for the turn.
    pub report: VerificationReport,
}

/// A live conversation against a model, bound to one workspace + policy + memory.
pub struct Session<M> {
    model: M,
    dispatch: Arc<dyn ToolDispatch>,
    tracer: Arc<Tracer>,
    journal: EvidenceJournal,
    interventions: InterventionLogger,
    verifier: Verifier,
    stream: Arc<dyn Stream>,
    store: Arc<dyn Store>,
    task: Arc<TaskStore>,
    embedder: Option<Arc<dyn Embedder>>,
    permission_mode: String,
    system: String,
    session_id: String,
    recall_k: usize,
    idle_threshold: usize,
    turn_counter: AtomicUsize,
    msg_counter: AtomicUsize,
    last_report: Mutex<Option<VerificationReport>>,
    last_unverified: AtomicBool,
    last_attribution: Mutex<Option<AttributionContext>>,
}

impl<M> Session<M>
where
    M: LanguageModel + TextInputSupport + ToolCallSupport + Clone,
{
    /// Build a top-level session (subagent depth 0).
    pub fn new(config: &Config, model: M) -> anyhow::Result<Self> {
        Self::new_at_depth(config, model, 0)
    }

    /// Build a session at subagent `depth` (0 = top-level). The registered
    /// `agent` tool spawns children at `depth + 1`, bounded by
    /// `RUSTYKEYS_MAX_AGENT_DEPTH` (ADR-0017).
    pub fn new_at_depth(config: &Config, model: M, depth: usize) -> anyhow::Result<Self> {
        let tracer = Arc::new(Tracer::new());
        let state_dir = config.workspace.join(".rustykeys");
        std::fs::create_dir_all(&state_dir)?;
        let session_id = new_session_id();

        let mode = PermissionMode::from_config(
            &config.permission_mode,
            config.allow_bypass,
            &config.allowed_tools,
        );
        // Mode gate runs first (cheapest, broadest), then the workspace boundary
        // and the bash security checkers (which log blocks to security.jsonl).
        let security_log = Arc::new(SecurityLog::new(
            state_dir.join("security.jsonl"),
            session_id.clone(),
        ));
        let policy = PolicyChain::new()
            .with(Arc::new(ModePolicy::new(mode.clone())))
            .with(Arc::new(WorkspacePolicy::new(config.workspace.clone())))
            .with(Arc::new(BashGuard::new().with_log(security_log)));

        let task = Arc::new(TaskStore::open(&state_dir));
        let mut registry = ToolRegistry::new(Arc::new(policy)).with_tracer(tracer.clone());
        register_builtins(&mut registry, config.workspace.clone());
        register_task_tools(&mut registry, task.clone());
        register_task_management_tools(&mut registry, Arc::new(BackgroundTaskStore::new()));
        if config.allow_web {
            register_web_tools(&mut registry);
        }
        let max_agent_depth = std::env::var("RUSTYKEYS_MAX_AGENT_DEPTH")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3);
        register_agent_tool(
            &mut registry,
            Arc::new(AgentFactory {
                config: config.clone(),
                model: model.clone(),
                depth: depth + 1,
                max_depth: max_agent_depth,
            }),
        );

        let stream = SqliteStream::open(&state_dir.join("stream.db"), session_id.clone())?;
        let store = SqliteStore::open(&state_dir.join("store.db"))?;

        let idle_threshold = std::env::var("RUSTYKEYS_IDLE_THRESHOLD")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8);

        Ok(Self {
            model,
            dispatch: Arc::new(registry),
            tracer,
            journal: EvidenceJournal::new(&state_dir),
            interventions: InterventionLogger::new(&state_dir, session_id.clone()),
            verifier: Verifier::deterministic(),
            stream: Arc::new(stream),
            store: Arc::new(store),
            task,
            embedder: None,
            system: system_prompt(config.harness_level),
            permission_mode: mode.as_str().to_string(),
            session_id,
            recall_k: DEFAULT_RECALL_K,
            idle_threshold,
            turn_counter: AtomicUsize::new(0),
            msg_counter: AtomicUsize::new(0),
            last_report: Mutex::new(None),
            last_unverified: AtomicBool::new(false),
            last_attribution: Mutex::new(None),
        })
    }

    /// Attach an embedder to enable semantic recall (Phase 5). Without one,
    /// recall falls back to FTS5 lexical.
    pub fn with_embedder(mut self, embedder: Arc<dyn Embedder>) -> Self {
        self.embedder = Some(embedder);
        self
    }

    /// Run one user turn: orient → dispatch (traced) → verify → journal → capture
    /// → promote/consolidate. Records `unverified_followup` if the prior turn was
    /// unverified.
    pub async fn send(&self, prompt: &str) -> anyhow::Result<TurnOutcome> {
        let msg_id = self.next_msg_id();
        if self.last_unverified.load(Ordering::Relaxed) {
            self.interventions
                .record(InterventionKind::UnverifiedFollowup, "", &msg_id)?;
        }

        // Orient: render the active Task State + recall long-term memory (the
        // recall query is anchored on the goal; the boost is the failure being
        // recovered from). Both land in the oriented context, NOT the static
        // system prompt (PRD 03).
        let boost = self
            .last_attribution
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let task_block = self.task.render();
        let goal = self.task.goal();
        // Criteria captured *at orient* — a turn that sets the task is not judged
        // against the criteria it just created.
        let criteria = self.task.success_criteria();
        let query = if goal.is_empty() {
            prompt.to_string()
        } else {
            format!("{goal} {prompt}")
        };
        let oriented = recall(
            self.store.as_ref(),
            &query,
            self.recall_k,
            now(),
            boost.as_ref(),
            self.embedder.as_deref(),
        )
        .await?;
        let extra: String = [task_block, oriented.block.clone()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        let prompt_with_context = if extra.is_empty() {
            prompt.to_string()
        } else {
            format!("{extra}\n\n{prompt}")
        };

        self.stream.append(&obs("user", "message", prompt)).await?;

        self.tracer.start_episode();
        let reply = run_turn(
            self.model.clone(),
            &self.system,
            &prompt_with_context,
            self.dispatch.clone(),
        )
        .await?;
        self.tracer.set_final_reached(true);

        let episode = self.tracer.episode();
        let mut report = self.verifier.verify(&reply, &episode);

        // Semantic verification: when the task has success criteria, the judge
        // evaluates the reply against them. A call/parse failure is
        // judge_unavailable — never a silent pass (PRD 05).
        if !criteria.is_empty() {
            let prompt = judge_prompt(&reply, &goal, &criteria);
            let jr = match complete(self.model.clone(), JUDGE_SYSTEM, &prompt).await {
                Ok(emit) => parse_judge(&emit),
                Err(e) => JudgeResult::unavailable(format!("judge call failed: {e}")),
            };
            report = report.with_judge(jr);
        }

        let n = self.turn_counter.fetch_add(1, Ordering::Relaxed);
        let turn_id = format!("{}_turn_{n}", self.session_id);
        self.journal
            .record_turn(&self.session_id, &turn_id, &reply, &episode, &report)?;

        // Capture the turn into the short-term stream.
        self.stream
            .append(&obs("assistant", "message", &reply))
            .await?;
        self.stream
            .append(&obs("system", "verification", &report.as_observation()))
            .await?;

        // Close the loop: a VERIFIED turn promotes the candidate skills it
        // recalled (ADR-0031); a failed turn records its attribution for the next
        // turn's boost + consolidation feed.
        if report.verified {
            self.promote_recalled_candidates(&oriented.entries).await?;
            *self
                .last_attribution
                .lock()
                .unwrap_or_else(|p| p.into_inner()) = None;
        } else {
            *self
                .last_attribution
                .lock()
                .unwrap_or_else(|p| p.into_inner()) = attribution_context(&report);
        }

        self.last_unverified
            .store(!report.verified, Ordering::Relaxed);
        *self.last_report.lock().unwrap_or_else(|p| p.into_inner()) = Some(report.clone());

        // Idle consolidation once enough observations have accrued.
        if self.stream.recent(self.idle_threshold).await?.len() >= self.idle_threshold {
            let _ = self.consolidate(ConsolidationScope::Idle).await;
        }

        Ok(TurnOutcome { reply, report })
    }

    /// Recall block for `query` (the `/memory`-style preview; also what `send`
    /// prepends). Exposed so cross-session recall is observable.
    pub async fn recall_block(&self, query: &str) -> anyhow::Result<String> {
        Ok(recall(
            self.store.as_ref(),
            query,
            self.recall_k,
            now(),
            None,
            self.embedder.as_deref(),
        )
        .await?
        .block)
    }

    /// Explicit idle-style consolidation (`/reflect`).
    pub async fn reflect(&self) -> anyhow::Result<ConsolidationStats> {
        self.consolidate(ConsolidationScope::Explicit).await
    }

    /// Session-end consolidation + prune + groom (`/sleep`).
    pub async fn sleep(&self) -> anyhow::Result<ConsolidationStats> {
        let mut stats = self.consolidate(ConsolidationScope::Sleep).await?;
        // Decay/prune non-validated, low-importance, stale memories.
        stats.pruned = self.store.prune(now(), 0.3).await?;
        stats.groomed = self.groom().await?.groomed;
        Ok(stats)
    }

    /// Skill grooming via the model (`/groom`).
    pub async fn groom(&self) -> anyhow::Result<ConsolidationStats> {
        let skills = self.store.skills().await?;
        if skills.is_empty() {
            return Ok(ConsolidationStats::default());
        }
        let emit = complete(
            self.model.clone(),
            CONSOLIDATE_SYSTEM,
            &groom_prompt(&skills),
        )
        .await?;
        let stats = groom_apply(self.store.as_ref(), &emit, now()).await?;
        self.journal.record_improvement(
            &self.session_id,
            "groom",
            stats.created,
            stats.updated,
            stats.pruned,
            stats.groomed,
        )?;
        Ok(stats)
    }

    /// The most-recently-created `n` memories (`/memory`).
    pub async fn memory_recent(&self, n: usize) -> anyhow::Result<Vec<Memory>> {
        Ok(self.store.recent(n).await?)
    }

    /// Set the active task from the CLI (`/task`).
    pub fn set_task(&self, goal: &str, criteria: Vec<String>, scope: Vec<String>) {
        self.task.set_task(goal, criteria, scope);
    }

    /// A snapshot of the current Task State (`/task` with no args).
    pub fn task_state(&self) -> TaskState {
        self.task.snapshot()
    }

    async fn consolidate(&self, scope: ConsolidationScope) -> anyhow::Result<ConsolidationStats> {
        let observations = self.stream.recent(CONSOLIDATE_WINDOW).await?;
        let attribution = self
            .last_attribution
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let prompt = consolidation_prompt(&observations, scope, attribution.as_ref());
        let emit = complete(self.model.clone(), CONSOLIDATE_SYSTEM, &prompt).await?;
        let stats =
            consolidate_apply(self.store.as_ref(), &emit, now(), self.embedder.as_deref()).await?;
        self.journal.record_improvement(
            &self.session_id,
            scope.as_str(),
            stats.created,
            stats.updated,
            stats.pruned,
            stats.groomed,
        )?;
        Ok(stats)
    }

    async fn promote_recalled_candidates(
        &self,
        entries: &[rk_feed::ContextEntry],
    ) -> anyhow::Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let recalled: std::collections::HashSet<&str> =
            entries.iter().map(|e| e.artifact.as_str()).collect();
        for skill in self.store.skills().await? {
            if !skill.validated && recalled.contains(skill.title.as_str()) {
                self.store.set_validated(&skill.title, true).await?;
            }
        }
        Ok(())
    }

    /// The most recent turn's verification report, if any (`/verify`).
    pub fn last_report(&self) -> Option<VerificationReport> {
        self.last_report
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Record that the user inspected verification (`manual_verify`, benign).
    pub fn note_manual_verify(&self) -> anyhow::Result<()> {
        let msg_id = self.next_msg_id();
        self.interventions
            .record(InterventionKind::ManualVerify, "", &msg_id)?;
        Ok(())
    }

    /// Compute M-HIR over the journaled turn count (`/mhir`).
    pub fn mhir(&self) -> anyhow::Result<MhirReport> {
        let turns = self.journal.count_turns()?;
        Ok(self.interventions.mhir(turns)?)
    }

    fn next_msg_id(&self) -> String {
        format!(
            "{}_msg_{}",
            self.session_id,
            self.msg_counter.fetch_add(1, Ordering::Relaxed)
        )
    }

    /// The active permission mode label (snake_case), for `/permissions`.
    pub fn permission_mode(&self) -> &str {
        &self.permission_mode
    }

    /// The advertised tool names (for the startup banner / diagnostics).
    pub fn tool_names(&self) -> Vec<String> {
        self.dispatch
            .schemas()
            .into_iter()
            .map(|(n, _)| n)
            .collect()
    }
}

fn obs(role: &str, kind: &str, content: &str) -> Observation {
    Observation {
        ts: now(),
        role: role.to_string(),
        kind: kind.to_string(),
        content: content.to_string(),
    }
}

/// Build the consolidation attribution feed from a failed report's first attribution.
fn attribution_context(report: &VerificationReport) -> Option<AttributionContext> {
    report.attributions.first().map(|a| AttributionContext {
        failure_type: serde_json::to_value(a.failure_type)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default(),
        layer: a.layer.clone(),
        evidence: a.evidence.clone(),
    })
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn new_session_id() -> String {
    format!("s_{:x}", now().to_bits())
}

/// Builds + runs child sessions for the `agent` tool (ADR-0017). Holds the
/// config + model to reconstruct a child; `depth` is the level children run at,
/// bounded by `max_depth` (the `AgentDepthPolicy`). v1 ignores the `tools`
/// subset — a child inherits the full registry.
struct AgentFactory<M> {
    config: Config,
    model: M,
    depth: usize,
    max_depth: usize,
}

#[async_trait::async_trait]
impl<M> SessionFactory for AgentFactory<M>
where
    M: LanguageModel + TextInputSupport + ToolCallSupport + Clone,
{
    async fn spawn(&self, task: &str, _tools: Option<Vec<String>>) -> Result<String, ToolError> {
        if self.depth > self.max_depth {
            return Err(ToolError::Other(format!(
                "agent depth {} exceeds RUSTYKEYS_MAX_AGENT_DEPTH={}",
                self.depth, self.max_depth
            )));
        }
        let child = Session::new_at_depth(&self.config, self.model.clone(), self.depth)
            .map_err(|e| ToolError::Other(e.to_string()))?;
        let outcome = child
            .send(task)
            .await
            .map_err(|e| ToolError::Other(e.to_string()))?;
        Ok(outcome.reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rk_kernel::fake::FakeLanguageModel;

    #[tokio::test]
    async fn agent_factory_blocks_beyond_max_depth() {
        let config = Config::resolve(|k| match k {
            "RUSTYKEYS_MODEL" => Some("fake".into()),
            "RUSTYKEYS_WORKSPACE" => Some("/tmp".into()),
            _ => None,
        })
        .unwrap();
        // depth 4 > max 3 ⇒ spawn refuses before building (or running) a child.
        let factory = AgentFactory {
            config,
            model: FakeLanguageModel::new(vec![]),
            depth: 4,
            max_depth: 3,
        };
        assert!(factory.spawn("task", None).await.is_err());
    }
}
