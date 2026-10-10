//! Connections to upstream MCP servers.
//!
//! A target is dialled once at startup and its connections are shared by every
//! request the gateway serves. That matters most for `stdio` targets:
//! spawning `npx @modelcontextprotocol/server-everything` per request would
//! cost far more than the call itself, and each spawn would lose whatever
//! state the server had built up.
//!
//! A native client runs one call at a time, so a target keeps a small pool of
//! them ([`HTTP_CONNECTIONS`] for an HTTP server, one for a child process,
//! which has a single stdin) and a call takes whichever is free, waiting for
//! one when all are busy. The first is dialled when the target comes up, so a
//! target that cannot be reached is reported at startup; the others are dialled
//! as calls need them.

use std::io;
use std::process::Command;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use agentgateway_config::{McpTarget, McpTargetKind};
use rusty_mcp_client_native::json::Value;
use rusty_mcp_client_native::proto::{
    CallToolParams, CallToolResponse, CallToolResult, GetPromptParams, GetPromptResult,
    ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult, ListToolsResult,
    PaginatedParams, Prompt, ReadResourceParams, ReadResourceResult, Resource, ResourceTemplate,
    ServerCapabilities, Tool, Wire,
};
use rusty_mcp_client_native::{
    Client, ClientConfig, ClientError, HttpConfig, HttpTransport, NoHandler, Recv, StdioTransport,
    Transport,
};

use crate::{
    gate::{GateError, TargetFilter},
    header_override::HeaderOverride,
};

/// How many connections a target that speaks HTTP may have open at once.
pub const HTTP_CONNECTIONS: usize = 8;

/// How long a call may wait for its answer when the route sets no backend
/// budget.
const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(600);

/// Failure to bring up a target.
#[derive(Debug, thiserror::Error)]
pub enum TargetError {
    /// The target's tool filters did not compile.
    #[error(transparent)]
    Gate(#[from] GateError),

    /// The subprocess could not be spawned.
    #[error("target `{name}`: spawning `{cmd}`: {source}")]
    Spawn {
        /// Target name.
        name: String,
        /// Command we tried to run.
        cmd: String,
        /// Underlying failure.
        #[source]
        source: io::Error,
    },

    /// The MCP handshake failed.
    #[error("target `{name}`: MCP handshake failed: {source}")]
    Handshake {
        /// Target name.
        name: String,
        /// Underlying failure.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// A transport this build cannot speak.
    #[error(
        "target `{name}`: the deprecated HTTP+SSE transport is not supported; \
         point the target at the server's Streamable HTTP endpoint with `mcp:` instead"
    )]
    UnsupportedTransport {
        /// Target name.
        name: String,
    },
}

/// Parts of a target's address a route's `urlRewrite` replaces.
///
/// Empty unless the federation has exactly one target: with more than one,
/// "the host" and "the path" have no answer, and choosing would be the gateway
/// deciding something the operator did not say. `Config::lint` reports the
/// cases this cannot cover.
#[derive(Debug, Clone, Default)]
pub struct Override {
    /// Replacement path, from `urlRewrite.path.full`.
    pub path: Option<String>,
    /// Replacement host and, when it names one, port.
    pub authority: Option<http::uri::Authority>,
}

/// The two ways to reach a server, as one transport type.
enum Link {
    Http(Box<HttpTransport>),
    Stdio(StdioTransport),
}

impl Transport for Link {
    fn send(&mut self, message: &rusty_mcp_client_native::proto::Message) -> io::Result<()> {
        match self {
            Self::Http(t) => t.send(message),
            Self::Stdio(t) => t.send(message),
        }
    }

    fn send_with(
        &mut self,
        message: &rusty_mcp_client_native::proto::Message,
        overrides: &rusty_mcp_client_native::HeaderOverride,
    ) -> io::Result<()> {
        match self {
            Self::Http(t) => t.send_with(message, overrides),
            Self::Stdio(t) => t.send(message),
        }
    }

    fn recv(&mut self, timeout: Duration) -> io::Result<Recv> {
        match self {
            Self::Http(t) => t.recv(timeout),
            Self::Stdio(t) => t.recv(timeout),
        }
    }

    fn set_protocol_version(&mut self, version: &rusty_mcp_client_native::proto::ProtocolVersion) {
        match self {
            Self::Http(t) => t.set_protocol_version(version),
            Self::Stdio(t) => t.set_protocol_version(version),
        }
    }

    fn clear_protocol_version(&mut self) {
        match self {
            Self::Http(t) => t.clear_protocol_version(),
            Self::Stdio(t) => t.clear_protocol_version(),
        }
    }
}

type Conn = Client<Link, NoHandler>;
/// Opens a connection, giving the handshake at most the time it is handed.
type Dial = dyn Fn(Duration) -> Result<Conn, ClientError> + Send + Sync;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The open connections of one target.
struct Pool {
    state: Mutex<Slots>,
    freed: Condvar,
    dial: Box<Dial>,
    max: usize,
    /// How many times a connection has been taken.
    leases: std::sync::atomic::AtomicUsize,
}

struct Slots {
    idle: Vec<Conn>,
    /// Connections that exist: idle, leased, or being dialled.
    open: usize,
}

/// A connection on loan; it goes back to the pool when dropped.
struct Lease<'a> {
    pool: &'a Pool,
    conn: Option<Conn>,
}

impl Lease<'_> {
    /// Run `f` on the connection.
    fn with<T>(
        &mut self,
        f: impl FnOnce(&mut Conn) -> Result<T, ClientError>,
    ) -> Result<T, ClientError> {
        // Held from the lease until it is dropped, so never `None` here.
        self.conn.as_mut().map_or(Err(ClientError::Closed), f)
    }
}

impl Lease<'_> {
    /// One request, given what is left of `deadline` to complete.
    fn call(
        &mut self,
        deadline: Instant,
        method: &str,
        params: Option<Value>,
        headers: &rusty_mcp_client_native::HeaderOverride,
    ) -> Result<Value, ClientError> {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(ClientError::Timeout);
        }
        self.with(|conn| {
            conn.set_call_timeout(left);
            conn.call_with(method, params, headers)
        })
    }
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take() {
            lock(&self.pool.state).idle.push(conn);
            self.pool.freed.notify_one();
        }
    }
}

impl Pool {
    /// A connection, waiting for one (or dialling one) until `deadline`.
    fn lease(&self, deadline: Instant) -> Result<Lease<'_>, ClientError> {
        self.leases
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut slots = lock(&self.state);
        loop {
            if let Some(conn) = slots.idle.pop() {
                return Ok(Lease {
                    pool: self,
                    conn: Some(conn),
                });
            }
            if slots.open < self.max {
                slots.open += 1;
                drop(slots);
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    lock(&self.state).open -= 1;
                    self.freed.notify_one();
                    return Err(ClientError::Timeout);
                }
                return match (self.dial)(left) {
                    Ok(conn) => Ok(Lease {
                        pool: self,
                        conn: Some(conn),
                    }),
                    Err(e) => {
                        lock(&self.state).open -= 1;
                        self.freed.notify_one();
                        Err(e)
                    }
                };
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(ClientError::Timeout);
            }
            slots = self
                .freed
                .wait_timeout(slots, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

/// A live connection to one upstream MCP server.
pub struct Target {
    /// Name used to qualify this target's tools.
    pub name: String,
    /// Which of its tools are federated.
    pub filter: TargetFilter,
    /// Whether this target speaks HTTP, and so has headers to mutate.
    ///
    /// A `stdio` target talks over a pipe. A guardrail's `headerMutation`
    /// aimed at one has nowhere to land, and is dropped rather than quietly
    /// appearing somewhere else.
    pub http: bool,
    capabilities: ServerCapabilities,
    pool: Pool,
    /// What one operation (a call, or a whole paged listing) may take, from
    /// asking for a connection to the last answer.
    timeout: Duration,
}

impl std::fmt::Debug for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Dumping live transports into a log line would not help anyone.
        f.debug_struct("Target")
            .field("name", &self.name)
            .field("filter", &self.filter)
            .finish_non_exhaustive()
    }
}

impl Target {
    /// Dial a target and complete the MCP handshake.
    ///
    /// Blocks while it does. `over` replaces parts of a Streamable HTTP
    /// target's address. It comes from the route's `urlRewrite`, and the
    /// federation only resolves one when there is a single target to be
    /// unambiguous about. `call_timeout` bounds each upstream call.
    pub fn connect(
        config: &McpTarget,
        over: &Override,
        call_timeout: Option<Duration>,
        at: &str,
    ) -> Result<Self, TargetError> {
        Self::connect_with(config, over, call_timeout, HTTP_CONNECTIONS, at)
    }

    /// [`Target::connect`] with at most `connections` open to an HTTP target
    /// (a child process always has one).
    pub fn connect_with(
        config: &McpTarget,
        over: &Override,
        call_timeout: Option<Duration>,
        connections: usize,
        at: &str,
    ) -> Result<Self, TargetError> {
        let filter = TargetFilter::new(&config.filters, at)?;
        let name = config.name.clone();
        let timeout = call_timeout.unwrap_or(DEFAULT_CALL_TIMEOUT);

        let (dial, max): (Box<Dial>, usize) = match &config.kind {
            McpTargetKind::Stdio(stdio) => {
                let (cmd, args, env) = (stdio.cmd.clone(), stdio.args.clone(), stdio.env.clone());
                let dial = move |left: Duration| {
                    let mut command = Command::new(&cmd);
                    command.args(&args);
                    for (key, value) in &env {
                        command.env(key, value);
                    }
                    let transport = StdioTransport::spawn(command)?;
                    handshake(Link::Stdio(transport), left, timeout)
                };
                (Box::new(dial), 1)
            }
            McpTargetKind::Mcp(http) => {
                let path = over.path.as_deref().unwrap_or(&http.path);
                // An authority without a port keeps the target's own. The
                // target names a port explicitly and the override did not, so
                // silently dropping to 80 would break a config that only meant
                // to move hosts.
                let (host, port) = match &over.authority {
                    Some(authority) => {
                        (authority.host(), authority.port_u16().unwrap_or(http.port))
                    }
                    None => (http.host.as_str(), http.port),
                };
                let url = format!("http://{host}:{port}{path}");
                let dial = move |left: Duration| {
                    let transport = HttpTransport::new(HttpConfig::new(url.as_str()))?;
                    handshake(Link::Http(Box::new(transport)), left, timeout)
                };
                (Box::new(dial), connections.max(1))
            }
            McpTargetKind::Sse(_) => {
                return Err(TargetError::UnsupportedTransport { name });
            }
        };

        let first = dial(timeout).map_err(|source| match (&config.kind, source) {
            (McpTargetKind::Stdio(stdio), ClientError::Io(source)) => TargetError::Spawn {
                name: name.clone(),
                cmd: stdio.cmd.clone(),
                source,
            },
            (_, source) => TargetError::Handshake {
                name: name.clone(),
                source: Box::new(source),
            },
        })?;
        let capabilities = first
            .session()
            .server_capabilities()
            .cloned()
            .unwrap_or_default();

        Ok(Target {
            name,
            filter,
            http: matches!(config.kind, McpTargetKind::Mcp(_)),
            capabilities,
            pool: Pool {
                state: Mutex::new(Slots {
                    idle: vec![first],
                    open: 1,
                }),
                freed: Condvar::new(),
                dial,
                max,
                leases: std::sync::atomic::AtomicUsize::new(0),
            },
            timeout,
        })
    }

    /// The headers for one request: `headers` (a guardrail's changes to the
    /// upstream HTTP request) if the target speaks HTTP.
    fn native(&self, headers: &HeaderOverride) -> rusty_mcp_client_native::HeaderOverride {
        if self.http {
            return headers.to_native();
        }
        if !headers.is_empty() {
            tracing::debug!(
                target = %self.name,
                "a guardrail asked to change headers on a stdio target; there are none"
            );
        }
        rusty_mcp_client_native::HeaderOverride::default()
    }

    /// One request to the target, within one operation budget.
    fn request(
        &self,
        method: &str,
        params: Option<Value>,
        headers: &HeaderOverride,
    ) -> Result<Value, ClientError> {
        let deadline = Instant::now() + self.timeout;
        self.pool
            .lease(deadline)?
            .call(deadline, method, params, &self.native(headers))
    }

    /// Every page of a list, on one connection (a cursor belongs to the
    /// session that issued it) and within one budget for the whole listing.
    fn pages<R: Wire, T>(
        &self,
        method: &str,
        headers: &HeaderOverride,
        split: impl Fn(R) -> (Vec<T>, Option<String>),
    ) -> Result<Vec<T>, ClientError> {
        let deadline = Instant::now() + self.timeout;
        let native = self.native(headers);
        let mut lease = self.pool.lease(deadline)?;
        let mut all = Vec::new();
        let mut cursor = None;
        loop {
            let params = PaginatedParams {
                cursor: cursor.take(),
                meta: None,
            };
            let page = lease.call(deadline, method, Some(params.to_value()), &native)?;
            let (items, next) = split(R::from_value(&page)?);
            all.extend(items);
            match next {
                Some(c) => cursor = Some(c),
                None => return Ok(all),
            }
        }
    }

    /// The tools this target exports, after its filters.
    ///
    /// `headers` are a guardrail's changes to the upstream HTTP request, and
    /// are ignored for a `stdio` target.
    pub fn tools(&self, headers: &HeaderOverride) -> Result<Vec<Tool>, ClientError> {
        let tools = self.pages("tools/list", headers, |r: ListToolsResult| {
            (r.tools, r.paging.next_cursor)
        })?;
        Ok(tools
            .into_iter()
            .filter(|tool| self.filter.permits(&tool.name))
            .collect())
    }

    /// Forward a tool call upstream.
    ///
    /// `headers` are a guardrail's changes to the upstream HTTP request, and
    /// are ignored for a `stdio` target. An upstream that answers with a
    /// request for input or a task is an error: the gateway has no client to
    /// put the question to.
    pub fn call(
        &self,
        params: &CallToolParams,
        headers: &HeaderOverride,
    ) -> Result<CallToolResult, ClientError> {
        let answer = self.request("tools/call", Some(params.to_value()), headers)?;
        match CallToolResponse::from_value(&answer)? {
            CallToolResponse::Complete(result) => Ok(result),
            CallToolResponse::InputRequired(_) | CallToolResponse::Task(_) => {
                Err(ClientError::Protocol(
                    "the upstream asked for input or started a task, which the gateway does not relay"
                        .to_owned(),
                ))
            }
        }
    }

    /// How many times a connection has been taken from the pool so far: one per
    /// call, and one per whole paged listing. For metrics and tests.
    pub fn leases(&self) -> usize {
        self.pool.leases.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Whether this target advertised prompts in its handshake.
    ///
    /// Read from the capabilities the server sent when it connected, so asking
    /// costs nothing. A target that never advertised prompts is not asked for
    /// them: `prompts/list` against such a server is a method-not-found error,
    /// and one target's missing capability should not read as a fault.
    pub fn serves_prompts(&self) -> bool {
        self.capabilities.prompts.is_some()
    }

    /// Whether this target advertised resources in its handshake.
    pub fn serves_resources(&self) -> bool {
        self.capabilities.resources.is_some()
    }

    /// The prompts this target exports.
    ///
    /// Prompts carry no per-target `filters`: `filters` names tools, and
    /// widening it silently to prompts would change what existing configs mean.
    /// `mcpAuthorization.rules` is what gates prompts.
    pub fn prompts(&self, headers: &HeaderOverride) -> Result<Vec<Prompt>, ClientError> {
        self.pages("prompts/list", headers, |r: ListPromptsResult| {
            (r.prompts, r.paging.next_cursor)
        })
    }

    /// Fetch one prompt.
    pub fn get_prompt(
        &self,
        params: &GetPromptParams,
        headers: &HeaderOverride,
    ) -> Result<GetPromptResult, ClientError> {
        let answer = self.request("prompts/get", Some(params.to_value()), headers)?;
        Ok(GetPromptResult::from_value(&answer)?)
    }

    /// The resources this target exports.
    pub fn resources(&self, headers: &HeaderOverride) -> Result<Vec<Resource>, ClientError> {
        self.pages("resources/list", headers, |r: ListResourcesResult| {
            (r.resources, r.paging.next_cursor)
        })
    }

    /// The resource templates this target exports.
    pub fn resource_templates(
        &self,
        headers: &HeaderOverride,
    ) -> Result<Vec<ResourceTemplate>, ClientError> {
        self.pages(
            "resources/templates/list",
            headers,
            |r: ListResourceTemplatesResult| (r.resource_templates, r.paging.next_cursor),
        )
    }

    /// Read one resource.
    pub fn read_resource(
        &self,
        params: &ReadResourceParams,
        headers: &HeaderOverride,
    ) -> Result<ReadResourceResult, ClientError> {
        let answer = self.request("resources/read", Some(params.to_value()), headers)?;
        Ok(ReadResourceResult::from_value(&answer)?)
    }
}

/// Run the MCP handshake over `link`, which may take `handshake` at most.
fn handshake(link: Link, handshake: Duration, call_timeout: Duration) -> Result<Conn, ClientError> {
    let mut config = ClientConfig::new("rusty-agent-gateway", env!("CARGO_PKG_VERSION"));
    config.call_timeout = call_timeout;
    Client::connect(link, config, NoHandler, handshake)
}
