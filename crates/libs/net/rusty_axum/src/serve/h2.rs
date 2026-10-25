//! HTTP/2 on a TLS connection that negotiated ALPN `h2` (feature `http2`,
//! #885). Each stream becomes the same buffered [`Request`] as on HTTP/1.1,
//! answered by the same [`Router`](crate::Router); the connection itself
//! (preface deadline, GOAWAY, drain) is `rusty_hyper::serve_connection`'s.
//!
//! The HTTP/1.1 limits carry over: `max_head_bytes` bounds a request's
//! decoded header list, `max_body_bytes` its body, and `idle_timeout` each
//! body read, each blocked write, and a connection with no open stream. At
//! most [`MAX_STREAMS`] streams are open per connection.

use std::future::poll_fn;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rusty_http::{HeaderMap, Method, StatusCode};
use rusty_hyper::{Body as Piece, HeaderField, Respond, SendStream};
use rusty_tokio::io::{AsyncRead, AsyncWrite};
use rusty_tokio::time::{sleep, timeout, Instant};

use super::{until_stopped, Shared};
use crate::body::{BodyStream, Inner};
use crate::request::Request;
use crate::response::Response;
use crate::stop::Stop;

/// The most streams one connection may have open at once; the client is
/// told so in SETTINGS and a stream past it is refused (RFC 9113 §5.1.2).
/// 100 is the floor RFC 9113 §6.5.2 recommends.
pub(crate) const MAX_STREAMS: u32 = 100;

/// Header fields that HTTP/2 forbids (RFC 9113 §8.2.2).
const CONNECTION_SPECIFIC: [&str; 5] = [
    "connection",
    "keep-alive",
    "proxy-connection",
    "transfer-encoding",
    "upgrade",
];

/// Streams on one connection: how many are open, and how many have begun.
#[derive(Default)]
struct Activity {
    open: AtomicUsize,
    started: AtomicU64,
}

/// Counts a stream open until dropped.
struct Open(Arc<Activity>);

impl Open {
    fn new(activity: &Arc<Activity>) -> Self {
        activity.started.fetch_add(1, Ordering::Relaxed);
        activity.open.fetch_add(1, Ordering::Relaxed);
        Open(activity.clone())
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        self.0.open.fetch_sub(1, Ordering::Relaxed);
    }
}

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
    let options = &shared.options;
    let mut settings = rusty_hyper::ServeOptions::default().h2;
    settings
        .max_concurrent_streams(MAX_STREAMS)
        .max_header_list_size(options.max_head_bytes);
    let activity = Arc::new(Activity::default());
    // Shut the connection down (GOAWAY, drain) when the server stops, or
    // when it has had no open stream for an idle period.
    let done = {
        let (stop, activity, idle) = (stop.clone(), activity.clone(), options.idle_timeout);
        async move {
            rusty_tokio::select! {
                biased;
                _ = stop.wait() => {}
                _ = idle_out(&activity, idle) => {}
            }
        }
    };
    let handler = move |req: rusty_hyper::Request, respond: Respond| {
        let open = Open::new(&activity);
        let (shared, stop) = (shared.clone(), stop.clone());
        async move {
            stream(req, respond, peer, &shared, &stop).await;
            drop(open);
        }
    };
    rusty_hyper::serve_connection(io, settings, deadline, done, handler).await;
}

/// Completes once the connection has had no stream open and none begun for
/// a whole `idle` period: between one and two `idle` periods after the
/// last stream ended.
async fn idle_out(activity: &Activity, idle: Duration) {
    let mut seen = activity.started.load(Ordering::Relaxed);
    loop {
        sleep(idle).await;
        let started = activity.started.load(Ordering::Relaxed);
        if started == seen && activity.open.load(Ordering::Relaxed) == 0 {
            return;
        }
        seen = started;
    }
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
    let options = &shared.options;
    let work = async {
        let (response, head_only) = match read(req, peer, options).await {
            Ok(req) => {
                let head_only = *req.method() == Method::Head;
                (shared.router.call(req).await, head_only)
            }
            Err(status) => (Response::new(status, Vec::new()), false),
        };
        answer(respond, response, head_only, options.idle_timeout, stop).await;
    };
    rusty_tokio::select! {
        biased;
        _ = closed => {}
        _ = work => {}
    }
}

/// The request on a stream, its head within `max_head_bytes` and its body
/// read whole within `max_body_bytes`, each read within `idle_timeout`.
/// The status to refuse it with otherwise.
async fn read(
    mut req: rusty_hyper::Request,
    peer: SocketAddr,
    options: &super::ServeOptions,
) -> Result<Request, StatusCode> {
    let (method, target, headers) = head(&req.headers, options.max_head_bytes)?;
    let mut body = Vec::new();
    loop {
        let piece = timeout(options.idle_timeout, req.body.next()).await;
        match piece.map_err(|_| StatusCode::REQUEST_TIMEOUT)? {
            None => break,
            Some(Ok(Piece::Data(data))) => {
                if (body.len() + data.len()) as u64 > options.max_body_bytes {
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

/// A header field's size as HTTP/2 counts it (RFC 9113 §6.5.2).
fn field_size(field: &HeaderField) -> usize {
    field.name.len() + field.value.len() + 32
}

/// The method, target and headers of a request's header block, as an
/// HTTP/1.1 handler expects them: `:authority` as `Host` and split `cookie`
/// fields joined (RFC 9113 §8.3.1, §8.2.3). Refused with 431 when the
/// decoded list is over `max` bytes, and with 400 when it is malformed or
/// names two hosts (repeated `Host`, or one that differs from `:authority`).
fn head(fields: &[HeaderField], max: usize) -> Result<(Method, String, HeaderMap), StatusCode> {
    if fields.iter().map(field_size).sum::<usize>() > max {
        return Err(StatusCode::from_u16(431));
    }
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
    let host = {
        let mut hosts = headers.get_all("host");
        match (hosts.next(), hosts.next()) {
            (Some(_), Some(_)) => return Err(bad),
            (host, _) => host.map(str::to_owned),
        }
    };
    match (authority, host) {
        (Some(authority), Some(host)) if !authority.eq_ignore_ascii_case(&host) => return Err(bad),
        (Some(authority), None) => headers.insert("host", authority).map_err(|_| bad)?,
        _ => {}
    }
    if !cookies.is_empty() {
        headers
            .insert("cookie", &cookies.join("; "))
            .map_err(|_| bad)?;
    }
    Ok((method, target, headers))
}

/// Sends `response` on the stream. A HEAD answer keeps the router's
/// `Content-Length` and sends no body. Each write that waits for the
/// client's flow-control window gives up after `idle`, which resets the
/// stream, so a client that stops reading cannot hold a response (or a
/// shutdown waiting on it) open.
async fn answer(
    respond: Respond,
    response: Response,
    head_only: bool,
    idle: Duration,
    stop: &Arc<Stop>,
) {
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
                // Unsent, the stream is dropped unended, which resets it.
                send_within(&mut send, bytes, true, idle).await;
            }
        }
        Inner::Stream(_) if bodiless => {
            let _ = respond.send_response(fields, true).await;
        }
        Inner::Stream(stream) => {
            if let Ok(mut send) = respond.send_response(fields, false).await {
                pump(&mut send, stream, idle, stop).await;
            }
        }
    }
}

/// Sends `data` within `idle`; whether it went.
async fn send_within(send: &mut SendStream, data: Vec<u8>, end: bool, idle: Duration) -> bool {
    matches!(timeout(idle, send.send_data(data, end)).await, Ok(Ok(())))
}

/// Sends each chunk of `stream` until it ends or the server stops, then
/// ends the body. A failed stream or a write that times out is dropped
/// unended, which resets it, so the client sees the body cut short.
async fn pump(send: &mut SendStream, mut stream: BodyStream, idle: Duration, stop: &Arc<Stop>) {
    loop {
        let next = poll_fn(|cx| stream.as_mut().poll_next(cx));
        let chunk = match until_stopped(next, stop).await {
            None | Some(None) => {
                send_within(send, Vec::new(), true, idle).await;
                return;
            }
            Some(Some(Err(_))) => return,
            Some(Some(Ok(chunk))) => chunk,
        };
        if !chunk.is_empty() && !send_within(send, chunk, false, idle).await {
            return;
        }
    }
}
