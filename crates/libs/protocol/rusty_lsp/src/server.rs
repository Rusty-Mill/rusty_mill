//! The [`Server`]: the runtime that drives an LSP connection.
//!
//! [`Server::serve`] takes ownership of a read/write byte stream, builds your
//! [`LanguageServer`] backend, and runs the message loop until the client
//! disconnects or sends `exit`. It owns everything protocol-related:
//!
//! - **Framing & JSON-RPC** via [`crate::transport`].
//! - **Lifecycle**: enforces `initialize` first, rejects work after `shutdown`,
//!   stops on `exit`.
//! - **Concurrency**: notifications run in receipt order on a dedicated
//!   serialized worker (so document state stays consistent) without blocking
//!   the message loop — `$/cancelRequest` and response delivery stay
//!   responsive even mid-`didChange` — while requests are spawned tasks that
//!   first wait for every notification received before them to be applied.
//! - **Cancellation**: `$/cancelRequest` aborts the in-flight handler and
//!   replies with [`REQUEST_CANCELLED`](crate::error::codes::REQUEST_CANCELLED),
//!   with the bookkeeping arranged so a request is answered exactly once.

use crate::cancel::{self, CancelToken};
use crate::client::{Client, Outbound};
use crate::error::{Error, ResponseError, Result, codes};
use crate::jsonrpc::{Message, Notification, Request, RequestId, Response};
use crate::lsp::{
    CallHierarchyIncomingCallsParams, CallHierarchyOutgoingCallsParams, CallHierarchyPrepareParams,
    CodeAction, CodeActionParams, CodeLens, CodeLensParams, ColorPresentationParams,
    CompletionItem, CompletionParams, CreateFilesParams, DeclarationParams, DefinitionParams,
    DeleteFilesParams, DidChangeConfigurationParams, DidChangeNotebookDocumentParams,
    DidChangeTextDocumentParams, DidChangeWatchedFilesParams, DidChangeWorkspaceFoldersParams,
    DidCloseNotebookDocumentParams, DidCloseTextDocumentParams, DidOpenNotebookDocumentParams,
    DidOpenTextDocumentParams, DidSaveNotebookDocumentParams, DidSaveTextDocumentParams,
    DocumentColorParams, DocumentDiagnosticParams, DocumentFormattingParams,
    DocumentHighlightParams, DocumentLink, DocumentLinkParams, DocumentOnTypeFormattingParams,
    DocumentRangeFormattingParams, DocumentSymbolParams, ExecuteCommandParams, FoldingRangeParams,
    HoverParams, ImplementationParams, InitializeParams, InlayHint, InlayHintParams,
    InlineCompletionParams, InlineValueParams, LinkedEditingRangeParams, MessageType,
    MonikerParams, ReferenceParams, RenameFilesParams, RenameParams, SelectionRangeParams,
    SemanticTokensDeltaParams, SemanticTokensParams, SemanticTokensRangeParams, SetTraceParams,
    SignatureHelpParams, TextDocumentPositionParams, TypeDefinitionParams,
    TypeHierarchyPrepareParams, TypeHierarchySubtypesParams, TypeHierarchySupertypesParams,
    WillSaveTextDocumentParams, WorkDoneProgressCancelParams, WorkspaceDiagnosticParams,
    WorkspaceSymbol, WorkspaceSymbolParams,
};
use crate::service::LanguageServer;
use crate::transport;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::{Semaphore, mpsc, watch};
use tokio::task::AbortHandle;

/// Bookkeeping for one running request handler.
struct InFlightEntry {
    /// Aborts the handler task at its next `.await` point.
    abort: AbortHandle,
    /// The cooperative signal visible to the handler via
    /// [`crate::cancel::current`], reaching work an abort cannot (blocking
    /// pools, helper tasks, CPU-bound stretches).
    token: CancelToken,
}

/// Map of request ids to their running handlers' bookkeeping.
type InFlight = Arc<Mutex<HashMap<RequestId, InFlightEntry>>>;

/// Params of the `$/cancelRequest` notification.
#[derive(serde::Deserialize)]
struct CancelParams {
    id: RequestId,
}

/// An LSP server bound to a byte transport.
///
/// Construct with [`Server::new`] over any async reader/writer, or
/// [`Server::stdio`] for the conventional stdin/stdout transport, then call
/// [`serve`](Server::serve).
pub struct Server<R, W> {
    reader: R,
    writer: W,
    max_concurrent_requests: Option<usize>,
    outbound_queue_limit: Option<usize>,
    teardown_grace: std::time::Duration,
    shutdown_signal: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
}

/// How long teardown waits for queued notification handlers before
/// aborting them (see [`Server::with_teardown_grace`]).
const DEFAULT_TEARDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

impl Server<tokio::io::Stdin, tokio::io::Stdout> {
    /// Build a server over the process's standard input and output.
    ///
    /// This is the transport editors use when they launch a language server as
    /// a child process.
    pub fn stdio() -> Self {
        Server::new(tokio::io::stdin(), tokio::io::stdout())
    }
}

#[cfg(feature = "tcp")]
#[cfg_attr(docsrs, doc(cfg(feature = "tcp")))]
impl Server<tokio::net::tcp::OwnedReadHalf, tokio::net::tcp::OwnedWriteHalf> {
    /// Build a server over an accepted TCP connection (requires the `tcp`
    /// feature), for clients that connect over a socket instead of spawning
    /// the server as a child process.
    pub fn from_tcp(stream: tokio::net::TcpStream) -> Self {
        let (reader, writer) = stream.into_split();
        Server::new(reader, writer)
    }
}

/// Serve LSP connections accepted from `listener`, each on its own task
/// with its own backend built by `factory` (requires the `tcp` feature) —
/// the socket-mode counterpart of [`Server::stdio`]:
///
/// ```rust,no_run
/// # use rusty_lsp::error::Result;
/// # struct Backend { client: rusty_lsp::Client }
/// # impl rusty_lsp::LanguageServer for Backend {
/// #     async fn initialize(&self, _p: rusty_lsp::lsp::InitializeParams)
/// #         -> Result<rusty_lsp::lsp::InitializeResult> { unimplemented!() }
/// # }
/// # async fn run() -> Result<()> {
/// let listener = tokio::net::TcpListener::bind("127.0.0.1:9257").await?;
/// rusty_lsp::server::serve_tcp(listener, |client| Backend { client }).await
/// # }
/// ```
///
/// Runs until `accept` fails; a connection whose serve loop errors is
/// logged nowhere and simply ends (each connection is independent).
#[cfg(feature = "tcp")]
#[cfg_attr(docsrs, doc(cfg(feature = "tcp")))]
pub async fn serve_tcp<B, F>(listener: tokio::net::TcpListener, factory: F) -> Result<()>
where
    B: LanguageServer,
    F: Fn(Client) -> B + Send + Sync + 'static,
{
    let factory = Arc::new(factory);
    loop {
        let (stream, _peer) = listener.accept().await?;
        let factory = Arc::clone(&factory);
        tokio::spawn(async move {
            let _ = Server::from_tcp(stream)
                .serve(move |client| factory(client))
                .await;
        });
    }
}

impl<R, W> Server<R, W>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    /// Build a server over an arbitrary reader and writer.
    pub fn new(reader: R, writer: W) -> Self {
        Server {
            reader,
            writer,
            max_concurrent_requests: None,
            outbound_queue_limit: None,
            teardown_grace: DEFAULT_TEARDOWN_GRACE,
            shutdown_signal: None,
        }
    }

    /// Stop serving when `signal` resolves, going through the normal
    /// teardown (in-flight aborts, notification grace) and returning
    /// `Ok(())` — an external termination path the wire cannot provide.
    /// Typical signals: `tokio::signal::ctrl_c()`, or a watchdog future
    /// that resolves when the parent editor process
    /// ([`InitializeParams::process_id`](crate::lsp::InitializeParams::process_id))
    /// has exited.
    #[must_use]
    pub fn with_shutdown_signal(
        mut self,
        signal: impl Future<Output = ()> + Send + 'static,
    ) -> Self {
        self.shutdown_signal = Some(Box::pin(signal));
        self
    }

    /// How long teardown waits for still-queued notification handlers to
    /// finish before aborting them (default: 2s). After a proper `shutdown`
    /// request the queue is already drained (the watermark wait guarantees
    /// it), so the grace only matters on abrupt endings — `exit` without
    /// `shutdown`, or EOF — where the client is gone and a slow handler
    /// should not keep the process alive indefinitely.
    #[must_use]
    pub fn with_teardown_grace(mut self, grace: std::time::Duration) -> Self {
        self.teardown_grace = grace;
        self
    }

    /// Cap how many request handlers may execute concurrently (default:
    /// unlimited). Excess requests still get their own task immediately —
    /// so cancellation and lifecycle bookkeeping work as usual — but wait
    /// their (FIFO) turn before the handler body runs. This bounds the
    /// damage a misbehaving client flooding requests can do.
    #[must_use]
    pub fn with_max_concurrent_requests(mut self, limit: usize) -> Self {
        self.max_concurrent_requests = Some(limit);
        self
    }

    /// Cap how many outbound messages may sit unwritten in the output queue
    /// (default: unlimited). The cap applies to [`Client`]-originated
    /// traffic (notifications and server→client requests), which fails with
    /// an error once the queue is full — a backpressure signal that the
    /// editor is not draining its end. Responses the protocol owes the
    /// client are never dropped and do not observe the cap.
    #[must_use]
    pub fn with_outbound_queue_limit(mut self, limit: usize) -> Self {
        self.outbound_queue_limit = Some(limit);
        self
    }

    /// Run the server until the connection closes or the client sends `exit`.
    ///
    /// `build` receives a [`Client`] and returns the backend; store the client
    /// so handlers can talk back to the editor. Returns `Ok(())` on a clean
    /// shutdown (a `shutdown` request followed by `exit`, or EOF at a frame
    /// boundary), and an error if the transport failed irrecoverably — or if
    /// `exit` arrived without a prior `shutdown`, which the spec says should
    /// end the process with exit code 1 (returning the error from a
    /// `fn main() -> Result<()>` does exactly that).
    pub async fn serve<B, F>(self, build: F) -> Result<()>
    where
        B: LanguageServer,
        F: FnOnce(Client) -> B,
    {
        let Server {
            reader,
            mut writer,
            max_concurrent_requests,
            outbound_queue_limit,
            teardown_grace,
            shutdown_signal,
        } = self;

        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Message>();
        let queue_depth = Arc::new(AtomicUsize::new(0));
        let outbound = Outbound::new(out_tx, Arc::clone(&queue_depth), outbound_queue_limit);
        let client = Client::new(outbound.clone());
        let backend = Arc::new(build(client.clone()));
        let request_permits = max_concurrent_requests.map(|limit| Arc::new(Semaphore::new(limit)));

        // Single writer task: serializes all outbound traffic and is the sole
        // owner of the write half. It ends once every sender is dropped — or
        // on a write error, which it signals so the read loop tears down
        // instead of serving a half-closed connection until reader EOF.
        let (writer_dead_tx, writer_dead) = watch::channel(false);
        let writer_task = tokio::spawn(async move {
            let mut batch = Vec::new();
            loop {
                let Some(first) = out_rx.recv().await else {
                    break;
                };
                queue_depth.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                batch.clear();
                let mut encode = transport::encode_message(&mut batch, &first);
                // Drain whatever else is already queued so a burst of
                // responses (e.g. many concurrent handlers finishing close
                // together) becomes one `write_all` + one `flush`, not one
                // syscall pair per message. `try_recv` never awaits, so this
                // only picks up messages already sitting in the channel —
                // it can't stall waiting for more to arrive.
                while let Ok(message) = out_rx.try_recv() {
                    queue_depth.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                    encode = encode.and_then(|()| transport::encode_message(&mut batch, &message));
                }
                if let Err(err) = encode {
                    let _ = writer_dead_tx.send(true);
                    return Err(err);
                }
                if let Err(err) = writer.write_all(&batch).await {
                    let _ = writer_dead_tx.send(true);
                    return Err(Error::from(err));
                }
                if let Err(err) = writer.flush().await {
                    let _ = writer_dead_tx.send(true);
                    return Err(Error::from(err));
                }
            }
            Ok::<(), Error>(())
        });

        let result = run_loop(
            reader,
            client,
            outbound,
            request_permits,
            teardown_grace,
            shutdown_signal,
            writer_dead,
            backend,
        )
        .await;

        // `run_loop` has dropped its senders and aborted in-flight handlers on
        // return; await the writer so buffered output is flushed before exit.
        match writer_task.await {
            // A writer failure carries the underlying io cause and takes
            // precedence over the loop's derived teardown error.
            Ok(write_result) => write_result.and(result),
            Err(join_err) if join_err.is_panic() => {
                Err(Error::internal(format!("writer task panicked: {join_err}")))
            }
            Err(_) => result,
        }
    }
}

/// The message loop. Owns the loop-side handles so they drop on return.
#[allow(clippy::too_many_arguments)]
async fn run_loop<R, B>(
    reader: R,
    client: Client,
    out_tx: Outbound,
    request_permits: Option<Arc<Semaphore>>,
    teardown_grace: std::time::Duration,
    mut shutdown_signal: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
    mut writer_dead: watch::Receiver<bool>,
    backend: Arc<B>,
) -> Result<()>
where
    R: AsyncRead + Unpin,
    B: LanguageServer,
{
    let mut reader = transport::buffered(reader);
    let in_flight: InFlight = Arc::new(Mutex::new(HashMap::new()));
    let mut initialized = false;
    let mut shutdown_requested = false;

    // Notifications must run in receipt order, but must not block this loop
    // — a slow `didChange` handler would otherwise delay `$/cancelRequest`
    // handling and response delivery to `Client::send_request` callers.
    // They run on a dedicated serialized worker instead; each spawned
    // request then waits on the watermark of notifications received before
    // it, preserving the guarantee that a request observes every earlier
    // notification's effects.
    let (note_tx, mut note_rx) = mpsc::unbounded_channel::<QueuedNotification>();
    let (notes_done_tx, notes_done) = watch::channel(0u64);
    let worker_backend = Arc::clone(&backend);
    let worker_client = client.clone();
    let mut notification_worker = tokio::spawn(async move {
        while let Some(note) = note_rx.recv().await {
            // Catch panics per notification: an unwinding handler must not
            // kill the worker, which would wedge every request waiting on
            // the watermark.
            let run = async {
                if note.method == "initialized" {
                    worker_backend.initialized().await;
                } else {
                    dispatch_notification(
                        worker_backend.as_ref(),
                        &worker_client,
                        &note.method,
                        note.params,
                    )
                    .await;
                }
            };
            if let Err(payload) = CatchUnwind(AssertUnwindSafe(run)).await {
                let _ = worker_client.log_message(
                    MessageType::Error,
                    format!(
                        "`{}` handler panicked: {}",
                        note.method,
                        panic_message(&payload)
                    ),
                );
            }
            let _ = notes_done_tx.send(note.seq);
        }
    });
    let mut notes_enqueued = 0u64;

    let loop_result = loop {
        let read = tokio::select! {
            result = transport::read_message(&mut reader) => result,
            // The writer failed: the client stopped reading (or the pipe
            // broke). Responses can no longer be delivered, so serving on
            // would be a zombie phase; tear down now. The writer task's own
            // error carries the underlying cause and wins in `serve`.
            _ = writer_dead.changed() => {
                break Err(Error::protocol(
                    "connection write side failed; tearing down",
                ));
            }
            // External shutdown (ctrl-c, parent-process watchdog): a clean
            // stop through the normal teardown.
            _ = poll_shutdown_signal(&mut shutdown_signal) => break Ok(()),
        };
        let message = match read {
            Ok(Some(message)) => message,
            Ok(None) => break Ok(()), // clean EOF at a frame boundary
            // The frame itself was read intact (exactly `Content-Length` bytes
            // consumed) but its body was not valid JSON, or classified to an
            // invalid `Message` shape (e.g. a non-numeric/string id). The
            // stream is not desynchronised, so report a Parse error for this
            // message and keep the connection alive, per JSON-RPC/LSP.
            Err(Error::Serde(err)) => {
                let _ = out_tx.send(
                    Response::error(
                        None,
                        ResponseError {
                            code: codes::PARSE_ERROR,
                            message: err.to_string(),
                            data: None,
                        },
                    )
                    .into(),
                );
                continue;
            }
            Err(err) => {
                // A framing error (bad headers, mid-frame EOF, ...)
                // desynchronises the stream; we cannot safely continue.
                // Surface it to the caller.
                break Err(err);
            }
        };

        #[cfg(feature = "tracing")]
        match &message {
            Message::Request(req) => {
                tracing::debug!(method = %req.method, id = %req.id, "<-- request");
            }
            Message::Notification(note) => {
                tracing::debug!(method = %note.method, "<-- notification");
            }
            Message::Response(response) => {
                tracing::debug!(id = ?response.id, ok = response.error.is_none(), "<-- response");
            }
        }

        match message {
            Message::Response(response) => client.resolve(response),

            Message::Notification(note) => match note.method.as_str() {
                // Per the spec, the server's process should exit with code 1
                // when `exit` arrives without a prior `shutdown` request.
                // Surfacing that case as an error lets a `fn main() ->
                // Result<()>` produce exactly that exit code.
                "exit" => {
                    break if shutdown_requested {
                        Ok(())
                    } else {
                        Err(Error::protocol("exit received before shutdown request"))
                    };
                }
                "$/cancelRequest" => cancel_request(&note, &in_flight, &out_tx),
                _ => {
                    // Per spec, drop notifications that arrive before
                    // `initialize` (other than `exit`, handled above).
                    // `initialized` rides the same queue so it stays ordered
                    // with respect to the document notifications after it.
                    if initialized {
                        notes_enqueued += 1;
                        let _ = note_tx.send(QueuedNotification {
                            seq: notes_enqueued,
                            method: note.method.clone(),
                            params: note.params,
                        });
                    }
                }
            },

            Message::Request(req) => match req.method.as_str() {
                "initialize" => {
                    if initialized {
                        send_error(
                            &out_tx,
                            req.id,
                            Error::invalid_request("server already initialized"),
                        );
                    } else {
                        let outcome = match parse_params::<InitializeParams>(req.params) {
                            Ok(params) => backend
                                .initialize(params)
                                .await
                                .and_then(|result| to_json(&result)),
                            Err(err) => Err(err),
                        };
                        match outcome {
                            Ok(value) => {
                                initialized = true;
                                let _ = out_tx.send(Response::success(req.id, value).into());
                            }
                            Err(err) => send_error(&out_tx, req.id, err),
                        }
                    }
                }
                "shutdown" => {
                    if !initialized {
                        send_error(&out_tx, req.id, Error::server_not_initialized());
                    } else {
                        // Let already-received notifications land first, so
                        // `shutdown` observes their effects like any request.
                        wait_for_notifications(&notes_done, notes_enqueued).await;
                        shutdown_requested = true;
                        match backend.shutdown().await {
                            Ok(()) => {
                                let _ = out_tx.send(Response::success(req.id, Value::Null).into());
                            }
                            Err(err) => send_error(&out_tx, req.id, err),
                        }
                    }
                }
                _ => {
                    if !initialized {
                        send_error(&out_tx, req.id, Error::server_not_initialized());
                    } else if shutdown_requested {
                        send_error(
                            &out_tx,
                            req.id,
                            Error::invalid_request("server is shutting down"),
                        );
                    } else {
                        spawn_request(
                            &backend,
                            &out_tx,
                            &in_flight,
                            &request_permits,
                            &notes_done,
                            notes_enqueued,
                            req,
                        );
                    }
                }
            },
        }
    };

    // Tear down: stop feeding the notification worker and abort any request
    // handlers still running so their captured senders and backend
    // references drop. Give the worker a bounded grace to drain its queue —
    // after a proper `shutdown` it is already empty, so the grace only
    // bites on abrupt endings, where a slow queued handler must not keep
    // the process alive indefinitely.
    drop(note_tx);
    for (_, entry) in lock(&in_flight).drain() {
        entry.token.cancel();
        entry.abort.abort();
    }
    if tokio::time::timeout(teardown_grace, &mut notification_worker)
        .await
        .is_err()
    {
        notification_worker.abort();
        let _ = notification_worker.await;
    }

    loop_result
}

/// One notification queued for the serialized notification worker.
struct QueuedNotification {
    /// Position in the receipt order, reported on the completion watermark.
    seq: u64,
    method: String,
    params: Option<Value>,
}

/// Await the configured external shutdown signal, or pend forever when
/// none was configured.
async fn poll_shutdown_signal(signal: &mut Option<Pin<Box<dyn Future<Output = ()> + Send>>>) {
    match signal {
        Some(signal) => signal.as_mut().await,
        None => std::future::pending().await,
    }
}

/// Wait until the notification worker has finished every notification up to
/// `watermark`. Returns immediately for a zero watermark, and gives up (the
/// caller is being torn down anyway) if the worker has gone away.
async fn wait_for_notifications(done: &watch::Receiver<u64>, watermark: u64) {
    if watermark == 0 {
        return;
    }
    let mut done = done.clone();
    let _ = done.wait_for(|&seq| seq >= watermark).await;
}

/// Spawn a feature request handler as its own task.
///
/// The task, any concurrent `$/cancelRequest`, and the panic watcher below
/// race to remove the id from [`InFlight`]; whoever wins sends the single
/// response, guaranteeing a request is answered exactly once.
///
/// A client must not reuse the id of a request it has not yet received a
/// response for (JSON-RPC/LSP); a request whose id is already in
/// [`InFlight`] is rejected outright rather than spawned, since letting it
/// through would silently steal the original request's `InFlight` entry —
/// corrupting `$/cancelRequest` targeting and dropping whichever response
/// loses the race. This function only ever runs on the single-threaded
/// message loop (never concurrently with itself), so the check-then-insert
/// below has no race window of its own.
#[allow(clippy::too_many_arguments)]
fn spawn_request<B: LanguageServer>(
    backend: &Arc<B>,
    out_tx: &Outbound,
    in_flight: &InFlight,
    request_permits: &Option<Arc<Semaphore>>,
    notes_done: &watch::Receiver<u64>,
    notes_watermark: u64,
    req: Request,
) {
    let request_id = req.id;
    let method = req.method;
    let params = req.params;

    let backend = Arc::clone(backend);
    let handler_out_tx = out_tx.clone();
    let in_flight_for_task = Arc::clone(in_flight);
    let response_id = request_id.clone();
    let token = CancelToken::new();
    let task_token = token.clone();
    let permits = request_permits.clone();
    let notes_done = notes_done.clone();

    // Hold `in_flight`'s lock across the "already in flight" check, the
    // spawn, and the insert below, as one critical section spanning both.
    //
    // This closes a race that a channel-based start gate used to handle
    // more expensively: a handler with no real `.await` inside it (the
    // common case for a trivial or already-cached result) can run to
    // completion on another worker thread and reach its own "remove myself
    // to claim the response" line before this thread finishes registering
    // it — `tokio::spawn` schedules eagerly on a multi-threaded runtime, so
    // this is not theoretical, it reproduces under real concurrent load.
    // That removal also locks `in_flight`, so as long as *this* thread is
    // still holding the lock when the task reaches it, the task's own
    // thread simply blocks — for the few nanoseconds `tokio::spawn` plus a
    // `HashMap` insert take — until this scope ends, at which point the
    // entry is already there and the removal succeeds correctly. A tiny,
    // bounded `std::sync::Mutex` hold (no `.await` inside it) is the
    // standard tool for this; it costs nothing close to what a full
    // channel poll/wake round trip on every request would.
    let mut guard = lock(in_flight);
    if guard.contains_key(&request_id) {
        drop(guard);
        send_error(
            out_tx,
            request_id,
            Error::invalid_request("request id is already in flight"),
        );
        return;
    }

    let join = tokio::spawn(cancel::scope(task_token, async move {
        // Wait for every notification received before this request to be
        // applied, so the handler observes document state as if dispatch
        // were fully sequential.
        wait_for_notifications(&notes_done, notes_watermark).await;
        // Under a concurrency cap, wait for a slot before running the
        // handler body. The task exists either way, so cancellation (which
        // aborts this task) and the exactly-once bookkeeping are unaffected
        // by queueing. `acquire_owned` on an unclosed semaphore cannot fail.
        let _permit = match permits {
            Some(semaphore) => semaphore.acquire_owned().await.ok(),
            None => None,
        };
        // Catch panics in-task so the panic itself becomes the response —
        // there is no separate watcher task racing for the entry.
        let outcome = CatchUnwind(AssertUnwindSafe(dispatch_request(
            backend.as_ref(),
            &method,
            params,
        )))
        .await;
        // Claim the right to respond. If the entry is already gone, a cancel
        // beat us to it and has sent the cancellation response. If we get
        // here before the spawning thread has inserted our entry, this
        // blocks (see above) rather than racing past an absent entry.
        if lock(&in_flight_for_task).remove(&response_id).is_some() {
            let response = match outcome {
                Ok(Ok(value)) => Response::success(response_id, value),
                Ok(Err(err)) => Response::error(Some(response_id), err.into_response_error()),
                Err(payload) => Response::error(
                    Some(response_id),
                    Error::internal(format!("handler panicked: {}", panic_message(&payload)))
                        .into_response_error(),
                ),
            };
            let _ = handler_out_tx.send(response.into());
        }
    }));

    guard.insert(
        request_id,
        InFlightEntry {
            abort: join.abort_handle(),
            token,
        },
    );
    // `guard` drops here (end of scope), releasing the lock — only now can
    // a racing task blocked on the same lock proceed.
}

/// A future adapter that catches panics from the inner future's `poll`,
/// resolving to `Err(payload)` instead of unwinding into the executor.
struct CatchUnwind<F>(F);

impl<F: Future> Future for CatchUnwind<F> {
    type Output = std::result::Result<F::Output, Box<dyn std::any::Any + Send>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: structural pinning of the sole field; it is never moved.
        let inner = unsafe { self.map_unchecked_mut(|s| &mut s.0) };
        match std::panic::catch_unwind(AssertUnwindSafe(|| inner.poll(cx))) {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(value)) => Poll::Ready(Ok(value)),
            Err(payload) => Poll::Ready(Err(payload)),
        }
    }
}

/// Best-effort extraction of a panic payload's message.
fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> &str {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        s
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s
    } else {
        "non-string panic payload"
    }
}

/// Handle `$/cancelRequest`: signal the handler's [`CancelToken`], abort its
/// task, and reply with a cancellation error, but only if the request is
/// still in flight.
fn cancel_request(note: &Notification, in_flight: &InFlight, out_tx: &Outbound) {
    let Some(params) = note.params.clone() else {
        return;
    };
    let Ok(CancelParams { id }) = serde_json::from_value(params) else {
        return;
    };
    // Removing here both stops the handler and prevents it from later sending
    // its own response (it will find its entry already gone).
    if let Some(entry) = lock(in_flight).remove(&id) {
        #[cfg(feature = "tracing")]
        tracing::debug!(id = %id, "request cancelled by client");
        // Trip the cooperative token first, so work the abort cannot reach
        // (spawn_blocking, helper tasks) observes the cancellation too.
        entry.token.cancel();
        entry.abort.abort();
        let _ = out_tx.send(
            Response::error(Some(id), Error::request_cancelled().into_response_error()).into(),
        );
    }
}

/// Route a feature request to the matching typed handler, falling back to the
/// untyped [`LanguageServer::handle_request`] escape hatch.
async fn dispatch_request<B: LanguageServer>(
    backend: &B,
    method: &str,
    params: Option<Value>,
) -> Result<Value> {
    match method {
        "textDocument/hover" => {
            to_json(&backend.hover(parse_params::<HoverParams>(params)?).await?)
        }
        "textDocument/completion" => to_json(
            &backend
                .completion(parse_params::<CompletionParams>(params)?)
                .await?,
        ),
        "textDocument/definition" => to_json(
            &backend
                .definition(parse_params::<DefinitionParams>(params)?)
                .await?,
        ),
        "textDocument/declaration" => to_json(
            &backend
                .declaration(parse_params::<DeclarationParams>(params)?)
                .await?,
        ),
        "textDocument/typeDefinition" => to_json(
            &backend
                .type_definition(parse_params::<TypeDefinitionParams>(params)?)
                .await?,
        ),
        "textDocument/implementation" => to_json(
            &backend
                .implementation(parse_params::<ImplementationParams>(params)?)
                .await?,
        ),
        "textDocument/references" => to_json(
            &backend
                .references(parse_params::<ReferenceParams>(params)?)
                .await?,
        ),
        "textDocument/documentHighlight" => to_json(
            &backend
                .document_highlight(parse_params::<DocumentHighlightParams>(params)?)
                .await?,
        ),
        "completionItem/resolve" => to_json(
            &backend
                .completion_resolve(parse_params::<CompletionItem>(params)?)
                .await?,
        ),
        "textDocument/documentSymbol" => to_json(
            &backend
                .document_symbol(parse_params::<DocumentSymbolParams>(params)?)
                .await?,
        ),
        "workspace/symbol" => to_json(
            &backend
                .symbol(parse_params::<WorkspaceSymbolParams>(params)?)
                .await?,
        ),
        "workspaceSymbol/resolve" => to_json(
            &backend
                .workspace_symbol_resolve(parse_params::<WorkspaceSymbol>(params)?)
                .await?,
        ),
        "textDocument/signatureHelp" => to_json(
            &backend
                .signature_help(parse_params::<SignatureHelpParams>(params)?)
                .await?,
        ),
        "textDocument/codeAction" => to_json(
            &backend
                .code_action(parse_params::<CodeActionParams>(params)?)
                .await?,
        ),
        "codeAction/resolve" => to_json(
            &backend
                .code_action_resolve(parse_params::<CodeAction>(params)?)
                .await?,
        ),
        "textDocument/rename" => to_json(
            &backend
                .rename(parse_params::<RenameParams>(params)?)
                .await?,
        ),
        "textDocument/prepareRename" => to_json(
            &backend
                .prepare_rename(parse_params::<TextDocumentPositionParams>(params)?)
                .await?,
        ),
        "workspace/executeCommand" => to_json(
            &backend
                .execute_command(parse_params::<ExecuteCommandParams>(params)?)
                .await?,
        ),
        "textDocument/formatting" => to_json(
            &backend
                .formatting(parse_params::<DocumentFormattingParams>(params)?)
                .await?,
        ),
        "textDocument/rangeFormatting" => to_json(
            &backend
                .range_formatting(parse_params::<DocumentRangeFormattingParams>(params)?)
                .await?,
        ),
        "textDocument/onTypeFormatting" => to_json(
            &backend
                .on_type_formatting(parse_params::<DocumentOnTypeFormattingParams>(params)?)
                .await?,
        ),
        "textDocument/foldingRange" => to_json(
            &backend
                .folding_range(parse_params::<FoldingRangeParams>(params)?)
                .await?,
        ),
        "textDocument/selectionRange" => to_json(
            &backend
                .selection_range(parse_params::<SelectionRangeParams>(params)?)
                .await?,
        ),
        "textDocument/codeLens" => to_json(
            &backend
                .code_lens(parse_params::<CodeLensParams>(params)?)
                .await?,
        ),
        "codeLens/resolve" => to_json(
            &backend
                .code_lens_resolve(parse_params::<CodeLens>(params)?)
                .await?,
        ),
        "textDocument/documentLink" => to_json(
            &backend
                .document_link(parse_params::<DocumentLinkParams>(params)?)
                .await?,
        ),
        "documentLink/resolve" => to_json(
            &backend
                .document_link_resolve(parse_params::<DocumentLink>(params)?)
                .await?,
        ),
        "textDocument/documentColor" => to_json(
            &backend
                .document_color(parse_params::<DocumentColorParams>(params)?)
                .await?,
        ),
        "textDocument/colorPresentation" => to_json(
            &backend
                .color_presentation(parse_params::<ColorPresentationParams>(params)?)
                .await?,
        ),
        "textDocument/semanticTokens/full" => to_json(
            &backend
                .semantic_tokens_full(parse_params::<SemanticTokensParams>(params)?)
                .await?,
        ),
        "textDocument/semanticTokens/full/delta" => to_json(
            &backend
                .semantic_tokens_full_delta(parse_params::<SemanticTokensDeltaParams>(params)?)
                .await?,
        ),
        "textDocument/semanticTokens/range" => to_json(
            &backend
                .semantic_tokens_range(parse_params::<SemanticTokensRangeParams>(params)?)
                .await?,
        ),
        "textDocument/inlayHint" => to_json(
            &backend
                .inlay_hint(parse_params::<InlayHintParams>(params)?)
                .await?,
        ),
        "inlayHint/resolve" => to_json(
            &backend
                .inlay_hint_resolve(parse_params::<InlayHint>(params)?)
                .await?,
        ),
        "textDocument/diagnostic" => to_json(
            &backend
                .diagnostic(parse_params::<DocumentDiagnosticParams>(params)?)
                .await?,
        ),
        "workspace/diagnostic" => to_json(
            &backend
                .workspace_diagnostic(parse_params::<WorkspaceDiagnosticParams>(params)?)
                .await?,
        ),
        "textDocument/willSaveWaitUntil" => to_json(
            &backend
                .will_save_wait_until(parse_params::<WillSaveTextDocumentParams>(params)?)
                .await?,
        ),
        "workspace/willCreateFiles" => to_json(
            &backend
                .will_create_files(parse_params::<CreateFilesParams>(params)?)
                .await?,
        ),
        "workspace/willRenameFiles" => to_json(
            &backend
                .will_rename_files(parse_params::<RenameFilesParams>(params)?)
                .await?,
        ),
        "workspace/willDeleteFiles" => to_json(
            &backend
                .will_delete_files(parse_params::<DeleteFilesParams>(params)?)
                .await?,
        ),
        "textDocument/prepareCallHierarchy" => to_json(
            &backend
                .prepare_call_hierarchy(parse_params::<CallHierarchyPrepareParams>(params)?)
                .await?,
        ),
        "callHierarchy/incomingCalls" => to_json(
            &backend
                .incoming_calls(parse_params::<CallHierarchyIncomingCallsParams>(params)?)
                .await?,
        ),
        "callHierarchy/outgoingCalls" => to_json(
            &backend
                .outgoing_calls(parse_params::<CallHierarchyOutgoingCallsParams>(params)?)
                .await?,
        ),
        "textDocument/prepareTypeHierarchy" => to_json(
            &backend
                .prepare_type_hierarchy(parse_params::<TypeHierarchyPrepareParams>(params)?)
                .await?,
        ),
        "typeHierarchy/supertypes" => to_json(
            &backend
                .supertypes(parse_params::<TypeHierarchySupertypesParams>(params)?)
                .await?,
        ),
        "typeHierarchy/subtypes" => to_json(
            &backend
                .subtypes(parse_params::<TypeHierarchySubtypesParams>(params)?)
                .await?,
        ),
        "textDocument/moniker" => to_json(
            &backend
                .moniker(parse_params::<MonikerParams>(params)?)
                .await?,
        ),
        "textDocument/linkedEditingRange" => to_json(
            &backend
                .linked_editing_range(parse_params::<LinkedEditingRangeParams>(params)?)
                .await?,
        ),
        "textDocument/inlineValue" => to_json(
            &backend
                .inline_value(parse_params::<InlineValueParams>(params)?)
                .await?,
        ),
        "textDocument/inlineCompletion" => to_json(
            &backend
                .inline_completion(parse_params::<InlineCompletionParams>(params)?)
                .await?,
        ),
        _ => backend.handle_request(method, params).await,
    }
}

/// Route a notification to the matching typed handler, falling back to the
/// untyped [`LanguageServer::handle_notification`] escape hatch.
///
/// Malformed params for a known notification are logged (notifications have no
/// response channel) rather than silently dropped.
async fn dispatch_notification<B: LanguageServer>(
    backend: &B,
    client: &Client,
    method: &str,
    params: Option<Value>,
) {
    match method {
        "textDocument/didOpen" => match parse_params::<DidOpenTextDocumentParams>(params) {
            Ok(p) => backend.did_open(p).await,
            Err(err) => log_bad_params(client, method, &err),
        },
        "textDocument/didChange" => match parse_params::<DidChangeTextDocumentParams>(params) {
            Ok(p) => backend.did_change(p).await,
            Err(err) => log_bad_params(client, method, &err),
        },
        "textDocument/didClose" => match parse_params::<DidCloseTextDocumentParams>(params) {
            Ok(p) => backend.did_close(p).await,
            Err(err) => log_bad_params(client, method, &err),
        },
        "textDocument/didSave" => match parse_params::<DidSaveTextDocumentParams>(params) {
            Ok(p) => backend.did_save(p).await,
            Err(err) => log_bad_params(client, method, &err),
        },
        "workspace/didChangeWorkspaceFolders" => {
            match parse_params::<DidChangeWorkspaceFoldersParams>(params) {
                Ok(p) => backend.did_change_workspace_folders(p).await,
                Err(err) => log_bad_params(client, method, &err),
            }
        }
        "window/workDoneProgress/cancel" => {
            match parse_params::<WorkDoneProgressCancelParams>(params) {
                Ok(p) => backend.work_done_progress_cancel(p).await,
                Err(err) => log_bad_params(client, method, &err),
            }
        }
        "workspace/didChangeConfiguration" => {
            match parse_params::<DidChangeConfigurationParams>(params) {
                Ok(p) => backend.did_change_configuration(p).await,
                Err(err) => log_bad_params(client, method, &err),
            }
        }
        "workspace/didChangeWatchedFiles" => {
            match parse_params::<DidChangeWatchedFilesParams>(params) {
                Ok(p) => backend.did_change_watched_files(p).await,
                Err(err) => log_bad_params(client, method, &err),
            }
        }
        "textDocument/willSave" => match parse_params::<WillSaveTextDocumentParams>(params) {
            Ok(p) => backend.will_save(p).await,
            Err(err) => log_bad_params(client, method, &err),
        },
        "workspace/didCreateFiles" => match parse_params::<CreateFilesParams>(params) {
            Ok(p) => backend.did_create_files(p).await,
            Err(err) => log_bad_params(client, method, &err),
        },
        "workspace/didRenameFiles" => match parse_params::<RenameFilesParams>(params) {
            Ok(p) => backend.did_rename_files(p).await,
            Err(err) => log_bad_params(client, method, &err),
        },
        "workspace/didDeleteFiles" => match parse_params::<DeleteFilesParams>(params) {
            Ok(p) => backend.did_delete_files(p).await,
            Err(err) => log_bad_params(client, method, &err),
        },
        "$/setTrace" => match parse_params::<SetTraceParams>(params) {
            Ok(p) => backend.set_trace(p).await,
            Err(err) => log_bad_params(client, method, &err),
        },
        "notebookDocument/didOpen" => match parse_params::<DidOpenNotebookDocumentParams>(params) {
            Ok(p) => backend.did_open_notebook_document(p).await,
            Err(err) => log_bad_params(client, method, &err),
        },
        "notebookDocument/didChange" => {
            match parse_params::<DidChangeNotebookDocumentParams>(params) {
                Ok(p) => backend.did_change_notebook_document(p).await,
                Err(err) => log_bad_params(client, method, &err),
            }
        }
        "notebookDocument/didSave" => match parse_params::<DidSaveNotebookDocumentParams>(params) {
            Ok(p) => backend.did_save_notebook_document(p).await,
            Err(err) => log_bad_params(client, method, &err),
        },
        "notebookDocument/didClose" => {
            match parse_params::<DidCloseNotebookDocumentParams>(params) {
                Ok(p) => backend.did_close_notebook_document(p).await,
                Err(err) => log_bad_params(client, method, &err),
            }
        }
        _ => backend.handle_notification(method, params).await,
    }
}

/// Log a parameter-decoding failure for a notification.
fn log_bad_params(client: &Client, method: &str, err: &Error) {
    let _ = client.log_message(
        MessageType::Warning,
        format!("ignoring `{method}` with invalid params: {err}"),
    );
}

/// Deserialize request/notification params, mapping failures to
/// [`Error::invalid_params`]. Per JSON-RPC, `params` is optional; absent
/// params are treated as an empty JSON object so structs whose fields are
/// all optional/defaultable still decode successfully.
fn parse_params<T: DeserializeOwned>(params: Option<Value>) -> Result<T> {
    serde_json::from_value(params.unwrap_or_else(|| Value::Object(Default::default())))
        .map_err(|e| Error::invalid_params(e.to_string()))
}

/// Serialize a handler result into the JSON value sent on the wire.
fn to_json<T: Serialize>(value: &T) -> Result<Value> {
    serde_json::to_value(value).map_err(Error::from)
}

/// Enqueue an error response for `id`.
fn send_error(out_tx: &Outbound, id: RequestId, err: Error) {
    let _ = out_tx.send(Response::error(Some(id), err.into_response_error()).into());
}

/// Lock a mutex, recovering the guard if a panicking task poisoned it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Default, PartialEq, Deserialize)]
    struct AllOptionalParams {
        #[serde(default)]
        foo: Option<i32>,
    }

    #[test]
    fn parse_params_treats_absent_params_as_empty_object() {
        let parsed: AllOptionalParams = parse_params(None).expect("absent params should default");
        assert_eq!(parsed, AllOptionalParams::default());
    }

    #[test]
    fn parse_params_still_rejects_wrong_shape() {
        let err = parse_params::<AllOptionalParams>(Some(Value::Bool(true)))
            .expect_err("a bool is not a valid params object");
        assert!(matches!(err, Error::Response(e) if e.code == crate::error::codes::INVALID_PARAMS));
    }
}
