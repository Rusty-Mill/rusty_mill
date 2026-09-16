//! ACP client-capability shims (Phase 16). When an editor advertises filesystem
//! or terminal capabilities at `initialize`, the agent reaches them through
//! these tools — `fs_read_text_file`, `fs_write_text_file`, `acp_terminal` —
//! which issue server→client JSON-RPC requests over the [`AcpClientBridge`].
//!
//! They are ordinary `feed::ToolFn`s, so every call clears the session's policy
//! chain (incl. the `AcpPolicy` workspace boundary) **before** any request
//! leaves the agent: an out-of-workspace path is blocked, never sent. Content
//! returning from the editor passes the Phase 12 tool-return inspector before it
//! can enter the model's context.

use std::sync::Arc;

use async_trait::async_trait;
use rk_feed::ToolFn;
use rk_mcp::{Inspection, ReturnInspector};
use rk_observe::{ToolOutcome, ToolStatus};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};

/// One server→client request issued by a shim, awaiting the editor's response.
pub struct ClientCall {
    /// ACP method (e.g. `fs/read_text_file`).
    pub method: String,
    /// JSON-RPC params.
    pub params: Value,
    /// Resolved with the client's `result` (`Ok`) or error message (`Err`).
    pub respond: oneshot::Sender<Result<Value, String>>,
}

/// Handle a shim uses to call back to the editor. Cloned into each shim; the ACP
/// run loop owns the receiver, writes the request, and resolves the response.
#[derive(Clone)]
pub struct AcpClientBridge {
    tx: mpsc::Sender<ClientCall>,
}

impl AcpClientBridge {
    /// New bridge over `tx` (the run loop holds the matching receiver).
    pub fn new(tx: mpsc::Sender<ClientCall>) -> Self {
        Self { tx }
    }

    /// Issue one request to the editor and await its response. Fails closed if
    /// the connection is gone.
    async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let (respond, rx) = oneshot::channel();
        self.tx
            .send(ClientCall {
                method: method.to_string(),
                params,
                respond,
            })
            .await
            .map_err(|_| "ACP client channel closed".to_string())?;
        rx.await
            .map_err(|_| "ACP client dropped the request".to_string())?
    }
}

/// Which client capabilities the editor advertised at `initialize`.
#[derive(Default, Clone, Copy, Debug)]
pub struct ClientCaps {
    /// `clientCapabilities.fs.readTextFile`.
    pub fs_read: bool,
    /// `clientCapabilities.fs.writeTextFile`.
    pub fs_write: bool,
    /// `clientCapabilities.terminal`.
    pub terminal: bool,
}

impl ClientCaps {
    /// Parse from `initialize` params.
    pub fn parse(params: &Value) -> Self {
        let caps = params.get("clientCapabilities");
        let fs = caps.and_then(|c| c.get("fs"));
        let flag = |v: Option<&Value>| v.and_then(Value::as_bool).unwrap_or(false);
        Self {
            fs_read: flag(fs.and_then(|f| f.get("readTextFile"))),
            fs_write: flag(fs.and_then(|f| f.get("writeTextFile"))),
            terminal: flag(caps.and_then(|c| c.get("terminal"))),
        }
    }

    /// Build the shim tools for the advertised capabilities, bridged over
    /// `bridge` and gated by `inspector` on inbound content.
    pub fn tools(
        &self,
        bridge: AcpClientBridge,
        inspector: Arc<dyn ReturnInspector>,
    ) -> Vec<Box<dyn ToolFn>> {
        let mut out: Vec<Box<dyn ToolFn>> = Vec::new();
        if self.fs_read {
            out.push(Box::new(AcpShimTool::new(
                ShimKind::FsRead,
                bridge.clone(),
                inspector.clone(),
            )));
        }
        if self.fs_write {
            out.push(Box::new(AcpShimTool::new(
                ShimKind::FsWrite,
                bridge.clone(),
                inspector.clone(),
            )));
        }
        if self.terminal {
            out.push(Box::new(AcpShimTool::new(
                ShimKind::Terminal,
                bridge,
                inspector,
            )));
        }
        out
    }
}

#[derive(Clone, Copy)]
enum ShimKind {
    FsRead,
    FsWrite,
    Terminal,
}

/// A `feed::ToolFn` bridging one ACP client capability to the editor.
struct AcpShimTool {
    kind: ShimKind,
    bridge: AcpClientBridge,
    inspector: Arc<dyn ReturnInspector>,
}

impl AcpShimTool {
    fn new(kind: ShimKind, bridge: AcpClientBridge, inspector: Arc<dyn ReturnInspector>) -> Self {
        Self {
            kind,
            bridge,
            inspector,
        }
    }

    /// Vet content returning from the editor before it can enter context.
    fn gate_return(&self, tool: &str, text: String) -> ToolOutcome {
        match self.inspector.inspect(tool, &text) {
            Inspection::Allow => ToolOutcome::ok(text),
            Inspection::Quarantine(reason) => ToolOutcome::new(
                ToolStatus::Blocked,
                format!("ACP return quarantined: {reason}"),
            ),
        }
    }

    async fn read(&self, args: Value) -> ToolOutcome {
        let path = args.get("path").cloned().unwrap_or(Value::Null);
        match self
            .bridge
            .call("fs/read_text_file", json!({ "path": path }))
            .await
        {
            Ok(result) => {
                let content = result
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                self.gate_return("fs_read_text_file", content)
            }
            Err(e) => ToolOutcome::error(format!("fs/read_text_file failed: {e}")),
        }
    }

    async fn write(&self, args: Value) -> ToolOutcome {
        let path = args.get("path").and_then(Value::as_str).unwrap_or("");
        let content = args.get("content").cloned().unwrap_or(Value::Null);
        match self
            .bridge
            .call(
                "fs/write_text_file",
                json!({ "path": path, "content": content }),
            )
            .await
        {
            Ok(_) => ToolOutcome::ok(format!("wrote {path}")),
            Err(e) => ToolOutcome::error(format!("fs/write_text_file failed: {e}")),
        }
    }

    async fn terminal(&self, args: Value) -> ToolOutcome {
        let command = args.get("command").cloned().unwrap_or(Value::Null);
        let cmd_args = args.get("args").cloned().unwrap_or(json!([]));
        let cwd = args.get("cwd").cloned();
        // ACP terminal lifecycle: create → wait_for_exit → output → release.
        let mut create = json!({ "command": command, "args": cmd_args });
        if let Some(cwd) = cwd {
            create["cwd"] = cwd;
        }
        let created = match self.bridge.call("terminal/create", create).await {
            Ok(r) => r,
            Err(e) => return ToolOutcome::error(format!("terminal/create failed: {e}")),
        };
        let term_id = created
            .get("terminalId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let id_params = json!({ "terminalId": term_id });
        // Best-effort wait; ignore its payload (output is read separately).
        let _ = self
            .bridge
            .call("terminal/wait_for_exit", id_params.clone())
            .await;
        let output = match self.bridge.call("terminal/output", id_params.clone()).await {
            Ok(r) => r
                .get("output")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            Err(e) => return ToolOutcome::error(format!("terminal/output failed: {e}")),
        };
        // Release the terminal; a failure here does not invalidate the output.
        let _ = self.bridge.call("terminal/release", id_params).await;
        self.gate_return("acp_terminal", output)
    }
}

#[async_trait]
impl ToolFn for AcpShimTool {
    fn name(&self) -> &str {
        match self.kind {
            ShimKind::FsRead => "fs_read_text_file",
            ShimKind::FsWrite => "fs_write_text_file",
            ShimKind::Terminal => "acp_terminal",
        }
    }

    fn schema(&self) -> Value {
        match self.kind {
            ShimKind::FsRead => json!({
                "type": "object",
                "description": "Read a text file through the connected editor (respects unsaved buffers).",
                "properties": { "path": { "type": "string", "description": "Workspace-relative or absolute path." } },
                "required": ["path"],
            }),
            ShimKind::FsWrite => json!({
                "type": "object",
                "description": "Write a text file through the connected editor.",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" },
                },
                "required": ["path", "content"],
            }),
            ShimKind::Terminal => json!({
                "type": "object",
                "description": "Run a command in the editor's terminal and return its output.",
                "properties": {
                    "command": { "type": "string" },
                    "args": { "type": "array", "items": { "type": "string" } },
                    "cwd": { "type": "string" },
                },
                "required": ["command"],
            }),
        }
    }

    async fn call(&self, args: Value) -> ToolOutcome {
        match self.kind {
            ShimKind::FsRead => self.read(args).await,
            ShimKind::FsWrite => self.write(args).await,
            ShimKind::Terminal => self.terminal(args).await,
        }
    }
}
