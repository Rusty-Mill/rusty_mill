//! Connections to upstream MCP servers.
//!
//! A target is dialled once at startup and its connections are shared by every
//! request the gateway serves. That matters most for `stdio` targets:
//! spawning `npx @modelcontextprotocol/server-everything` per request would
//! cost far more than the call itself, and each spawn would lose whatever
//! state the server had built up.
//!
//! A native client runs one call at a time, so a target keeps a small pool of
//! them ([`HTTP_CONNECTIONS`] for an HTTP server unless `connections` says
//! otherwise, one for a child process, which has a single stdin) and a call
//! takes whichever is free, waiting for one when all are busy. The first is
//! dialled when the target comes up, so a target that cannot be reached is
//! reported at startup; the others are dialled as calls need them.
//!
//! # Who shares a pool
//!
//! A target's `mode` decides (726-sandbox-design §4, LEL D102, D223):
//!
//! - `pooled`: one pool for every caller.
//! - `per_credential`: one pool per credential the gateway presents upstream
//!   (the outgoing `authorization` header).
//! - `per_session`, the default: one connection (or child) per downstream
//!   session and principal, so two agents never share a stateful server. It is
//!   closed when the session ends ([`Target::end_session`]) or after
//!   [`SESSION_IDLE`] unused.
//!
//! Only calls, prompt gets and resource reads are keyed. Listings and the
//! startup warm-up have no caller and use the first connection.

use std::collections::HashMap;
use std::io;
use std::process::Command;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use agentgateway_config::{McpTarget, McpTargetKind, UpstreamMode};
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
    identity::{Identity, IdentityError},
};

/// How many connections a target that speaks HTTP may have open at once.
pub const HTTP_CONNECTIONS: usize = 8;

/// How long a `per_session` upstream lives unused before it is closed.
pub const SESSION_IDLE: Duration = Duration::from_secs(15 * 60);

/// How long a call may wait for its answer when the route sets no backend
/// budget.
const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(600);

/// Failure to bring up a target.
#[derive(Debug, thiserror::Error)]
pub enum TargetError {
    /// The target's tool filters did not compile.
    #[error(transparent)]
    Gate(#[from] GateError),

    /// The target's `identity` did not compile.
    #[error(transparent)]
    Identity(#[from] IdentityError),

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

/// The open connections of one pool.
struct Pool {
    state: Mutex<Slots>,
    freed: Condvar,
    dial: Arc<Dial>,
    max: usize,
    /// How many times a connection has been taken.
    leases: std::sync::atomic::AtomicUsize,
}

impl Pool {
    /// A pool holding `first` and allowed `max` connections.
    fn new(dial: Arc<Dial>, max: usize, first: Option<Conn>) -> Self {
        let open = usize::from(first.is_some());
        Pool {
            state: Mutex::new(Slots {
                idle: first.into_iter().collect(),
                open,
                last_used: Instant::now(),
                closed: false,
            }),
            freed: Condvar::new(),
            dial,
            max,
            leases: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

/// Who a keyed operation is for: the downstream session and the principal the
/// verified token names. Built by the federation from the call.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionKey {
    session: String,
    principal: String,
}

impl SessionKey {
    /// The key for a call under `session` by the holder of `claims`.
    ///
    /// `session` is the classic session the MCP server validated for the
    /// request ([`rusty_mcp_server::Caller::session`]), never the raw
    /// `Mcp-Session-Id` header. A stateless (2026-07-28) request has none: it
    /// is keyed on its principal alone, and that upstream closes only when it
    /// sits idle. The principal is `iss` and `sub`, or the whole claims when
    /// there is no `sub`, the same rule session binding uses. Calls with
    /// neither a session nor a token share one key: there is no identity to
    /// keep apart.
    pub fn new(session: Option<&str>, claims: Option<&serde_json::Value>) -> Self {
        let principal = match claims {
            None => String::new(),
            Some(claims) => match (claims.get("iss"), claims.get("sub")) {
                (iss, Some(sub)) => format!("{}\n{sub}", iss.unwrap_or(&serde_json::Value::Null)),
                _ => claims.to_string(),
            },
        };
        SessionKey {
            session: session.unwrap_or_default().to_owned(),
            principal,
        }
    }
}

/// One keyed pool.
struct Keyed {
    pool: Arc<Pool>,
    /// The downstream session it serves, for [`Target::end_session`].
    session: String,
}

impl Keyed {
    /// Unused for longer than `idle` and held by no call. A pool a call still
    /// holds is never closed under it, however long the call runs.
    fn expired(&self, now: Instant, idle: Duration) -> bool {
        Arc::strong_count(&self.pool) == 1
            && now.duration_since(lock(&self.pool.state).last_used) > idle
    }
}

/// The keyed pools of one target, and the sessions that ended lately.
#[derive(Default)]
struct Keys {
    pools: HashMap<Vec<u8>, Keyed>,
    /// Ended sessions, kept for one idle period, so a call admitted just
    /// before its session ended can't open a pool for it afterwards.
    /// Session ids are random and never reused.
    ended: HashMap<String, Instant>,
    /// Whether a reaper thread is sweeping these pools.
    reaping: bool,
}

impl Keys {
    fn forget_old_ends(&mut self, now: Instant, idle: Duration) {
        self.ended.retain(|_, at| now.duration_since(*at) <= idle);
    }
}

/// The pools of one target.
struct Pools {
    mode: UpstreamMode,
    /// The first connection's pool: every operation under `pooled`, and
    /// listings under the other modes.
    shared: Pool,
    by_key: Mutex<Keys>,
    /// Connections per keyed pool.
    per_key: usize,
    idle: Duration,
}

impl Pools {
    /// The pool for an operation by `who` with `headers` going upstream.
    ///
    /// # Errors
    /// `who`'s session has ended: a call admitted before the end opens
    /// nothing after it.
    fn for_call(
        self: &Arc<Self>,
        who: &SessionKey,
        headers: &HeaderOverride,
    ) -> Result<Option<Arc<Pool>>, ClientError> {
        let (key, session) = match self.mode {
            UpstreamMode::Pooled => return Ok(None),
            UpstreamMode::PerCredential => (credential_key(headers), String::new()),
            UpstreamMode::PerSession => (
                format!("{}\u{0}{}", who.session, who.principal).into_bytes(),
                who.session.clone(),
            ),
        };
        let now = Instant::now();
        let mut keys = lock(&self.by_key);
        keys.forget_old_ends(now, self.idle);
        if !session.is_empty() && keys.ended.contains_key(&session) {
            return Err(ClientError::Io(io::Error::new(
                io::ErrorKind::NotConnected,
                "the session has ended",
            )));
        }
        let expired = take(&mut keys.pools, |k| k.expired(now, self.idle));
        let entry = keys.pools.entry(key).or_insert_with(|| Keyed {
            pool: Arc::new(Pool::new(Arc::clone(&self.shared.dial), self.per_key, None)),
            session,
        });
        let pool = Arc::clone(&entry.pool);
        if !keys.reaping {
            keys.reaping = true;
            let pools = Arc::downgrade(self);
            std::thread::spawn(move || reap(&pools));
        }
        drop(keys);
        close(expired);
        Ok(Some(pool))
    }

    /// Close the pools gone idle. Answers whether there is still anything to
    /// sweep; when there is not, the reaper stops until the next keyed pool.
    fn sweep(&self) -> bool {
        let now = Instant::now();
        let mut keys = lock(&self.by_key);
        keys.forget_old_ends(now, self.idle);
        let expired = take(&mut keys.pools, |k| k.expired(now, self.idle));
        let more = !keys.pools.is_empty() || !keys.ended.is_empty();
        keys.reaping = more;
        drop(keys);
        close(expired);
        more
    }

    /// Close every keyed pool serving `session`.
    fn end_session(&self, session: &str) -> usize {
        let now = Instant::now();
        let mut keys = lock(&self.by_key);
        keys.forget_old_ends(now, self.idle);
        let ended = take(&mut keys.pools, |k| {
            !k.session.is_empty() && k.session == session
        });
        if !session.is_empty() {
            keys.ended.insert(session.to_owned(), now);
        }
        drop(keys);
        let count = ended.len();
        close(ended);
        count
    }
}

/// Close `pools` (without the lock: closing a child can take a moment).
/// Callers waiting on one are refused; a call running on one finishes,
/// within its timeout, and its connection is then dropped, not reused.
fn close(pools: Vec<Keyed>) {
    for keyed in pools {
        keyed.pool.close();
    }
}

/// Sweep `pools` for idle upstreams until there are none, or the target is
/// gone, so an upstream nobody calls again still closes.
fn reap(pools: &Weak<Pools>) {
    loop {
        let Some(tick) = pools.upgrade().map(|p| p.idle / 2) else {
            return;
        };
        std::thread::sleep(tick.clamp(Duration::from_millis(10), Duration::from_secs(60)));
        let Some(live) = pools.upgrade() else {
            return;
        };
        if !live.sweep() {
            return;
        }
    }
}

/// The `per_credential` key: the outgoing `authorization` value's exact
/// bytes, so two credentials never share a pool. No header and an empty one
/// are kept apart by the leading tag.
fn credential_key(headers: &HeaderOverride) -> Vec<u8> {
    let value = headers
        .set
        .iter()
        .find(|(name, _)| *name == http::header::AUTHORIZATION)
        .map(|(_, value)| value.as_bytes());
    match value {
        None => vec![0],
        Some(bytes) => [&[1], bytes].concat(),
    }
}

/// Remove and return the entries `pick` chooses.
fn take(map: &mut HashMap<Vec<u8>, Keyed>, pick: impl Fn(&Keyed) -> bool) -> Vec<Keyed> {
    let keys: Vec<Vec<u8>> = map
        .iter()
        .filter(|(_, k)| pick(k))
        .map(|(key, _)| key.clone())
        .collect();
    keys.iter().filter_map(|key| map.remove(key)).collect()
}

struct Slots {
    idle: Vec<Conn>,
    /// Connections that exist: idle, leased, or being dialled.
    open: usize,
    /// When a connection was last taken or given back.
    last_used: Instant,
    /// Closed by teardown: no more leases, and returned connections drop.
    closed: bool,
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
            let mut slots = lock(&self.pool.state);
            slots.last_used = Instant::now();
            if slots.closed {
                slots.open -= 1;
                drop(slots);
                drop(conn);
                return;
            }
            slots.idle.push(conn);
            drop(slots);
            self.pool.freed.notify_one();
        }
    }
}

impl Pool {
    /// Refuse new leases, wake every waiter, and drop the idle connections.
    fn close(&self) {
        let mut slots = lock(&self.state);
        slots.closed = true;
        let idle = std::mem::take(&mut slots.idle);
        slots.open -= idle.len();
        drop(slots);
        self.freed.notify_all();
        drop(idle);
    }

    /// A connection, waiting for one (or dialling one) until `deadline`.
    fn lease(&self, deadline: Instant) -> Result<Lease<'_>, ClientError> {
        self.leases
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut slots = lock(&self.state);
        slots.last_used = Instant::now();
        loop {
            if slots.closed {
                return Err(ClientError::Closed);
            }
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
    /// Who the gateway says it acts for on each call, from the token.
    pub identity: Identity,
    capabilities: ServerCapabilities,
    pools: Arc<Pools>,
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
        let connections = config.connections.unwrap_or(HTTP_CONNECTIONS);
        Self::connect_with(config, over, call_timeout, connections, at)
    }

    /// [`Target::connect`] with at most `connections` open to an HTTP target
    /// per pool (a child process always has one), whatever the config says.
    pub fn connect_with(
        config: &McpTarget,
        over: &Override,
        call_timeout: Option<Duration>,
        connections: usize,
        at: &str,
    ) -> Result<Self, TargetError> {
        let filter = TargetFilter::new(&config.filters, at)?;
        let identity = match &config.identity {
            Some(identity) => Identity::new(identity, at)?,
            None => Identity::default(),
        };
        let http = matches!(config.kind, McpTargetKind::Mcp(_));
        if identity.has_headers() && !http {
            // Dropping them would send the call with no identity at all.
            return Err(IdentityError::Config {
                at: format!("{at}.identity.headers"),
                value: config.name.clone(),
                kind: "target for headers: only an `mcp:` target has any",
            }
            .into());
        }
        let name = config.name.clone();
        let timeout = call_timeout.unwrap_or(DEFAULT_CALL_TIMEOUT);

        let (dial, max): (Arc<Dial>, usize) = match &config.kind {
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
                (Arc::new(dial), 1)
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
                (Arc::new(dial), connections.max(1))
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
            http,
            identity,
            capabilities,
            pools: Arc::new(Pools {
                mode: config.mode(),
                shared: Pool::new(dial, max, Some(first)),
                by_key: Mutex::new(Keys::default()),
                // A stateful server gets one connection per session: two
                // concurrent calls on one session queue, as they would on a
                // client of its own.
                per_key: match config.mode() {
                    UpstreamMode::PerSession => 1,
                    _ => max,
                },
                idle: SESSION_IDLE,
            }),
            timeout,
        })
    }

    /// How long a `per_session` upstream may sit unused before it is closed
    /// (default [`SESSION_IDLE`]). Set it before the target serves a call.
    #[must_use]
    pub fn with_session_idle(mut self, idle: Duration) -> Self {
        // Unshared until the first keyed call starts the reaper.
        match Arc::get_mut(&mut self.pools) {
            Some(pools) => pools.idle = idle,
            None => rusty_log::warn!(
                target = %self.name,
                "session idle set after the target was in use; ignored"
            ),
        }
        self
    }

    /// The downstream session `session` ended: close its upstreams. Answers
    /// how many keyed pools were closed.
    pub fn end_session(&self, session: &str) -> usize {
        self.pools.end_session(session)
    }

    /// How many keyed pools are open now. For metrics and tests.
    pub fn sessions(&self) -> usize {
        lock(&self.pools.by_key).pools.len()
    }

    /// The headers for one request: `headers` (a guardrail's changes to the
    /// upstream HTTP request) if the target speaks HTTP.
    fn native(&self, headers: &HeaderOverride) -> rusty_mcp_client_native::HeaderOverride {
        if self.http {
            return headers.to_native();
        }
        if !headers.is_empty() {
            rusty_log::debug!(
                target = %self.name,
                "a guardrail asked to change headers on a stdio target; there are none"
            );
        }
        rusty_mcp_client_native::HeaderOverride::default()
    }

    /// One request by `who` to the target, within one operation budget, on
    /// the pool the target's mode picks for it.
    fn request(
        &self,
        method: &str,
        params: Option<Value>,
        headers: &HeaderOverride,
        who: &SessionKey,
    ) -> Result<Value, ClientError> {
        let deadline = Instant::now() + self.timeout;
        let keyed = self.pools.for_call(who, headers)?;
        let pool = keyed.as_deref().unwrap_or(&self.pools.shared);
        pool.lease(deadline)?
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
        let mut lease = self.pools.shared.lease(deadline)?;
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
        who: &SessionKey,
    ) -> Result<CallToolResult, ClientError> {
        let answer = self.request("tools/call", Some(params.to_value()), headers, who)?;
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
        let keyed: usize = lock(&self.pools.by_key)
            .pools
            .values()
            .map(|k| k.pool.leases.load(std::sync::atomic::Ordering::Relaxed))
            .sum();
        keyed
            + self
                .pools
                .shared
                .leases
                .load(std::sync::atomic::Ordering::Relaxed)
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
        who: &SessionKey,
    ) -> Result<GetPromptResult, ClientError> {
        let answer = self.request("prompts/get", Some(params.to_value()), headers, who)?;
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
        who: &SessionKey,
    ) -> Result<ReadResourceResult, ClientError> {
        let answer = self.request("resources/read", Some(params.to_value()), headers, who)?;
        Ok(ReadResourceResult::from_value(&answer)?)
    }
}

/// Run the MCP handshake over `link`, which may take `handshake` at most.
fn handshake(link: Link, handshake: Duration, call_timeout: Duration) -> Result<Conn, ClientError> {
    let mut config = ClientConfig::new("rusty-agent-gateway", env!("CARGO_PKG_VERSION"));
    config.call_timeout = call_timeout;
    Client::connect(link, config, NoHandler, handshake)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_authorization(value: Option<&[u8]>) -> HeaderOverride {
        let set = value
            .map(|v| {
                let value = http::HeaderValue::from_bytes(v).expect("header value");
                vec![(http::header::AUTHORIZATION, value)]
            })
            .unwrap_or_default();
        HeaderOverride {
            set,
            remove: Vec::new(),
        }
    }

    #[test]
    fn credential_keys_are_the_exact_bytes() {
        let key = |v: Option<&[u8]>| credential_key(&with_authorization(v));
        assert_ne!(
            key(Some(b"Bearer \xff")),
            key(Some(b"Bearer \xfe")),
            "values that are not UTF-8 stay apart"
        );
        assert_ne!(key(None), key(Some(b"")), "no header is not an empty one");
        assert_eq!(key(Some(b"Bearer a")), key(Some(b"Bearer a")));
    }
}
