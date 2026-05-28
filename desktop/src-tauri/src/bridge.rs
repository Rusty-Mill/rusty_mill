//! The Tauri IPC bridge: one `#[tauri::command]` per name in the canonical
//! contract (`rk_app::contract::command`), and the `rk://` event emitters
//! (`rk_app::contract::event`). Commands reference the contract SSOT rather than
//! re-deriving names — the anti-drift guarantee the round-3 audit required.
//!
//! Commands are generic over the Tauri runtime so the headless IPC smoke test can
//! drive the exact same handlers on the mock runtime.

use rk_app::contract::{command, event};
use rk_constrain::{ApprovalResponse, PlanDecision};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Runtime, State};

use crate::error::BoundaryErrorPayload;
use crate::state::AppState;
use crate::{secrets, COMMANDS, EVENTS};

/// Emit a canonical `rk://` event, best-effort.
fn emit<R: Runtime>(app: &AppHandle<R>, name: &str, payload: impl serde::Serialize + Clone) {
    let _ = app.emit(&event::uri(name), payload);
}

fn now_tag() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// `session_send` — run one turn, mirroring the canonical events: `turn_start`
/// before the kernel runs, then (on success) each `tool_event`, the post-turn
/// `entropy` audit, any `plan_exit`, and finally `turn_complete`. A failure does
/// **not** emit `turn_complete`; it rejects with the boundary taxonomy so the
/// frontend's `catch` is the only path that clears the composer lock.
#[tauri::command]
async fn session_send<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    message: String,
    #[allow(unused_variables)] attachments: Option<Vec<String>>,
) -> Result<rk_app::contract::TurnResult, BoundaryErrorPayload> {
    let turn_id = format!("turn_{}", now_tag());
    emit(&app, event::TURN_START, json!({ "turn_id": turn_id }));

    // Mirror streamed output live: text deltas as `rk://token`, `bash` stdout/
    // stderr as `rk://bash_output`. The session pushes onto unbounded channels;
    // pump tasks emit until the senders drop (turn done), then we drain both
    // before `turn_complete`.
    let (token_tx, mut token_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let (bash_tx, mut bash_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let token_app = app.clone();
    let bash_app = app.clone();
    let token_pump = tauri::async_runtime::spawn(async move {
        while let Some(delta) = token_rx.recv().await {
            let _ = token_app.emit(&event::uri(event::TOKEN), delta);
        }
    });
    let bash_pump = tauri::async_runtime::spawn(async move {
        while let Some(chunk) = bash_rx.recv().await {
            let _ = bash_app.emit(&event::uri(event::BASH_OUTPUT), chunk);
        }
    });
    state.session.set_bash_sink(Some(bash_tx));
    let send_result = state.session.send(&message, token_tx).await;
    state.session.set_bash_sink(None); // drop the bash sender so its pump ends
    let _ = token_pump.await;
    let _ = bash_pump.await;
    let result = send_result?;

    for ev in state.session.last_tool_events() {
        emit(&app, event::TOOL_EVENT, ev);
    }
    if let Some(audit) = state.session.entropy_recent(1).into_iter().next_back() {
        emit(&app, event::ENTROPY, audit);
    }
    if let Some(plan) = state.session.plan_exit_pending() {
        emit(&app, event::PLAN_EXIT, plan);
    }
    emit(
        &app,
        event::TURN_COMPLETE,
        json!({
            "turn_id": turn_id,
            "reply": result.reply,
            "verified": result.verified,
            "limits": result.limits,
        }),
    );
    Ok(result)
}

/// `session_command` — run a slash command. Memory-mutating commands emit
/// `rk://consolidation` with their stats; plan/verify map to the matching
/// `Session` hooks. Returns `void` per the contract.
#[tauri::command]
async fn session_command<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    command: String,
) -> Result<(), BoundaryErrorPayload> {
    let cmd = command.trim();
    match cmd {
        "/reflect" => emit(&app, event::CONSOLIDATION, state.session.reflect().await?),
        "/sleep" => emit(&app, event::CONSOLIDATION, state.session.sleep().await?),
        "/groom" => emit(&app, event::CONSOLIDATION, state.session.groom().await?),
        "/compact" => state.session.compact_now().await?,
        "/verify" => state.session.note_manual_verify(),
        "/plan" => state.session.enter_plan_mode(),
        _ => {} // unknown slash commands are a no-op in v1
    }
    Ok(())
}

#[tauri::command]
fn session_last_report(state: State<'_, AppState>) -> Option<Value> {
    state.session.last_report()
}

#[tauri::command]
fn session_mhir(state: State<'_, AppState>) -> Value {
    state.session.mhir()
}

#[tauri::command]
fn session_config(state: State<'_, AppState>) -> Value {
    state.session.config()
}

/// `config_set` — record a session override and return the merged set. v1 records
/// overrides; restart-only keys are flagged rather than applied live.
#[tauri::command]
fn config_set(state: State<'_, AppState>, key: String, value: Value) -> Value {
    let mut o = state.overrides.lock().unwrap_or_else(|p| p.into_inner());
    o.insert(key, value);
    json!({ "overrides": Value::Object(o.clone()) })
}

#[tauri::command]
async fn session_memory_snapshot(
    state: State<'_, AppState>,
) -> Result<Value, BoundaryErrorPayload> {
    Ok(json!({ "recent": state.session.memory_recent(20).await }))
}

#[tauri::command]
fn session_evidence_recent(state: State<'_, AppState>, n: usize) -> Vec<Value> {
    state.session.evidence_recent(n)
}

#[tauri::command]
fn session_entropy_history(state: State<'_, AppState>) -> Value {
    json!({
        "recent": state.session.entropy_recent(50),
        "cumulative_delta": state.session.entropy_total_delta(),
    })
}

#[tauri::command]
fn session_token_budget(state: State<'_, AppState>) -> Value {
    state.session.token_budget()
}

/// `approval_respond` — answer the in-flight `rk://approval_request`. `always`
/// upgrades Allow to AllowAlways; `!approved` blocks (→ `tool_block` intervention).
#[tauri::command]
fn approval_respond(state: State<'_, AppState>, approved: bool, always: bool) {
    let response = match (approved, always) {
        (true, true) => ApprovalResponse::AllowAlways,
        (true, false) => ApprovalResponse::Allow,
        (false, _) => ApprovalResponse::Block,
    };
    state.answer_approval(response);
}

#[tauri::command]
fn secrets_set(provider: String, key: String) -> Result<(), BoundaryErrorPayload> {
    secrets::set(&provider, &key)
}

#[tauri::command]
fn secrets_get(provider: String) -> Result<String, BoundaryErrorPayload> {
    secrets::get(&provider)
}

#[tauri::command]
fn secrets_delete(provider: String) -> Result<(), BoundaryErrorPayload> {
    secrets::delete(&provider)
}

#[tauri::command]
async fn mcp_servers_list(state: State<'_, AppState>) -> Result<Vec<Value>, BoundaryErrorPayload> {
    Ok(state
        .session
        .mcp_summary()
        .await
        .into_iter()
        .map(|(name, tool_count)| json!({ "name": name, "tool_count": tool_count }))
        .collect())
}

#[tauri::command]
fn mcp_server_add(server: Value) -> Result<(), BoundaryErrorPayload> {
    let _ = server;
    Err(BoundaryErrorPayload::internal(
        "MCP server management is restart-only in v1 (set RUSTYKEYS MCP config and relaunch)",
    ))
}

#[tauri::command]
fn mcp_server_remove(name: String) -> Result<(), BoundaryErrorPayload> {
    let _ = name;
    Err(BoundaryErrorPayload::internal(
        "MCP server management is restart-only in v1 (set RUSTYKEYS MCP config and relaunch)",
    ))
}

#[tauri::command]
fn mcp_server_test(name: String) -> Result<Value, BoundaryErrorPayload> {
    let _ = name;
    Err(BoundaryErrorPayload::internal(
        "MCP server testing is restart-only in v1 (set RUSTYKEYS MCP config and relaunch)",
    ))
}

/// `fs_list_workspace` — workspace-relative file paths for the `@file` picker,
/// skipping VCS/build/state dirs and capping the count.
#[tauri::command]
fn fs_list_workspace(state: State<'_, AppState>) -> Vec<String> {
    const CAP: usize = 5000;
    const SKIP: [&str; 5] = [".git", "target", "node_modules", ".rustykeys", "dist"];
    let root = state.workspace.clone();
    let mut out = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if SKIP.contains(&name.as_ref()) {
                continue;
            }
            match entry.file_type() {
                Ok(ft) if ft.is_dir() => stack.push(path),
                Ok(ft) if ft.is_file() => {
                    if let Ok(rel) = path.strip_prefix(&root) {
                        out.push(rel.to_string_lossy().into_owned());
                        if out.len() >= CAP {
                            out.sort();
                            return out;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out.sort();
    out
}

#[tauri::command]
async fn session_memory_search(
    state: State<'_, AppState>,
    q: String,
) -> Result<Vec<Value>, BoundaryErrorPayload> {
    Ok(state.session.memory_search(&q).await)
}

/// `session_commands_list` — the slash commands the composer palette offers.
#[tauri::command]
fn session_commands_list() -> Vec<String> {
    [
        "/compact", "/reflect", "/sleep", "/groom", "/verify", "/plan", "/mhir", "/cost", "/task",
        "/memory", "/entropy", "/commit", "/review", "/diff",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Park an approval responder and emit `rk://approval_request` — called from the
/// setup task that drains the gate's channel.
pub fn emit_approval_request<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    req: rk_constrain::ApprovalRequest,
) {
    let payload = json!({
        "tool": req.tool,
        "args": req.args,
        "trigger": format!("{:?}", req.trigger),
    });
    state.park_approval(req.respond);
    emit(app, event::APPROVAL_REQUEST, payload);
}

/// Resolve a `{ proceed | reject | annotate }` plan decision string. Used by the
/// frontend plan-confirmation flow via `session_command` follow-ups.
pub fn plan_decision(s: &str) -> PlanDecision {
    match s.trim() {
        "" | "proceed" => PlanDecision::Proceed,
        other => match other.strip_prefix("annotate ") {
            Some(note) => PlanDecision::Annotate(note.trim().to_string()),
            None => PlanDecision::Reject,
        },
    }
}

/// The Tauri invoke handler over every contract command. The setup task is wired
/// separately in [`crate::configure`].
pub fn invoke_handler<R: Runtime>() -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static
{
    tauri::generate_handler![
        session_send,
        session_command,
        session_last_report,
        session_mhir,
        session_config,
        config_set,
        session_memory_snapshot,
        session_evidence_recent,
        session_entropy_history,
        session_token_budget,
        approval_respond,
        secrets_set,
        secrets_get,
        secrets_delete,
        mcp_servers_list,
        mcp_server_add,
        mcp_server_remove,
        mcp_server_test,
        fs_list_workspace,
        session_memory_search,
        session_commands_list,
    ]
}

/// Compile-time assertion that the registered command/event catalogs match the
/// contract SSOT exactly (length checks; the per-name equality lives in tests).
pub fn assert_catalogs() {
    debug_assert_eq!(COMMANDS.len(), command::ALL.len());
    debug_assert_eq!(EVENTS.len(), event::ALL.len());
}
