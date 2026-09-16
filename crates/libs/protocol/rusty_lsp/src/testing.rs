//! An in-memory test harness for backends built on this framework.
//!
//! [`TestClient`] plays the editor's role: it spawns your
//! [`LanguageServer`] on a real [`crate::Server`] over in-memory pipes and
//! speaks framed JSON-RPC to it, so tests exercise the full stack (framing,
//! dispatch, lifecycle, cancellation) rather than calling handler methods
//! directly.
//!
//! ```rust,no_run
//! use rusty_lsp::error::Result;
//! use rusty_lsp::lsp::{Hover, InitializeParams, InitializeResult};
//! use rusty_lsp::testing::TestClient;
//! use rusty_lsp::{Client, LanguageServer};
//!
//! struct Backend;
//! impl LanguageServer for Backend {
//!     async fn initialize(&self, _params: InitializeParams) -> Result<InitializeResult> {
//!         Ok(InitializeResult::default())
//!     }
//! }
//!
//! # async fn example() -> Result<()> {
//! let mut client = TestClient::spawn(|_client: Client| Backend);
//! client.initialize(InitializeParams::default()).await?;
//! client.notify("textDocument/didOpen", serde_json::json!({
//!     "textDocument": {
//!         "uri": "file:///a.txt", "languageId": "plaintext",
//!         "version": 1, "text": "hello",
//!     }
//! })).await?;
//! let hover: Option<Hover> = client.request("textDocument/hover", serde_json::json!({
//!     "textDocument": {"uri": "file:///a.txt"},
//!     "position": {"line": 0, "character": 0},
//! })).await?;
//! client.shutdown_and_exit().await?;
//! # Ok(())
//! # }
//! ```
//!
//! Reads are demand-driven and single-threaded: nothing is consumed from the
//! server until you call a `recv_*`/`request` method, and messages that
//! arrive while waiting for something specific are buffered, not lost.
//! If a handler under test calls the server-side [`crate::Client`] request
//! API (e.g. `workspace/configuration`), answer it between sending the
//! request and awaiting its response: use [`TestClient::start_request`],
//! then [`TestClient::recv_request`] + [`TestClient::respond`], then
//! [`TestClient::response`].

use crate::client::Client;
use crate::error::{Error, Result};
use crate::jsonrpc::{Message, Notification, Request, RequestId, Response};
use crate::lsp::{InitializeParams, InitializeResult};
use crate::server::Server;
use crate::service::LanguageServer;
use crate::transport;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::VecDeque;
use std::time::Duration;
use tokio::io::{BufReader, DuplexStream, ReadHalf, WriteHalf};
use tokio::task::JoinHandle;

/// How long [`TestClient`] waits for an expected message before failing the
/// test with a descriptive error instead of hanging it.
const DEFAULT_RECV_TIMEOUT: Duration = Duration::from_secs(10);

/// An in-memory LSP client driving a spawned [`crate::Server`].
pub struct TestClient {
    writer: WriteHalf<DuplexStream>,
    reader: BufReader<ReadHalf<DuplexStream>>,
    /// Messages read while scanning for something else, oldest first.
    buffered: VecDeque<Message>,
    next_id: i64,
    server: JoinHandle<Result<()>>,
    timeout: Duration,
}

impl TestClient {
    /// Spawn `build`'s backend on a server over in-memory pipes and return
    /// the client end. The server runs as a background task until
    /// [`shutdown_and_exit`](Self::shutdown_and_exit) (or drop, which severs
    /// the pipe).
    pub fn spawn<B, F>(build: F) -> Self
    where
        B: LanguageServer,
        F: FnOnce(Client) -> B + Send + 'static,
    {
        TestClient::spawn_configured(|server| server, build)
    }

    /// Like [`spawn`](Self::spawn), applying `configure` to the [`Server`]
    /// first — for testing a backend under builder options such as
    /// [`Server::with_max_concurrent_requests`],
    /// [`Server::with_outbound_queue_limit`], or
    /// [`Server::with_teardown_grace`]:
    ///
    /// ```rust,ignore
    /// let mut client = TestClient::spawn_configured(
    ///     |server| server.with_max_concurrent_requests(1),
    ///     |client| Backend { client },
    /// );
    /// ```
    pub fn spawn_configured<B, F, C>(configure: C, build: F) -> Self
    where
        B: LanguageServer,
        F: FnOnce(Client) -> B + Send + 'static,
        C: FnOnce(
                Server<ReadHalf<DuplexStream>, WriteHalf<DuplexStream>>,
            ) -> Server<ReadHalf<DuplexStream>, WriteHalf<DuplexStream>>
            + Send
            + 'static,
    {
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let (server_read, server_write) = tokio::io::split(server_io);
        let server = tokio::spawn(async move {
            configure(Server::new(server_read, server_write))
                .serve(build)
                .await
        });
        let (client_read, client_write) = tokio::io::split(client_io);
        TestClient {
            writer: client_write,
            reader: transport::buffered(client_read),
            buffered: VecDeque::new(),
            next_id: 0,
            server,
            timeout: DEFAULT_RECV_TIMEOUT,
        }
    }

    /// Change how long every receive waits before failing (default: 10s).
    /// A timed-out receive returns an error naming the timeout and the
    /// methods of the messages buffered so far, instead of hanging the
    /// test run.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Send a raw [`Message`] to the server.
    pub async fn send(&mut self, message: Message) -> Result<()> {
        transport::write_message(&mut self.writer, &message).await
    }

    /// Send a notification with typed params.
    pub async fn notify<P: Serialize>(&mut self, method: &str, params: P) -> Result<()> {
        let params = serde_json::to_value(params)?;
        self.send(Message::Notification(Notification {
            method: method.to_owned(),
            params: Some(params),
        }))
        .await
    }

    /// Send a request without awaiting its response; returns the id to pass
    /// to [`response`](Self::response) later. Use this (instead of
    /// [`request`](Self::request)) when the handler will call back into the
    /// client — e.g. `workspace/configuration` — so you can answer that
    /// callback before awaiting the response.
    pub async fn start_request<P: Serialize>(
        &mut self,
        method: &str,
        params: P,
    ) -> Result<RequestId> {
        self.next_id += 1;
        let id = RequestId::Number(self.next_id);
        self.send(Message::Request(Request {
            id: id.clone(),
            method: method.to_owned(),
            params: Some(serde_json::to_value(params)?),
        }))
        .await?;
        Ok(id)
    }

    /// Send a request and await its typed result. An error response becomes
    /// [`Error::Response`].
    pub async fn request<P, R>(&mut self, method: &str, params: P) -> Result<R>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let id = self.start_request(method, params).await?;
        let response = self.response(&id).await?;
        if let Some(error) = response.error {
            return Err(Error::Response(error));
        }
        Ok(serde_json::from_value(
            response.result.unwrap_or(Value::Null),
        )?)
    }

    /// Read until the response for `id` arrives; other messages are buffered
    /// for later `recv_*` calls.
    pub async fn response(&mut self, id: &RequestId) -> Result<Response> {
        if let Some(index) = self
            .buffered
            .iter()
            .position(|m| matches!(m, Message::Response(r) if r.id.as_ref() == Some(id)))
        {
            let Some(Message::Response(response)) = self.buffered.remove(index) else {
                unreachable!("position matched a response");
            };
            return Ok(response);
        }
        loop {
            match self.read().await? {
                Message::Response(response) if response.id.as_ref() == Some(id) => {
                    return Ok(response);
                }
                other => self.buffered.push_back(other),
            }
        }
    }

    /// Read until a notification with `method` arrives; other messages are
    /// buffered.
    pub async fn recv_notification(&mut self, method: &str) -> Result<Notification> {
        if let Some(index) = self
            .buffered
            .iter()
            .position(|m| matches!(m, Message::Notification(n) if n.method == method))
        {
            let Some(Message::Notification(note)) = self.buffered.remove(index) else {
                unreachable!("position matched a notification");
            };
            return Ok(note);
        }
        loop {
            match self.read().await? {
                Message::Notification(note) if note.method == method => return Ok(note),
                other => self.buffered.push_back(other),
            }
        }
    }

    /// Read until a server→client request with `method` arrives; other
    /// messages are buffered.
    pub async fn recv_request(&mut self, method: &str) -> Result<Request> {
        if let Some(index) = self
            .buffered
            .iter()
            .position(|m| matches!(m, Message::Request(r) if r.method == method))
        {
            let Some(Message::Request(request)) = self.buffered.remove(index) else {
                unreachable!("position matched a request");
            };
            return Ok(request);
        }
        loop {
            match self.read().await? {
                Message::Request(request) if request.method == method => return Ok(request),
                other => self.buffered.push_back(other),
            }
        }
    }

    /// Answer a server→client request with a success result.
    pub async fn respond<T: Serialize>(&mut self, id: RequestId, result: T) -> Result<()> {
        let result = serde_json::to_value(result)?;
        self.send(Message::Response(Response::success(id, result)))
            .await
    }

    /// Send `$/cancelRequest` for an in-flight request.
    pub async fn cancel(&mut self, id: &RequestId) -> Result<()> {
        self.notify("$/cancelRequest", serde_json::json!({ "id": id }))
            .await
    }

    /// Drive the `initialize` request and `initialized` notification,
    /// returning the server's capabilities.
    pub async fn initialize(&mut self, params: InitializeParams) -> Result<InitializeResult> {
        let result: InitializeResult = self.request("initialize", params).await?;
        self.notify("initialized", serde_json::json!({})).await?;
        Ok(result)
    }

    /// Drive the `shutdown` request and `exit` notification, then await the
    /// server task's own result — the full clean-teardown path.
    pub async fn shutdown_and_exit(mut self) -> Result<()> {
        let _: Value = self.request("shutdown", Value::Null).await?;
        self.notify("exit", serde_json::json!({})).await?;
        match tokio::time::timeout(self.timeout, self.server).await {
            Ok(Ok(result)) => result,
            Ok(Err(join_err)) => Err(Error::internal(format!("server task failed: {join_err}"))),
            Err(_elapsed) => Err(Error::internal(format!(
                "server task did not stop within {:?} after exit",
                self.timeout
            ))),
        }
    }

    /// Read one message off the wire, failing with a descriptive error if
    /// nothing arrives within the configured timeout.
    async fn read(&mut self) -> Result<Message> {
        match tokio::time::timeout(self.timeout, transport::read_message(&mut self.reader)).await {
            Ok(result) => result?.ok_or_else(|| Error::protocol("server closed the connection")),
            Err(_elapsed) => {
                let seen: Vec<String> = self
                    .buffered
                    .iter()
                    .map(|message| match message {
                        Message::Request(r) => format!("request `{}`", r.method),
                        Message::Notification(n) => format!("notification `{}`", n.method),
                        Message::Response(r) => format!("response to {:?}", r.id),
                    })
                    .collect();
                Err(Error::internal(format!(
                    "no message from the server within {:?}; {} message(s) buffered while                      scanning: [{}]",
                    self.timeout,
                    seen.len(),
                    seen.join(", ")
                )))
            }
        }
    }
}

impl TestClient {
    /// Open a document (`textDocument/didOpen`) with version 1.
    pub async fn open(&mut self, uri: &str, language_id: &str, text: &str) -> Result<()> {
        self.notify(
            "textDocument/didOpen",
            serde_json::json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": language_id,
                    "version": 1,
                    "text": text,
                }
            }),
        )
        .await
    }

    /// Replace a document's entire content (`textDocument/didChange`, full
    /// sync) at the given version.
    pub async fn change_full(&mut self, uri: &str, version: i32, text: &str) -> Result<()> {
        self.notify(
            "textDocument/didChange",
            serde_json::json!({
                "textDocument": {"uri": uri, "version": version},
                "contentChanges": [{"text": text}],
            }),
        )
        .await
    }

    /// Close a document (`textDocument/didClose`).
    pub async fn close(&mut self, uri: &str) -> Result<()> {
        self.notify(
            "textDocument/didClose",
            serde_json::json!({"textDocument": {"uri": uri}}),
        )
        .await
    }
}
