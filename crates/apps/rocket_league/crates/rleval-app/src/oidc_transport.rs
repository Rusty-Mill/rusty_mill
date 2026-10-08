//! The real [`HttpTransport`]: blocking HTTP/1.1 over `std::net`, with TLS for
//! `https://` via the Rusty-Mill `rusty_tls` crate (OS trust anchors).
//!
//! Framing (request head, `Content-Length`, chunked bodies) is done by
//! `rusty_http`'s sync adapter, so no HTTP parsing lives here. One connection
//! per request with `Connection: close`: sign-in makes at most three calls per
//! login, so pooling would be complexity for nothing.
//!
//! Responses are bounded (`rusty_http`'s default body limit) and every socket
//! read and write has a timeout, so a stalled provider cannot pin a server
//! thread forever.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use rusty_http::body::response_framing;
use rusty_http::head::RequestHead;
use rusty_http::header::HeaderMap;
use rusty_http::sync::SyncTransport;
use rusty_http::{Method, Url, Version};
use rusty_oauth::request::{HttpRequest, Method as OauthMethod};
use rusty_tls::{TlsStream, TrustPolicy};

use crate::authn::LoginError;
use crate::oidc::{HttpReply, HttpTransport};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const IO_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_HEAD_LEN: usize = 64 * 1024;

#[derive(Debug, Default, Clone, Copy)]
pub struct StdTransport;

impl HttpTransport for StdTransport {
    fn send(&self, request: &HttpRequest) -> Result<HttpReply, LoginError> {
        let url = Url::parse(&request.url).map_err(|e| err("parsing URL", e))?;
        let addr = (url.host.as_str(), url.port)
            .to_socket_addrs()
            .map_err(|e| err("resolving host", e))?
            .next()
            .ok_or_else(|| LoginError(format!("no address for {}", url.host)))?;
        let tcp =
            TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).map_err(|e| err("connecting", e))?;
        tcp.set_read_timeout(Some(IO_TIMEOUT))
            .map_err(|e| err("socket setup", e))?;
        tcp.set_write_timeout(Some(IO_TIMEOUT))
            .map_err(|e| err("socket setup", e))?;

        match url.scheme.as_str() {
            "https" => {
                let tls = TlsStream::new(tcp, &url.host, &TrustPolicy::System)
                    .map_err(|e| err("TLS handshake", e))?;
                exchange(tls, &url, request)
            }
            _ => exchange(tcp, &url, request),
        }
    }
}

fn exchange<T: Read + Write>(
    io: T,
    url: &Url,
    request: &HttpRequest,
) -> Result<HttpReply, LoginError> {
    let method = match request.method {
        OauthMethod::Get => Method::Get,
        OauthMethod::Post => Method::Post,
    };
    let mut headers = HeaderMap::new();
    let mut put = |name: &str, value: &str| {
        // Credentials are marked sensitive so they are redacted from Debug output.
        let result = if name.eq_ignore_ascii_case("authorization") {
            headers.insert_sensitive(name, value)
        } else {
            headers.insert(name, value)
        };
        result.map_err(|e| err("building request", e))
    };
    put("Host", &url.host_header())?;
    put("Connection", "close")?;
    put("Accept", "application/json")?;
    if !request.body.is_empty() || method == Method::Post {
        put("Content-Length", &request.body.len().to_string())?;
    }
    for (name, value) in &request.headers {
        put(name, value)?;
    }

    let head = RequestHead {
        method: method.clone(),
        target: url.request_target(),
        version: Version::Http11,
        headers,
    };
    let mut transport = SyncTransport::new(io);
    transport
        .write_request_head(&head)
        .map_err(|e| err("sending request", e))?;
    transport
        .write_body(&request.body)
        .map_err(|e| err("sending request", e))?;

    let response = transport
        .read_response_head(MAX_HEAD_LEN)
        .map_err(|e| err("reading response", e))?;
    let framing = response_framing(&response.headers, &method, response.status)
        .map_err(|e| err("reading response", e))?;
    let body = transport
        .read_body(framing)
        .map_err(|e| err("reading response", e))?;
    Ok(HttpReply {
        status: response.status.as_u16(),
        body: String::from_utf8(body).map_err(|_| LoginError("response was not UTF-8".into()))?,
    })
}

fn err(step: &str, e: impl std::fmt::Display) -> LoginError {
    LoginError(format!("{step} failed: {e}"))
}
