//! Serves the [`HttpHandler`] from inside a `rusty_axum` app (feature
//! `rusty-axum`). It replaces the `rusty_mcp_axum` bridge into third-party
//! axum (#875).
//!
//! The handler is blocking, so each request runs on a `rusty_tokio` blocking
//! thread: the request is moved across, the handler runs, and a streamed
//! reply (server-sent events) is pumped from that same thread into the
//! response body. A client that hangs up drops the body, which ends the pump
//! and, with it, the handler's work for that request.
//!
//! ```ignore
//! let mcp = Mount::new(handler);
//! let app = rusty_axum::Router::new()
//!     .route("/health", get(|| async { "ok" }))
//!     .route("/mcp", any(move |req| mcp.call(req)));
//! ```
//!
//! A guard for the MCP route alone wraps that closure: check the request,
//! then `mcp.call(req)` or answer the refusal. The request body was already
//! read whole, within the server's `max_body_bytes`.

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use rusty_axum::{Body, BoxFuture, Request, Response, StatusCode, Stream};
use rusty_http::HeaderMap;
use rusty_serve::{Body as ServeBody, Request as ServeRequest};
use rusty_tokio::sync::{mpsc, oneshot};

use crate::http::HttpHandler;
use crate::json::Value;

/// Chunks buffered between the handler's thread and the connection.
const STREAM_BUFFER: usize = 8;

/// The MCP endpoint as a `rusty_axum` handler. Cloning is cheap.
#[derive(Clone)]
pub struct Mount {
    handler: Arc<HttpHandler>,
    path: Option<Arc<str>>,
}

impl Mount {
    /// Serves `handler` at the path each request arrives on, which must be
    /// the one its [`HttpConfig`](crate::HttpConfig) names.
    pub fn new(handler: Arc<HttpHandler>) -> Self {
        Mount {
            handler,
            path: None,
        }
    }

    /// Treats every request as addressed to `path` (its query kept), whatever
    /// path it arrived on: for an app that routes the endpoint somewhere
    /// else, such as under a secret in the path, or as a fallback.
    pub fn at(mut self, path: &str) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Answers `req`.
    pub fn call(&self, req: Request) -> BoxFuture<'static, Response> {
        self.call_as(req, None)
    }

    /// Answers `req` on behalf of `principal`, which the handlers see as
    /// [`Caller::principal`](crate::Caller::principal). Deciding who the
    /// caller is stays the application's business; this only carries it.
    pub fn call_as(
        &self,
        mut req: Request,
        principal: Option<Value>,
    ) -> BoxFuture<'static, Response> {
        let handler = Arc::clone(&self.handler);
        let target = match (&self.path, req.query()) {
            (Some(path), Some(query)) => format!("{path}?{query}"),
            (Some(path), None) => path.to_string(),
            (None, _) => req.target().to_owned(),
        };
        let method = req.method().clone();
        let headers = req.headers().clone();
        let body = req.take_body();
        Box::pin(async move {
            let (head_tx, head_rx) = oneshot::channel();
            rusty_tokio::spawn_blocking(move || {
                let request = ServeRequest {
                    method: &method,
                    target: &target,
                    authorization: headers.get("authorization"),
                    if_match: headers.get("if-match"),
                    headers: &headers,
                    body: &body,
                };
                run(&handler, &request, principal, head_tx);
            });
            match head_rx.await {
                Ok(head) => respond(head),
                // The handler panicked before answering.
                Err(_) => Response::new(StatusCode::INTERNAL_SERVER_ERROR, "internal error"),
            }
        })
    }
}

/// What the blocking thread hands back first.
struct Head {
    status: StatusCode,
    headers: HeaderMap,
    content_type: &'static str,
    body: Payload,
}

enum Payload {
    Whole(Vec<u8>),
    Stream(mpsc::Receiver<Vec<u8>>),
}

/// Runs the handler and delivers its reply: a whole reply at once, a stream
/// as its head first and its chunks after.
fn run(
    handler: &HttpHandler,
    request: &ServeRequest<'_>,
    principal: Option<Value>,
    head_tx: oneshot::Sender<Head>,
) {
    let response = handler.handle_with(request, principal);
    let mut head = Head {
        status: response.status,
        headers: response.headers,
        content_type: "application/json",
        body: Payload::Whole(Vec::new()),
    };
    match response.body {
        ServeBody::Json(bytes) => head.body = Payload::Whole(bytes),
        ServeBody::Deferred(job) => {
            let (status, bytes) = job();
            head.status = status;
            head.body = Payload::Whole(bytes);
        }
        ServeBody::Stream {
            content_type,
            chunks,
        } => {
            let (tx, rx) = mpsc::channel(STREAM_BUFFER);
            head.content_type = content_type;
            head.body = Payload::Stream(rx);
            if head_tx.send(head).is_err() {
                return;
            }
            for chunk in chunks {
                if tx.blocking_send(chunk).is_err() {
                    break; // the client went away
                }
            }
            return;
        }
    }
    let _ = head_tx.send(head);
}

fn respond(head: Head) -> Response {
    let body = match head.body {
        Payload::Whole(bytes) => Body::from(bytes),
        Payload::Stream(rx) => Body::from_stream(Chunks::new(rx)),
    };
    let mut response = Response::new(head.status, body)
        .with_header("content-type", head.content_type)
        .with_header("cache-control", "no-store")
        .with_header("x-content-type-options", "nosniff");
    // The handler's own headers replace the defaults above. Its headers
    // were valid where it built them, so a failed insert is unreachable.
    for (name, value) in head.headers.iter() {
        let _ = response.headers.insert(name, value);
    }
    response
}

type Recv = Pin<Box<dyn Future<Output = (Option<Vec<u8>>, mpsc::Receiver<Vec<u8>>)> + Send>>;

/// The pump's channel as the body's stream.
struct Chunks(Option<Recv>);

impl Chunks {
    fn new(rx: mpsc::Receiver<Vec<u8>>) -> Self {
        Chunks(Some(recv(rx)))
    }
}

fn recv(mut rx: mpsc::Receiver<Vec<u8>>) -> Recv {
    Box::pin(async move { (rx.recv().await, rx) })
}

impl Stream for Chunks {
    type Item = Result<Vec<u8>, Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let Some(next) = self.0.as_mut() else {
            return Poll::Ready(None);
        };
        let Poll::Ready((chunk, rx)) = next.as_mut().poll(cx) else {
            return Poll::Pending;
        };
        self.0 = chunk.is_some().then(|| recv(rx));
        Poll::Ready(chunk.map(Ok))
    }
}
