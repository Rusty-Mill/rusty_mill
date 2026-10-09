//! Serves `rusty_mcp_server`'s HTTP handler from inside an axum app.
//!
//! The handler is blocking and written for `rusty_serve`, so each request is
//! handed to a blocking thread: the request is copied across, the handler
//! runs, and a streamed reply (server-sent events) is pumped from that same
//! thread into the response body. A client that hangs up drops the body,
//! which ends the pump and, with it, the handler's work for that request.

use std::convert::Infallible;
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::OriginalUri;
use axum::http::{header, HeaderName, HeaderValue, StatusCode};
use axum::response::Response;
use axum::Router;
use rusty_http::{HeaderMap, Method};
use rusty_mcp_server::HttpHandler;
use rusty_serve::{Body as ServeBody, Request, SharedHandler};
use tokio::sync::{mpsc, oneshot};

/// Chunks buffered between the handler's thread and the connection.
const STREAM_BUFFER: usize = 8;

/// What the blocking thread hands back first.
struct Head {
    status: u16,
    headers: HeaderMap,
    content_type: &'static str,
    body: Payload,
}

enum Payload {
    Whole(Vec<u8>),
    Stream(mpsc::Receiver<Bytes>),
}

/// A router answering every request with `handler`, whose request bodies may
/// be at most `max_body_bytes`. Put layers on it, then nest it at the path
/// `handler` serves (see the crate docs).
pub fn router(handler: Arc<HttpHandler>, max_body_bytes: usize) -> Router {
    Router::new().fallback(
        move |OriginalUri(uri): OriginalUri, request: axum::extract::Request| {
            let target = uri
                .path_and_query()
                .map_or_else(|| uri.path().to_owned(), |pq| pq.as_str().to_owned());
            serve(Arc::clone(&handler), target, request, max_body_bytes)
        },
    )
}

/// Like [`router`], but every request is treated as addressed to `path`
/// (its query string kept), whatever path it arrived on. For an app that
/// rewrites or strips paths itself before the request reaches the mount, such
/// as one that carries a secret in the path.
pub fn router_at(handler: Arc<HttpHandler>, max_body_bytes: usize, path: &str) -> Router {
    let path = path.to_owned();
    Router::new().fallback(move |request: axum::extract::Request| {
        let target = request
            .uri()
            .query()
            .map_or_else(|| path.clone(), |q| format!("{path}?{q}"));
        serve(Arc::clone(&handler), target, request, max_body_bytes)
    })
}

/// Answer `request` (whose path was `target` before any mount prefix was
/// stripped) with `handler`. `max_body_bytes` bounds the request body.
async fn serve(
    handler: Arc<HttpHandler>,
    target: String,
    request: axum::extract::Request,
    max_body_bytes: usize,
) -> Response {
    let (parts, body) = request.into_parts();
    let Ok(body) = axum::body::to_bytes(body, max_body_bytes).await else {
        return plain(StatusCode::PAYLOAD_TOO_LARGE, "request body too large");
    };
    let mut headers = HeaderMap::new();
    for (name, value) in &parts.headers {
        // A header that is not text cannot matter to the MCP handler.
        if let Ok(value) = value.to_str() {
            let _ = headers.append(name.as_str(), value);
        }
    }
    let method = Method::parse(parts.method.as_str());
    let (head_tx, head_rx) = oneshot::channel();
    tokio::task::spawn_blocking(move || {
        let request = Request {
            method: &method,
            target: &target,
            authorization: headers.get("authorization"),
            if_match: headers.get("if-match"),
            headers: &headers,
            body: &body,
        };
        run(&handler, &request, head_tx);
    });
    match head_rx.await {
        Ok(head) => respond(head),
        Err(_) => plain(StatusCode::INTERNAL_SERVER_ERROR, "internal error"),
    }
}

/// Run the handler and deliver its reply: whole replies at once, a stream
/// as head first and chunks after.
fn run(handler: &HttpHandler, request: &Request<'_>, head_tx: oneshot::Sender<Head>) {
    let response = handler.handle(request);
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

fn respond(head: Head) -> Response {
    let status = StatusCode::from_u16(head.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = match head.body {
        Payload::Whole(bytes) => Body::from(bytes),
        Payload::Stream(rx) => {
            Body::from_stream(futures::stream::unfold(rx, |mut rx| async move {
                rx.recv()
                    .await
                    .map(|chunk| (Ok::<_, Infallible>(chunk), rx))
            }))
        }
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

fn plain(status: StatusCode, message: &'static str) -> Response {
    let mut response = Response::new(Body::from(message));
    *response.status_mut() = status;
    response
}
