//! Runs the gRPC transport for callers on `tokio`.
//!
//! The first-party gRPC stack (`rusty_tonic` over `rusty_hyper` and `rusty_h2`) is async on
//! `rusty_tokio`, its own runtime. This crate's callers, its engine, `axum` and `reqwest` run on
//! `tokio`. The two meet here and nowhere else, by message passing only: a task on the transport's
//! runtime is awaited from `tokio` through its `JoinHandle`, and a stream crosses through a
//! `tokio::sync::mpsc` channel. Neither side enters the other's runtime context.
//!
//! Dropping the future a caller was awaiting, or the stream it was reading, aborts the transport
//! task, which drops the call and so resets its HTTP/2 stream: cancelling a call from `tokio`
//! cancels it on the wire.
//!
//! When this crate leaves `tokio` altogether, this module goes with it.

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;
use std::task::{Context, Poll};

use futures_core::Stream;
use rusty_prost::Message;
use rusty_tonic::client::{Channel, Request};
use rusty_tonic::Status;

/// Replies read ahead of a slow consumer. HTTP/2 flow control pushes back on the server beyond it.
const PREFETCH: usize = 16;

/// Threads of the transport's runtime: it moves bytes and frames, nothing heavier.
const WORKERS: usize = 2;

/// The process-wide runtime the transport runs on, started on first use.
fn runtime() -> Result<&'static rusty_tokio::Runtime, Status> {
    static RUNTIME: OnceLock<Result<rusty_tokio::Runtime, String>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            rusty_tokio::Runtime::builder()
                .worker_threads(WORKERS)
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| Status::unavailable(format!("could not start the gRPC transport: {e}")))
}

/// A task on the transport's runtime, awaited from anywhere and aborted when dropped.
struct Task<T>(rusty_tokio::task::JoinHandle<T>);

impl<T> Drop for Task<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl<T> Future for Task<T> {
    type Output = Result<T, Status>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.0).poll(cx).map(|joined| {
            joined.map_err(|e| Status::internal(format!("the gRPC transport task failed: {e}")))
        })
    }
}

/// One request, one reply.
pub(crate) async fn unary<Req, Resp>(
    channel: &Channel,
    path: &'static str,
    request: Request<Req>,
) -> Result<Resp, Status>
where
    Req: Message + Send + 'static,
    Resp: Message + Send + 'static,
{
    let channel = channel.clone();
    let task = Task(runtime()?.spawn(async move {
        channel
            .unary::<Req, Resp>(path, request)
            .await
            .map(|reply| reply.message)
    }));
    task.await?
}

/// The replies of a server-streaming call. Dropping it cancels the call.
pub(crate) struct Replies<T> {
    rx: tokio::sync::mpsc::Receiver<Result<T, Status>>,
    _task: Task<()>,
}

impl<T> Stream for Replies<T> {
    type Item = Result<T, Status>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

/// One request, a stream of replies. Returns once the response has started, so a call the server
/// refuses outright fails here, not on the first read.
pub(crate) async fn server_streaming<Req, Resp>(
    channel: &Channel,
    path: &'static str,
    request: Request<Req>,
) -> Result<Replies<Resp>, Status>
where
    Req: Message + Send + 'static,
    Resp: Message + Send + 'static,
{
    let channel = channel.clone();
    let (opened_tx, opened_rx) = tokio::sync::oneshot::channel::<Result<(), Status>>();
    let (tx, rx) = tokio::sync::mpsc::channel(PREFETCH);
    let task = Task(runtime()?.spawn(async move {
        let mut stream = match channel.server_streaming::<Req, Resp>(path, request).await {
            Ok(stream) => {
                let _ = opened_tx.send(Ok(()));
                stream
            }
            Err(status) => {
                let _ = opened_tx.send(Err(status));
                return;
            }
        };
        loop {
            let item = match stream.message().await {
                Ok(Some(message)) => Ok(message),
                Ok(None) => return,
                Err(status) => Err(status),
            };
            let failed = item.is_err();
            // A closed channel means the consumer dropped the stream: stop, and cancel the call.
            if tx.send(item).await.is_err() || failed {
                return;
            }
        }
    }));
    // `task` is held across this wait, so a caller that gives up here cancels the call too.
    match opened_rx.await {
        Ok(Ok(())) => Ok(Replies { rx, _task: task }),
        Ok(Err(status)) => Err(status),
        Err(_) => Err(Status::internal(
            "the gRPC transport task ended before the call opened",
        )),
    }
}
