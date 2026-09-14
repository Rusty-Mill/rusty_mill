//! Inbound ACP surface (BL-145 / Hermes Feature 7).
//!
//! [`AcpServer`] reads line-delimited JSON-RPC 2.0 requests from one
//! reader, dispatches each through a [`nexus_kernel::PluginContext`]
//! to a closed allow-list of `com.nexus.agent` IPC verbs, and writes
//! the result/error envelope to one writer. Pure proxy — no
//! capability re-checks beyond what the host's `ipc_call` boundary
//! already enforces.
//!
//! The transport is the same line-delimited framing as the host
//! ([`crate::transport`]). The binary entry-point `nexus acp serve`
//! is the only intended caller; it wires `stdin`/`stdout` and lets a
//! Hermes-compatible parent process drive Nexus's agent loop via
//! JSON-RPC.
//!
//! # Exposed methods
//!
//! | JSON-RPC method | Routed IPC call |
//! |---|---|
//! | `agent/run` | `com.nexus.agent::session_run` |
//! | `agent/list` | `com.nexus.agent::session_list` |
//! | `agent/get` | `com.nexus.agent::session_get` |
//!
//! Unknown methods return JSON-RPC error `-32601` (method not found).
//! Invalid params (missing required fields) return `-32602`. Underlying
//! `ipc_call` failures return `-32000` (server error) with the kernel
//! error string in `message`.

use std::sync::Arc;
use std::time::Duration;

use nexus_kernel::{Ipc as _, KernelPluginContext};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite, BufReader};
use tokio::sync::Mutex;
use tokio::task::JoinSet;

use crate::transport::{
    read_message, write_message, JsonRpcError, JsonRpcMessage, JsonRpcRequest, JsonRpcResponse,
    TransportError,
};

/// Default per-`ipc_call` timeout. Generous because `agent/run` can
/// drive an LLM tool loop with many round trips.
pub const DEFAULT_DISPATCH_TIMEOUT: Duration = Duration::from_secs(600);

/// Errors raised by [`AcpServer`].
#[derive(Debug, thiserror::Error)]
pub enum AcpServerError {
    /// Wire-level failure on the inbound stream — the parent process
    /// hung up or sent a malformed frame.
    #[error("transport: {0}")]
    Transport(#[from] TransportError),
    /// A response failed to write back — the parent process closed
    /// the outbound pipe.
    #[error("write: {0}")]
    Write(String),
}

/// JSON-RPC stdio server that exposes a fixed subset of Nexus's
/// `com.nexus.agent` IPC surface to an external parent process.
///
/// Stateless: every inbound request takes the routing table, looks up
/// the target verb, blocks on `ipc_call`, and writes the response.
/// Concurrent requests share the outbound writer through a Mutex so
/// responses don't tear bytes mid-line.
///
/// The context is held behind an `Arc` because [`KernelPluginContext`]
/// is not `Clone`; mirrors the construction pattern
/// [`nexus_mcp::NexusMcpServer`] uses. `AcpServer` itself derives
/// `Clone` — every field is `Arc`-backed or `Copy` — so `serve` can
/// hand each spawned dispatch task its own cheap handle.
#[derive(Clone)]
pub struct AcpServer {
    context: Arc<KernelPluginContext>,
    timeout: Duration,
}

impl AcpServer {
    /// Construct a server bound to `context`. The context is the only
    /// shared state — every method dispatch routes through
    /// `context.ipc_call(...)`.
    #[must_use]
    pub fn new(context: Arc<KernelPluginContext>) -> Self {
        Self {
            context,
            timeout: DEFAULT_DISPATCH_TIMEOUT,
        }
    }

    /// Override the default per-call IPC timeout. Mostly useful for
    /// tests; the binary entry point keeps the default.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Serve requests on the supplied `reader`/`writer` pair until
    /// the reader returns EOF.
    ///
    /// Each inbound request is dispatched on its own task so a
    /// long-running `agent/run` no longer blocks a concurrent
    /// `agent/list`/`agent/get` on the same connection (finding #4,
    /// round 7) — mirrors the D1 fix already applied to
    /// [`nexus_remote::RemoteServer::serve`]. The outbound writer
    /// stays behind the existing `Mutex` so interleaved writes never
    /// tear a line. Responses are written in completion order, not
    /// arrival order — JSON-RPC correlates each response to its
    /// request via `id`, so callers must not assume strict FIFO
    /// ordering across concurrent requests.
    ///
    /// Per-request tasks are tracked in a connection-scoped
    /// `JoinSet`, reaped non-blockingly each loop iteration so it
    /// can't grow unboundedly under a hot client, and given a brief
    /// grace window to finish writing once the reader hits EOF.
    /// Dropping the returned future (caller shutdown) aborts any
    /// still-pending dispatch via the `JoinSet`'s drop glue.
    ///
    /// # Errors
    /// - [`AcpServerError::Transport`] only when the reader fails
    ///   irrecoverably (read past EOF returns `Ok(())`).
    /// - [`AcpServerError::Write`] when the outbound writer breaks.
    pub async fn serve<R, W>(&self, reader: R, writer: W) -> Result<(), AcpServerError>
    where
        R: AsyncRead + Unpin + Send,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let mut reader = BufReader::new(reader);
        let writer: Arc<Mutex<W>> = Arc::new(Mutex::new(writer));
        // Track per-request dispatch tasks in a JoinSet scoped to this
        // connection — mirrors `nexus_remote::RemoteServer::serve`'s D1
        // fix (2026-05-21 audit). Dropping the JoinSet aborts any
        // still-pending dispatch if `serve`'s own future is dropped.
        let mut pending: JoinSet<()> = JoinSet::new();

        loop {
            // Reap completed per-request tasks so the JoinSet doesn't
            // grow unboundedly under a hot client. `try_join_next` is
            // non-blocking; drain whatever is ready.
            while pending.try_join_next().is_some() {}

            let msg = match read_message(&mut reader).await {
                Ok(m) => m,
                Err(TransportError::Eof) => break,
                Err(e) => return Err(AcpServerError::Transport(e)),
            };
            match msg {
                JsonRpcMessage::Request(req) => {
                    let server = self.clone();
                    let writer = Arc::clone(&writer);
                    pending.spawn(async move {
                        let response = server.dispatch_request(req).await;
                        write_response(&writer, response).await;
                    });
                }
                JsonRpcMessage::Notification(_) => {
                    // ACP server doesn't subscribe to client-sent
                    // notifications today — silently ignore.
                }
                JsonRpcMessage::Response(_) => {
                    // We don't issue server-initiated requests, so a
                    // response on the inbound stream is unexpected. Log
                    // and continue.
                    tracing::warn!("acp server: unexpected response on inbound stream");
                }
            }
        }

        // Give in-flight per-request handlers a brief grace window to
        // finish writing before returning. 2s mirrors the pre-existing
        // patience window in `nexus_remote::RemoteServer::serve`.
        let _ = tokio::time::timeout(Duration::from_secs(2), async {
            while pending.join_next().await.is_some() {}
        })
        .await;
        Ok(())
    }

    async fn dispatch_request(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        let id = req.id.clone();
        let params = req.params.unwrap_or(Value::Null);
        let routed = route_method(&req.method);
        match routed {
            RoutedMethod::Unknown => {
                error_response(id, -32601, format!("method not found: {}", req.method))
            }
            RoutedMethod::Known { plugin_id, command } => match self
                .context
                .ipc_call(plugin_id, command, params, self.timeout)
                .await
            {
                Ok(v) => JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id,
                    result: Some(v),
                    error: None,
                },
                Err(e) => error_response(id, -32000, format!("server error: {e}")),
            },
        }
    }
}

/// Write one response through the shared outbound writer. A failure
/// (parent process closed the pipe) is logged, not surfaced, so one
/// broken write doesn't unwind an unrelated in-flight dispatch.
async fn write_response<W>(writer: &Arc<Mutex<W>>, response: JsonRpcResponse)
where
    W: AsyncWrite + Unpin + Send,
{
    let mut w = writer.lock().await;
    if let Err(e) = write_message(&mut *w, &JsonRpcMessage::Response(response)).await {
        tracing::warn!(error = %e, "acp server: failed to write response");
    }
}

/// Method routing table. Pure function — separated from
/// `dispatch_request` so unit tests don't need a live runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutedMethod {
    /// Method maps to a known `(plugin_id, command)` pair.
    Known {
        /// Reverse-DNS id of the target plugin.
        plugin_id: &'static str,
        /// IPC command name.
        command: &'static str,
    },
    /// Method is not on the allow-list. Server returns
    /// `-32601 method not found`.
    Unknown,
}

#[must_use]
pub fn route_method(method: &str) -> RoutedMethod {
    match method {
        "agent/run" => RoutedMethod::Known {
            plugin_id: "com.nexus.agent",
            command: "session_run",
        },
        "agent/list" => RoutedMethod::Known {
            plugin_id: "com.nexus.agent",
            command: "session_list",
        },
        "agent/get" => RoutedMethod::Known {
            plugin_id: "com.nexus.agent",
            command: "session_get",
        },
        _ => RoutedMethod::Unknown,
    }
}

fn error_response(id: Value, code: i64, message: String) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result: None,
        error: Some(JsonRpcError {
            code,
            message,
            data: None,
        }),
    }
}

/// Construct a JSON-RPC `-32602 invalid params` response. Exposed for
/// callers that pre-validate inbound params before reaching the IPC
/// boundary.
#[must_use]
pub fn invalid_params_response(id: Value, message: impl Into<String>) -> JsonRpcResponse {
    error_response(id, -32602, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_kernel::{
        Capability, CapabilitySet, EventBus, InMemoryKvStore, IpcDispatcher, IpcFuture, KvStore,
    };
    use serde_json::json;
    use tokio::sync::Notify;

    #[test]
    fn route_method_table_covers_three_documented_verbs() {
        assert_eq!(
            route_method("agent/run"),
            RoutedMethod::Known {
                plugin_id: "com.nexus.agent",
                command: "session_run",
            },
        );
        assert_eq!(
            route_method("agent/list"),
            RoutedMethod::Known {
                plugin_id: "com.nexus.agent",
                command: "session_list",
            },
        );
        assert_eq!(
            route_method("agent/get"),
            RoutedMethod::Known {
                plugin_id: "com.nexus.agent",
                command: "session_get",
            },
        );
        assert_eq!(route_method("agent/nope"), RoutedMethod::Unknown);
        assert_eq!(route_method(""), RoutedMethod::Unknown);
        assert_eq!(route_method("session_run"), RoutedMethod::Unknown);
    }

    #[test]
    fn invalid_params_response_shape_is_jsonrpc_compliant() {
        let r = invalid_params_response(json!(7), "missing 'goal'");
        assert_eq!(r.jsonrpc, "2.0");
        assert_eq!(r.id, json!(7));
        assert!(r.result.is_none());
        let err = r.error.unwrap();
        assert_eq!(err.code, -32602);
        assert!(err.message.contains("missing"));
    }

    #[test]
    fn error_response_serialises_without_result_field() {
        let r = error_response(json!(1), -32601, "method not found: x".into());
        let body = serde_json::to_string(&r).unwrap();
        // `result` is `None` and `skip_serializing_if`d.
        assert!(!body.contains("\"result\""));
        assert!(body.contains("\"error\""));
        assert!(body.contains("-32601"));
    }

    /// Test-only [`IpcDispatcher`] whose `session_run` handler blocks
    /// on a [`Notify`] gate (after signalling it has actually started)
    /// until the test releases it; every other command resolves
    /// immediately. Lets the regression test below prove a fast
    /// request answers while a slow one is still in flight, rather
    /// than relying on a fixed sleep duration racing the assertion.
    struct GatedDispatcher {
        run_started: Arc<Notify>,
        release_run: Arc<Notify>,
    }

    impl IpcDispatcher for GatedDispatcher {
        fn dispatch(
            &self,
            _caller_plugin_id: &str,
            _target_plugin_id: &str,
            _command_id: &str,
            _args: &Value,
        ) -> std::result::Result<Value, nexus_kernel::IpcError> {
            unreachable!("test routes through dispatch_async only");
        }

        fn dispatch_async(
            &self,
            _caller_plugin_id: &str,
            _target_plugin_id: &str,
            command_id: &str,
            _args: Value,
        ) -> Option<IpcFuture> {
            if command_id == "session_run" {
                let started = Arc::clone(&self.run_started);
                let release = Arc::clone(&self.release_run);
                Some(Box::pin(async move {
                    started.notify_one();
                    release.notified().await;
                    Ok(json!({"done": true}))
                }))
            } else {
                Some(Box::pin(async move { Ok(json!({"ok": true})) }))
            }
        }
    }

    /// Finding #4 (round 7) regression: a long-running `agent/run`
    /// must not block a concurrent `agent/list` on the same
    /// connection. Pre-fix, `serve`'s loop awaited each dispatch to
    /// completion before reading the next request, so the fast
    /// request wasn't even read off the wire until the slow one
    /// resolved — the `timeout` below would fire. Post-fix, each
    /// request is dispatched on its own task and the fast response
    /// arrives well before the slow one completes.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fast_request_answers_before_a_concurrent_slow_request_completes() {
        let dir = tempfile::tempdir().unwrap();
        let kv: Arc<dyn KvStore> = Arc::new(InMemoryKvStore::new());
        let bus = Arc::new(EventBus::new(16));
        let run_started = Arc::new(Notify::new());
        let release_run = Arc::new(Notify::new());
        let dispatcher: Arc<dyn IpcDispatcher> = Arc::new(GatedDispatcher {
            run_started: Arc::clone(&run_started),
            release_run: Arc::clone(&release_run),
        });
        let ctx = KernelPluginContext::new(
            "com.test.caller",
            "1.0.0",
            [Capability::IpcCall].into_iter().collect::<CapabilitySet>(),
            kv,
            bus,
            dir.path(),
            Some(dispatcher),
        )
        .unwrap();
        let server = AcpServer::new(Arc::new(ctx)).with_timeout(Duration::from_secs(30));

        let (mut client_writer, server_reader) = tokio::io::duplex(64 * 1024);
        let (server_writer, client_reader) = tokio::io::duplex(64 * 1024);
        let mut client_reader = BufReader::new(client_reader);

        let server_task =
            tokio::spawn(async move { server.serve(server_reader, server_writer).await });

        // Kick off the slow `agent/run` request first.
        write_message(
            &mut client_writer,
            &JsonRpcMessage::Request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: json!(1),
                method: "agent/run".to_string(),
                params: Some(json!({})),
            }),
        )
        .await
        .unwrap();

        // Don't send request #2 until the slow dispatch has genuinely
        // started, so we know it's holding a task open when the fast
        // request lands on the same connection.
        run_started.notified().await;

        write_message(
            &mut client_writer,
            &JsonRpcMessage::Request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: json!(2),
                method: "agent/list".to_string(),
                params: Some(json!({})),
            }),
        )
        .await
        .unwrap();

        // Pre-fix: the serial loop doesn't read request #2 off the wire
        // until request #1's dispatch resolves (up to the 30s timeout
        // configured above), so this times out. Post-fix: #2 is read
        // and dispatched concurrently, so its response arrives promptly
        // even though #1 is still gated.
        let first = tokio::time::timeout(Duration::from_secs(5), read_message(&mut client_reader))
            .await
            .expect("fast request must not queue behind the in-flight slow one")
            .expect("valid frame");
        let JsonRpcMessage::Response(resp) = first else {
            panic!("expected a response frame, got {first:?}");
        };
        assert_eq!(resp.id, json!(2), "fast agent/list should answer first");
        assert_eq!(resp.result, Some(json!({"ok": true})));

        // Release the slow dispatch and confirm its response follows.
        release_run.notify_one();
        let second = tokio::time::timeout(Duration::from_secs(5), read_message(&mut client_reader))
            .await
            .expect("slow request should eventually respond")
            .expect("valid frame");
        let JsonRpcMessage::Response(resp2) = second else {
            panic!("expected a response frame, got {second:?}");
        };
        assert_eq!(resp2.id, json!(1));

        drop(client_writer);
        server_task
            .await
            .expect("server task should not panic")
            .expect("server should exit cleanly on EOF");
    }
}
