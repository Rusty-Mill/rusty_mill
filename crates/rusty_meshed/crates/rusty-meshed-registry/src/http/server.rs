//! The accept loop: binds nothing itself (the caller owns the
//! listener, same seam this workspace's fake test servers already
//! use), reads one request per connection, dispatches it through
//! [`cors::handle`], and writes the response back.
//!
//! One request per connection, always `Connection: close` -- no
//! keep-alive reuse. That's a real simplification versus the source
//! (uvicorn happily keeps connections alive), but this app's clients
//! are a local dev-server frontend and this crate's own tests, neither
//! of which need connection reuse to function correctly, and it
//! sidesteps an entire class of framing bugs (a client that thinks a
//! connection is still alive after the server considers the exchange
//! over) that a "minimal" first pass doesn't need to take on.

use super::cors;
use super::request::Request;
use super::router::Router;
use rusty_http::async_tokio::AsyncTransport;
use rusty_http::head::ResponseHead;
use rusty_http::Version;
use rusty_tokio::io::{AsyncRead, AsyncWrite, TcpListener};
use std::sync::Arc;

/// A generous bound on an incoming request head's size -- this server
/// only ever faces its own frontend and its own tests, not arbitrary
/// internet peers, so it uses the same larger bound `rusty_request`
/// applies to a response it trusts, rather than `rusty_http`'s tighter
/// untrusted-input default.
const MAX_HEAD_LEN: usize = 1024 * 1024;

/// Accepts connections from `listener` forever, dispatching each one
/// through `router` (wrapped in CORS handling). Spawns one task per
/// connection via `rusty_tokio::spawn`, so a slow or hung client can't
/// block any other connection.
pub async fn serve(listener: TcpListener, router: Arc<Router>) -> std::io::Result<()> {
    loop {
        let (stream, _peer) = listener.accept().await?;
        let router = router.clone();
        rusty_tokio::spawn(async move {
            let _ = handle_connection(stream, router).await;
        });
    }
}

/// Handles exactly one request/response exchange over `stream`, then
/// returns (the caller is expected to drop/close the connection).
pub async fn handle_connection<T>(stream: T, router: Arc<Router>) -> rusty_http::TransportResult<()>
where
    T: AsyncRead + AsyncWrite + Unpin + Send,
{
    let mut transport = AsyncTransport::new(stream);
    let head = transport.read_request_head(MAX_HEAD_LEN).await?;
    let framing = rusty_http::body::request_framing(&head.headers)?;
    let body = transport.read_body(framing).await?;

    let req = Request::from_head(&head, body);
    let mut response = cors::handle(&router, req).await;
    response
        .headers
        .insert("Content-Length", &response.body.len().to_string())?;
    response.headers.insert("Connection", "close")?;

    let response_head = ResponseHead {
        status: response.status,
        reason: response.status.canonical_reason().unwrap_or("").to_string(),
        version: Version::Http11,
        headers: response.headers,
    };
    transport.write_response_head(&response_head).await?;
    transport.write_body(&response.body).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::response::Response;
    use rusty_http::StatusCode;
    use rusty_tokio::io::duplex;

    #[rusty_tokio::test]
    async fn handles_one_request_and_writes_a_response() {
        let router = Arc::new(Router::new().get("/health", |_req| async {
            Response::text(StatusCode::OK, "ok")
        }));
        let (client, server) = duplex(4096);

        let server_task = rusty_tokio::spawn(async move {
            handle_connection(server, router).await.unwrap();
        });

        let mut transport = AsyncTransport::new(client);
        let request_head = rusty_http::head::RequestHead {
            method: rusty_http::Method::Get,
            target: "/health".to_string(),
            version: Version::Http11,
            headers: rusty_http::HeaderMap::new(),
        };
        transport.write_request_head(&request_head).await.unwrap();
        transport.write_body(&[]).await.unwrap();

        let response_head = transport.read_response_head(MAX_HEAD_LEN).await.unwrap();
        let framing = rusty_http::body::response_framing(
            &response_head.headers,
            &rusty_http::Method::Get,
            response_head.status,
        )
        .unwrap();
        let body = transport.read_body(framing).await.unwrap();

        server_task.await.unwrap();
        assert_eq!(response_head.status, StatusCode::OK);
        assert_eq!(body, b"ok");
        assert_eq!(response_head.headers.get("Content-Length"), Some("2"));
    }
}
