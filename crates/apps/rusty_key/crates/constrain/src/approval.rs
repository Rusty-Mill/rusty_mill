//! Interactive approval (PRD 02). For calls that pass every automated check but
//! still warrant a human look (first bash use, writing a new path, first MCP
//! tool), the gate sends an [`ApprovalRequest`] over a channel and awaits an
//! [`ApprovalResponse`]. Awaiting a human is exactly why `before_tool` is
//! `async` (ADR-0016). `Block` returns [`PolicyError::ApprovalDenied`]; the
//! session records the resulting block as a `tool_block` intervention.

use std::collections::HashSet;
use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use crate::{Policy, PolicyError};

/// What kind of action triggered an approval prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalTrigger {
    /// The first `bash` call of the session.
    BashFirstUse,
    /// A `write_file`/`edit_file` to a path not already tracked.
    NewFilePath,
    /// The first call to an MCP tool from `server`.
    McpToolFirstUse {
        /// The MCP server name.
        server: String,
    },
}

/// A pending approval, sent to the adapter (CLI/desktop).
#[derive(Debug)]
pub struct ApprovalRequest {
    /// The tool awaiting approval.
    pub tool: String,
    /// The (un-redacted) call arguments, for the human to inspect.
    pub args: Value,
    /// Why approval was requested.
    pub trigger: ApprovalTrigger,
    /// The adapter answers on this one-shot channel.
    pub respond: oneshot::Sender<ApprovalResponse>,
}

/// The human's (or remote ACL's) decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalResponse {
    /// Allow this call once; prompt again next time.
    Allow,
    /// Allow and auto-approve every later call to this tool in-session.
    AllowAlways,
    /// Deny the call → [`PolicyError::ApprovalDenied`].
    Block,
}

/// Gates triggering tool calls behind a human response over a channel.
pub struct ApprovalGate {
    triggers: Vec<ApprovalTrigger>,
    tx: mpsc::Sender<ApprovalRequest>,
    auto_approve: Mutex<HashSet<String>>,
    fired: Mutex<HashSet<String>>,
}

impl ApprovalGate {
    /// Build a gate with `triggers`, sending requests on `tx`. The adapter end
    /// (the matching [`mpsc::Receiver`]) renders prompts and answers each
    /// request's one-shot.
    pub fn new(triggers: Vec<ApprovalTrigger>, tx: mpsc::Sender<ApprovalRequest>) -> Self {
        Self {
            triggers,
            tx,
            auto_approve: Mutex::new(HashSet::new()),
            fired: Mutex::new(HashSet::new()),
        }
    }

    /// Which configured trigger (if any) this call matches. `*FirstUse` triggers
    /// fire at most once per session — `McpToolFirstUse` fires at most once per
    /// configured `server`, keyed by that server (not just any `mcp__`-prefixed
    /// tool), so approving one server's first call does not silently satisfy
    /// another server's separately configured trigger.
    fn match_trigger(&self, name: &str) -> Option<ApprovalTrigger> {
        let mut fired = self.fired.lock().unwrap_or_else(|p| p.into_inner());
        for t in &self.triggers {
            let (key, hit): (String, bool) = match t {
                ApprovalTrigger::BashFirstUse => ("bash_first_use".to_string(), name == "bash"),
                ApprovalTrigger::NewFilePath => (
                    "new_file_path".to_string(),
                    matches!(name, "write_file" | "edit_file"),
                ),
                ApprovalTrigger::McpToolFirstUse { server } => (
                    format!("mcp_first_use:{server}"),
                    Self::mcp_server_of(name) == Some(server.as_str()),
                ),
            };
            if !hit {
                continue;
            }
            let once = matches!(
                t,
                ApprovalTrigger::BashFirstUse | ApprovalTrigger::McpToolFirstUse { .. }
            );
            if once {
                if fired.contains(&key) {
                    continue;
                }
                fired.insert(key);
            }
            return Some(t.clone());
        }
        None
    }

    /// The server segment of a `mcp__<server>__<tool>` namespaced tool name.
    fn mcp_server_of(name: &str) -> Option<&str> {
        name.strip_prefix("mcp__")
            .and_then(|rest| rest.split_once("__"))
            .map(|(server, _)| server)
    }
}

#[async_trait]
impl Policy for ApprovalGate {
    async fn before_tool(&self, name: &str, args: &Value) -> Result<(), PolicyError> {
        if self
            .auto_approve
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains(name)
        {
            return Ok(());
        }
        let Some(trigger) = self.match_trigger(name) else {
            return Ok(());
        };

        let (respond, rx) = oneshot::channel();
        let req = ApprovalRequest {
            tool: name.to_string(),
            args: args.clone(),
            trigger,
            respond,
        };
        // Fail closed: if no adapter is listening, deny rather than silently allow.
        if self.tx.send(req).await.is_err() {
            return Err(PolicyError::ApprovalDenied);
        }
        match rx.await {
            Ok(ApprovalResponse::Allow) => Ok(()),
            Ok(ApprovalResponse::AllowAlways) => {
                self.auto_approve
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(name.to_string());
                Ok(())
            }
            Ok(ApprovalResponse::Block) | Err(_) => Err(PolicyError::ApprovalDenied),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Spawn a scripted adapter that answers each request with `responses` in order.
    fn responder(
        mut rx: mpsc::Receiver<ApprovalRequest>,
        responses: Vec<ApprovalResponse>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut it = responses.into_iter();
            while let Some(req) = rx.recv().await {
                let r = it.next().unwrap_or(ApprovalResponse::Block);
                let _ = req.respond.send(r);
            }
        })
    }

    #[tokio::test]
    async fn allow_block_and_allow_always_round_trip() {
        let (tx, rx) = mpsc::channel(8);
        let _adapter = responder(
            rx,
            vec![
                ApprovalResponse::Block,
                ApprovalResponse::Allow,
                ApprovalResponse::AllowAlways,
            ],
        );
        let gate = ApprovalGate::new(vec![ApprovalTrigger::NewFilePath], tx);

        // 1st write → Block.
        assert!(matches!(
            gate.before_tool("write_file", &json!({"path": "a"})).await,
            Err(PolicyError::ApprovalDenied)
        ));
        // 2nd → Allow (one-time).
        assert!(gate
            .before_tool("write_file", &json!({"path": "b"}))
            .await
            .is_ok());
        // 3rd → AllowAlways: this and every later write auto-approve.
        assert!(gate
            .before_tool("write_file", &json!({"path": "c"}))
            .await
            .is_ok());
        // 4th → auto-approved without consulting the adapter.
        assert!(gate
            .before_tool("write_file", &json!({"path": "d"}))
            .await
            .is_ok());
        // A non-triggering tool is never gated.
        assert!(gate
            .before_tool("read_file", &json!({"path": "e"}))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn bash_first_use_fires_only_once() {
        let (tx, rx) = mpsc::channel(8);
        // Only one response is scripted; a second prompt would deadlock the
        // adapter (it returns Block on exhaustion), so the test proves the gate
        // consults the adapter exactly once.
        let _adapter = responder(rx, vec![ApprovalResponse::Allow]);
        let gate = ApprovalGate::new(vec![ApprovalTrigger::BashFirstUse], tx);

        assert!(gate
            .before_tool("bash", &json!({"command": "ls"}))
            .await
            .is_ok());
        // Second bash call does not re-trigger.
        assert!(gate
            .before_tool("bash", &json!({"command": "pwd"}))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn no_adapter_fails_closed() {
        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        let gate = ApprovalGate::new(vec![ApprovalTrigger::BashFirstUse], tx);
        assert!(matches!(
            gate.before_tool("bash", &json!({"command": "ls"})).await,
            Err(PolicyError::ApprovalDenied)
        ));
    }

    #[tokio::test]
    async fn mcp_first_use_is_scoped_per_server() {
        // Two separately configured McpToolFirstUse triggers, one per server.
        // Only ONE response is scripted (for the first server). If the gate
        // incorrectly keys `fired` by a single shared "mcp_first_use" bucket
        // (the pre-fix bug), the first call marks that shared bucket fired
        // and the second server's call would never re-consult the adapter,
        // silently returning `Ok`. Post-fix, the second server's own
        // trigger has not fired yet, so it must consult the adapter again;
        // since responses are exhausted, the scripted adapter answers
        // `Block`, and the call must be denied.
        let (tx, rx) = mpsc::channel(8);
        let _adapter = responder(rx, vec![ApprovalResponse::Allow]);
        let gate = ApprovalGate::new(
            vec![
                ApprovalTrigger::McpToolFirstUse {
                    server: "filesystem".into(),
                },
                ApprovalTrigger::McpToolFirstUse {
                    server: "web".into(),
                },
            ],
            tx,
        );

        // First server's first tool call: consults the adapter, approved.
        assert!(gate
            .before_tool("mcp__filesystem__read_file", &json!({"path": "a"}))
            .await
            .is_ok());
        // Second server's first tool call must still require its own
        // approval, not be silently let through by the first server's
        // already-fired trigger.
        assert!(matches!(
            gate.before_tool("mcp__web__fetch", &json!({"url": "b"}))
                .await,
            Err(PolicyError::ApprovalDenied)
        ));
    }
}
