//! The HTTP surface: the UI page, `POST /api/<op>`, and `/mcp`, behind one
//! guard (allowed `Host`, optional bearer token, JSON-only writes).

use std::sync::Arc;

use rusty_crypto_key::constant_time_eq;
use rusty_http::{Method, StatusCode};
use rusty_mcp_server::HttpHandler;
use rusty_serve::{Request, Response, SharedHandler};
use serde_json::{Value, json};

use crate::api::{CallError, Op, Shared};
use crate::domain::BoardError;

const UI: &str = include_str!("ui.html");

/// The combined handler.
pub struct Web {
    board: Arc<Shared>,
    ops: Vec<Op>,
    mcp: HttpHandler,
    mcp_path: String,
    hosts: Vec<String>,
    token: Option<String>,
}

impl Web {
    /// `hosts` are accepted `Host` values (port ignored); `token`, if set,
    /// is required as a bearer on `/api/*` and `/mcp`.
    pub fn new(
        board: Arc<Shared>,
        mcp: HttpHandler,
        mcp_path: String,
        hosts: Vec<String>,
        token: Option<String>,
    ) -> Self {
        Self {
            board,
            ops: crate::api::ops(),
            mcp,
            mcp_path,
            hosts,
            token,
        }
    }

    fn host_ok(&self, request: &Request<'_>) -> bool {
        let Some(host) = request.headers.get("host") else {
            return false;
        };
        let name = match host.rsplit_once(':') {
            Some((name, port)) if port.bytes().all(|b| b.is_ascii_digit()) => name,
            _ => host,
        };
        self.hosts.iter().any(|h| h == name)
    }

    fn authorized(&self, request: &Request<'_>) -> bool {
        let Some(expected) = &self.token else {
            return true;
        };
        request
            .authorization
            .and_then(rusty_oauth::bearer::token_from_authorization)
            .is_some_and(|got| constant_time_eq(got.as_bytes(), expected.as_bytes()))
    }

    fn api(&self, name: &str, request: &Request<'_>) -> Response {
        let json_body = request
            .headers
            .get("content-type")
            .is_some_and(|c| c.starts_with("application/json"));
        if request.method != &Method::Post || !json_body {
            return reply(
                StatusCode::BAD_REQUEST,
                json!({"error": "POST application/json required"}),
            );
        }
        let Some(op) = self.ops.iter().find(|o| o.name == name) else {
            return reply(StatusCode::NOT_FOUND, json!({"error": "no such operation"}));
        };
        let args: Value = serde_json::from_slice(request.body).unwrap_or(Value::Null);
        match op.call(&self.board, args) {
            Ok(out) => reply(StatusCode::OK, out),
            Err(CallError::BadArgs(why)) => reply(StatusCode::BAD_REQUEST, json!({"error": why})),
            Err(CallError::Board(e)) => reply(status_of(&e), json!({"error": e.to_string()})),
        }
    }
}

fn status_of(e: &BoardError) -> StatusCode {
    match e {
        BoardError::NotFound(_) => StatusCode::NOT_FOUND,
        BoardError::Conflict | BoardError::Claimed(_) => StatusCode::CONFLICT,
        BoardError::Forbidden(_) | BoardError::NotClaimant => StatusCode::FORBIDDEN,
        BoardError::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
        BoardError::Terminal | BoardError::NoSuchMove { .. } | BoardError::Invalid(_) => {
            StatusCode::UNPROCESSABLE_ENTITY
        }
    }
}

fn reply(status: StatusCode, body: Value) -> Response {
    Response::json(status, body.to_string().into_bytes())
}

impl SharedHandler for Web {
    fn handle(&self, request: &Request<'_>) -> Response {
        let path = request.target.split('?').next().unwrap_or("");
        if path == self.mcp_path {
            return if self.authorized(request) {
                self.mcp.handle(request)
            } else {
                reply(StatusCode::UNAUTHORIZED, json!({"error": "unauthorized"}))
            };
        }
        if !self.host_ok(request) {
            return reply(StatusCode::FORBIDDEN, json!({"error": "host not allowed"}));
        }
        if path == "/" && request.method == &Method::Get {
            // The page holds no data; it reads the token from the URL fragment.
            return Response::stream(
                "text/html; charset=utf-8",
                std::iter::once(UI.as_bytes().to_vec()),
            );
        }
        let Some(name) = path.strip_prefix("/api/") else {
            return reply(StatusCode::NOT_FOUND, json!({"error": "not found"}));
        };
        if !self.authorized(request) {
            return reply(StatusCode::UNAUTHORIZED, json!({"error": "unauthorized"}));
        }
        self.api(name, request)
    }
}
