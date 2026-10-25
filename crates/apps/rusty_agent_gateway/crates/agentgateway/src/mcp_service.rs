//! The federation's MCP handler as a tower service on the gateway's tokio
//! runtime (#875: it replaces `rusty_mcp_axum` here).
//!
//! The handler is blocking and written for `rusty_serve`, so each request is
//! handed to a blocking thread: the request is copied across, the handler
//! runs, and a streamed reply (server-sent events) is pumped from that same
//! thread into the response body. A client that hangs up drops the body,
//! which ends the pump and, with it, the handler's work for that request.

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use bytes::Bytes;
use http::request::Parts;
use http::{HeaderName, HeaderValue, Request, Response, StatusCode, header};
use http_body_util::BodyExt as _;
use hyper::body::{Body as HttpBody, Frame};
use rusty_mcp_server::HttpHandler;
use rusty_mcp_server::json::Value;
use rusty_serve::{Body as ServeBody, Request as ServeRequest};
use tokio::sync::{mpsc, oneshot};

use crate::body::{Body, BoxError};

/// Chunks buffered between the handler's thread and the connection.
const STREAM_BUFFER: usize = 8;

/// Reads who made a request from what an earlier layer left on it.
pub type PrincipalOf = fn(&Parts) -> Option<Value>;

/// Answers every request with `handler`, as if addressed to `path` (its query
/// kept). Cloning is cheap, and clones share the handler's session manager.
#[derive(Clone)]
pub struct McpService {
    handler: Arc<HttpHandler>,
    path: Arc<str>,
    max_body_bytes: usize,
    principal_of: PrincipalOf,
}

impl McpService {
    /// Serves `handler` at `path`, refusing request bodies over
    /// `max_body_bytes`. Each request's principal is read by `principal_of`.
    pub fn new(
        handler: Arc<HttpHandler>,
        path: &str,
        max_body_bytes: usize,
        principal_of: PrincipalOf,
    ) -> Self {
        McpService {
            handler,
            path: path.into(),
            max_body_bytes,
            principal_of,
        }
    }

    async fn serve<B>(self, request: Request<B>) -> Response<Body>
    where
        B: HttpBody<Data = Bytes> + Send + 'static,
        B::Error: Into<BoxError>,
    {
        let (parts, body) = request.into_parts();
        let target = match parts.uri.query() {
            Some(query) => format!("{}?{query}", self.path),
            None => self.path.to_string(),
        };
        let principal = (self.principal_of)(&parts);
        let Ok(body) = http_body_util::Limited::new(body, self.max_body_bytes)
            .collect()
            .await
        else {
            return plain(StatusCode::PAYLOAD_TOO_LARGE, "request body too large");
        };
        let body = body.to_bytes();
        let mut headers = rusty_http::HeaderMap::new();
        for (name, value) in &parts.headers {
            // A header that is not text cannot matter to the MCP handler.
            if let Ok(value) = value.to_str() {
                let _ = headers.append(name.as_str(), value);
            }
        }
        let method = rusty_http::Method::parse(parts.method.as_str());
        let handler = self.handler;
        let (head_tx, head_rx) = oneshot::channel();
        tokio::task::spawn_blocking(move || {
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
            Err(_) => plain(StatusCode::INTERNAL_SERVER_ERROR, "internal error"),
        }
    }
}

impl<B> tower_service::Service<Request<B>> for McpService
where
    B: HttpBody<Data = Bytes> + Send + 'static,
    B::Error: Into<BoxError>,
{
    type Response = Response<Body>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Response<Body>, Infallible>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Request<B>) -> Self::Future {
        let this = self.clone();
        Box::pin(async move { Ok(this.serve(request).await) })
    }
}

/// What the blocking thread hands back first.
struct Head {
    status: u16,
    headers: rusty_http::HeaderMap,
    content_type: &'static str,
    body: Payload,
}

enum Payload {
    Whole(Vec<u8>),
    Stream(mpsc::Receiver<Bytes>),
}

/// Runs the handler and delivers its reply: whole replies at once, a stream
/// as head first and chunks after.
fn run(
    handler: &HttpHandler,
    request: &ServeRequest<'_>,
    principal: Option<Value>,
    head_tx: oneshot::Sender<Head>,
) {
    let response = handler.handle_with(request, principal);
    let mut head = Head {
        status: response.status.as_u16(),
        headers: response.headers,
        content_type: "application/json",
        body: Payload::Whole(Vec::new()),
    };
    match response.body {
        ServeBody::Json(bytes) => head.body = Payload::Whole(bytes),
        ServeBody::Deferred(job) => {
            let (status, bytes) = job();
            head.status = status.as_u16();
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
                if tx.blocking_send(Bytes::from(chunk)).is_err() {
                    break; // the client went away
                }
            }
            return;
        }
    }
    let _ = head_tx.send(head);
}

/// A streamed reply's chunks, as they arrive from the handler's thread.
struct Chunks(mpsc::Receiver<Bytes>);

impl HttpBody for Chunks {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        self.0
            .poll_recv(cx)
            .map(|chunk| chunk.map(|bytes| Ok(Frame::data(bytes))))
    }
}

fn respond(head: Head) -> Response<Body> {
    let status = StatusCode::from_u16(head.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = match head.body {
        Payload::Whole(bytes) => Body::from(Bytes::from(bytes)),
        Payload::Stream(rx) => Body::new(Chunks(rx)),
    };
    let mut response = Response::new(body);
    *response.status_mut() = status;
    let out = response.headers_mut();
    out.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(head.content_type),
    );
    out.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    out.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    // The handler's own headers replace the defaults above.
    for (name, value) in head.headers.iter() {
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            out.insert(name, value);
        }
    }
    response
}

fn plain(status: StatusCode, message: &'static str) -> Response<Body> {
    let mut response = Response::new(Body::from(Bytes::from_static(message.as_bytes())));
    *response.status_mut() = status;
    response
}
