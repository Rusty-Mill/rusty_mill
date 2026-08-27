//! The [`Client`] handle: the server's outbound channel to the editor.
//!
//! A `Client` is handed to your backend when the server starts. It is cheap to
//! [`Clone`] and safe to share across tasks. Use it to push notifications
//! (diagnostics, log/show-message) and to issue server-to-client requests
//! (e.g. `workspace/configuration`).

use crate::error::{Error, Result, codes};
use crate::jsonrpc::{Message, Notification, Request, RequestId, Response};
use crate::lsp::{
    ApplyWorkspaceEditParams, ApplyWorkspaceEditResult, ConfigurationItem, ConfigurationParams,
    Diagnostic, LogMessageParams, LogTraceParams, MessageActionItem, MessageType, ProgressParams,
    ProgressToken, PublishDiagnosticsParams, Registration, RegistrationParams, ShowDocumentParams,
    ShowDocumentResult, ShowMessageParams, ShowMessageRequestParams, Unregistration,
    UnregistrationParams, Uri, WorkDoneProgress, WorkDoneProgressBegin,
    WorkDoneProgressCreateParams, WorkDoneProgressEnd, WorkDoneProgressParams,
    WorkDoneProgressReport, WorkspaceEdit, WorkspaceFolder,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

/// Map of in-flight server-to-client requests awaiting their responses.
type PendingResponses = Arc<Mutex<HashMap<RequestId, oneshot::Sender<Response>>>>;

/// The shared outbound message queue: an unbounded channel plus a depth
/// counter, so [`Client`]-originated traffic can honour an optional queue
/// limit while protocol-owed responses always get through.
#[derive(Clone)]
pub(crate) struct Outbound {
    tx: mpsc::UnboundedSender<Message>,
    depth: Arc<AtomicUsize>,
    limit: usize,
}

impl Outbound {
    /// Build the queue handle. `depth` must be decremented by the writer
    /// task for every message it dequeues.
    pub(crate) fn new(
        tx: mpsc::UnboundedSender<Message>,
        depth: Arc<AtomicUsize>,
        limit: Option<usize>,
    ) -> Self {
        Outbound {
            tx,
            depth,
            limit: limit.unwrap_or(usize::MAX),
        }
    }

    /// Enqueue unconditionally (used for responses the protocol owes the
    /// peer). Fails only if the connection has closed.
    pub(crate) fn send(&self, message: Message) -> Result<()> {
        self.depth.fetch_add(1, Ordering::Relaxed);
        self.tx
            .send(message)
            .map_err(|_| Error::internal("server output channel closed"))
    }

    /// Enqueue subject to the configured queue limit (used for
    /// [`Client`]-originated traffic). Fails with an internal error when the
    /// queue is full — a backpressure signal that the client is not reading.
    fn send_limited(&self, message: Message) -> Result<()> {
        if self.depth.load(Ordering::Relaxed) >= self.limit {
            return Err(Error::internal(
                "outbound queue limit reached; the client is not draining output",
            ));
        }
        self.send(message)
    }
}

/// A cloneable handle for sending messages from the server to the client.
#[derive(Clone)]
pub struct Client {
    sender: Outbound,
    next_id: Arc<AtomicI64>,
    pending: PendingResponses,
}

impl Client {
    /// Build a client over an outbound message channel.
    ///
    /// Each [`Client`] owns the map of in-flight server-to-client requests;
    /// clones share it, so a response delivered to any clone resolves the
    /// original caller.
    pub(crate) fn new(sender: Outbound) -> Self {
        Client {
            sender,
            next_id: Arc::new(AtomicI64::new(1)),
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Enqueue a notification with typed params.
    ///
    /// Returns an error only if the params fail to serialize or the connection
    /// has already closed. The send itself does not await — notifications are
    /// fire-and-forget.
    pub fn notify<P: Serialize>(&self, method: &str, params: P) -> Result<()> {
        let params = serde_json::to_value(params)?;
        self.send(Message::Notification(Notification {
            method: method.to_owned(),
            params: Some(params),
        }))
    }

    /// Send a request to the client and await its response.
    ///
    /// The result is deserialized into `R`. A client error response becomes
    /// [`Error::Response`]; a dropped connection becomes an internal error.
    /// Params that serialize to `null` (e.g. `()`, for parameterless
    /// methods like the `workspace/*/refresh` family) are sent with the
    /// `params` member omitted entirely, matching what those methods expect
    /// on the wire.
    pub async fn send_request<P, R>(&self, method: &str, params: P) -> Result<R>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let (_id, rx) = self.start_request(method, params)?;
        let response = rx
            .await
            .map_err(|_| Error::internal("server-to-client response channel dropped"))?;
        decode_response(response)
    }

    /// Like [`send_request`](Self::send_request), but give up after
    /// `timeout`. On expiry the pending entry is discarded (a late response
    /// is dropped silently), a `$/cancelRequest` for the request is sent to
    /// the client, and the call fails with a `REQUEST_FAILED` error.
    ///
    /// Use this for requests where a wedged or slow editor must not stall
    /// the server indefinitely (e.g. configuration lookups during startup).
    pub async fn send_request_with_timeout<P, R>(
        &self,
        method: &str,
        params: P,
        timeout: Duration,
    ) -> Result<R>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let (id, rx) = self.start_request(method, params)?;
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(response)) => decode_response(response),
            Ok(Err(_)) => Err(Error::internal("server-to-client response channel dropped")),
            Err(_elapsed) => {
                self.lock_pending().remove(&id);
                // Best-effort: tell the client we no longer want the answer.
                let _ = self.notify("$/cancelRequest", serde_json::json!({ "id": id }));
                Err(Error::response(
                    codes::REQUEST_FAILED,
                    format!("no response to `{method}` within {timeout:?}"),
                ))
            }
        }
    }

    /// Allocate an id, register the pending-response slot, and enqueue the
    /// request. Shared by the awaiting variants above.
    fn start_request<P: Serialize>(
        &self,
        method: &str,
        params: P,
    ) -> Result<(RequestId, oneshot::Receiver<Response>)> {
        let id = RequestId::Number(self.next_id.fetch_add(1, Ordering::Relaxed));
        let params = serde_json::to_value(params)?;
        let (tx, rx) = oneshot::channel();
        self.lock_pending().insert(id.clone(), tx);

        if let Err(err) = self.send(Message::Request(Request {
            id: id.clone(),
            method: method.to_owned(),
            params: if params.is_null() { None } else { Some(params) },
        })) {
            // Roll back the pending entry so it cannot leak.
            self.lock_pending().remove(&id);
            return Err(err);
        }
        Ok((id, rx))
    }

    /// Publish diagnostics for a document, replacing any previous set.
    ///
    /// Pass an empty `diagnostics` vector to clear them.
    pub fn publish_diagnostics(
        &self,
        uri: Uri,
        diagnostics: Vec<Diagnostic>,
        version: Option<i32>,
    ) -> Result<()> {
        self.notify(
            "textDocument/publishDiagnostics",
            PublishDiagnosticsParams {
                uri,
                version,
                diagnostics,
            },
        )
    }

    /// Send a `window/logMessage` notification (routed to the client's log).
    pub fn log_message(&self, typ: MessageType, message: impl Into<String>) -> Result<()> {
        self.notify(
            "window/logMessage",
            LogMessageParams {
                typ,
                message: message.into(),
            },
        )
    }

    /// [`log_message`](Self::log_message) at [`MessageType::Log`].
    pub fn log(&self, message: impl Into<String>) -> Result<()> {
        self.log_message(MessageType::Log, message)
    }

    /// [`log_message`](Self::log_message) at [`MessageType::Debug`]
    /// (LSP 3.18).
    pub fn log_debug(&self, message: impl Into<String>) -> Result<()> {
        self.log_message(MessageType::Debug, message)
    }

    /// [`log_message`](Self::log_message) at [`MessageType::Info`].
    pub fn log_info(&self, message: impl Into<String>) -> Result<()> {
        self.log_message(MessageType::Info, message)
    }

    /// [`log_message`](Self::log_message) at [`MessageType::Warning`].
    pub fn log_warning(&self, message: impl Into<String>) -> Result<()> {
        self.log_message(MessageType::Warning, message)
    }

    /// [`log_message`](Self::log_message) at [`MessageType::Error`].
    pub fn log_error(&self, message: impl Into<String>) -> Result<()> {
        self.log_message(MessageType::Error, message)
    }

    /// Send a `window/showMessage` notification (shown to the user).
    pub fn show_message(&self, typ: MessageType, message: impl Into<String>) -> Result<()> {
        self.notify(
            "window/showMessage",
            ShowMessageParams {
                typ,
                message: message.into(),
            },
        )
    }

    /// Show the user a message with a set of actions to choose from
    /// (`window/showMessageRequest`), and await their choice. Returns
    /// `Ok(None)` if the user dismissed the prompt without choosing.
    pub async fn show_message_request(
        &self,
        typ: MessageType,
        message: impl Into<String>,
        actions: Vec<MessageActionItem>,
    ) -> Result<Option<MessageActionItem>> {
        self.send_request(
            "window/showMessageRequest",
            ShowMessageRequestParams {
                typ,
                message: message.into(),
                actions: if actions.is_empty() {
                    None
                } else {
                    Some(actions)
                },
            },
        )
        .await
    }

    /// Ask the client to open or reveal a document (`window/showDocument`,
    /// LSP 3.16) — e.g. a generated file, or an external URL when `external`
    /// is set on `params`.
    pub async fn show_document(&self, params: ShowDocumentParams) -> Result<ShowDocumentResult> {
        self.send_request("window/showDocument", params).await
    }

    /// Register interest in one or more capabilities after `initialize`
    /// (`client/registerCapability`), e.g. to scope a feature to a
    /// `documentSelector` or advertise something not declared statically in
    /// [`crate::lsp::ServerCapabilities`].
    pub async fn register_capability(&self, registrations: Vec<Registration>) -> Result<()> {
        self.send_request(
            "client/registerCapability",
            RegistrationParams { registrations },
        )
        .await
    }

    /// Undo a previous [`register_capability`](Self::register_capability)
    /// call (`client/unregisterCapability`).
    pub async fn unregister_capability(&self, unregistrations: Vec<Unregistration>) -> Result<()> {
        self.send_request(
            "client/unregisterCapability",
            UnregistrationParams {
                unregisterations: unregistrations,
            },
        )
        .await
    }

    /// Send a `$/logTrace` notification: a protocol-level trace message,
    /// gated by the verbosity the client most recently set via
    /// `$/setTrace` (see
    /// [`LanguageServer::set_trace`](crate::LanguageServer::set_trace)).
    /// Distinct from [`log_message`](Self::log_message), which is ordinary
    /// user-facing logging.
    pub fn log_trace(&self, message: impl Into<String>, verbose: Option<String>) -> Result<()> {
        self.notify(
            "$/logTrace",
            LogTraceParams {
                message: message.into(),
                verbose,
            },
        )
    }

    /// Ask the client to reserve `token` for a subsequent work-done-progress
    /// sequence not tied to a specific client request (e.g. background
    /// indexing). Await the result before calling
    /// [`progress_begin`](Self::progress_begin).
    pub async fn create_progress(&self, token: impl Into<ProgressToken>) -> Result<()> {
        self.send_request(
            "window/workDoneProgress/create",
            WorkDoneProgressCreateParams {
                token: token.into(),
            },
        )
        .await
    }

    /// Start a work-done-progress sequence for `token` (reserved beforehand
    /// via [`create_progress`](Self::create_progress), or a token supplied by
    /// the client on a request's `workDoneToken`).
    pub fn progress_begin(
        &self,
        token: impl Into<ProgressToken>,
        begin: WorkDoneProgressBegin,
    ) -> Result<()> {
        self.notify(
            "$/progress",
            ProgressParams {
                token: token.into(),
                value: WorkDoneProgress::Begin(begin),
            },
        )
    }

    /// Report incremental progress within a sequence started with
    /// [`progress_begin`](Self::progress_begin).
    pub fn progress_report(
        &self,
        token: impl Into<ProgressToken>,
        report: WorkDoneProgressReport,
    ) -> Result<()> {
        self.notify(
            "$/progress",
            ProgressParams {
                token: token.into(),
                value: WorkDoneProgress::Report(report),
            },
        )
    }

    /// End a progress sequence started with
    /// [`progress_begin`](Self::progress_begin).
    pub fn progress_end(
        &self,
        token: impl Into<ProgressToken>,
        end: WorkDoneProgressEnd,
    ) -> Result<()> {
        self.notify(
            "$/progress",
            ProgressParams {
                token: token.into(),
                value: WorkDoneProgress::End(end),
            },
        )
    }

    /// Ask the client for the value of one or more configuration sections
    /// (`workspace/configuration`). The result has one entry per item in
    /// `items`, in the same order; a section the client has no value for
    /// comes back as `Value::Null`.
    pub async fn configuration(&self, items: Vec<ConfigurationItem>) -> Result<Vec<Value>> {
        self.send_request("workspace/configuration", ConfigurationParams { items })
            .await
    }

    /// Ask the client for a single configuration section and deserialize it
    /// — the common `workspace/configuration` call without positional
    /// `Vec<Value>` handling:
    ///
    /// ```rust,ignore
    /// #[derive(serde::Deserialize, Default)]
    /// #[serde(default)]
    /// struct Settings { max_problems: u32 }
    ///
    /// let settings: Settings = client.config_section("myServer", None).await?;
    /// ```
    ///
    /// A section the client has no value for comes back as JSON `null`; use
    /// an `Option<T>` (or a `#[serde(default)]` struct that tolerates null?
    /// — prefer `Option`) to distinguish "unset" from a deserialization
    /// failure.
    pub async fn config_section<T: DeserializeOwned>(
        &self,
        section: impl Into<String>,
        scope_uri: Option<Uri>,
    ) -> Result<T> {
        let mut values = self
            .configuration(vec![ConfigurationItem {
                section: Some(section.into()),
                scope_uri,
            }])
            .await?;
        let value = if values.is_empty() {
            Value::Null
        } else {
            values.swap_remove(0)
        };
        serde_json::from_value(value).map_err(Error::from)
    }

    /// Ask the client to apply a [`WorkspaceEdit`] to its buffers
    /// (`workspace/applyEdit`).
    pub async fn apply_edit(
        &self,
        edit: WorkspaceEdit,
        label: Option<String>,
    ) -> Result<ApplyWorkspaceEditResult> {
        self.send_request(
            "workspace/applyEdit",
            ApplyWorkspaceEditParams { label, edit },
        )
        .await
    }

    /// Send a raw `$/progress` notification carrying `value` on `token`.
    ///
    /// Unlike [`progress_begin`](Self::progress_begin)/
    /// [`progress_report`](Self::progress_report)/
    /// [`progress_end`](Self::progress_end) (which wrap `value` in the
    /// [`WorkDoneProgress`] begin/report/end shape), this sends `value`
    /// as-is — the shape a client streaming a request's
    /// `partialResultToken` expects (e.g. a chunk of a `references` result:
    /// `Vec<Location>`).
    pub fn send_progress<T: Serialize>(
        &self,
        token: impl Into<ProgressToken>,
        value: T,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct RawProgressParams<T> {
            token: ProgressToken,
            value: T,
        }

        self.notify(
            "$/progress",
            RawProgressParams {
                token: token.into(),
                value,
            },
        )
    }

    /// Ask the client to re-pull semantic tokens for all open documents
    /// (`workspace/semanticTokens/refresh`), e.g. after a change that
    /// invalidates previously reported tokens.
    pub async fn refresh_semantic_tokens(&self) -> Result<()> {
        self.send_request("workspace/semanticTokens/refresh", ())
            .await
    }

    /// Ask the client to re-pull code lenses for all open documents
    /// (`workspace/codeLens/refresh`).
    pub async fn refresh_code_lenses(&self) -> Result<()> {
        self.send_request("workspace/codeLens/refresh", ()).await
    }

    /// Ask the client to re-pull inlay hints for all open documents
    /// (`workspace/inlayHint/refresh`).
    pub async fn refresh_inlay_hints(&self) -> Result<()> {
        self.send_request("workspace/inlayHint/refresh", ()).await
    }

    /// Ask the client to re-pull diagnostics (`workspace/diagnostic/refresh`),
    /// e.g. after a configuration change that alters which diagnostics a
    /// server reports.
    pub async fn refresh_diagnostics(&self) -> Result<()> {
        self.send_request("workspace/diagnostic/refresh", ()).await
    }

    /// Ask the client to re-pull folding ranges for all open documents
    /// (`workspace/foldingRange/refresh`, LSP 3.18).
    pub async fn refresh_folding_ranges(&self) -> Result<()> {
        self.send_request("workspace/foldingRange/refresh", ())
            .await
    }

    /// Ask the client to re-pull inline values for all open documents
    /// (`workspace/inlineValue/refresh`), e.g. after the debug session
    /// state changes.
    pub async fn refresh_inline_values(&self) -> Result<()> {
        self.send_request("workspace/inlineValue/refresh", ()).await
    }

    /// Ask the client for the currently configured workspace folders
    /// (`workspace/workspaceFolders`). `Ok(None)` means the client has no
    /// folders configured (e.g. a single loose file is open).
    pub async fn workspace_folders(&self) -> Result<Option<Vec<WorkspaceFolder>>> {
        self.send_request("workspace/workspaceFolders", ()).await
    }

    /// Send a `telemetry/event` notification. The payload shape is entirely
    /// server-defined.
    pub fn telemetry_event<P: Serialize>(&self, data: P) -> Result<()> {
        self.notify("telemetry/event", data)
    }

    /// Start a work-done-progress sequence and get back an RAII guard that
    /// guarantees the matching end notification: dropping the guard (early
    /// return, `?`, panic unwind) sends a default `end`, or call
    /// [`ProgressGuard::finish`] to end it with a message.
    ///
    /// The token must be one the client supplied on a request's
    /// `workDoneToken`, or one reserved beforehand via
    /// [`create_progress`](Self::create_progress).
    pub fn begin_progress(
        &self,
        token: impl Into<ProgressToken>,
        begin: WorkDoneProgressBegin,
    ) -> Result<ProgressGuard> {
        let token = token.into();
        self.progress_begin(token.clone(), begin)?;
        Ok(ProgressGuard {
            client: self.clone(),
            token,
            finished: false,
        })
    }

    /// Start a progress sequence on the `workDoneToken` the client attached
    /// to a request, if it attached one — no
    /// [`create_progress`](Self::create_progress) round trip needed, since
    /// the client pre-created the token. Returns `Ok(None)` when the client
    /// did not offer a token (report no progress in that case):
    ///
    /// ```rust,ignore
    /// async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
    ///     let progress = self.client.begin_progress_for(
    ///         &params.work_done,
    ///         WorkDoneProgressBegin { title: "Searching".into(), ..Default::default() },
    ///     )?;
    ///     // ... if let Some(progress) = &progress { progress.report(...)? } ...
    /// }
    /// ```
    pub fn begin_progress_for(
        &self,
        params: &WorkDoneProgressParams,
        begin: WorkDoneProgressBegin,
    ) -> Result<Option<ProgressGuard>> {
        match &params.work_done_token {
            Some(token) => self.begin_progress(token.clone(), begin).map(Some),
            None => Ok(None),
        }
    }

    /// Deliver a response received from the client to its waiting caller.
    ///
    /// Called by the server loop when a [`Message::Response`] arrives. Unknown
    /// or already-resolved ids are dropped silently.
    pub(crate) fn resolve(&self, response: Response) {
        let Some(id) = response.id.clone() else {
            return;
        };
        if let Some(tx) = self.lock_pending().remove(&id) {
            // The receiver may have been dropped (caller gave up); ignore.
            let _ = tx.send(response);
        }
    }

    /// Send a raw message over the outbound channel, honouring the queue
    /// limit configured via
    /// [`Server::with_outbound_queue_limit`](crate::Server::with_outbound_queue_limit).
    fn send(&self, message: Message) -> Result<()> {
        self.sender.send_limited(message)
    }

    /// Lock the pending map, recovering from a poisoned mutex.
    ///
    /// Poisoning means a task panicked while holding the lock; the map itself
    /// stays consistent, so recovering the guard is preferable to wedging every
    /// future request.
    fn lock_pending(&self) -> MutexGuard<'_, HashMap<RequestId, oneshot::Sender<Response>>> {
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Turn a client response into the typed result, mapping error responses to
/// [`Error::Response`].
fn decode_response<R: DeserializeOwned>(response: Response) -> Result<R> {
    if let Some(error) = response.error {
        return Err(Error::Response(error));
    }
    let value = response.result.unwrap_or(Value::Null);
    Ok(serde_json::from_value(value)?)
}

/// An in-progress work-done-progress sequence that cannot leak: obtained
/// from [`Client::begin_progress`], it sends the `end` notification when
/// [`finish`](Self::finish)ed or dropped — whichever comes first.
#[must_use = "dropping the guard ends the progress sequence immediately"]
pub struct ProgressGuard {
    client: Client,
    token: ProgressToken,
    finished: bool,
}

impl ProgressGuard {
    /// The token this sequence reports under.
    pub fn token(&self) -> &ProgressToken {
        &self.token
    }

    /// Report incremental progress within the sequence.
    pub fn report(&self, report: WorkDoneProgressReport) -> Result<()> {
        self.client.progress_report(self.token.clone(), report)
    }

    /// End the sequence with an explicit final payload (e.g. a message).
    pub fn finish(mut self, end: WorkDoneProgressEnd) -> Result<()> {
        self.finished = true;
        self.client.progress_end(self.token.clone(), end)
    }
}

impl Drop for ProgressGuard {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self
                .client
                .progress_end(self.token.clone(), WorkDoneProgressEnd { message: None });
        }
    }
}
