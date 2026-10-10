//! Proxies tools from other, already-running MCP servers -- Direction B
//! ("rusty_provider as an MCP gateway") from the design doc.
//!
//! Each upstream is reached with [`rusty_mcp_client::McpClient`] (a child
//! process over stdio, or a Streamable HTTP endpoint), and its tools are
//! offered to our own clients through [`GatewaySource`], renamed
//! `"{upstream}/{tool}"`.
//!
//! A connection that fails at *startup* is logged and skipped -- same
//! soft-fail convention as `[jwt]`/`[webhook]`/`[persistence]` elsewhere in
//! this codebase -- rather than a hard failure of the whole server; it stays
//! absent from the tool list until restart. A connection that drops *after*
//! connecting is different: a background supervisor task per upstream
//! (spawned in [`McpGateway::connect`]) probes it every
//! [`LIVENESS_POLL`] and, once it is gone, reconnects it with exponential
//! backoff (`[mcp].reconnect_backoff_secs`/`reconnect_backoff_max_secs`/
//! `max_reconnect_attempts`), so a transient upstream outage recovers on its
//! own instead of needing a full `rp-server` restart.
//!
//! Each connection runs one call at a time, so an HTTP upstream is given
//! `[mcp].connections` of them and a call goes to the one with the fewest in
//! flight; a call that hangs holds up only its own connection until
//! `[mcp].timeout_secs` ends it. A stdio upstream is one child process and
//! gets a single connection.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rusty_mcp_client::{
    McpAuth, McpAuthSecret, McpClient, McpClientError, McpServerSpec, McpTransport,
};
use rusty_mcp_server::proto::{CallToolParams, CallToolResult, ErrorCode, ErrorData, Tool};
use rusty_mcp_server::{CallContext, ToolSource};
use rusty_retry::Backoff;
use tokio::runtime::Handle;
use tokio::sync::RwLock;

use rp_router::{McpConfig, McpUpstreamConfig, McpUpstreamTransport};

/// How often a connected upstream is probed to notice that it dropped.
pub const LIVENESS_POLL: Duration = Duration::from_secs(2);

/// Connected upstreams by configured name, in name order so the tool list is
/// stable between requests.
type Peers = BTreeMap<String, Arc<Pool>>;

/// The connections to one upstream.
struct Pool {
    slots: Vec<Slot>,
}

struct Slot {
    client: McpClient,
    /// Calls running on `client` now.
    busy: AtomicUsize,
}

/// A connection taken from a [`Pool`] for one call; releases it on drop.
struct Lease<'a> {
    slot: &'a Slot,
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        self.slot.busy.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Lease<'_> {
    fn client(&self) -> &McpClient {
        &self.slot.client
    }
}

impl Pool {
    /// `size` connections to `upstream` (at least one; a stdio upstream gets
    /// exactly one).
    async fn connect(
        upstream: &McpUpstreamConfig,
        size: usize,
        call_timeout: Duration,
    ) -> Result<Self, McpClientError> {
        let size = match upstream.transport {
            McpUpstreamTransport::Stdio { .. } => 1,
            McpUpstreamTransport::Http { .. } => size.max(1),
        };
        let mut slots = Vec::with_capacity(size);
        for _ in 0..size {
            slots.push(Slot {
                client: connect_one(upstream, call_timeout).await?,
                busy: AtomicUsize::new(0),
            });
        }
        Ok(Self { slots })
    }

    /// The connection with the fewest calls in flight (the first of equals).
    fn lease(&self) -> Lease<'_> {
        let slot = self
            .slots
            .iter()
            .min_by_key(|s| s.busy.load(Ordering::SeqCst))
            .unwrap_or(&self.slots[0]);
        slot.busy.fetch_add(1, Ordering::SeqCst);
        Lease { slot }
    }

    /// Whether every connection still works.
    async fn is_alive(&self) -> bool {
        for slot in &self.slots {
            if !slot.client.is_alive().await {
                return false;
            }
        }
        true
    }
}

/// Backoff policy for reconnecting a dropped (previously-connected)
/// upstream. Doesn't apply to a startup connection failure -- see this
/// module's doc comment.
#[derive(Debug, Clone, Copy)]
struct ReconnectPolicy {
    initial_backoff: Duration,
    max_backoff: Duration,
    max_attempts: Option<u32>,
}

impl From<&McpConfig> for ReconnectPolicy {
    fn from(config: &McpConfig) -> Self {
        Self {
            initial_backoff: Duration::from_secs(config.reconnect_backoff_secs),
            max_backoff: Duration::from_secs(config.reconnect_backoff_max_secs),
            max_attempts: config.max_reconnect_attempts,
        }
    }
}

/// Aggregates tools from every configured upstream MCP server behind one
/// name-prefixed tool namespace (`"{upstream}/{tool}"`).
pub struct McpGateway {
    peers: Arc<RwLock<Peers>>,
    /// Per-call timeout for `tools/list`/`tools/call` requests forwarded to
    /// an upstream -- see `McpConfig::timeout_secs`. Without this, a
    /// misbehaving or hung upstream could stall a client-facing request
    /// forever, unlike every other outbound integration in this router
    /// family (`ProviderConfig`/`ModerationConfig`/`WebSearchConfig`/
    /// webhook `timeout_secs`).
    timeout: Duration,
}

/// Mirrors `McpConfig::timeout_secs`'s default, for [`McpGateway::empty`]
/// (which has no `McpConfig` to read one from since it connects nothing).
const DEFAULT_TIMEOUT_SECS: u64 = 30;

impl McpGateway {
    /// A gateway with no upstreams configured.
    pub fn empty() -> Self {
        Self {
            peers: Arc::new(RwLock::new(BTreeMap::new())),
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        }
    }

    /// Connect to every configured upstream, skipping (with a warning) any
    /// that fails at startup. Each upstream that *does* connect gets its
    /// own background supervisor task that reconnects it with backoff if
    /// the connection later drops -- see this module's doc comment.
    pub async fn connect(upstreams: &[McpUpstreamConfig], config: &McpConfig) -> Self {
        let size = config.connections;
        let peers = Arc::new(RwLock::new(BTreeMap::new()));
        let policy = ReconnectPolicy::from(config);
        let timeout = Duration::from_secs(config.timeout_secs);
        for upstream in upstreams {
            match Pool::connect(upstream, size, timeout).await {
                Ok(pool) => {
                    tracing::info!(upstream = %upstream.name, "connected MCP upstream");
                    let pool = Arc::new(pool);
                    peers
                        .write()
                        .await
                        .insert(upstream.name.clone(), Arc::clone(&pool));
                    spawn_supervisor(
                        upstream.clone(),
                        pool,
                        size,
                        Arc::clone(&peers),
                        policy,
                        timeout,
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        upstream = %upstream.name,
                        %error,
                        "failed to connect MCP upstream; its tools won't be available until restart"
                    );
                }
            }
        }
        Self { peers, timeout }
    }

    /// Every proxied tool across every connected upstream, each renamed to
    /// `"{upstream}/{tool}"`. An upstream whose `tools/list` call fails or
    /// exceeds `timeout` is logged and skipped for this call, rather than
    /// failing the whole listing. An upstream mid-reconnect simply has no
    /// entry here at all (the supervisor removes it the moment its
    /// connection drops), so this never attempts a call it already knows
    /// will fail.
    pub async fn list_tools(&self) -> Vec<Tool> {
        let peers = self.peers.read().await;
        let mut tools = Vec::new();
        for (name, pool) in peers.iter() {
            let lease = pool.lease();
            match tokio::time::timeout(self.timeout, lease.client().list_tools()).await {
                Ok(Ok(listed)) => {
                    for mut tool in listed {
                        tool.name = format!("{name}/{}", tool.name);
                        tools.push(tool);
                    }
                }
                Ok(Err(error)) => {
                    tracing::warn!(upstream = %name, %error, "failed to list tools from MCP upstream");
                }
                Err(_elapsed) => {
                    tracing::warn!(
                        upstream = %name,
                        timeout_secs = self.timeout.as_secs(),
                        "MCP upstream tools/list call timed out"
                    );
                }
            }
        }
        tools
    }

    /// Forward a `tools/call` to `upstream`'s `tool`, verbatim. Bounded by
    /// `timeout`, returning [`GatewayError::Timeout`] rather than hanging
    /// forever if the upstream never responds.
    pub async fn call_tool(
        &self,
        upstream: &str,
        tool: &str,
        arguments: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<CallToolResult, GatewayError> {
        let pool = {
            let peers = self.peers.read().await;
            peers
                .get(upstream)
                .cloned()
                .ok_or_else(|| GatewayError::UnknownUpstream(upstream.to_string()))?
        };
        let lease = pool.lease();
        tokio::time::timeout(self.timeout, lease.client().call_tool(tool, arguments))
            .await
            .map_err(|_elapsed| GatewayError::Timeout(upstream.to_string()))?
            .map_err(GatewayError::Service)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error("no MCP upstream named '{0}' is connected")]
    UnknownUpstream(String),
    #[error(transparent)]
    Service(#[from] McpClientError),
    #[error("call to MCP upstream '{0}' timed out")]
    Timeout(String),
}

/// A [`McpGateway`] as the server's [`ToolSource`]. The server's handlers
/// block, so each request runs the gateway's async calls on `rt`.
pub struct GatewaySource {
    gateway: Arc<McpGateway>,
    rt: Handle,
}

impl GatewaySource {
    /// Offer `gateway`'s tools; `rt` is the runtime to drive it on.
    pub fn new(gateway: Arc<McpGateway>, rt: Handle) -> Self {
        Self { gateway, rt }
    }
}

impl ToolSource for GatewaySource {
    fn tools(&self) -> Vec<Tool> {
        self.rt.block_on(self.gateway.list_tools())
    }

    fn call(
        &self,
        _ctx: &CallContext,
        call: &CallToolParams,
    ) -> Option<Result<CallToolResult, ErrorData>> {
        let (upstream, tool) = call.name.split_once('/')?;
        Some(self.forward(upstream, tool, call))
    }
}

impl GatewaySource {
    fn forward(
        &self,
        upstream: &str,
        tool: &str,
        call: &CallToolParams,
    ) -> Result<CallToolResult, ErrorData> {
        let invalid = |message: String| ErrorData::new(ErrorCode::INVALID_PARAMS, message);
        let arguments = call
            .arguments
            .as_ref()
            .map(|a| serde_json::from_str(&a.to_json_string()))
            .transpose()
            .map_err(|e| invalid(format!("invalid arguments: {e}")))?;
        self.rt
            .block_on(self.gateway.call_tool(upstream, tool, arguments))
            .map_err(|e| invalid(e.to_string()))
    }
}

/// The client spec for one configured upstream.
fn spec_for(upstream: &McpUpstreamConfig) -> McpServerSpec {
    match &upstream.transport {
        McpUpstreamTransport::Stdio { command, args } => McpServerSpec {
            transport: McpTransport::Stdio,
            command: command.clone(),
            args: args.clone(),
            ..McpServerSpec::default()
        },
        McpUpstreamTransport::Http {
            url,
            bearer_token_env,
        } => McpServerSpec {
            transport: McpTransport::Http,
            url: Some(url.clone()),
            auth: bearer_token_env.as_ref().map(|env| McpAuth::Bearer {
                token: McpAuthSecret::Env { env: env.clone() },
            }),
            ..McpServerSpec::default()
        },
    }
}

async fn connect_one(
    upstream: &McpUpstreamConfig,
    call_timeout: Duration,
) -> Result<McpClient, McpClientError> {
    McpClient::connect_with(&upstream.name, &spec_for(upstream), call_timeout).await
}

/// Watches one connected upstream. Every [`LIVENESS_POLL`] it asks the pool
/// whether its connections still work; once it does not, it removes the
/// upstream from `peers` (so `list_tools`/`call_tool` stop attempting doomed
/// calls to it) and retries [`connect_one`] with exponential backoff until it
/// reconnects or `policy.max_attempts` is exhausted, at which point this
/// upstream is given up on for good -- same as if it had failed at startup.
fn spawn_supervisor(
    upstream: McpUpstreamConfig,
    mut pool: Arc<Pool>,
    size: usize,
    peers: Arc<RwLock<Peers>>,
    policy: ReconnectPolicy,
    call_timeout: Duration,
) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(LIVENESS_POLL).await;
            if pool.is_alive().await {
                continue;
            }
            tracing::warn!(
                upstream = %upstream.name,
                "MCP upstream connection ended; attempting to reconnect"
            );
            peers.write().await.remove(&upstream.name);

            let backoff = Backoff::exponential(policy.initial_backoff, policy.max_backoff, 0.0);
            let mut attempts: u32 = 0;
            let reconnected = loop {
                if should_give_up(attempts, policy.max_attempts) {
                    tracing::warn!(
                        upstream = %upstream.name,
                        attempts,
                        "giving up reconnecting to MCP upstream; its tools will stay unavailable until restart"
                    );
                    break None;
                }
                tokio::time::sleep(backoff.delay_for(attempts)).await;
                attempts += 1;
                match Pool::connect(&upstream, size, call_timeout).await {
                    Ok(new_pool) => {
                        tracing::info!(upstream = %upstream.name, attempts, "reconnected MCP upstream");
                        break Some(Arc::new(new_pool));
                    }
                    Err(error) => {
                        tracing::warn!(
                            upstream = %upstream.name,
                            %error,
                            attempts,
                            next_backoff_secs = backoff.delay_for(attempts).as_secs(),
                            "MCP upstream reconnect attempt failed"
                        );
                    }
                }
            };

            match reconnected {
                Some(new_pool) => {
                    peers
                        .write()
                        .await
                        .insert(upstream.name.clone(), Arc::clone(&new_pool));
                    pool = new_pool;
                }
                None => return,
            }
        }
    });
}

/// `true` once `attempts` has reached `max_attempts` (if one is
/// configured) -- `None` retries forever.
fn should_give_up(attempts: u32, max_attempts: Option<u32>) -> bool {
    max_attempts.is_some_and(|max| attempts >= max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_give_up_is_false_when_max_attempts_is_unset() {
        assert!(!should_give_up(0, None));
        assert!(!should_give_up(1_000_000, None));
    }

    #[test]
    fn should_give_up_respects_the_configured_cap() {
        assert!(!should_give_up(0, Some(3)));
        assert!(!should_give_up(2, Some(3)));
        assert!(should_give_up(3, Some(3)));
        assert!(should_give_up(4, Some(3)));
    }

    #[test]
    fn a_bearer_env_becomes_env_auth_and_stdio_args_carry_over() {
        let http = McpUpstreamConfig {
            name: "h".into(),
            transport: McpUpstreamTransport::Http {
                url: "http://x/mcp".into(),
                bearer_token_env: Some("TOK".into()),
            },
        };
        let spec = spec_for(&http);
        assert_eq!(spec.transport, McpTransport::Http);
        assert_eq!(spec.url.as_deref(), Some("http://x/mcp"));
        assert!(matches!(
            spec.auth,
            Some(McpAuth::Bearer { token: McpAuthSecret::Env { env } }) if env == "TOK"
        ));
        let stdio = McpUpstreamConfig {
            name: "s".into(),
            transport: McpUpstreamTransport::Stdio {
                command: "srv".into(),
                args: vec!["--x".into()],
            },
        };
        let spec = spec_for(&stdio);
        assert_eq!((spec.command.as_str(), spec.args.len()), ("srv", 1));
        assert!(spec.auth.is_none());
    }
}
