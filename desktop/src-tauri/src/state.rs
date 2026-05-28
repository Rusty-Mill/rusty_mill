//! Adapter state for the Tauri bridge.
//!
//! The Tauri `#[tauri::command]` layer must be concrete, but `rk_app::Session` is
//! generic over the language model. [`SessionApi`] is the type-erasure seam: the
//! commands hold an `Arc<dyn SessionApi>` and never name the model, so the same
//! bridge serves the real `OpenAICompatible` session in production and a scripted
//! `FakeLanguageModel` session in the headless IPC smoke test.
//!
//! Each `SessionApi` method projects an `rk_*` type into the JSON shape the
//! frontend consumes; turn-level failures collapse into the boundary error
//! taxonomy ([`crate::error`]).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use aisdk::core::capabilities::{TextInputSupport, ToolCallSupport};
use aisdk::core::language_model::LanguageModel;
use rk_app::contract::TurnResult;
use rk_app::Session;
use rk_config::Config;
use rk_constrain::{
    ApprovalGate, ApprovalRequest, ApprovalResponse, ApprovalTrigger, PlanDecision,
};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};

use crate::error::{classify, BoundaryErrorPayload};

/// The receiver end of the approval channel — drained by the bridge's setup task,
/// which emits `rk://approval_request` and parks the responder for `approval_respond`.
pub type ApprovalRx = mpsc::Receiver<ApprovalRequest>;

/// A model-agnostic façade over [`Session`]. Every method returns frontend-ready
/// JSON (or a [`BoundaryErrorPayload`]); nothing here names the concrete model.
#[async_trait::async_trait]
pub trait SessionApi: Send + Sync {
    /// Run one turn (`session_send`), pushing each streamed text delta onto
    /// `tokens` so the bridge can mirror them as `rk://token`. An owned `'static`
    /// channel (not a borrowed closure) keeps this object-safe across async-trait.
    async fn send(
        &self,
        message: &str,
        tokens: mpsc::UnboundedSender<String>,
    ) -> Result<TurnResult, BoundaryErrorPayload>;
    /// Install (or clear) the live `bash` output sink for the next turn — the
    /// bridge mirrors chunks as `rk://bash_output`.
    fn set_bash_sink(&self, tx: Option<mpsc::UnboundedSender<String>>);
    /// The tool events from the most recent turn, redacted, as `rk://tool_event`
    /// payloads.
    fn last_tool_events(&self) -> Vec<Value>;
    /// The most recent `VerificationReport` (`session_last_report`).
    fn last_report(&self) -> Option<Value>;
    /// The M-HIR report (`session_mhir`).
    fn mhir(&self) -> Value;
    /// The active config projection (`session_config`).
    fn config(&self) -> Value;
    /// Recent evidence-journal entries (`session_evidence_recent`).
    fn evidence_recent(&self, n: usize) -> Vec<Value>;
    /// Entropy-audit history (`session_entropy_history`).
    fn entropy_recent(&self, n: usize) -> Vec<Value>;
    /// Cumulative entropy delta (for the entropy panel footer).
    fn entropy_total_delta(&self) -> i64;
    /// Token-budget snapshot (`session_token_budget`).
    fn token_budget(&self) -> Value;
    /// Long-term memory snapshot (`session_memory_snapshot`).
    async fn memory_recent(&self, n: usize) -> Vec<Value>;
    /// Memory search for the `#memory` picker (`session_memory_search`).
    async fn memory_search(&self, q: &str) -> Vec<Value>;
    /// `(server, tool_count)` pairs for connected MCP servers (`mcp_servers_list`).
    async fn mcp_summary(&self) -> Vec<(String, usize)>;
    /// Record a `manual_verify` intervention (benign).
    fn note_manual_verify(&self);
    /// Enter plan mode (`/plan`).
    fn enter_plan_mode(&self);
    /// The pending plan text if `exit_plan_mode` fired (drives `rk://plan_exit`).
    fn plan_exit_pending(&self) -> Option<String>;
    /// Resolve a pending plan-exit decision.
    fn resolve_plan_exit(&self, decision: PlanDecision) -> Option<String>;
    /// Explicit consolidation (`/reflect`); returns stats for `rk://consolidation`.
    async fn reflect(&self) -> Result<Value, BoundaryErrorPayload>;
    /// Session-end consolidation + prune + groom (`/sleep`).
    async fn sleep(&self) -> Result<Value, BoundaryErrorPayload>;
    /// Skill grooming (`/groom`).
    async fn groom(&self) -> Result<Value, BoundaryErrorPayload>;
    /// Force a full compaction now (`/compact`).
    async fn compact_now(&self) -> Result<(), BoundaryErrorPayload>;
}

#[async_trait::async_trait]
impl<M> SessionApi for Session<M>
where
    M: LanguageModel + TextInputSupport + ToolCallSupport + Clone + Send + Sync + 'static,
{
    async fn send(
        &self,
        message: &str,
        tokens: mpsc::UnboundedSender<String>,
    ) -> Result<TurnResult, BoundaryErrorPayload> {
        // A concrete, owned `'static` closure — no trait object or borrow crosses
        // the async-trait boundary, so the HRTB lifetime checks resolve cleanly.
        let on_token = move |delta: &str| {
            let _ = tokens.send(delta.to_string());
        };
        match Session::send_streaming(self, message, on_token).await {
            Ok(outcome) => Ok(TurnResult::from_outcome(&outcome)),
            Err(e) => Err(classify(&e)),
        }
    }

    fn set_bash_sink(&self, tx: Option<mpsc::UnboundedSender<String>>) {
        Session::set_bash_sink(self, tx);
    }

    fn last_tool_events(&self) -> Vec<Value> {
        // Redact before emission (ADR-0026 / IPC contract §1): scrub denylisted
        // keys and token-scrub string values, preserving structure.
        Session::last_tool_events(self)
            .iter()
            .map(|e| {
                let v = serde_json::to_value(e).unwrap_or(Value::Null);
                rk_observe::redact::redact_value(&v)
            })
            .collect()
    }

    fn last_report(&self) -> Option<Value> {
        Session::last_report(self).map(|r| r.to_json())
    }

    fn mhir(&self) -> Value {
        match Session::mhir(self) {
            Ok(m) => {
                let breakdown: serde_json::Map<String, Value> = m
                    .breakdown
                    .iter()
                    .map(|(k, v)| (format!("{k:?}"), json!(v)))
                    .collect();
                json!({
                    "rate": m.rate,
                    "n_interventions": m.n_interventions,
                    "n_turns": m.n_turns,
                    "n_unavoidable": m.n_unavoidable,
                    "n_benign": m.n_benign,
                    "breakdown": breakdown,
                })
            }
            Err(e) => json!({ "error": e.to_string() }),
        }
    }

    fn config(&self) -> Value {
        json!({
            "permission_mode": self.permission_mode(),
            "isolation": self.isolation(),
            "explore_enabled": self.explore_enabled(),
        })
    }

    fn evidence_recent(&self, n: usize) -> Vec<Value> {
        Session::evidence_recent(self, n).unwrap_or_default()
    }

    fn entropy_recent(&self, n: usize) -> Vec<Value> {
        Session::entropy_recent(self, n).unwrap_or_default()
    }

    fn entropy_total_delta(&self) -> i64 {
        Session::entropy_total_delta(self)
    }

    fn token_budget(&self) -> Value {
        let (used, limit, fraction, session_total, compactions) = self.cost();
        json!({
            "used": used,
            "limit": limit,
            "fraction": fraction,
            "session_total": session_total,
            "compactions": compactions,
        })
    }

    async fn memory_recent(&self, n: usize) -> Vec<Value> {
        match Session::memory_recent(self, n).await {
            Ok(mems) => mems
                .iter()
                .map(|m| serde_json::to_value(m).unwrap_or(Value::Null))
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    async fn memory_search(&self, q: &str) -> Vec<Value> {
        // No dedicated search method in v1: filter the recent set by a
        // case-insensitive substring over title/body (the `#memory` picker).
        let needle = q.to_ascii_lowercase();
        match Session::memory_recent(self, 100).await {
            Ok(mems) => mems
                .iter()
                .filter(|m| {
                    needle.is_empty()
                        || m.title.to_ascii_lowercase().contains(&needle)
                        || m.body.to_ascii_lowercase().contains(&needle)
                })
                .map(|m| serde_json::to_value(m).unwrap_or(Value::Null))
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    async fn mcp_summary(&self) -> Vec<(String, usize)> {
        Session::mcp_summary(self).await
    }

    fn note_manual_verify(&self) {
        let _ = Session::note_manual_verify(self);
    }

    fn enter_plan_mode(&self) {
        Session::enter_plan_mode(self);
    }

    fn plan_exit_pending(&self) -> Option<String> {
        Session::plan_exit_pending(self)
    }

    fn resolve_plan_exit(&self, decision: PlanDecision) -> Option<String> {
        Session::resolve_plan_exit(self, decision)
    }

    async fn reflect(&self) -> Result<Value, BoundaryErrorPayload> {
        Session::reflect(self)
            .await
            .map(stats_json)
            .map_err(|e| classify(&e))
    }

    async fn sleep(&self) -> Result<Value, BoundaryErrorPayload> {
        Session::sleep(self)
            .await
            .map(stats_json)
            .map_err(|e| classify(&e))
    }

    async fn groom(&self) -> Result<Value, BoundaryErrorPayload> {
        Session::groom(self)
            .await
            .map(stats_json)
            .map_err(|e| classify(&e))
    }

    async fn compact_now(&self) -> Result<(), BoundaryErrorPayload> {
        Session::compact_now(self)
            .await
            .map(|_| ())
            .map_err(|e| classify(&e))
    }
}

fn stats_json(s: rk_feed::ConsolidationStats) -> Value {
    json!({
        "created": s.created,
        "updated": s.updated,
        "pruned": s.pruned,
        "groomed": s.groomed,
    })
}

/// Shared, model-agnostic state held by the Tauri app and every command.
pub struct AppState {
    /// The type-erased session every command drives.
    pub session: Arc<dyn SessionApi>,
    /// The workspace root (for the `@file` picker).
    pub workspace: PathBuf,
    /// The in-flight approval responder, parked by the setup task until
    /// `approval_respond` answers it. One approval pends at a time per turn.
    pub pending_approval: Arc<Mutex<Option<oneshot::Sender<ApprovalResponse>>>>,
    /// Session config overrides recorded by `config_set` (restart-only keys
    /// are flagged, not applied live, in v1).
    pub overrides: Mutex<serde_json::Map<String, Value>>,
}

impl AppState {
    /// Park an approval responder; called by the setup task per `rk://approval_request`.
    pub fn park_approval(&self, responder: oneshot::Sender<ApprovalResponse>) {
        *self
            .pending_approval
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some(responder);
    }

    /// Answer the parked approval (if any) with `response`.
    pub fn answer_approval(&self, response: ApprovalResponse) {
        if let Some(tx) = self
            .pending_approval
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            let _ = tx.send(response);
        }
    }
}

/// Build the production state: an `OpenAICompatible` session whose policy chain
/// ends in an [`ApprovalGate`] (first bash, new-file writes), plus the approval
/// receiver the bridge drains. Mirrors the CLI's provider wiring (`main.rs`).
pub fn build_production_state() -> anyhow::Result<(AppState, ApprovalRx)> {
    use aisdk::core::capabilities::DynamicModel;
    use aisdk::providers::OpenAICompatible;

    let config = Config::from_env()?;
    let base_url = std::env::var("RUSTYKEYS_BASE_URL")
        .unwrap_or_else(|_| "http://localhost:11434/v1".to_string());
    let api_key = std::env::var("RUSTYKEYS_API_KEY").unwrap_or_else(|_| "ollama".to_string());

    let model = OpenAICompatible::<DynamicModel>::builder()
        .model_name(config.model.clone())
        .base_url(base_url.clone())
        .api_key(api_key.clone())
        .build()?;

    let (tx, rx) = mpsc::channel::<ApprovalRequest>(8);
    let gate = ApprovalGate::new(
        vec![ApprovalTrigger::NewFilePath, ApprovalTrigger::BashFirstUse],
        tx,
    );
    let mut session = Session::new_with_policy(&config, model, Arc::new(gate))?;

    if let Some(embed_model) = &config.embed_model {
        let em = OpenAICompatible::<DynamicModel>::builder()
            .model_name(embed_model.clone())
            .base_url(base_url)
            .api_key(api_key)
            .build()?;
        session = session.with_embedder(Arc::new(rk_app::AiSdkEmbedder::new(em)));
    }

    let state = AppState {
        session: Arc::new(session),
        workspace: config.workspace,
        pending_approval: Arc::new(Mutex::new(None)),
        overrides: Mutex::new(serde_json::Map::new()),
    };
    Ok((state, rx))
}
