//! Web gateway (PRD 06 / Phase 14): a `rusty_axum` HTTP+SSE surface over
//! `Session::send()`. The harness layer is not bypassed — `/chat` runs the same
//! turn cycle, verification, and evidence journal as the CLI. Behind the
//! `gateway` feature.
//!
//! Single mode shares one `Session` behind a mutex; multi mode routes a
//! `session_id` header to a per-tenant `Session` with idle-TTL + max-session
//! eviction. A bearer secret (when set) gates every request and scopes which
//! `session_id`s a caller may reach.
//!
//! The server runs on `rusty_tokio` (#879). `Session` is still tokio-based
//! (aisdk/reqwest, `tokio::spawn`), so the router's outermost layer is
//! `rusty_axum_tokio::TokioBridge`, entering the caller's tokio runtime on
//! every poll of a handler and of the `/stream` body.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use aisdk::core::capabilities::{TextInputSupport, ToolCallSupport};
use aisdk::core::language_model::LanguageModel;
use rk_config::Config;
use rusty_axum::cors::{Allow, Cors};
use rusty_axum::response::sse::{Event, Sse};
use rusty_axum::routing::{get, post};
use rusty_axum::serve::ServeOptions;
use rusty_axum::{HeaderMap, IntoResponse, Json, Method, Query, Router, State, StatusCode};
use rusty_axum_tokio::TokioBridge;
use rusty_crypto_key::constant_time_eq;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::Session;
use rk_constrain::{ApprovalGate, ApprovalTrigger};

/// Gateway session model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// One shared session per server instance.
    Single,
    /// `session_id`-routed per-tenant sessions with TTL eviction.
    Multi,
}

impl Mode {
    fn from_env() -> Self {
        match std::env::var("RUSTYKEYS_GATEWAY_MODE").as_deref() {
            Ok("multi") => Mode::Multi,
            _ => Mode::Single,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Mode::Single => "single",
            Mode::Multi => "multi",
        }
    }
}

struct Entry<M> {
    session: Arc<Session<M>>,
    last_active: Instant,
}

/// Shared gateway state: builds + routes sessions, holds auth config.
pub struct Gateway<M> {
    config: Config,
    model: M,
    mode: Mode,
    secret: Option<String>,
    cors_origin: String,
    ttl: Duration,
    max_sessions: usize,
    single: Mutex<Option<Arc<Session<M>>>>,
    multi: Mutex<HashMap<String, Entry<M>>>,
}

impl<M> Gateway<M>
where
    M: LanguageModel + TextInputSupport + ToolCallSupport + Clone + Send + Sync + 'static,
{
    /// Build from config + model, reading gateway settings from the environment.
    pub fn new(config: Config, model: M) -> Self {
        let ttl_secs = std::env::var("RUSTYKEYS_SESSION_TTL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3600);
        let max_sessions = std::env::var("RUSTYKEYS_MAX_SESSIONS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(64);
        Self {
            config,
            model,
            mode: Mode::from_env(),
            secret: std::env::var("RUSTYKEYS_GATEWAY_SECRET")
                .ok()
                .filter(|s| !s.is_empty()),
            // Same-origin by default (round-4 finding 1c): an unconfigured
            // origin allows no cross-origin caller, rather than mirroring `*`
            // back to everyone. Set `RUSTYKEYS_GATEWAY_CORS_ORIGIN` explicitly
            // to allow a specific origin (or `*`, if genuinely wanted).
            cors_origin: std::env::var("RUSTYKEYS_GATEWAY_CORS_ORIGIN").unwrap_or_default(),
            ttl: Duration::from_secs(ttl_secs),
            max_sessions,
            single: Mutex::new(None),
            multi: Mutex::new(HashMap::new()),
        }
    }

    /// Override the bearer secret (used in tests).
    pub fn with_secret(mut self, secret: Option<String>) -> Self {
        self.secret = secret;
        self
    }

    /// Force the session mode (used in tests).
    pub fn with_mode(mut self, mode: Mode) -> Self {
        self.mode = mode;
        self
    }

    /// Override the allowed CORS origin (used in tests).
    pub fn with_cors_origin(mut self, origin: &str) -> Self {
        self.cors_origin = origin.to_string();
        self
    }

    fn build_session(&self) -> anyhow::Result<Arc<Session<M>>> {
        // No interactive adapter answers the gateway's approval prompts, so
        // the channel receiver is intentionally dropped: `ApprovalGate`
        // already fails closed when no adapter is listening (`approval.rs`),
        // which blocks first-bash-use / new-file-write actions exactly like
        // ACP/desktop gate them behind a human, instead of leaving the
        // gateway tool-approval-free (round-4 finding 1d).
        let (tx, _rx) = rk_constrain::approval_channel(8);
        let gate = ApprovalGate::new(
            vec![ApprovalTrigger::NewFilePath, ApprovalTrigger::BashFirstUse],
            tx,
        );
        Ok(Arc::new(Session::new_with_policy(
            &self.config,
            self.model.clone(),
            Arc::new(gate),
        )?))
    }

    /// Authorize a request: when a secret is set, require `Authorization:
    /// Bearer <secret>`. Returns the bearer token (None when unauthenticated).
    fn authorize(&self, headers: &HeaderMap) -> Result<Option<String>, StatusCode> {
        let presented = headers
            .get("authorization")
            .and_then(rusty_oauth::bearer::token_from_authorization)
            .map(str::to_string);
        match &self.secret {
            Some(expected) => match &presented {
                Some(tok) if constant_time_eq(tok.as_bytes(), expected.as_bytes()) => Ok(presented),
                _ => Err(StatusCode::UNAUTHORIZED),
            },
            None => Ok(presented),
        }
    }

    /// Resolve the session for a request, evicting idle entries first (multi).
    async fn session_for(&self, headers: &HeaderMap) -> Result<Arc<Session<M>>, StatusCode> {
        match self.mode {
            Mode::Single => {
                let mut guard = self.single.lock().await;
                if guard.is_none() {
                    *guard = Some(
                        self.build_session()
                            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
                    );
                }
                guard.clone().ok_or(StatusCode::INTERNAL_SERVER_ERROR)
            }
            Mode::Multi => {
                // `session_id` ↔ auth binding: a multi-mode caller must name a
                // session via the `x-session-id` header (rejected otherwise).
                let sid = headers
                    .get("x-session-id")
                    .map(str::to_string)
                    .ok_or(StatusCode::BAD_REQUEST)?;
                let mut map = self.multi.lock().await;
                // Evict idle sessions.
                let ttl = self.ttl;
                map.retain(|_, e| e.last_active.elapsed() < ttl);
                if let Some(e) = map.get_mut(&sid) {
                    e.last_active = Instant::now();
                    return Ok(e.session.clone());
                }
                if map.len() >= self.max_sessions {
                    return Err(StatusCode::TOO_MANY_REQUESTS);
                }
                let session = self
                    .build_session()
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                map.insert(
                    sid,
                    Entry {
                        session: session.clone(),
                        last_active: Instant::now(),
                    },
                );
                Ok(session)
            }
        }
    }

    /// The router over this gateway. `tokio` is the runtime its handlers'
    /// tokio work runs on (see the module docs).
    pub fn router(self: Arc<Self>, tokio: tokio::runtime::Handle) -> Router {
        // Methods/headers are scoped to what the gateway's routes actually
        // use, not `Any` (round-4 finding 1). An empty/unconfigured
        // `cors_origin` allows no origin at all (same-origin default); `*`
        // allows any; anything else is compared exactly, so a malformed
        // origin matches nothing.
        let origins = match self.cors_origin.as_str() {
            "" => Allow::List(Vec::new()),
            "*" => Allow::Any,
            origin => Allow::List(vec![origin.to_string()]),
        };
        let cors = Cors::new()
            .allow_methods(Allow::List(vec![Method::Get, Method::Post]))
            .allow_headers(Allow::List(vec![
                "authorization".into(),
                "content-type".into(),
                "x-session-id".into(),
            ]))
            .allow_origin(origins);
        Router::new()
            .route("/health", get(health::<M>))
            .route("/ready", get(ready::<M>))
            .route("/chat", post(chat::<M>))
            .route("/stream", get(stream::<M>))
            .route("/verify", get(verify::<M>))
            .route("/evidence", get(evidence::<M>))
            .route("/mhir", get(mhir::<M>))
            .route("/entropy", get(entropy::<M>))
            .route("/metrics", get(metrics::<M>))
            .layer(cors)
            .layer(TokioBridge::new(tokio))
            .with_state(self)
    }
}

type Gw<M> = State<Arc<Gateway<M>>>;

async fn health<M>(State(gw): Gw<M>, headers: HeaderMap) -> impl IntoResponse
where
    M: LanguageModel + TextInputSupport + ToolCallSupport + Clone + Send + Sync + 'static,
{
    if let Err(code) = gw.authorize(&headers) {
        return code.into_response();
    }
    Json(json!({
        "status": "ok",
        "check": "liveness",
        "model": gw.config.model,
        "mode": gw.mode.as_str(),
    }))
    .into_response()
}

/// Readiness (vs `/health`'s liveness): the gateway can actually serve a turn —
/// the model is configured, the workspace is a writable dir, and (single mode)
/// the shared session's SQLite store/stream round-trip. Returns `503` until
/// ready. Multi mode is per-tenant, so its data layer is checked at first use,
/// not here. `200` ⇒ ready.
async fn ready<M>(State(gw): Gw<M>, headers: HeaderMap) -> impl IntoResponse
where
    M: LanguageModel + TextInputSupport + ToolCallSupport + Clone + Send + Sync + 'static,
{
    if let Err(code) = gw.authorize(&headers) {
        return code.into_response();
    }
    let model_ok = !gw.config.model.trim().is_empty();
    let workspace_ok = gw.config.workspace.is_dir();
    let sqlite_ok = match gw.mode {
        // Reuse the lazily-built, cached shared session so the probe stays cheap.
        Mode::Single => match gw.session_for(&headers).await {
            Ok(s) => s.recall_block("ready").await.is_ok(),
            Err(_) => false,
        },
        Mode::Multi => true,
    };
    let ready = model_ok && workspace_ok && sqlite_ok;
    let body = Json(json!({
        "ready": ready,
        "check": "readiness",
        "checks": { "model": model_ok, "workspace": workspace_ok, "sqlite": sqlite_ok },
    }));
    let code = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (code, body).into_response()
}

async fn chat<M>(State(gw): Gw<M>, headers: HeaderMap, Json(body): Json<Value>) -> impl IntoResponse
where
    M: LanguageModel + TextInputSupport + ToolCallSupport + Clone + Send + Sync + 'static,
{
    if let Err(code) = gw.authorize(&headers) {
        return code.into_response();
    }
    let session = match gw.session_for(&headers).await {
        Ok(s) => s,
        Err(code) => return code.into_response(),
    };
    let message = body.get("message").and_then(Value::as_str).unwrap_or("");
    match session.send(message).await {
        Ok(outcome) => Json(json!({
            "reply": outcome.reply,
            "verified": outcome.report.verified,
            "checks": outcome.report.checks,
            "limits": outcome.report.limits,
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "turn_failed", "message": e.to_string() })),
        )
            .into_response(),
    }
}

async fn stream<M>(
    State(gw): Gw<M>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse
where
    M: LanguageModel + TextInputSupport + ToolCallSupport + Clone + Send + Sync + 'static,
{
    if let Err(code) = gw.authorize(&headers) {
        return code.into_response();
    }
    let session = match gw.session_for(&headers).await {
        Ok(s) => s,
        Err(code) => return code.into_response(),
    };
    let message = params.get("message").cloned().unwrap_or_default();

    // Drive the turn on a task and mirror the canonical rk:// events as named SSE
    // frames *live*: `turn_start`, each `token` as it streams (via the kernel's
    // stream_turn), then `turn_complete` + the SSE-specific `done`/`error`
    // sentinel. Names come from the single contract SSOT.
    use crate::contract::event;
    let turn_id = format!("turn_{}", now_tag());
    let (tx, rx) = futures::channel::mpsc::unbounded::<Result<Event, std::convert::Infallible>>();

    tokio::spawn(async move {
        let _ = tx.unbounded_send(Ok(Event::default()
            .event(event::TURN_START)
            .id(turn_id.clone())
            .data(json!({ "turn_id": turn_id }).to_string())));

        let token_tx = tx.clone();
        let result = session
            .send_streaming(&message, move |delta| {
                let _ =
                    token_tx.unbounded_send(Ok(Event::default().event(event::TOKEN).data(delta)));
            })
            .await;

        match result {
            Ok(outcome) => {
                let r = crate::contract::TurnResult::from_outcome(&outcome);
                let _ = tx.unbounded_send(Ok(Event::default()
                    .event(event::TURN_COMPLETE)
                    .id(turn_id.clone())
                    .data(
                        json!({
                            "turn_id": turn_id,
                            "reply": r.reply,
                            "verified": r.verified,
                        })
                        .to_string(),
                    )));
                let _ = tx.unbounded_send(Ok(Event::default()
                    .event("done")
                    .data(json!({ "turn_id": turn_id }).to_string())));
            }
            Err(e) => {
                let _ = tx.unbounded_send(Ok(Event::default().event("error").data(
                    json!({ "error": "turn_failed", "message": e.to_string() }).to_string(),
                )));
            }
        }
        // `tx` and the token sender drop here, terminating the SSE stream.
    });

    Sse::new(rx).into_response()
}

async fn verify<M>(State(gw): Gw<M>, headers: HeaderMap) -> impl IntoResponse
where
    M: LanguageModel + TextInputSupport + ToolCallSupport + Clone + Send + Sync + 'static,
{
    if let Err(code) = gw.authorize(&headers) {
        return code.into_response();
    }
    let session = match gw.session_for(&headers).await {
        Ok(s) => s,
        Err(code) => return code.into_response(),
    };
    match session.last_report() {
        Some(r) => Json(r.to_json()).into_response(),
        None => Json(json!(null)).into_response(),
    }
}

async fn evidence<M>(State(gw): Gw<M>, headers: HeaderMap) -> impl IntoResponse
where
    M: LanguageModel + TextInputSupport + ToolCallSupport + Clone + Send + Sync + 'static,
{
    if let Err(code) = gw.authorize(&headers) {
        return code.into_response();
    }
    let session = match gw.session_for(&headers).await {
        Ok(s) => s,
        Err(code) => return code.into_response(),
    };
    // The journal is already redacted at write time (ADR-0026).
    Json(json!(session.evidence_recent(20).unwrap_or_default())).into_response()
}

async fn mhir<M>(State(gw): Gw<M>, headers: HeaderMap) -> impl IntoResponse
where
    M: LanguageModel + TextInputSupport + ToolCallSupport + Clone + Send + Sync + 'static,
{
    if let Err(code) = gw.authorize(&headers) {
        return code.into_response();
    }
    let session = match gw.session_for(&headers).await {
        Ok(s) => s,
        Err(code) => return code.into_response(),
    };
    match session.mhir() {
        Ok(m) => Json(json!({
            "rate": m.rate,
            "n_interventions": m.n_interventions,
            "n_turns": m.n_turns,
            "n_unavoidable": m.n_unavoidable,
            "n_benign": m.n_benign,
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn entropy<M>(State(gw): Gw<M>, headers: HeaderMap) -> impl IntoResponse
where
    M: LanguageModel + TextInputSupport + ToolCallSupport + Clone + Send + Sync + 'static,
{
    if let Err(code) = gw.authorize(&headers) {
        return code.into_response();
    }
    let session = match gw.session_for(&headers).await {
        Ok(s) => s,
        Err(code) => return code.into_response(),
    };
    Json(json!({
        "recent": session.entropy_recent(10).unwrap_or_default(),
        "cumulative_delta": session.entropy_total_delta(),
    }))
    .into_response()
}

/// Pull-based OTLP telemetry scrape (ADR-0034 / Phase 7B). Returns the
/// session's accumulated token/cost/latency + tool-status counters. The
/// exporter reports `enabled: false` with zero counters unless
/// `RUSTYKEYS_OTLP_ENDPOINT` is set. This is a host-boundary pull, so an
/// isolated `ToolExecutor` cannot blind operators.
async fn metrics<M>(State(gw): Gw<M>, headers: HeaderMap) -> impl IntoResponse
where
    M: LanguageModel + TextInputSupport + ToolCallSupport + Clone + Send + Sync + 'static,
{
    if let Err(code) = gw.authorize(&headers) {
        return code.into_response();
    }
    let session = match gw.session_for(&headers).await {
        Ok(s) => s,
        Err(code) => return code.into_response(),
    };
    Json(json!(session.metrics_snapshot())).into_response()
}

fn now_tag() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// Serve the gateway on `addr` until shutdown, with settings from the
/// environment. See [`Gateway::serve`].
pub async fn serve<M>(config: Config, model: M, addr: &str) -> anyhow::Result<()>
where
    M: LanguageModel + TextInputSupport + ToolCallSupport + Clone + Send + Sync + 'static,
{
    Gateway::new(config, model).serve(addr).await
}

impl<M> Gateway<M>
where
    M: LanguageModel + TextInputSupport + ToolCallSupport + Clone + Send + Sync + 'static,
{
    /// Serve on `addr` until this future is dropped. Refuses to start unless
    /// a bearer secret is configured: a gateway is network-exposed by
    /// definition, so silently disabling auth when
    /// `RUSTYKEYS_GATEWAY_SECRET` is unset is not a safe default (round-4
    /// finding 1b).
    pub async fn serve(self, addr: &str) -> anyhow::Result<()> {
        if self.secret.is_none() {
            anyhow::bail!(
                "refusing to start the gateway: RUSTYKEYS_GATEWAY_SECRET is not set. Set it to \
                 a bearer token callers must present, or the gateway would serve every \
                 request unauthenticated on the network."
            );
        }
        let app = Arc::new(self).router(tokio::runtime::Handle::current());
        let addr = addr.to_string();
        // Dropping this future drops `_stop`, which ends `stopped`: the
        // server then shuts down and releases the listener, and the
        // blocking-pool thread it occupies returns.
        let (_stop, stopped) = tokio::sync::oneshot::channel::<()>();
        tokio::task::spawn_blocking(move || serve_on_rusty_tokio(app, &addr, stopped)).await?
    }
}

/// Runs the server on a `rusty_tokio` runtime of its own until `stopped`
/// completes.
fn serve_on_rusty_tokio(
    app: Router,
    addr: &str,
    stopped: tokio::sync::oneshot::Receiver<()>,
) -> anyhow::Result<()> {
    let runtime = rusty_tokio::Builder::new_multi_thread()
        .thread_name("rk-gateway")
        .build()?;
    runtime.block_on(async {
        let listener = rusty_tokio::io::TcpListener::bind_addrs(addr).await?;
        let server = rusty_axum::serve(listener, None, ServeOptions::default(), app)?;
        // A oneshot needs no reactor, so it can be awaited on rusty_tokio.
        server
            .shutdown_on(async {
                let _ = stopped.await;
            })
            .await;
        Ok(())
    })
}
