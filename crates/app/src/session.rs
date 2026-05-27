//! `Session` — the centre of the harness (ARCHITECTURE §6). Phase-2: wires
//! config + a policy-vetted, traced tool registry + a model, runs one turn per
//! [`Session::send`], then verifies the turn and journals an evidence record.
//! Memory and the semantic judge land in later phases.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use aisdk::core::capabilities::{TextInputSupport, ToolCallSupport};
use aisdk::core::language_model::LanguageModel;
use rk_compose::{EvidenceJournal, VerificationReport, Verifier};
use rk_config::Config;
use rk_constrain::{PolicyChain, ToolDispatch, WorkspacePolicy};
use rk_feed::{register_builtins, system_prompt, ToolRegistry};
use rk_kernel::run_turn;
use rk_observe::Tracer;

/// The result of one turn: the reply plus its verification verdict.
pub struct TurnOutcome {
    /// The model's final reply.
    pub reply: String,
    /// The deterministic verification report for the turn.
    pub report: VerificationReport,
}

/// A live conversation against a model, bound to one workspace + policy.
pub struct Session<M> {
    model: M,
    dispatch: Arc<dyn ToolDispatch>,
    tracer: Arc<Tracer>,
    journal: EvidenceJournal,
    verifier: Verifier,
    system: String,
    session_id: String,
    turn_counter: AtomicUsize,
}

impl<M> Session<M>
where
    M: LanguageModel + TextInputSupport + ToolCallSupport + Clone,
{
    /// Build a session: workspace policy + traced built-in tools + the static
    /// system prompt + a deterministic verifier + the evidence journal.
    pub fn new(config: &Config, model: M) -> Self {
        let tracer = Arc::new(Tracer::new());
        let policy =
            PolicyChain::new().with(Arc::new(WorkspacePolicy::new(config.workspace.clone())));
        let mut registry = ToolRegistry::new(Arc::new(policy)).with_tracer(tracer.clone());
        register_builtins(&mut registry, config.workspace.clone());

        Self {
            model,
            dispatch: Arc::new(registry),
            tracer,
            journal: EvidenceJournal::new(&config.workspace.join(".rustykeys")),
            verifier: Verifier::deterministic(),
            system: system_prompt(config.harness_level),
            session_id: new_session_id(),
            turn_counter: AtomicUsize::new(0),
        }
    }

    /// Run one user turn: dispatch (traced) → verify → journal. Returns the reply
    /// and its verification report.
    pub async fn send(&self, prompt: &str) -> anyhow::Result<TurnOutcome> {
        self.tracer.start_episode();
        let reply = run_turn(
            self.model.clone(),
            &self.system,
            prompt,
            self.dispatch.clone(),
        )
        .await?;
        self.tracer.set_final_reached(true);

        let episode = self.tracer.episode();
        let report = self.verifier.verify(&reply, &episode);

        let n = self.turn_counter.fetch_add(1, Ordering::Relaxed);
        let turn_id = format!("{}_turn_{n}", self.session_id);
        self.journal
            .record_turn(&self.session_id, &turn_id, &reply, &episode, &report)?;

        Ok(TurnOutcome { reply, report })
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

fn new_session_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("s_{nanos:x}")
}
