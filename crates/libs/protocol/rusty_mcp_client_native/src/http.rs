//! Streamable HTTP, client side, on `rusty_request`.
//!
//! The transport keeps a small private `rusty_tokio` runtime on its own
//! threads, so the blocking [`crate::Client`] works unchanged: `send` spawns a
//! task that POSTs the message, and whatever comes back (a JSON body, or an
//! event stream) is queued for `recv`.
//!
//! - **Revision headers.** After the handshake every request names the
//!   revision (`MCP-Protocol-Version`); on a stateless revision it also
//!   carries `Mcp-Method` and, for the methods that have one, `Mcp-Name`
//!   (base64-wrapped when it is not header-safe).
//! - **Sessions.** A server that issues `Mcp-Session-Id` is echoed; `DELETE`
//!   ends the session when the transport is dropped. On a classic session the
//!   transport also opens the standalone `GET` stream by itself (when
//!   [`HttpConfig::push_stream`] is on), reconnecting with `Last-Event-ID`
//!   until the server answers `405`/`404` or the transport is dropped.
//! - **Cancellation.** Cancelling a request aborts its POST, which closes the
//!   connection (that is how a stateless server learns of it); a classic
//!   session also gets `notifications/cancelled`.
//! - **Failures.** A POST that fails below JSON-RPC (connection refused, a
//!   non-JSON error page) is delivered as a JSON-RPC error for its request, so
//!   a waiting call fails at once instead of timing out. A *notification*
//!   whose POST fails has no one waiting; it is counted in
//!   [`HttpTransport::failed_notifications`].
//!
//! Not done: resuming an interrupted POST event stream (a stream cut before
//! its response leaves the call to time out), and `subscriptions/listen` as a
//! long-lived call (the blocking client has one call at a time).

use crate::sse::SseParser;
use crate::transport::{Recv, Transport};
use rusty_json::Value;
use rusty_mcp_proto::{ErrorCode, ErrorData, Message, ProtocolVersion, RequestId, Wire};
use rusty_request::{Client, StatusCode};
use rusty_tokio::task::JoinHandle;
use rusty_tokio::Runtime;
use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// Largest error body read.
const MAX_ERROR_BODY: usize = 1024 * 1024;

/// Where and how to connect.
#[derive(Clone, Debug)]
pub struct HttpConfig {
    /// The MCP endpoint, for example `http://127.0.0.1:8080/mcp`.
    pub url: String,
    /// Sent as `Authorization: Bearer ...` on every request.
    pub bearer_token: Option<String>,
    /// More headers for every request.
    pub headers: Vec<(String, String)>,
    /// Open the standalone `GET` stream on a classic session. Default on.
    pub push_stream: bool,
    /// Wait before reconnecting the push stream when the server gave no
    /// `retry`. Default one second.
    pub reconnect_delay: Duration,
}

impl HttpConfig {
    /// Defaults for `url`.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            bearer_token: None,
            headers: Vec::new(),
            push_stream: true,
            reconnect_delay: Duration::from_secs(1),
        }
    }
}

enum Event {
    Message(Message),
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Default)]
struct Shared {
    session_id: Mutex<Option<String>>,
    version: Mutex<Option<ProtocolVersion>>,
    closed: AtomicBool,
    push_started: AtomicBool,
    failed_notifications: AtomicUsize,
}

impl Shared {
    fn stateless(&self) -> bool {
        lock(&self.version)
            .as_ref()
            .is_some_and(ProtocolVersion::is_stateless)
    }
}

/// The transport.
pub struct HttpTransport {
    runtime: Runtime,
    client: Client,
    config: Arc<HttpConfig>,
    shared: Arc<Shared>,
    tx: Sender<Event>,
    inbox: Receiver<Event>,
    in_flight: HashMap<RequestId, JoinHandle<()>>,
}

fn build_client(config: &HttpConfig) -> io::Result<Client> {
    // No total timeout: a reply may be an event stream that stays open for as
    // long as the call runs. The client's own call timeout bounds the wait.
    let mut builder = Client::builder().no_timeout();
    if let Some(token) = &config.bearer_token {
        builder = builder
            .bearer_auth(token)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
    }
    for (name, value) in &config.headers {
        builder = builder
            .default_header(name, value)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
    }
    Ok(builder.build())
}

impl HttpTransport {
    /// A transport for `config`. Nothing is sent until the first message.
    ///
    /// # Errors
    /// The runtime could not start, or a header in the config is invalid.
    pub fn new(config: HttpConfig) -> io::Result<Self> {
        let (tx, inbox) = channel();
        Ok(Self {
            runtime: Runtime::new()?,
            client: build_client(&config)?,
            config: Arc::new(config),
            shared: Arc::new(Shared::default()),
            tx,
            inbox,
            in_flight: HashMap::new(),
        })
    }

    /// The session id the server issued, if any.
    pub fn session_id(&self) -> Option<String> {
        lock(&self.shared.session_id).clone()
    }

    /// Notifications whose POST failed (nobody was waiting to be told).
    pub fn failed_notifications(&self) -> usize {
        self.shared.failed_notifications.load(Ordering::SeqCst)
    }
}

/// `Mcp-Name` for `method`, from its parameters.
fn route_name(method: &str, params: Option<&Value>) -> Option<String> {
    let key = match method {
        "tools/call" | "prompts/get" => "name",
        "resources/read" | "resources/subscribe" | "resources/unsubscribe" => "uri",
        "tasks/get" | "tasks/update" | "tasks/cancel" => "taskId",
        _ => return None,
    };
    params?.get(key)?.as_str().map(str::to_owned)
}

/// A header value as the server expects it: plain when it is visible ASCII,
/// otherwise `=?base64?...?=`.
fn header_safe(value: &str) -> String {
    let plain = !value.is_empty()
        && value.trim() == value
        && value.bytes().all(|b| (0x20..=0x7e).contains(&b));
    if plain {
        value.to_owned()
    } else {
        format!(
            "=?base64?{}?=",
            rusty_base64::encode_standard(value.as_bytes())
        )
    }
}

fn cancelled_id(message: &Message) -> Option<RequestId> {
    let Message::Notification { method, params } = message else {
        return None;
    };
    if method != "notifications/cancelled" {
        return None;
    }
    rusty_mcp_proto::CancelledParams::from_value(params.as_ref()?)
        .ok()?
        .request_id
}

fn failure(id: Option<&RequestId>, code: ErrorCode, why: String) -> Option<Message> {
    id.map(|id| Message::error(Some(id.clone()), ErrorData::new(code, why)))
}

/// Deliver every message in `text` (one message, or a batch).
fn deliver(tx: &Sender<Event>, text: &str) {
    let Ok(value) = Value::from_json_str(text) else {
        return;
    };
    let items = match value.as_array() {
        Some(items) => items.to_vec(),
        None => vec![value],
    };
    for item in items {
        if let Ok(message) = Message::from_value(&item) {
            let _ = tx.send(Event::Message(message));
        }
    }
}

async fn read_all(resp: &mut rusty_request::StreamingResponse, limit: usize) -> Vec<u8> {
    let mut body = Vec::new();
    while let Ok(Some(chunk)) = resp.chunk().await {
        body.extend_from_slice(&chunk);
        if body.len() > limit {
            break;
        }
    }
    body
}

/// POST `message` and deliver whatever comes back.
async fn post(
    client: Client,
    config: Arc<HttpConfig>,
    shared: Arc<Shared>,
    tx: Sender<Event>,
    message: Message,
) {
    let request_id = match &message {
        Message::Request { id, .. } => Some(id.clone()),
        _ => None,
    };
    let fail_with = |code: ErrorCode, why: String| match failure(request_id.as_ref(), code, why) {
        Some(m) => {
            let _ = tx.send(Event::Message(m));
        }
        None => {
            shared.failed_notifications.fetch_add(1, Ordering::SeqCst);
        }
    };
    let fail = |why: String| fail_with(ErrorCode::INTERNAL_ERROR, why);
    let is_discover =
        matches!(&message, Message::Request { method, .. } if method == "server/discover");
    let mut req = match client.post(&config.url) {
        Ok(r) => r,
        Err(e) => return fail(format!("bad url: {e}")),
    };
    let mut headers: Vec<(String, String)> = vec![
        ("Content-Type".into(), "application/json".into()),
        (
            "Accept".into(),
            "application/json, text/event-stream".into(),
        ),
    ];
    let version = lock(&shared.version).clone();
    if let Some(v) = &version {
        headers.push(("MCP-Protocol-Version".into(), v.as_str().to_owned()));
        if v.is_stateless() {
            if let Message::Request { method, params, .. }
            | Message::Notification { method, params } = &message
            {
                headers.push(("Mcp-Method".into(), method.clone()));
                if let Some(name) = route_name(method, params.as_ref()) {
                    headers.push(("Mcp-Name".into(), header_safe(&name)));
                }
            }
        }
    }
    if let Some(id) = lock(&shared.session_id).clone() {
        headers.push(("Mcp-Session-Id".into(), id));
    }
    for (k, v) in &headers {
        req = match req.header(k, v) {
            Ok(r) => r,
            Err(e) => return fail(format!("bad header {k}: {e}")),
        };
    }
    let mut resp = match req.body(message.to_json()).send_streaming().await {
        Ok(r) => r,
        Err(e) => return fail(format!("request failed: {e}")),
    };
    if let Some(sid) = resp.headers().get("mcp-session-id") {
        let first = lock(&shared.session_id).replace(sid.to_owned()).is_none();
        if first && config.push_stream {
            start_push(&client, &config, &shared, &tx);
        }
    }
    let status = resp.status();
    if status == StatusCode::ACCEPTED {
        return;
    }
    let is_stream = resp
        .headers()
        .get("content-type")
        .is_some_and(|c| c.starts_with("text/event-stream"));
    if !status.is_success() {
        let body = read_all(&mut resp, MAX_ERROR_BODY).await;
        let text = String::from_utf8_lossy(&body);
        // A JSON-RPC error body is the server's real answer; use it.
        if let Ok(value) = Value::from_json_str(&text) {
            if let Ok(m @ Message::Error { .. }) = Message::from_value(&value) {
                let m = match (m, request_id.as_ref()) {
                    (Message::Error { id: None, error }, Some(id)) => {
                        Message::error(Some(id.clone()), error)
                    }
                    (m, _) => m,
                };
                let _ = tx.send(Event::Message(m));
                return;
            }
        }
        let hint = if status.as_u16() == 404 && lock(&shared.session_id).is_some() {
            " (the session ended; reconnect)"
        } else {
            ""
        };
        let snippet: String = text.trim().chars().take(200).collect();
        let detail = if snippet.is_empty() {
            String::new()
        } else {
            format!(": {snippet}")
        };
        // A classic server may reject `server/discover` at the HTTP level (the
        // `rmcp` server answers 422) instead of with a JSON-RPC error. That
        // means "I do not speak this": report it as method-not-found so the
        // client falls back to `initialize`.
        let code = if is_discover && (400..500).contains(&status.as_u16()) {
            ErrorCode::METHOD_NOT_FOUND
        } else {
            ErrorCode::INTERNAL_ERROR
        };
        return fail_with(code, format!("HTTP {}{hint}{detail}", status.as_u16()));
    }
    if is_stream {
        let mut parser = SseParser::new();
        loop {
            match resp.chunk().await {
                Ok(Some(chunk)) => {
                    for event in parser.feed(&chunk) {
                        if event.event.as_deref().is_none_or(|e| e == "message") {
                            deliver(&tx, &event.data);
                        }
                    }
                }
                Ok(None) => return,
                Err(e) => return fail(format!("stream broke: {e}")),
            }
        }
    }
    let body = read_all(&mut resp, usize::MAX).await;
    deliver(&tx, &String::from_utf8_lossy(&body));
}

/// Start the standalone stream once, in the background.
fn start_push(client: &Client, config: &Arc<HttpConfig>, shared: &Arc<Shared>, tx: &Sender<Event>) {
    if shared.push_started.swap(true, Ordering::SeqCst) {
        return;
    }
    let (client, config, shared, tx) = (
        client.clone(),
        Arc::clone(config),
        Arc::clone(shared),
        tx.clone(),
    );
    rusty_tokio::spawn(push_loop(client, config, shared, tx));
}

async fn push_loop(
    client: Client,
    config: Arc<HttpConfig>,
    shared: Arc<Shared>,
    tx: Sender<Event>,
) {
    let mut parser = SseParser::new();
    // The revision header is set once the handshake finishes; wait for it.
    for _ in 0..500 {
        if lock(&shared.version).is_some() || shared.closed.load(Ordering::SeqCst) {
            break;
        }
        rusty_tokio::time::sleep(Duration::from_millis(10)).await;
    }
    while !shared.closed.load(Ordering::SeqCst) {
        let Some(session) = lock(&shared.session_id).clone() else {
            return;
        };
        let Ok(mut req) = client.get(&config.url) else {
            return;
        };
        let mut headers = vec![
            ("Accept".to_owned(), "text/event-stream".to_owned()),
            ("Mcp-Session-Id".to_owned(), session),
        ];
        if let Some(v) = lock(&shared.version).as_ref() {
            headers.push(("MCP-Protocol-Version".into(), v.as_str().to_owned()));
        }
        if let Some(id) = parser.last_event_id() {
            headers.push(("Last-Event-ID".into(), id.to_owned()));
        }
        for (k, v) in &headers {
            let Ok(r) = req.header(k, v) else { return };
            req = r;
        }
        if let Ok(mut resp) = req.send_streaming().await {
            let status = resp.status().as_u16();
            if matches!(status, 400 | 404 | 405 | 406) {
                return; // no stream on offer, or the session is gone
            }
            if status == 200 {
                while let Ok(Some(chunk)) = resp.chunk().await {
                    for event in parser.feed(&chunk) {
                        if event.event.as_deref().is_none_or(|e| e == "message") {
                            deliver(&tx, &event.data);
                        }
                    }
                }
            }
        }
        let wait = parser
            .retry_ms()
            .map_or(config.reconnect_delay, Duration::from_millis);
        rusty_tokio::time::sleep(wait).await;
    }
}

impl Transport for HttpTransport {
    fn send(&mut self, message: &Message) -> io::Result<()> {
        self.in_flight.retain(|_, h| !h.is_finished());
        if let Some(id) = cancelled_id(message) {
            if let Some(handle) = self.in_flight.remove(&id) {
                handle.abort();
            }
            if self.shared.stateless() {
                // Closing the connection was the cancellation.
                return Ok(());
            }
        }
        let id = match message {
            Message::Request { id, .. } => Some(id.clone()),
            _ => None,
        };
        let handle = self.runtime.spawn(post(
            self.client.clone(),
            Arc::clone(&self.config),
            Arc::clone(&self.shared),
            self.tx.clone(),
            message.clone(),
        ));
        if let Some(id) = id {
            self.in_flight.insert(id, handle);
        }
        Ok(())
    }

    fn recv(&mut self, timeout: Duration) -> io::Result<Recv> {
        match self.inbox.recv_timeout(timeout) {
            Ok(Event::Message(m)) => Ok(Recv::Message(m)),
            Err(RecvTimeoutError::Timeout) => Ok(Recv::Timeout),
            Err(RecvTimeoutError::Disconnected) => Ok(Recv::Closed),
        }
    }

    fn set_protocol_version(&mut self, version: &ProtocolVersion) {
        *lock(&self.shared.version) = Some(version.clone());
    }
}

impl Drop for HttpTransport {
    /// End the session (best effort, bounded) and stop the background tasks.
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::SeqCst);
        for (_, handle) in self.in_flight.drain() {
            handle.abort();
        }
        let Some(session) = lock(&self.shared.session_id).take() else {
            return;
        };
        let (client, url) = (self.client.clone(), self.config.url.clone());
        self.runtime.block_on(async move {
            let Ok(req) = client.delete(&url) else { return };
            let Ok(req) = req.header("Mcp-Session-Id", &session) else {
                return;
            };
            let _ = rusty_tokio::time::timeout(Duration::from_secs(2), req.send()).await;
        });
    }
}
