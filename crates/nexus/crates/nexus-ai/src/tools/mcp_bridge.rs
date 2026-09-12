//! MCP → AI tool-registry bridge (G5b).
//!
//! Surfaces tools advertised by external MCP servers
//! (`com.nexus.mcp.host`) to the AI plugin's [`ToolRegistry`] so the
//! model can invoke them through the same tool-loop that handles the
//! native built-ins. Activated by [`AiToolPolicy::AutoWithMcp`].
//!
//! ## Discovery flow (per `stream_chat` call)
//!
//! 1. `com.nexus.mcp.host::list_servers` (sync, fast).
//! 2. For every non-disabled server, `list_tools` is dispatched in
//!    parallel with a 5 s per-call timeout.
//! 3. Each `McpToolEntry` becomes a registry entry whose name is
//!    `mcp__<server>__<tool>` (sanitised to satisfy provider regexes).
//! 4. The executor proxies invocations back to
//!    `com.nexus.mcp.host::call_tool` with the model's argument JSON.
//!
//! Failures in any of these steps are logged and the affected server
//! is skipped — the chat call still proceeds with whatever tools were
//! discoverable plus the built-ins.
//!
//! No caching: each `stream_chat` re-runs discovery. Adding a TTL'd
//! cache + reload handler is the natural follow-up if the latency
//! becomes a problem in practice.
//!
//! [`AiToolPolicy::AutoWithMcp`]: crate::ipc::AiToolPolicy::AutoWithMcp

use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::future::join_all;
use nexus_kernel::{Ipc as _, KernelPluginContext};
use serde::Deserialize;

use super::registry::{ToolError, ToolExecutor, ToolRegistry, ToolSchema};

/// Plugin id of the MCP host.
const MCP_PLUGIN: &str = "com.nexus.mcp.host";

/// Per-IPC timeout for the discovery phase (`list_servers`,
/// `list_tools`). Short because a slow server shouldn't block chat
/// from starting; the model still has the built-ins to work with.
const MCP_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(5);

/// Timeout for `call_tool` invocations made by the model. Generous
/// because real MCP tools (web fetches, code execution, …) routinely
/// take tens of seconds.
const MCP_CALL_TIMEOUT: Duration = Duration::from_secs(60);

/// Maximum length of a synthesised tool name — Anthropic accepts up
/// to 64 chars in `^[a-zA-Z0-9_-]+$`; `OpenAI`'s limit is the same.
const MAX_TOOL_NAME_LEN: usize = 64;

/// Decoded shape of one entry in `com.nexus.mcp.host::list_servers`.
#[derive(Debug, Deserialize)]
struct ServerEntry {
    name: String,
    #[serde(default)]
    disabled: bool,
}

/// Decoded shape of one entry in `com.nexus.mcp.host::list_tools` —
/// matches `nexus_mcp::ipc::McpToolEntry` (post-G5a).
#[derive(Debug, Deserialize)]
struct ToolEntry {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    input_schema: Option<serde_json::Value>,
}

/// Executor that forwards a model tool-call to
/// `com.nexus.mcp.host::call_tool`. The MCP reply is returned to the
/// model as a JSON string — providers tolerate JSON-in-JSON better
/// than ad-hoc text shaping, and the model can navigate the structure.
pub struct McpToolExecutor {
    ctx: Arc<KernelPluginContext>,
    server: String,
    tool: String,
}

impl McpToolExecutor {
    /// Bind an executor to the given MCP server + tool. The kernel
    /// context must hold `Capability::IpcCall` for the dispatch to
    /// succeed at runtime.
    #[must_use]
    pub fn new(ctx: Arc<KernelPluginContext>, server: String, tool: String) -> Self {
        Self { ctx, server, tool }
    }
}

#[async_trait]
impl ToolExecutor for McpToolExecutor {
    async fn execute(&self, input: serde_json::Value) -> Result<String, ToolError> {
        let response = self
            .ctx
            .ipc_call(
                MCP_PLUGIN,
                "call_tool",
                serde_json::json!({
                    "server": self.server,
                    "tool": self.tool,
                    "arguments": input,
                }),
                MCP_CALL_TIMEOUT,
            )
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("mcp call_tool: {e}")))?;

        Ok(response.to_string())
    }
}

/// Build a registry that includes every tool in `builtins` plus
/// every tool advertised by an enabled MCP server.
///
/// On any discovery failure this still returns a valid registry —
/// either the unchanged built-ins (if `list_servers` itself failed)
/// or the built-ins plus whatever MCP tools could be enumerated.
pub async fn discover_mcp_tools(
    ctx: Arc<KernelPluginContext>,
    builtins: Arc<ToolRegistry>,
) -> Arc<ToolRegistry> {
    let mut merged: ToolRegistry = (*builtins).clone();

    let servers_resp = match ctx
        .ipc_call(
            MCP_PLUGIN,
            "list_servers",
            serde_json::json!({}),
            MCP_DISCOVERY_TIMEOUT,
        )
        .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "mcp bridge: list_servers failed; serving built-ins only");
            return Arc::new(merged);
        }
    };

    let servers: Vec<ServerEntry> = match serde_json::from_value(servers_resp) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "mcp bridge: list_servers decode failed");
            return Arc::new(merged);
        }
    };

    let active: Vec<String> = servers
        .into_iter()
        .filter(|s| !s.disabled)
        .map(|s| s.name)
        .collect();

    if active.is_empty() {
        return Arc::new(merged);
    }

    let lookups = active.into_iter().map(|server| {
        let ctx = Arc::clone(&ctx);
        async move {
            let resp = ctx
                .ipc_call(
                    MCP_PLUGIN,
                    "list_tools",
                    serde_json::json!({ "server": server }),
                    MCP_DISCOVERY_TIMEOUT,
                )
                .await;
            (server, resp)
        }
    });

    for (server, resp) in join_all(lookups).await {
        let payload = match resp {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(server = %server, error = %e, "mcp bridge: list_tools failed; skipping");
                continue;
            }
        };
        let tools: Vec<ToolEntry> = match serde_json::from_value(payload) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(server = %server, error = %e, "mcp bridge: list_tools decode failed");
                continue;
            }
        };
        for tool in tools {
            // Skip tools without a schema — the model has no way to
            // build valid arguments without one. After G5a every
            // entry SHOULD carry a schema, but the field is Option<>
            // so we tolerate absence.
            let Some(input_schema) = tool.input_schema else {
                tracing::debug!(
                    server = %server,
                    tool = %tool.name,
                    "mcp bridge: tool has no input_schema; skipping"
                );
                continue;
            };
            let bridge_name = mcp_tool_name(&server, &tool.name);
            merged.register(
                &bridge_name,
                ToolSchema {
                    name: bridge_name.clone(),
                    description: tool.description.unwrap_or_default(),
                    input_schema,
                },
                Arc::new(McpToolExecutor::new(
                    Arc::clone(&ctx),
                    server.clone(),
                    tool.name,
                )),
            );
        }
    }

    Arc::new(merged)
}

/// Compose `mcp__<server>__<tool>`, sanitised so the result matches
/// the provider tool-name regex (`^[a-zA-Z0-9_-]+$`) and fits the
/// 64-char cap. Non-conforming chars become `_`.
///
/// When the combined name fits, it's returned as-is. When it doesn't,
/// a bare prefix slice would silently collide two distinct
/// `(server, tool)` pairs that happen to share their first 64
/// characters — `ToolRegistry::register` overwrites on a name clash,
/// so one of the two tools would simply vanish. Instead the last
/// `_` + 8 hex chars are reserved for a deterministic disambiguator
/// (a hash of the *untruncated* pair), so distinct pairs still
/// produce distinct names even after truncation.
fn mcp_tool_name(server: &str, tool: &str) -> String {
    fn sanitize(s: &str) -> String {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    }
    let combined = format!("mcp__{}__{}", sanitize(server), sanitize(tool));
    if combined.len() <= MAX_TOOL_NAME_LEN {
        return combined;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    server.hash(&mut hasher);
    // Separator byte outside the sanitised alphabet so e.g. ("ab", "c")
    // and ("a", "bc") hash differently despite an identical concatenation.
    0xffu8.hash(&mut hasher);
    tool.hash(&mut hasher);
    // Intentional: only the low 32 bits are needed for an 8-hex-char
    // disambiguator, not the full 64-bit digest.
    #[allow(clippy::cast_possible_truncation)]
    let digest = format!("{:08x}", hasher.finish() as u32);
    // Char-boundary safe: combined is ASCII after sanitize.
    let prefix_len = MAX_TOOL_NAME_LEN - digest.len() - 1;
    format!("{}_{digest}", &combined[..prefix_len])
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_kernel::{
        CapabilitySet, EventBus, InMemoryKvStore, IpcDispatcher, IpcError, IpcFuture,
        KernelPluginContext, KvStore,
    };
    use std::sync::Mutex;

    #[test]
    fn mcp_tool_name_composes_namespaced_form() {
        assert_eq!(mcp_tool_name("notes", "fetch"), "mcp__notes__fetch");
    }

    #[test]
    fn mcp_tool_name_replaces_disallowed_chars() {
        assert_eq!(
            mcp_tool_name("my server", "do.thing"),
            "mcp__my_server__do_thing"
        );
    }

    #[test]
    fn mcp_tool_name_truncates_to_64_chars() {
        let server = "s".repeat(40);
        let tool = "t".repeat(40);
        let out = mcp_tool_name(&server, &tool);
        assert_eq!(out.len(), 64);
        assert!(out.starts_with("mcp__"));
    }

    /// Regression test — a bare 64-char prefix slice previously let two
    /// distinct `(server, tool)` pairs that share the same first 64
    /// characters collapse to the identical registry key (the second
    /// silently overwrites the first in `ToolRegistry::register`). The
    /// reserved disambiguator suffix must keep them distinct.
    #[test]
    fn mcp_tool_name_disambiguates_shared_64_char_prefix() {
        let server = "svc";
        let tool_a = format!("{}AAA", "x".repeat(100));
        let tool_b = format!("{}BBB", "x".repeat(100));
        // Sanity: the two untruncated names really do share their first
        // 64 characters, so a naive `[..64]` slice would collide them.
        let combined_a = format!("mcp__{server}__{tool_a}");
        let combined_b = format!("mcp__{server}__{tool_b}");
        assert_eq!(
            &combined_a[..MAX_TOOL_NAME_LEN],
            &combined_b[..MAX_TOOL_NAME_LEN]
        );

        let name_a = mcp_tool_name(server, &tool_a);
        let name_b = mcp_tool_name(server, &tool_b);
        assert_ne!(
            name_a, name_b,
            "distinct (server, tool) pairs must not collide after truncation"
        );
        assert_eq!(name_a.len(), MAX_TOOL_NAME_LEN);
        assert_eq!(name_b.len(), MAX_TOOL_NAME_LEN);
    }

    /// Stub IPC dispatcher: returns canned `list_servers` /
    /// `list_tools` responses so we can exercise `discover_mcp_tools`
    /// without spinning up a real MCP host.
    struct StubMcp {
        servers: serde_json::Value,
        tools_by_server: std::collections::HashMap<String, serde_json::Value>,
        seen: Mutex<Vec<(String, String, serde_json::Value)>>,
    }

    impl IpcDispatcher for StubMcp {
        fn dispatch(
            &self,
            target: &str,
            command: &str,
            _args: &serde_json::Value,
        ) -> Result<serde_json::Value, IpcError> {
            Err(IpcError::CommandNotFound {
                plugin_id: target.to_string(),
                command: command.to_string(),
            })
        }

        fn dispatch_async(
            &self,
            target: &str,
            command: &str,
            args: serde_json::Value,
        ) -> Option<IpcFuture> {
            self.seen
                .lock()
                .unwrap()
                .push((target.to_string(), command.to_string(), args.clone()));
            if target != MCP_PLUGIN {
                return None;
            }
            match command {
                "list_servers" => {
                    let r = self.servers.clone();
                    Some(Box::pin(async move { Ok(r) }))
                }
                "list_tools" => {
                    let server = args
                        .get("server")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let r = self
                        .tools_by_server
                        .get(&server)
                        .cloned()
                        .unwrap_or_else(|| serde_json::Value::Array(Vec::new()));
                    Some(Box::pin(async move { Ok(r) }))
                }
                _ => None,
            }
        }
    }

    fn make_ctx(dispatcher: Arc<dyn IpcDispatcher>) -> (KernelPluginContext, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let kv: Arc<dyn KvStore> = Arc::new(InMemoryKvStore::new());
        let bus = Arc::new(EventBus::new(16));
        let caps: CapabilitySet = [nexus_kernel::Capability::IpcCall].into_iter().collect();
        let ctx = KernelPluginContext::new(
            "com.nexus.ai",
            "0.0.1",
            caps,
            kv,
            bus,
            dir.path(),
            Some(dispatcher),
        )
        .unwrap();
        (ctx, dir)
    }

    #[tokio::test]
    async fn discovery_merges_mcp_tools_into_registry() {
        let stub = StubMcp {
            servers: serde_json::json!([
                { "name": "alpha", "disabled": false },
                { "name": "beta", "disabled": true },
            ]),
            tools_by_server: [(
                "alpha".to_string(),
                serde_json::json!([
                    {
                        "name": "fetch",
                        "description": "fetch a URL",
                        "input_schema": { "type": "object" }
                    },
                ]),
            )]
            .into_iter()
            .collect(),
            seen: Mutex::new(Vec::new()),
        };
        let dispatcher = Arc::new(stub);
        let (ctx, _tmp) = make_ctx(dispatcher.clone());

        let builtins = Arc::new(ToolRegistry::new());
        let merged = discover_mcp_tools(Arc::new(ctx), builtins).await;

        assert!(merged.contains("mcp__alpha__fetch"));
        // Disabled server is skipped.
        let names: Vec<String> = merged.schemas().into_iter().map(|s| s.name).collect();
        assert!(!names.iter().any(|n| n.starts_with("mcp__beta__")));
    }

    #[tokio::test]
    async fn discovery_returns_builtins_when_list_servers_decode_fails() {
        let stub = StubMcp {
            // Wrong shape — should decode-fail and we should fall back.
            servers: serde_json::json!({ "broken": true }),
            tools_by_server: std::collections::HashMap::new(),
            seen: Mutex::new(Vec::new()),
        };
        let dispatcher: Arc<dyn IpcDispatcher> = Arc::new(stub);
        let (ctx, _tmp) = make_ctx(dispatcher);

        let mut builtins = ToolRegistry::new();
        builtins.register(
            "echo",
            ToolSchema {
                name: "echo".into(),
                description: "echo".into(),
                input_schema: serde_json::json!({"type":"object"}),
            },
            Arc::new(StubExec),
        );
        let merged = discover_mcp_tools(Arc::new(ctx), Arc::new(builtins)).await;

        assert!(merged.contains("echo"));
        assert_eq!(merged.len(), 1);
    }

    #[tokio::test]
    async fn discovery_skips_tools_without_input_schema() {
        let stub = StubMcp {
            servers: serde_json::json!([{ "name": "a", "disabled": false }]),
            tools_by_server: [(
                "a".to_string(),
                serde_json::json!([
                    { "name": "with", "description": "ok", "input_schema": { "type": "object" } },
                    { "name": "without", "description": "no schema" },
                ]),
            )]
            .into_iter()
            .collect(),
            seen: Mutex::new(Vec::new()),
        };
        let dispatcher: Arc<dyn IpcDispatcher> = Arc::new(stub);
        let (ctx, _tmp) = make_ctx(dispatcher);

        let merged = discover_mcp_tools(Arc::new(ctx), Arc::new(ToolRegistry::new())).await;
        assert!(merged.contains("mcp__a__with"));
        assert!(!merged.contains("mcp__a__without"));
    }

    struct StubExec;
    #[async_trait]
    impl ToolExecutor for StubExec {
        async fn execute(&self, _input: serde_json::Value) -> Result<String, ToolError> {
            Ok("ok".into())
        }
    }
}
