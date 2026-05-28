//! Agent Client Protocol server (PRD 07 / Phase 16): expose the `Session` as an
//! ACP agent so an external editor (Zed, …) can drive it. ACP is the editor↔agent
//! inverse of MCP — newline-delimited JSON-RPC 2.0 over stdio. This is hand-rolled
//! over async byte streams (no heavy SDK) so the whole surface is offline-testable
//! with an in-process fake client.
//!
//! Wiring: `session/new` builds a `Session` whose policy chain ends in an
//! `ApprovalGate`; a write requested mid-turn surfaces as `session/request_permission`
//! to the client and round-trips the gate (Allow/AllowAlways/Reject). A denied
//! action becomes a `Blocked` outcome (and a `tool_block` intervention) — the
//! harness boundary is never bypassed.
//!
//! Deferred (flagged): client fs/terminal capability shims (`fs/read_text_file`
//! etc.) + their `AcpPolicy`/inspection — they need the server→client I/O flow.

use std::collections::HashMap;

use aisdk::core::capabilities::{TextInputSupport, ToolCallSupport};
use aisdk::core::language_model::LanguageModel;
use rk_config::Config;
use rk_constrain::{ApprovalGate, ApprovalRequest, ApprovalResponse, ApprovalTrigger};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::Session;

/// The protocol version advertised in `initialize`.
const ACP_VERSION: &str = "0.1";

/// Serialize a server→client JSON-RPC request line.
fn request_line(id: u64, method: &str, params: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
}

/// Serialize a server→client notification line.
fn notify_line(method: &str, params: Value) -> String {
    json!({"jsonrpc": "2.0", "method": method, "params": params}).to_string()
}

/// Serialize a successful response line.
fn result_line(id: &Value, result: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
}

/// Serialize an error response line (boundary error taxonomy).
fn error_line(id: &Value, code: i64, message: &str) -> String {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}).to_string()
}

/// Run the ACP server over `reader`/`writer` until the client disconnects.
/// `reader` is newline-delimited JSON-RPC from the client; `writer` receives the
/// agent's responses, requests, and `session/update` notifications.
pub async fn run<M, R, W>(config: Config, model: M, reader: R, mut writer: W) -> anyhow::Result<()>
where
    M: LanguageModel + TextInputSupport + ToolCallSupport + Clone + Send + Sync + 'static,
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut lines = reader.lines();

    // Per-connection state.
    let mut session: Option<Arc<Session<M>>> = None;
    let (approval_tx, mut approval_rx) = mpsc::channel::<ApprovalRequest>(8);
    let mut pending_perms: HashMap<u64, tokio::sync::oneshot::Sender<ApprovalResponse>> =
        HashMap::new();
    let mut next_perm_id: u64 = 1;
    // The in-flight prompt: (request id, the send() task).
    let mut inflight: Option<(Value, JoinHandle<(String, bool)>)> = None;

    loop {
        tokio::select! {
            // ---- client → server messages ----
            line = lines.next_line() => {
                let Some(line) = line? else { break };
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let Ok(msg) = serde_json::from_str::<Value>(line) else {
                    let _ = write_line(&mut writer, &error_line(&Value::Null, -32700, "parse error")).await;
                    continue;
                };

                // A response to our `session/request_permission` request.
                if msg.get("method").is_none() {
                    if let Some(id) = msg.get("id").and_then(Value::as_u64) {
                        if let Some(respond) = pending_perms.remove(&id) {
                            let _ = respond.send(parse_permission(&msg));
                        }
                    }
                    continue;
                }

                let id = msg.get("id").cloned().unwrap_or(Value::Null);
                let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
                let params = msg.get("params").cloned().unwrap_or(Value::Null);

                match method {
                    "initialize" => {
                        let out = result_line(&id, json!({
                            "protocolVersion": ACP_VERSION,
                            "agentCapabilities": { "promptCapabilities": { "image": false } },
                            "agentInfo": { "name": "rusty-keys", "version": env!("CARGO_PKG_VERSION") },
                        }));
                        write_line(&mut writer, &out).await?;
                    }
                    "authenticate" => {
                        write_line(&mut writer, &result_line(&id, json!({}))).await?;
                    }
                    "session/new" => {
                        let gate = ApprovalGate::new(
                            vec![ApprovalTrigger::NewFilePath, ApprovalTrigger::BashFirstUse],
                            approval_tx.clone(),
                        );
                        match Session::new_with_policy(&config, model.clone(), Arc::new(gate)) {
                            Ok(s) => {
                                let sid = format!("acp_{next_perm_id}");
                                session = Some(Arc::new(s));
                                write_line(&mut writer, &result_line(&id, json!({"sessionId": sid}))).await?;
                            }
                            Err(e) => {
                                write_line(&mut writer, &error_line(&id, -32000, &e.to_string())).await?;
                            }
                        }
                    }
                    "session/load" => {
                        // Resume is a follow-on; for now report no stored session.
                        write_line(&mut writer, &error_line(&id, -32601, "session/load not supported")).await?;
                    }
                    "session/cancel" => {
                        if let Some((pid, handle)) = inflight.take() {
                            handle.abort();
                            write_line(&mut writer, &result_line(&pid, json!({"stopReason": "cancelled"}))).await?;
                        }
                        write_line(&mut writer, &result_line(&id, json!({}))).await?;
                    }
                    "session/prompt" => {
                        if inflight.is_some() {
                            write_line(&mut writer, &error_line(&id, -32002, "a prompt is already in flight")).await?;
                            continue;
                        }
                        let Some(sess) = session.clone() else {
                            write_line(&mut writer, &error_line(&id, -32001, "no session — call session/new first")).await?;
                            continue;
                        };
                        let text = prompt_text(&params);
                        let handle = tokio::spawn(async move {
                            match sess.send(&text).await {
                                Ok(o) => (o.reply, o.report.verified),
                                Err(e) => (format!("error: {e}"), false),
                            }
                        });
                        inflight = Some((id, handle));
                    }
                    other => {
                        write_line(&mut writer, &error_line(&id, -32601, &format!("method not found: {other}"))).await?;
                    }
                }
            }

            // ---- a tool is awaiting approval mid-turn ----
            Some(req) = approval_rx.recv() => {
                let pid = next_perm_id;
                next_perm_id += 1;
                pending_perms.insert(pid, req.respond);
                let out = request_line(pid, "session/request_permission", json!({
                    "toolCall": { "name": req.tool, "input": req.args },
                    "options": [
                        {"optionId": "allow", "name": "Allow"},
                        {"optionId": "allow_always", "name": "Allow always"},
                        {"optionId": "reject", "name": "Reject"},
                    ],
                }));
                write_line(&mut writer, &out).await?;
            }

            // ---- the in-flight prompt finished ----
            done = wait_inflight(&mut inflight) => {
                let (pid, reply, verified) = done;
                // session/update notifications mirror the rk:// event table.
                write_line(&mut writer, &notify_line("session/update", json!({
                    "update": { "sessionUpdate": "agent_message_chunk",
                                "content": { "type": "text", "text": reply } },
                }))).await?;
                write_line(&mut writer, &notify_line("session/update", json!({
                    "update": { "sessionUpdate": "verification", "verified": verified },
                }))).await?;
                write_line(&mut writer, &result_line(&pid, json!({
                    "stopReason": "end_turn", "verified": verified,
                }))).await?;
            }
        }
    }
    Ok(())
}

/// Await the in-flight prompt task, or pend forever when there is none (so the
/// `select!` arm only fires when a prompt is running).
async fn wait_inflight(
    slot: &mut Option<(Value, JoinHandle<(String, bool)>)>,
) -> (Value, String, bool) {
    match slot {
        Some((id, handle)) => {
            let id = id.clone();
            let (reply, verified) = handle.await.unwrap_or_else(|_| ("cancelled".into(), false));
            *slot = None;
            (id, reply, verified)
        }
        None => std::future::pending().await,
    }
}

fn prompt_text(params: &Value) -> String {
    // ACP prompt content is an array of content blocks; concatenate the text.
    if let Some(arr) = params.get("prompt").and_then(Value::as_array) {
        return arr
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
    }
    params
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn parse_permission(msg: &Value) -> ApprovalResponse {
    let opt = msg
        .get("result")
        .and_then(|r| r.get("outcome"))
        .and_then(|o| o.get("optionId"))
        .and_then(Value::as_str)
        .or_else(|| {
            msg.get("result")
                .and_then(|r| r.get("optionId"))
                .and_then(Value::as_str)
        })
        .unwrap_or("reject");
    match opt {
        "allow" => ApprovalResponse::Allow,
        "allow_always" => ApprovalResponse::AllowAlways,
        _ => ApprovalResponse::Block,
    }
}

async fn write_line<W: AsyncWrite + Unpin>(writer: &mut W, line: &str) -> std::io::Result<()> {
    writer.write_all(line.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await
}
