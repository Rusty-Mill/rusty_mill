//! Proxies tools from other, already-running MCP servers -- Direction B
//! ("rusty_provider as an MCP gateway") from the design doc.
//!
//! `rusty_mcp` only covers the server side of MCP (its `client` feature is
//! dev-dependency-only), so this module talks to `rmcp`'s client API
//! directly: spawning stdio subprocesses via `TokioChildProcess`, or
//! connecting to Streamable HTTP endpoints via `StreamableHttpClientTransport`.
//!
//! A connection that fails at *startup* is logged and skipped -- same
//! soft-fail convention as `[jwt]`/`[webhook]`/`[persistence]` elsewhere in
//! this codebase -- rather than a hard failure of the whole server; it stays
//! absent from the tool list until restart. A connection that drops *after*
//! connecting is different: a background supervisor task per upstream
//! (spawned in [`McpGateway::connect`]) reconnects it with exponential
//! backoff (`[mcp].reconnect_backoff_secs`/`reconnect_backoff_max_secs`/
//! `max_reconnect_attempts`), so a transient upstream outage recovers on its
//! own instead of needing a full `rp-server` restart.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use rmcp::model::{CallToolRequestParams, CallToolResponse, JsonObject, Tool};
use rmcp::service::RunningService;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{ConfigureCommandExt, StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{Peer, RoleClient, ServiceExt};
use tokio::sync::RwLock;

use rp_router::{McpConfig, McpUpstreamConfig, McpUpstreamTransport};

/// A connected upstream's `Peer` handle, keyed by its configured name --
/// `Peer` is cheaply `Clone`, so this is what `list_tools`/`call_tool`
/// read, kept separate from the `RunningService` each per-upstream
/// supervisor task owns exclusively (see [`spawn_supervisor`]) so both can
/// be used concurrently without fighting over ownership: `waiting()` on a
/// `RunningService` consumes it, which a value shared behind a `RwLock`
/// read guard can never do.
type Peers = HashMap<String, Peer<RoleClient>>;

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
}

impl McpGateway {
    /// A gateway with no upstreams configured.
    pub fn empty() -> Self {
        Self {
            peers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Connect to every configured upstream, skipping (with a warning) any
    /// that fails at startup. Each upstream that *does* connect gets its
    /// own background supervisor task that reconnects it with backoff if
    /// the connection later drops -- see this module's doc comment.
    pub async fn connect(upstreams: &[McpUpstreamConfig], config: &McpConfig) -> Self {
        let peers = Arc::new(RwLock::new(HashMap::new()));
        let policy = ReconnectPolicy::from(config);
        for upstream in upstreams {
            match connect_one(upstream).await {
                Ok(service) => {
                    tracing::info!(upstream = %upstream.name, "connected MCP upstream");
                    peers
                        .write()
                        .await
                        .insert(upstream.name.clone(), service.peer().clone());
                    spawn_supervisor(upstream.clone(), service, Arc::clone(&peers), policy);
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
        Self { peers }
    }

    /// Every proxied tool across every connected upstream, each renamed to
    /// `"{upstream}/{tool}"`. An upstream whose `tools/list` call fails is
    /// logged and skipped for this call, rather than failing the whole
    /// listing. An upstream mid-reconnect simply has no entry here at all
    /// (the supervisor removes it the moment its connection drops), so this
    /// never attempts a call it already knows will fail.
    pub async fn list_tools(&self) -> Vec<Tool> {
        let peers = self.peers.read().await;
        let mut tools = Vec::new();
        for (name, peer) in peers.iter() {
            match peer.list_tools(None).await {
                Ok(result) => {
                    for mut tool in result.tools {
                        tool.name = format!("{name}/{}", tool.name).into();
                        tools.push(tool);
                    }
                }
                Err(error) => {
                    tracing::warn!(upstream = %name, %error, "failed to list tools from MCP upstream");
                }
            }
        }
        tools
    }

    /// Forward a `tools/call` to `upstream`'s `tool`, verbatim.
    pub async fn call_tool(
        &self,
        upstream: &str,
        tool: &str,
        arguments: Option<JsonObject>,
    ) -> Result<CallToolResponse, GatewayError> {
        let peer = {
            let peers = self.peers.read().await;
            peers
                .get(upstream)
                .cloned()
                .ok_or_else(|| GatewayError::UnknownUpstream(upstream.to_string()))?
        };

        let mut params = CallToolRequestParams::new(tool.to_string());
        if let Some(arguments) = arguments {
            params = params.with_arguments(arguments);
        }
        peer.call_tool_once(params)
            .await
            .map_err(GatewayError::Service)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error("no MCP upstream named '{0}' is connected")]
    UnknownUpstream(String),
    #[error(transparent)]
    Service(#[from] rmcp::ServiceError),
}

async fn connect_one(
    upstream: &McpUpstreamConfig,
) -> anyhow::Result<RunningService<RoleClient, ()>> {
    match &upstream.transport {
        McpUpstreamTransport::Stdio { command, args } => {
            let transport =
                TokioChildProcess::new(tokio::process::Command::new(command).configure(|c| {
                    c.args(args);
                }))?;
            Ok(().serve(transport).await?)
        }
        McpUpstreamTransport::Http {
            url,
            bearer_token_env,
        } => {
            let mut config = StreamableHttpClientTransportConfig::with_uri(url.clone());
            if let Some(var) = bearer_token_env {
                let token = std::env::var(var).map_err(|_| {
                    anyhow::anyhow!("bearer_token_env '{var}' is not set in the environment")
                })?;
                config = config.auth_header(token);
            }
            let transport = StreamableHttpClientTransport::from_config(config);
            Ok(().serve(transport).await?)
        }
    }
}

/// Owns `service` for as long as it's alive, blocking on
/// [`RunningService::waiting`] (which requires ownership -- exactly why
/// this task, not [`McpGateway`] itself, holds the `RunningService`; the
/// shared `peers` map only ever holds the cheaply-`Clone`able `Peer`
/// handle). Once that connection ends, removes it from `peers` (so
/// `list_tools`/`call_tool` stop attempting doomed calls to it) and retries
/// [`connect_one`] with exponential backoff until it reconnects or
/// `policy.max_attempts` is exhausted, at which point this upstream is
/// given up on for good -- same as if it had failed at startup.
fn spawn_supervisor(
    upstream: McpUpstreamConfig,
    mut service: RunningService<RoleClient, ()>,
    peers: Arc<RwLock<Peers>>,
    policy: ReconnectPolicy,
) {
    tokio::spawn(async move {
        loop {
            match service.waiting().await {
                Ok(reason) => tracing::warn!(
                    upstream = %upstream.name,
                    ?reason,
                    "MCP upstream connection ended; attempting to reconnect"
                ),
                Err(error) => tracing::warn!(
                    upstream = %upstream.name,
                    %error,
                    "MCP upstream connection task failed; attempting to reconnect"
                ),
            }
            peers.write().await.remove(&upstream.name);

            let mut backoff = policy.initial_backoff;
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
                tokio::time::sleep(backoff).await;
                attempts += 1;
                match connect_one(&upstream).await {
                    Ok(new_service) => {
                        tracing::info!(upstream = %upstream.name, attempts, "reconnected MCP upstream");
                        break Some(new_service);
                    }
                    Err(error) => {
                        tracing::warn!(
                            upstream = %upstream.name,
                            %error,
                            attempts,
                            next_backoff_secs = backoff.as_secs(),
                            "MCP upstream reconnect attempt failed"
                        );
                        backoff = next_backoff(backoff, policy.max_backoff);
                    }
                }
            };

            match reconnected {
                Some(new_service) => {
                    peers
                        .write()
                        .await
                        .insert(upstream.name.clone(), new_service.peer().clone());
                    service = new_service;
                }
                None => return,
            }
        }
    });
}

/// Doubles `current`, capped at `max` -- standard exponential backoff, no
/// jitter (a handful of upstreams reconnecting in lockstep isn't a
/// thundering-herd concern at this scale).
fn next_backoff(current: Duration, max: Duration) -> Duration {
    (current * 2).min(max)
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
    fn next_backoff_doubles_each_time() {
        let max = Duration::from_secs(60);
        let a = Duration::from_secs(1);
        let b = next_backoff(a, max);
        let c = next_backoff(b, max);
        assert_eq!(b, Duration::from_secs(2));
        assert_eq!(c, Duration::from_secs(4));
    }

    #[test]
    fn next_backoff_caps_at_max() {
        let max = Duration::from_secs(60);
        let near_max = Duration::from_secs(50);
        assert_eq!(next_backoff(near_max, max), max);
        // Once at the cap, doubling again stays at the cap, not above it.
        assert_eq!(next_backoff(max, max), max);
    }

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
}
