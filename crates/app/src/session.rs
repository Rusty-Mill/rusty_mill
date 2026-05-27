//! `Session` — the centre of the harness (ARCHITECTURE §6). Phase-2: wires
//! config + a policy-vetted, traced tool registry + a model, runs one turn per
//! [`Session::send`], then verifies the turn and journals an evidence record.
//! Memory and the semantic judge land in later phases.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use aisdk::core::capabilities::{TextInputSupport, ToolCallSupport};
use aisdk::core::language_model::LanguageModel;
use rk_compose::{EvidenceJournal, VerificationReport, Verifier};
use rk_config::Config;
use rk_constrain::{PolicyChain, ToolDispatch, WorkspacePolicy};
use rk_feed::{register_builtins, system_prompt, ToolRegistry};
use rk_kernel::run_turn;
use rk_observe::{InterventionKind, InterventionLogger, MhirReport, Tracer};

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
    interventions: InterventionLogger,
    verifier: Verifier,
    system: String,
    session_id: String,
    turn_counter: AtomicUsize,
    msg_counter: AtomicUsize,
    last_report: Mutex<Option<VerificationReport>>,
    last_unverified: AtomicBool,
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

        let state_dir = config.workspace.join(".rustykeys");
        let session_id = new_session_id();
        Self {
            model,
            dispatch: Arc::new(registry),
            tracer,
            journal: EvidenceJournal::new(&state_dir),
            interventions: InterventionLogger::new(&state_dir, session_id.clone()),
            verifier: Verifier::deterministic(),
            system: system_prompt(config.harness_level),
            session_id,
            turn_counter: AtomicUsize::new(0),
            msg_counter: AtomicUsize::new(0),
            last_report: Mutex::new(None),
            last_unverified: AtomicBool::new(false),
        }
    }

    /// Run one user turn: dispatch (traced) → verify → journal. If the *previous*
    /// turn was unverified, records an `unverified_followup` intervention against
    /// this message first (PRD 04).
    pub async fn send(&self, prompt: &str) -> anyhow::Result<TurnOutcome> {
        let msg_id = self.next_msg_id();
        if self.last_unverified.load(Ordering::Relaxed) {
            self.interventions
                .record(InterventionKind::UnverifiedFollowup, "", &msg_id)?;
        }

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

        self.last_unverified
            .store(!report.verified, Ordering::Relaxed);
        *self.last_report.lock().unwrap_or_else(|p| p.into_inner()) = Some(report.clone());

        Ok(TurnOutcome { reply, report })
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
