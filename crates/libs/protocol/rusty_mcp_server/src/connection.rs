//! One client's conversation with a [`Server`], without any I/O: messages in,
//! messages out. A transport reads a [`Message`], hands it to
//! [`Connection::start`], and writes what comes back; notifications the
//! server originates (progress) go to the [`Notifier`] the connection was
//! built with.
//!
//! Both generations of the protocol are served from the same handlers. A
//! classic client (`initialize`) has its revision remembered per
//! connection; a stateless client (`server/discover`, or no handshake at
//! all) names its revision in every request's `_meta`.

use crate::page::{self, Kind};
use crate::server::Server;
use rusty_json::Value;
use rusty_mcp_proto::lifecycle::{self, DiscoverParams, InitializeParams, InitializeResult};
use rusty_mcp_proto::notify::{self, CancelledParams, ProgressParams};
use rusty_mcp_proto::rpc::params as decode_params;
use rusty_mcp_proto::tool;
use rusty_mcp_proto::{
    CacheScope, CallToolParams, ClientCapabilities, DiscoverResult, ErrorCode, ErrorData,
    Implementation, ListToolsResult, Message, PaginatedParams, Paging, ProtocolVersion, RequestId,
    RequestMeta, ResultType, ServerCapabilities, ToolsCapability, Wire,
};
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// Where the server's own notifications go (a transport's writer).
pub trait Notifier: Send + Sync {
    /// Deliver `message` to the client. Delivery failures are the
    /// transport's to handle; the server carries on.
    fn notify(&self, message: Message);
}

/// A flag a client raises with `notifications/cancelled`. A long-running tool
/// should poll it and stop early; its response is dropped once it is set.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// Whether the client withdrew the request.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    pub(crate) fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// What a tool knows about the call it is serving.
pub struct CallContext {
    request_id: RequestId,
    version: ProtocolVersion,
    client_info: Option<Implementation>,
    client_capabilities: Option<ClientCapabilities>,
    meta: RequestMeta,
    cancel: CancelToken,
    notifier: Arc<dyn Notifier>,
}

impl CallContext {
    /// The id of the `tools/call` request.
    pub fn request_id(&self) -> &RequestId {
        &self.request_id
    }

    /// The protocol revision in force for this call.
    pub fn protocol_version(&self) -> &ProtocolVersion {
        &self.version
    }

    /// Who is calling, from the handshake or this request's `_meta`.
    pub fn client_info(&self) -> Option<&Implementation> {
        self.client_info.as_ref()
    }

    /// What the caller offers.
    pub fn client_capabilities(&self) -> Option<&ClientCapabilities> {
        self.client_capabilities.as_ref()
    }

    /// The request's `_meta`: trace context and vendor keys are in
    /// [`RequestMeta::extra`].
    pub fn meta(&self) -> &RequestMeta {
        &self.meta
    }

    /// Whether the client withdrew the request.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// The cancellation flag, to hand to a helper thread.
    pub fn cancel_token(&self) -> CancelToken {
        self.cancel.clone()
    }

    /// Report progress. Does nothing unless the caller asked for it with a
    /// progress token, or once the request is cancelled. `progress` must
    /// increase with every call; that is the caller's to keep.
    pub fn progress(&self, progress: f64, total: Option<f64>, message: Option<&str>) {
        let Some(token) = &self.meta.progress_token else {
            return;
        };
        if self.cancel.is_cancelled() {
            return;
        }
        self.notifier.notify(Message::notification(
            notify::method::PROGRESS,
            &ProgressParams {
                progress_token: token.clone(),
                progress,
                total,
                message: message.map(String::from),
                meta: None,
            },
        ));
    }
}

#[derive(Default)]
struct State {
    /// The revision `initialize` settled on (classic clients only).
    negotiated: Option<ProtocolVersion>,
    client_info: Option<Implementation>,
    client_capabilities: Option<ClientCapabilities>,
}

/// One client's conversation with a server. Share it with an `Arc`.
pub struct Connection {
    server: Arc<Server>,
    notifier: Arc<dyn Notifier>,
    state: Mutex<State>,
    inflight: Mutex<HashMap<RequestId, CancelToken>>,
}

/// What [`Connection::start`] decided about a message.
pub enum Started {
    /// Nothing more to do; send the reply if there is one.
    Done(Option<Message>),
    /// A request admitted for execution: call [`Job::run`], on this thread or
    /// another, and send the reply it returns.
    Run(Job),
}

/// An admitted request, ready to run on any thread. Dropping it unrun frees
/// its slot.
pub struct Job {
    guard: InflightGuard,
    method: String,
    params: Option<Value>,
    token: CancelToken,
}

/// Holds a request's place in the in-flight table; gives it back on drop,
/// however the job ends (answered, cancelled, panicked, never started).
struct InflightGuard {
    conn: Arc<Connection>,
    id: RequestId,
}

impl Drop for InflightGuard {
    fn drop(&mut self) {
        lock(&self.conn.inflight).remove(&self.id);
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic while holding these locks leaves plain maps and options, which
    // stay valid; carry on rather than take the connection down.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn invalid_params(e: impl std::fmt::Display) -> ErrorData {
    ErrorData::new(ErrorCode::INVALID_PARAMS, e.to_string())
}

impl Connection {
    /// A connection to `server` whose own notifications go to `notifier`.
    pub fn new(server: Arc<Server>, notifier: Arc<dyn Notifier>) -> Self {
        Self {
            server,
            notifier,
            state: Mutex::new(State::default()),
            inflight: Mutex::new(HashMap::new()),
        }
    }

    /// A connection whose protocol revision is already known, for a
    /// transport that learns it out of band (the Streamable HTTP
    /// `MCP-Protocol-Version` header) instead of from an `initialize` on this
    /// very connection. A request that names its own revision in `_meta`
    /// still wins.
    pub fn with_protocol_version(
        server: Arc<Server>,
        notifier: Arc<dyn Notifier>,
        version: Option<ProtocolVersion>,
    ) -> Self {
        let conn = Self::new(server, notifier);
        lock(&conn.state).negotiated = version;
        conn
    }

    /// Admit a message. Cheap and non-blocking: a notification is acted on
    /// here (a cancellation raises its flag at once), `initialize`,
    /// `server/discover` and `ping` are answered here, and any other request
    /// is checked against the in-flight limit and handed back as a [`Job`]. Splitting
    /// this from [`Job::run`] lets a transport read the next message while a
    /// tool is still working.
    pub fn start(self: &Arc<Self>, message: Message) -> Started {
        match message {
            // The handshake and liveness checks are cheap and run no user
            // code, so they are answered here, before the next message is
            // admitted: a client that pipelines `initialize` and `tools/list`
            // sees them take effect in order.
            Message::Request { id, method, params }
                if matches!(
                    method.as_str(),
                    lifecycle::method::INITIALIZE
                        | lifecycle::method::DISCOVER
                        | lifecycle::method::PING
                ) =>
            {
                let outcome = self.dispatch(&id, &method, &params, &CancelToken::default());
                Started::Done(Some(reply(id, outcome)))
            }
            Message::Request { id, method, params } => {
                let mut inflight = lock(&self.inflight);
                if inflight.contains_key(&id) {
                    // Answering under the same id would be mistaken for the
                    // answer to the request already running.
                    return Started::Done(Some(Message::error(
                        None,
                        ErrorData::new(ErrorCode::INVALID_REQUEST, "request id already in flight"),
                    )));
                }
                if inflight.len() >= self.server.max_in_flight {
                    return Started::Done(Some(Message::error(
                        Some(id),
                        ErrorData::new(ErrorCode::INTERNAL_ERROR, "too many requests in flight"),
                    )));
                }
                let token = CancelToken::default();
                inflight.insert(id.clone(), token.clone());
                Started::Run(Job {
                    guard: InflightGuard {
                        conn: Arc::clone(self),
                        id,
                    },
                    method,
                    params,
                    token,
                })
            }
            Message::Notification { method, params } => {
                if method == notify::method::CANCELLED {
                    self.cancel(&params);
                }
                Started::Done(None)
            }
            // This server asks the client nothing, so there is nothing a
            // client response could answer.
            Message::Response { .. } | Message::Error { .. } => Started::Done(None),
        }
    }

    /// [`Connection::start`] then, for a request, run it here.
    pub fn handle(self: &Arc<Self>, message: Message) -> Option<Message> {
        match self.start(message) {
            Started::Done(reply) => reply,
            Started::Run(job) => job.run(),
        }
    }

    /// Raise the cancellation flag of every request in flight. Tools that
    /// poll it stop early; their answers are dropped.
    pub fn cancel_all(&self) {
        for token in lock(&self.inflight).values() {
            token.cancel();
        }
    }

    fn cancel(&self, params: &Option<Value>) {
        let Ok(CancelledParams {
            request_id: Some(id),
            ..
        }) = decode_params(params)
        else {
            return;
        };
        if let Some(token) = lock(&self.inflight).get(&id) {
            token.cancel();
        }
    }

    fn capabilities(&self) -> ServerCapabilities {
        ServerCapabilities {
            tools: (!self.server.tools.is_empty()).then(ToolsCapability::default),
            ..ServerCapabilities::default()
        }
    }

    fn dispatch(
        self: &Arc<Self>,
        id: &RequestId,
        method: &str,
        params: &Option<Value>,
        token: &CancelToken,
    ) -> Result<Value, ErrorData> {
        match method {
            lifecycle::method::INITIALIZE => self.initialize(params),
            lifecycle::method::DISCOVER => self.discover(params),
            lifecycle::method::PING => Ok(Value::object()),
            tool::method::LIST => self.tools_list(params),
            tool::method::CALL => self.tools_call(id, params, token),
            other => Err(ErrorData::new(
                ErrorCode::METHOD_NOT_FOUND,
                format!("unknown method {other:?}"),
            )),
        }
    }

    fn initialize(&self, params: &Option<Value>) -> Result<Value, ErrorData> {
        let p: InitializeParams = decode_params(params).map_err(invalid_params)?;
        let chosen =
            lifecycle::negotiate_classic(&p.protocol_version, &self.server.classic_versions())
                .ok_or_else(|| {
                    ErrorData::new(
                        ErrorCode::UNSUPPORTED_PROTOCOL_VERSION,
                        "this server has no initialize handshake; use server/discover",
                    )
                })?;
        let mut state = lock(&self.state);
        state.negotiated = Some(chosen.clone());
        state.client_info = Some(p.client_info);
        state.client_capabilities = Some(p.capabilities);
        Ok(InitializeResult {
            protocol_version: chosen,
            capabilities: self.capabilities(),
            server_info: self.server.info.clone(),
            instructions: self.server.instructions.clone(),
            meta: None,
        }
        .to_value())
    }

    fn discover(&self, params: &Option<Value>) -> Result<Value, ErrorData> {
        let _: DiscoverParams = decode_params(params).map_err(invalid_params)?;
        let mut result = DiscoverResult::new(self.server.versions.clone(), self.capabilities())
            .with_server_info(&self.server.info);
        result.instructions = self.server.instructions.clone();
        Ok(result.to_value())
    }

    /// The revision in force for a request and its parsed `_meta`: the one
    /// the request names, else the one `initialize` settled on.
    fn version_for(
        &self,
        params: &Option<Value>,
    ) -> Result<(ProtocolVersion, RequestMeta), ErrorData> {
        let meta = match params.as_ref().and_then(|p| p.get("_meta")) {
            Some(m) => RequestMeta::from_value(m).map_err(invalid_params)?,
            None => RequestMeta::new(),
        };
        if let Some(requested) = &meta.protocol_version {
            if !self.server.supports(requested) {
                let mut data = Value::object();
                data.insert("requested", requested.as_str());
                data.insert(
                    "supported",
                    Value::Array(
                        self.server
                            .versions
                            .iter()
                            .map(|v| Value::from(v.as_str()))
                            .collect(),
                    ),
                );
                return Err(ErrorData {
                    code: ErrorCode::UNSUPPORTED_PROTOCOL_VERSION,
                    message: format!(
                        "protocol version {requested:?} is not supported",
                        requested = requested.as_str()
                    ),
                    data: Some(data),
                });
            }
            return Ok((requested.clone(), meta));
        }
        match lock(&self.state).negotiated.clone() {
            Some(version) => Ok((version, meta)),
            None => Err(ErrorData::new(
                ErrorCode::INVALID_REQUEST,
                "no protocol version: send initialize first, or name one in _meta",
            )),
        }
    }

    fn tools_list(&self, params: &Option<Value>) -> Result<Value, ErrorData> {
        let tools = &self.server.tools;
        if tools.is_empty() {
            return Err(ErrorData::new(
                ErrorCode::METHOD_NOT_FOUND,
                "this server has no tools",
            ));
        }
        let (version, _) = self.version_for(params)?;
        let p: PaginatedParams = decode_params(params).map_err(invalid_params)?;
        let start = match &p.cursor {
            None => 0,
            Some(c) => page::decode(Kind::Tools, c, tools.len())
                .ok_or_else(|| invalid_params("invalid cursor"))?,
        };
        let end = (start + self.server.page_size).min(tools.len());
        let mut paging = Paging {
            next_cursor: (end < tools.len()).then(|| page::encode(Kind::Tools, end)),
            ..Paging::default()
        };
        if version.is_stateless() {
            paging.ttl_ms = Some(0);
            paging.cache_scope = Some(CacheScope::Public);
        }
        Ok(ListToolsResult {
            tools: tools[start..end].iter().map(|t| t.tool.clone()).collect(),
            paging,
        }
        .to_value())
    }

    fn tools_call(
        &self,
        id: &RequestId,
        params: &Option<Value>,
        token: &CancelToken,
    ) -> Result<Value, ErrorData> {
        let (version, meta) = self.version_for(params)?;
        let call: CallToolParams = decode_params(params).map_err(invalid_params)?;
        let entry = self
            .server
            .tools
            .iter()
            .find(|t| t.tool.name == call.name)
            .ok_or_else(|| invalid_params(format!("unknown tool {:?}", call.name)))?;
        let (client_info, client_capabilities) = {
            let state = lock(&self.state);
            (
                meta.client_info
                    .clone()
                    .or_else(|| state.client_info.clone()),
                meta.client_capabilities
                    .clone()
                    .or_else(|| state.client_capabilities.clone()),
            )
        };
        let ctx = CallContext {
            request_id: id.clone(),
            version: version.clone(),
            client_info,
            client_capabilities,
            meta,
            cancel: token.clone(),
            notifier: Arc::clone(&self.notifier),
        };
        let mut result = (entry.handler)(&ctx, call)?;
        if version.is_stateless() && result.result_type.is_none() {
            result.result_type = Some(ResultType(ResultType::COMPLETE.to_owned()));
        }
        Ok(result.to_value())
    }
}

impl Job {
    /// The id of the request this job answers.
    pub fn id(&self) -> &RequestId {
        &self.guard.id
    }

    /// The job's cancellation flag, for a transport that cancels a request
    /// when its client goes away.
    pub fn cancel_token(&self) -> CancelToken {
        self.token.clone()
    }

    /// Run the request to completion and return the reply to send, or `None`
    /// when the client cancelled it meanwhile (a cancelled request gets no
    /// answer). A panicking tool becomes an internal error rather than a
    /// request that never answers.
    pub fn run(self) -> Option<Message> {
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            self.guard
                .conn
                .dispatch(&self.guard.id, &self.method, &self.params, &self.token)
        }))
        .unwrap_or_else(|_| Err(ErrorData::new(ErrorCode::INTERNAL_ERROR, "internal error")));
        let cancelled = self.token.is_cancelled();
        let id = self.guard.id.clone();
        // Free the slot before the reply is sent, so the client may reuse
        // the id as soon as it has the answer.
        drop(self);
        (!cancelled).then(|| reply(id, outcome))
    }
}

fn reply(id: RequestId, outcome: Result<Value, ErrorData>) -> Message {
    match outcome {
        Ok(result) => Message::Response { id, result },
        Err(error) => Message::error(Some(id), error),
    }
}
