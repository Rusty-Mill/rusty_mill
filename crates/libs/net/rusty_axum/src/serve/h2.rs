//! HTTP/2 on a TLS connection that negotiated ALPN `h2` (feature `http2`,
//! #885). Each stream becomes the same buffered [`Request`] as on HTTP/1.1,
//! answered by the same [`Router`](crate::Router); the connection itself
//! (preface deadline, GOAWAY, drain) is `rusty_hyper::serve_connection`'s.

use std::future::poll_fn;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rusty_http::{HeaderMap, Method, StatusCode};
use rusty_hyper::{Body as Piece, HeaderField, Respond, SendStream};
use rusty_tokio::io::{AsyncRead, AsyncWrite};
use rusty_tokio::time::{timeout, Instant};

use super::{until_stopped, Shared};
use crate::body::{BodyStream, Inner};
use crate::request::Request;
use crate::response::Response;
use crate::stop::Stop;

/// Header fields that HTTP/2 forbids (RFC 9113 §8.2.2).
const CONNECTION_SPECIFIC: [&str; 5] = [
    "connection",
    "keep-alive",
    "proxy-connection",
    "transfer-encoding",
    "upgrade",
];

/// Serves `io` as HTTP/2 until the connection ends; the client must have
/// sent its preface by `deadline`.
pub(super) async fn run<T>(
    io: T,
    peer: SocketAddr,
    shared: Arc<Shared>,
    stop: Arc<Stop>,
    deadline: Instant,
) where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let stopped = stop.wait();
    let handler = move |req: rusty_hyper::Request, respond: Respond| {
        let (shared, stop) = (shared.clone(), stop.clone());
        async move { stream(req, respond, peer, &shared, &stop).await }
    };
    // The default HTTP/2 settings, as `rusty_hyper::serve` uses.
    let settings = rusty_hyper::ServeOptions::default().h2;
    rusty_hyper::serve_connection(io, settings, deadline, stopped, handler).await;
}

/// Answers one stream. A client that resets it drops the handler, and with
/// it a streamed body, as a hang-up does on HTTP/1.1.
async fn stream(
    req: rusty_hyper::Request,
    respond: Respond,
    peer: SocketAddr,
    shared: &Shared,
    stop: &Arc<Stop>,
) {
    let closed = respond.closed();
    let work = async {
        let options = &shared.options;
        let (response, head_only) =
            match read(req, peer, options.max_body_bytes, options.idle_timeout).await {
                Ok(req) => {
                    let head_only = *req.method() == Method::Head;
                    (shared.router.call(req).await, head_only)
                }
                Err(status) => (Response::new(status, Vec::new()), false),
            };
        answer(respond, response, head_only, stop).await;
    };
    rusty_tokio::select! {
        biased;
        _ = closed => {}
        _ = work => {}
    }
}

/// The request on a stream, its body read whole within `max` bytes, each
/// read within `idle`. The status to refuse it with otherwise.
async fn read(
    mut req: rusty_hyper::Request,
    peer: SocketAddr,
    max: u64,
    idle: Duration,
) -> Result<Request, StatusCode> {
    let (method, target, headers) = head(&req.headers)?;
    let mut body = Vec::new();
    loop {
        let piece = timeout(idle, req.body.next()).await;
        match piece.map_err(|_| StatusCode::REQUEST_TIMEOUT)? {
            None => break,
            Some(Ok(Piece::Data(data))) => {
                if (body.len() + data.len()) as u64 > max {
                    return Err(StatusCode::PAYLOAD_TOO_LARGE);
                }
                body.extend(data);
            }
            // Request trailers have no place in a buffered `Request`.
            Some(Ok(Piece::Trailers(_))) => {}
            Some(Err(_)) => return Err(StatusCode::BAD_REQUEST),
        }
    }
    Ok(Request::new(method, target)
        .with_headers(headers)
        .with_body(body)
        .with_peer_addr(peer))
}

/// The method, target and headers of a request's header block, with
/// `:authority` as `Host` and split `cookie` fields joined (RFC 9113
/// §8.3.1, §8.2.3), as an HTTP/1.1 handler expects them.
fn head(fields: &[HeaderField]) -> Result<(Method, String, HeaderMap), StatusCode> {
    let bad = StatusCode::BAD_REQUEST;
    let (mut method, mut target, mut authority) = (None, None, None);
    let mut cookies = Vec::new();
    let mut headers = HeaderMap::new();
    for field in fields {
        let name = std::str::from_utf8(&field.name).map_err(|_| bad)?;
        let value = std::str::from_utf8(&field.value).map_err(|_| bad)?;
        match name {
            ":method" => method = Some(Method::parse(value)),
            ":path" => target = Some(value.to_owned()),
            ":authority" => authority = Some(value),
            "cookie" => cookies.push(value),
            n if n.starts_with(':') => {}
            n => headers.append(n, value).map_err(|_| bad)?,
        }
    }
    let (Some(method), Some(target)) = (method, target) else {
        return Err(bad);
    };
    if let Some(authority) = authority.filter(|_| !headers.contains("host")) {
        headers.insert("host", authority).map_err(|_| bad)?;
    }
    if !cookies.is_empty() {
        headers
            .insert("cookie", &cookies.join("; "))
            .map_err(|_| bad)?;
    }
    Ok((method, target, headers))
}

/// Sends `response` on the stream. A HEAD answer keeps the router's
/// `Content-Length` and sends no body.
async fn answer(respond: Respond, response: Response, head_only: bool, stop: &Arc<Stop>) {
    let Response {
        status,
        headers,
        body,
    } = response;
    let code = status.as_u16();
    let bodiless = head_only || status.is_informational() || code == 204 || code == 304;
    let mut fields = vec![HeaderField::new(":status", code.to_string())];
    for (name, value) in headers.iter() {
        let name = name.to_ascii_lowercase();
        let dropped = CONNECTION_SPECIFIC.contains(&name.as_str())
            || (name == "content-length" && !head_only);
        if !dropped {
            fields.push(HeaderField::new(name, value));
        }
    }
    match body.0 {
        Inner::Full(bytes) => {
            if !bodiless {
                fields.push(HeaderField::new("content-length", bytes.len().to_string()));
            }
            let end = bodiless || bytes.is_empty();
            let Ok(mut send) = respond.send_response(fields, end).await else {
                return;
            };
            if !end {
                // A failed write means the client is gone; nothing to tell.
                let _ = send.send_data(bytes, true).await;
            }
        }
        Inner::Stream(_) if bodiless => {
            let _ = respond.send_response(fields, true).await;
        }
        Inner::Stream(stream) => {
            if let Ok(mut send) = respond.send_response(fields, false).await {
                pump(&mut send, stream, stop).await;
            }
        }
    }
}

/// Sends each chunk of `stream` until it ends or the server stops, then
/// ends the body. A failed stream is dropped unended, which resets it, so
/// the client sees the body cut short.
async fn pump(send: &mut SendStream, mut stream: BodyStream, stop: &Arc<Stop>) {
    loop {
        let next = poll_fn(|cx| stream.as_mut().poll_next(cx));
        let chunk = match until_stopped(next, stop).await {
            None | Some(None) => {
                let _ = send.send_data(Vec::new(), true).await;
                return;
            }
            Some(Some(Err(_))) => return,
            Some(Some(Ok(chunk))) => chunk,
        };
        if !chunk.is_empty() && send.send_data(chunk, false).await.is_err() {
            return;
        }
    }
}
