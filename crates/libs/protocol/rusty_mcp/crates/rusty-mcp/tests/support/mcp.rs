//! A `rusty_mcp_server` handler as a tower service in an axum router, the
//! stack these layers sit in (#875: it replaces `rusty_mcp_axum` here).
//!
//! The handler is blocking, so each request runs on a blocking thread, and a
//! streamed reply is pumped from that thread into the response body. A client
//! that hangs up drops the body, which ends the pump.

use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::Router;
use axum::body::{Body, Bytes, HttpBody};
use axum::http::request::Parts;
use axum::http::{HeaderName, HeaderValue, Request, Response, StatusCode, header};
use http_body::Frame;
use rusty_mcp_server::HttpHandler;
use rusty_mcp_server::json::Value;
use rusty_serve::{Body as ServeBody, Request as ServeRequest};
use tokio::sync::{mpsc, oneshot};

/// The largest request body accepted.
const MAX_BODY_BYTES: usize = 1 << 20;

/// Chunks buffered between the handler's thread and the connection.
const STREAM_BUFFER: usize = 8;

/// A router answering every request with `handler`, as if addressed to
/// `path` (its query kept). The caller's principal is the `sub` of a
/// `VerifiedToken` an earlier auth layer left in the request extensions.
pub fn router(handler: Arc<HttpHandler>, path: &str) -> Router {
    let path: Arc<str> = path.into();
    Router::new().fallback(move |request: Request<Body>| {
        let (handler, path) = (Arc::clone(&handler), Arc::clone(&path));
        async move { serve(handler, &path, request).await }
    })
}

fn principal_of(parts: &Parts) -> Option<Value> {
    let token = parts.extensions.get::<rusty_mcp::auth::VerifiedToken>()?;
    let mut principal = Value::object();
    principal.insert("sub", token.subject.clone()?);
    Some(principal)
}

async fn serve(handler: Arc<HttpHandler>, path: &str, request: Request<Body>) -> Response<Body> {
    let (parts, body) = request.into_parts();
    let target = match parts.uri.query() {
        Some(query) => format!("{path}?{query}"),
        None => path.to_owned(),
    };
    let principal = principal_of(&parts);
    let Ok(body) = axum::body::to_bytes(body, MAX_BODY_BYTES).await else {
        return plain(StatusCode::PAYLOAD_TOO_LARGE, "request body too large");
    };
    let mut headers = rusty_http::HeaderMap::new();
    for (name, value) in &parts.headers {
        if let Ok(value) = value.to_str() {
            let _ = headers.append(name.as_str(), value);
        }
    }
    let method = rusty_http::Method::parse(parts.method.as_str());
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
        Err(_) => plain(StatusCode::INTERNAL_SERVER_ERROR, "internal error"),
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
    let body = match head.body {
        Payload::Whole(bytes) => Body::from(bytes),
        Payload::Stream(rx) => Body::new(Chunks(rx)),
    };
    let mut response = Response::new(body);
    *response.status_mut() =
        StatusCode::from_u16(head.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
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
    let mut response = Response::new(Body::from(message));
    *response.status_mut() = status;
    response
}
