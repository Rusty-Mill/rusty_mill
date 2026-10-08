//! The Streamable HTTP transport on `rusty_serve`, stateless: no session
//! ids, nothing kept between POSTs. Every `POST` carries one JSON-RPC
//! message and is served by a connection of its own, so any number of
//! server instances can sit behind a plain load balancer.
//!
//! Which protocol revision a request speaks comes from, in order: its
//! `_meta` (2026-07-28), the `MCP-Protocol-Version` header (2025-06-18 and
//! later, so classic clients that `initialize` once and then send the header
//! work without sessions), and `2025-03-26` when neither is present.
//!
//! A reply is plain JSON when the tool answers within
//! [`HttpConfig::sse_after`] and sent no progress; otherwise it is a
//! `text/event-stream` carrying the progress notifications and the final
//! response, with a keep-alive comment every [`HttpConfig::keep_alive`]. A
//! client that hangs up mid-stream cancels the request, which is the only
//! cancellation available: without sessions a later `notifications/cancelled`
//! POST cannot be matched to a request, and is acknowledged and ignored.
//!
//! `GET` (a server-push stream) and `DELETE` (session end) are answered
//! `405`: there are no sessions and nothing to push.

use crate::connection::{CancelToken, Connection, Notifier, Started};
use crate::server::Server;
use rusty_base64::decode_standard;
use rusty_http::{Method, StatusCode};
use rusty_json::Value;
use rusty_mcp_proto::{ErrorCode, ErrorData, Message, ProtocolVersion, RequestMeta, Wire};
use rusty_serve::{HeaderMap, Limits, Request, Response, SharedHandler};
use std::collections::VecDeque;
use std::io;
use std::net::SocketAddr;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;

/// The revision assumed when a request names none, per the 2025-06-18 spec.
const FALLBACK_VERSION: &str = ProtocolVersion::V_2025_03_26;

/// The `Mcp-Name` sentinel for a value that is not safe as a header.
const BASE64_PREFIX: &str = "=?base64?";
const BASE64_SUFFIX: &str = "?=";

/// What the transport accepts and how it answers.
#[derive(Clone, Debug)]
pub struct HttpConfig {
    /// The one path that serves MCP. Default `/mcp`.
    pub path: String,
    /// `Host` values accepted, with or without a port; empty accepts any.
    /// The default is loopback only, which with the `Origin` rule below is
    /// what keeps a web page from reaching a local server (DNS rebinding).
    pub allowed_hosts: Vec<String>,
    /// `Origin` values accepted. A request with no `Origin` (every non-browser
    /// client) is accepted; one with an `Origin` that is not listed here is
    /// refused, so the default of none refuses all browsers.
    pub allowed_origins: Vec<String>,
    /// How long a call may run before its reply switches from plain JSON to
    /// an event stream. Default 250 ms.
    pub sse_after: Duration,
    /// Interval of the keep-alive comment on an event stream, which is also
    /// how a vanished client is noticed. Default 15 s.
    pub keep_alive: Duration,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            path: "/mcp".to_owned(),
            allowed_hosts: ["localhost", "127.0.0.1", "::1"].map(String::from).to_vec(),
            allowed_origins: Vec::new(),
            sse_after: Duration::from_millis(250),
            keep_alive: Duration::from_secs(15),
        }
    }
}

/// Bind `server` to `addr` over Streamable HTTP; run the result with
/// `.run()`. Each open event stream and each running call holds a
/// connection thread, so size `limits.max_connections` for the concurrency
/// wanted. TLS is not terminated here: front it with a proxy for anything
/// beyond loopback.
///
/// # Errors
/// The bind error, if the address is taken or not allowed.
pub fn bind_http(
    server: Arc<Server>,
    addr: SocketAddr,
    config: HttpConfig,
    limits: Limits,
) -> io::Result<rusty_serve::Server> {
    Ok(
        rusty_serve::Server::bind_shared(addr, HttpHandler::new(server, config))?
            .with_limits(limits),
    )
}

/// The HTTP face of a [`Server`], as a `rusty_serve` handler. Use it to mount
/// MCP in a server that also serves other routes; [`bind_http`] covers the
/// usual case.
pub struct HttpHandler {
    server: Arc<Server>,
    config: HttpConfig,
}

impl HttpHandler {
    /// Serve `server` per `config`.
    pub fn new(server: Arc<Server>, config: HttpConfig) -> Self {
        Self { server, config }
    }
}

impl SharedHandler for HttpHandler {
    fn handle(&self, request: &Request<'_>) -> Response {
        self.respond(request).unwrap_or_else(|rejection| rejection)
    }
}

/// A plain-JSON reply under a status.
fn json_reply(status: StatusCode, message: &Message) -> Response {
    Response::json(status, message.to_json().into_bytes())
}

/// An HTTP-level refusal, shaped as a JSON-RPC error with no id.
fn reject(status: StatusCode, code: ErrorCode, why: impl Into<String>) -> Response {
    json_reply(status, &Message::error(None, ErrorData::new(code, why)))
}

fn reject_for(
    id: Option<rusty_mcp_proto::RequestId>,
    status: StatusCode,
    code: ErrorCode,
    why: impl Into<String>,
) -> Response {
    json_reply(status, &Message::error(id, ErrorData::new(code, why)))
}

/// `host` or `[v6]` plus an optional `:port`, reduced to the bare host.
fn host_only(host: &str) -> &str {
    if let Some(rest) = host.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest);
    }
    match host.rsplit_once(':') {
        Some((name, port)) if port.bytes().all(|b| b.is_ascii_digit()) && !name.contains(':') => {
            name
        }
        _ => host,
    }
}

fn host_allowed(host: &str, allowed: &[String]) -> bool {
    allowed.is_empty()
        || allowed
            .iter()
            .any(|a| a.eq_ignore_ascii_case(host) || a.eq_ignore_ascii_case(host_only(host)))
}

/// An `Mcp-Name` value, undoing the `=?base64?...?=` wrapper.
fn decode_header_value(raw: &str) -> Option<String> {
    match raw
        .strip_prefix(BASE64_PREFIX)
        .and_then(|r| r.strip_suffix(BASE64_SUFFIX))
    {
        Some(encoded) => String::from_utf8(decode_standard(encoded).ok()?).ok(),
        None => Some(raw.to_owned()),
    }
}

/// The value `Mcp-Name` must carry for `method`, from its parameters.
fn expected_name(method: &str, params: Option<&Value>) -> Option<String> {
    let key = match method {
        "tools/call" | "prompts/get" => "name",
        "resources/read" | "resources/subscribe" | "resources/unsubscribe" => "uri",
        "tasks/get" | "tasks/update" | "tasks/cancel" => "taskId",
        _ => return None,
    };
    params?.get(key)?.as_str().map(str::to_owned)
}

/// 2026-07-28 requires `Mcp-Method` (and `Mcp-Name` where the method has
/// one) to agree with the body, so a proxy can route without parsing it.
fn check_standard_headers(
    headers: &HeaderMap,
    method: &str,
    params: Option<&Value>,
) -> Result<(), String> {
    match headers.get("mcp-method") {
        None => return Err("missing required Mcp-Method header".to_owned()),
        Some(h) if h != method => {
            return Err(format!(
                "Mcp-Method header `{h}` does not match body method `{method}`"
            ));
        }
        Some(_) => {}
    }
    let Some(expected) = expected_name(method, params) else {
        return Ok(());
    };
    let raw = headers
        .get("mcp-name")
        .ok_or_else(|| format!("missing required Mcp-Name header for `{method}`"))?;
    let decoded = decode_header_value(raw).ok_or("Mcp-Name header is not valid Base64")?;
    if decoded == expected {
        Ok(())
    } else {
        Err(format!(
            "Mcp-Name header `{decoded}` does not match body value `{expected}`"
        ))
    }
}

/// Modern (stateless) requests get HTTP statuses that mirror the error;
/// classic ones keep HTTP 200 for every JSON-RPC answer.
fn status_of(modern: bool, reply: &Message) -> StatusCode {
    let Message::Error { error, .. } = reply else {
        return StatusCode::OK;
    };
    if !modern {
        return StatusCode::OK;
    }
    match error.code {
        ErrorCode::UNSUPPORTED_PROTOCOL_VERSION
        | ErrorCode::MISSING_REQUIRED_CLIENT_CAPABILITY
        | ErrorCode::INVALID_PARAMS => StatusCode::BAD_REQUEST,
        ErrorCode::METHOD_NOT_FOUND => StatusCode::NOT_FOUND,
        _ => StatusCode::OK,
    }
}

/// What the worker thread reports back to the thread holding the socket.
enum Event {
    Note(Message),
    Done(Option<Message>),
}

struct EventNotifier(Sender<Event>);

impl Notifier for EventNotifier {
    fn notify(&self, message: Message) {
        // The receiver is gone once the response ended; nothing to tell.
        let _ = self.0.send(Event::Note(message));
    }
}

fn frame(message: &Message) -> Vec<u8> {
    format!("event: message\ndata: {}\n\n", message.to_json()).into_bytes()
}

/// The body of an event-stream reply. Ends after the final response, or
/// when the worker has nothing more to say; cancels the request if it is
/// dropped first, which is what a client hanging up looks like.
struct SseStream {
    ready: VecDeque<Vec<u8>>,
    events: Receiver<Event>,
    keep_alive: Duration,
    cancel: CancelToken,
    finished: bool,
}

impl Iterator for SseStream {
    type Item = Vec<u8>;

    fn next(&mut self) -> Option<Vec<u8>> {
        if let Some(chunk) = self.ready.pop_front() {
            return Some(chunk);
        }
        if self.finished {
            return None;
        }
        match self.events.recv_timeout(self.keep_alive) {
            Ok(Event::Note(m)) => Some(frame(&m)),
            Ok(Event::Done(Some(m))) => {
                self.finished = true;
                Some(frame(&m))
            }
            Ok(Event::Done(None)) | Err(RecvTimeoutError::Disconnected) => {
                self.finished = true;
                None
            }
            Err(RecvTimeoutError::Timeout) => Some(b": ping\n\n".to_vec()),
        }
    }
}

impl Drop for SseStream {
    fn drop(&mut self) {
        if !self.finished {
            self.cancel.cancel();
        }
    }
}

impl HttpHandler {
    fn respond(&self, request: &Request<'_>) -> Result<Response, Response> {
        let path = request.target.split('?').next().unwrap_or("");
        if path != self.config.path {
            return Err(reject(
                StatusCode::NOT_FOUND,
                ErrorCode::INVALID_REQUEST,
                "not found",
            ));
        }
        if request.method != &Method::Post {
            let mut refusal = reject(
                StatusCode::METHOD_NOT_ALLOWED,
                ErrorCode::INVALID_REQUEST,
                "this server accepts POST only",
            );
            let _ = refusal.headers.insert("Allow", "POST");
            return Err(refusal);
        }
        self.check_origin(request.headers)?;
        let headers = request.headers;
        let accept = headers.get("accept").unwrap_or("");
        if !(accept.contains("application/json") && accept.contains("text/event-stream")) {
            return Err(reject(
                StatusCode::NOT_ACCEPTABLE,
                ErrorCode::INVALID_REQUEST,
                "the client must accept both application/json and text/event-stream",
            ));
        }
        if !headers
            .get("content-type")
            .is_some_and(|c| c.starts_with("application/json"))
        {
            return Err(reject(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                ErrorCode::INVALID_REQUEST,
                "Content-Type must be application/json",
            ));
        }
        let header_version = self.header_version(headers)?;
        let message = parse_body(request.body)?;
        let modern = self.check_version_rules(headers, header_version.as_ref(), &message)?;
        Ok(self.dispatch(message, header_version, modern))
    }

    fn check_origin(&self, headers: &HeaderMap) -> Result<(), Response> {
        let forbid = |why: &str| {
            Err(reject(
                StatusCode::FORBIDDEN,
                ErrorCode::INVALID_REQUEST,
                why,
            ))
        };
        if !host_allowed(
            headers.get("host").unwrap_or(""),
            &self.config.allowed_hosts,
        ) {
            return forbid("Host header is not allowed");
        }
        match headers.get("origin") {
            Some(origin)
                if !self
                    .config
                    .allowed_origins
                    .iter()
                    .any(|o| o.eq_ignore_ascii_case(origin)) =>
            {
                forbid("Origin header is not allowed")
            }
            _ => Ok(()),
        }
    }

    /// The `MCP-Protocol-Version` header, refused when it names a revision
    /// this server does not speak.
    fn header_version(&self, headers: &HeaderMap) -> Result<Option<ProtocolVersion>, Response> {
        let Some(raw) = headers.get("mcp-protocol-version") else {
            return Ok(None);
        };
        let version = ProtocolVersion::new(raw);
        if self.server.supports(&version) {
            Ok(Some(version))
        } else {
            Err(reject(
                StatusCode::BAD_REQUEST,
                ErrorCode::UNSUPPORTED_PROTOCOL_VERSION,
                format!("unsupported MCP-Protocol-Version: {raw}"),
            ))
        }
    }

    /// The cross-checks between header and body. Returns whether the
    /// request is a modern (2026-07-28) one, which decides how errors map
    /// to HTTP statuses.
    fn check_version_rules(
        &self,
        headers: &HeaderMap,
        header: Option<&ProtocolVersion>,
        message: &Message,
    ) -> Result<bool, Response> {
        let (id, method, params) = match message {
            Message::Request { id, method, params } => (Some(id.clone()), method, params),
            Message::Notification { method, params } => (None, method, params),
            Message::Response { .. } | Message::Error { .. } => return Ok(false),
        };
        let mismatch = |why: String| {
            Err(reject_for(
                id.clone(),
                StatusCode::BAD_REQUEST,
                ErrorCode::HEADER_MISMATCH,
                why,
            ))
        };
        if method == "initialize" {
            // The one request that names its version in the body, not `_meta`.
            let body_version = params
                .as_ref()
                .and_then(|p| p.get("protocolVersion"))
                .and_then(Value::as_str);
            return match (header, body_version) {
                (Some(h), Some(b)) if h.as_str() != b => Err(reject_for(
                    id,
                    StatusCode::BAD_REQUEST,
                    ErrorCode::INVALID_REQUEST,
                    format!("MCP-Protocol-Version header ({h}) does not match params.protocolVersion ({b})", h = h.as_str()),
                )),
                _ => Ok(false),
            };
        }
        let meta = match params.as_ref().and_then(|p| p.get("_meta")) {
            Some(m) => RequestMeta::from_value(m).map_err(|e| {
                reject_for(
                    id.clone(),
                    StatusCode::BAD_REQUEST,
                    ErrorCode::INVALID_PARAMS,
                    e.to_string(),
                )
            })?,
            None => RequestMeta::new(),
        };
        let modern = match (&meta.protocol_version, header) {
            (Some(m), Some(h)) if m != h => {
                return mismatch(format!(
                    "MCP-Protocol-Version header ({}) does not match request _meta protocolVersion ({})",
                    h.as_str(),
                    m.as_str()
                ));
            }
            (Some(_), None) => {
                return mismatch(
                    "request _meta protocolVersion requires the MCP-Protocol-Version header"
                        .to_owned(),
                );
            }
            (Some(m), Some(_)) => m.is_stateless(),
            (None, Some(h)) if h.is_stateless() => {
                return Err(reject_for(
                    id,
                    StatusCode::BAD_REQUEST,
                    ErrorCode::INVALID_PARAMS,
                    "request _meta must carry io.modelcontextprotocol/protocolVersion",
                ));
            }
            (None, _) => false,
        };
        if modern {
            if let Err(why) = check_standard_headers(headers, method, params.as_ref()) {
                return mismatch(why);
            }
        }
        Ok(modern)
    }

    fn dispatch(
        &self,
        message: Message,
        header_version: Option<ProtocolVersion>,
        modern: bool,
    ) -> Response {
        // What a request without `_meta` is taken to speak: its header, else
        // the oldest revision that has no header.
        let assumed = header_version.or_else(|| {
            let fallback = ProtocolVersion::new(FALLBACK_VERSION);
            self.server.supports(&fallback).then_some(fallback)
        });
        let (tx, events) = channel();
        let conn = Arc::new(Connection::with_protocol_version(
            Arc::clone(&self.server),
            Arc::new(EventNotifier(tx.clone())),
            assumed,
        ));
        match conn.start(message) {
            Started::Done(None) => Response::json(StatusCode::ACCEPTED, Vec::new()),
            Started::Done(Some(reply)) => json_reply(status_of(modern, &reply), &reply),
            Started::Run(job) => {
                let cancel = job.cancel_token();
                let worker = std::thread::Builder::new().spawn(move || {
                    // A send fails only when the client is gone.
                    let _ = tx.send(Event::Done(job.run()));
                });
                if worker.is_err() {
                    return reject(
                        StatusCode::SERVICE_UNAVAILABLE,
                        ErrorCode::INTERNAL_ERROR,
                        "could not start a worker thread",
                    );
                }
                self.await_reply(events, cancel, modern)
            }
        }
    }

    /// Wait briefly: a quick, silent call is answered in plain JSON; a call
    /// that reports progress, or that is still running when the wait ends,
    /// becomes an event stream.
    fn await_reply(&self, events: Receiver<Event>, cancel: CancelToken, modern: bool) -> Response {
        let wait = self.config.sse_after;
        match events.recv_timeout(wait) {
            Ok(Event::Done(None)) => Response::json(StatusCode::ACCEPTED, Vec::new()),
            Ok(Event::Done(Some(reply))) => json_reply(status_of(modern, &reply), &reply),
            Ok(Event::Note(note)) => self.stream(VecDeque::from([frame(&note)]), events, cancel),
            Err(RecvTimeoutError::Timeout) => self.stream(VecDeque::new(), events, cancel),
            Err(RecvTimeoutError::Disconnected) => reject(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::INTERNAL_ERROR,
                "the request ended without an answer",
            ),
        }
    }

    fn stream(
        &self,
        ready: VecDeque<Vec<u8>>,
        events: Receiver<Event>,
        cancel: CancelToken,
    ) -> Response {
        Response::stream(
            "text/event-stream",
            SseStream {
                ready,
                events,
                keep_alive: self.config.keep_alive,
                cancel,
                finished: false,
            },
        )
    }
}

/// The one JSON-RPC message in a POST body, or the refusal to send.
fn parse_body(body: &[u8]) -> Result<Message, Response> {
    let text = std::str::from_utf8(body)
        .map_err(|_| reject(StatusCode::BAD_REQUEST, ErrorCode::PARSE_ERROR, "not UTF-8"))?;
    Message::from_json(text).map_err(|e| match e {
        rusty_mcp_proto::Error::Json(_) => {
            reject(StatusCode::BAD_REQUEST, ErrorCode::PARSE_ERROR, "not JSON")
        }
        rusty_mcp_proto::Error::Decode { .. } => reject(
            StatusCode::BAD_REQUEST,
            ErrorCode::INVALID_REQUEST,
            e.to_string(),
        ),
    })
}
