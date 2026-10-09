//! A stateless Streamable HTTP endpoint for `rusty_serve`.
//!
//! One `POST` carries one JSON-RPC message. A request is answered with its
//! response as `application/json`; a notification or a stray response is
//! acknowledged with `202 Accepted`. There is no session and no server-sent
//! event stream, so `GET` and `DELETE` are answered `405`: this is the
//! stateless shape the 2026-07-28 revision makes the default, and older
//! clients fall back to it too.
//!
//! Requests are dispatched in a deferred job, so a slow tool does not hold
//! `rusty_serve`'s handler lock and other requests run on their own
//! connection threads. A dropped connection is not noticed while a handler
//! runs, so [`crate::Context::is_cancelled`] is never set here.
//!
//! Browsers can be tricked into calling a local server (DNS rebinding), so a
//! request carrying an `Origin` header is refused unless the origin was
//! allowed with [`McpHttp::allow_origin`]. Clients that are not browsers send
//! no `Origin` and are unaffected.

use crate::dispatch::{parse_failure, SUPPORTED_VERSIONS};
use crate::{Context, Dispatcher, Service};
use rusty_http::{Method, StatusCode};
use rusty_mcp_proto::jsonrpc::code;
use rusty_mcp_proto::{ErrorObject, Message};
use rusty_serve::{Handler, Request, Response};
use std::sync::Arc;

type Authorize = Box<dyn Fn(&Request<'_>) -> bool + Send>;

/// The `rusty_serve` handler for an MCP server.
pub struct McpHttp<S> {
    dispatcher: Arc<Dispatcher<S>>,
    path: String,
    allowed_origins: Vec<String>,
    allowed_hosts: Option<Vec<String>>,
    authorize: Option<Authorize>,
}

impl<S: Service + 'static> McpHttp<S> {
    /// Serve `dispatcher` at `path` (for example `/mcp`).
    pub fn new(dispatcher: Dispatcher<S>, path: impl Into<String>) -> Self {
        McpHttp {
            dispatcher: Arc::new(dispatcher),
            path: path.into(),
            allowed_origins: Vec::new(),
            allowed_hosts: None,
            authorize: None,
        }
    }

    /// Accept requests whose `Origin` header is exactly `origin`.
    pub fn allow_origin(mut self, origin: impl Into<String>) -> Self {
        self.allowed_origins.push(origin.into());
        self
    }

    /// Accept only these `Host` header values (with port, as sent).
    pub fn allowed_hosts(mut self, hosts: Vec<String>) -> Self {
        self.allowed_hosts = Some(hosts);
        self
    }

    /// Run `check` on every request to the endpoint; `false` answers `401`.
    pub fn authorize(mut self, check: impl Fn(&Request<'_>) -> bool + Send + 'static) -> Self {
        self.authorize = Some(Box::new(check));
        self
    }

    fn refuse(&self, request: &Request<'_>) -> Option<Response> {
        let path = request.target.split('?').next().unwrap_or("");
        if path != self.path {
            return Some(failure(
                StatusCode::NOT_FOUND,
                code::INVALID_REQUEST,
                "no such endpoint",
            ));
        }
        if let Some(origin) = request.headers.get("origin") {
            if !self.allowed_origins.iter().any(|o| o == origin) {
                return Some(failure(
                    StatusCode::FORBIDDEN,
                    code::INVALID_REQUEST,
                    "origin not allowed",
                ));
            }
        }
        if let Some(hosts) = &self.allowed_hosts {
            let host = request.headers.get("host").unwrap_or("");
            if !hosts.iter().any(|h| h == host) {
                return Some(failure(
                    StatusCode::FORBIDDEN,
                    code::INVALID_REQUEST,
                    "host not allowed",
                ));
            }
        }
        if let Some(check) = &self.authorize {
            if !check(request) {
                return Some(failure(
                    StatusCode::UNAUTHORIZED,
                    code::INVALID_REQUEST,
                    "unauthorized",
                ));
            }
        }
        if *request.method != Method::Post {
            return Some(failure(
                StatusCode::METHOD_NOT_ALLOWED,
                code::INVALID_REQUEST,
                "this server is stateless: POST only",
            ));
        }
        if let Some(accept) = request.headers.get("accept") {
            let ok = ["application/json", "*/*", "application/*"]
                .iter()
                .any(|kind| accept.contains(kind));
            if !ok {
                return Some(failure(
                    StatusCode::NOT_ACCEPTABLE,
                    code::INVALID_REQUEST,
                    "Accept must allow application/json",
                ));
            }
        }
        if let Some(version) = request.headers.get("mcp-protocol-version") {
            if !SUPPORTED_VERSIONS.contains(&version) {
                return Some(failure(
                    StatusCode::BAD_REQUEST,
                    code::INVALID_REQUEST,
                    &format!("unsupported MCP-Protocol-Version: {version}"),
                ));
            }
        }
        None
    }
}

impl<S: Service + 'static> Handler for McpHttp<S> {
    fn handle(&mut self, request: &Request<'_>) -> Response {
        if let Some(refusal) = self.refuse(request) {
            return refusal;
        }
        let text = match std::str::from_utf8(request.body) {
            Ok(text) => text,
            Err(_) => {
                return failure(
                    StatusCode::BAD_REQUEST,
                    code::PARSE_ERROR,
                    "body is not UTF-8",
                );
            }
        };
        let message = match Message::from_json(text) {
            Ok(message) => message,
            Err(error) => {
                let body = parse_failure(&error, None).to_json().into_bytes();
                return Response::json(StatusCode::BAD_REQUEST, body);
            }
        };
        if !matches!(message, Message::Request { .. }) {
            return Response::json(StatusCode::ACCEPTED, Vec::new());
        }
        let dispatcher = Arc::clone(&self.dispatcher);
        Response::deferred(move || {
            let reply = dispatcher.handle(&Context::new(), message);
            match reply {
                Some(reply) => (StatusCode::OK, reply.to_json().into_bytes()),
                None => (StatusCode::ACCEPTED, Vec::new()),
            }
        })
    }
}

fn failure(status: StatusCode, code: i64, message: &str) -> Response {
    let body = Message::Error {
        id: None,
        error: ErrorObject::new(code, message),
    }
    .to_json()
    .into_bytes();
    Response::json(status, body)
}
