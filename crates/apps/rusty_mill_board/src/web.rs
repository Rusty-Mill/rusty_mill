//! The HTTP surface: the UI page, `POST /api/<op>`, and `/mcp`, behind one
//! guard (allowed `Host`, optional bearer token, JSON-only writes).

use std::sync::{Arc, PoisonError, RwLock};

use rusty_crypto_key::constant_time_eq;
use rusty_http::{Method, StatusCode};
use rusty_mcp_server::HttpHandler;
use rusty_mcp_server::json::Value as WireValue;
use rusty_serve::{Request, Response, SharedHandler};
use serde_json::{Value, json};

use crate::api::{CallError, Identity, Op, Shared};
use crate::domain::BoardError;
use crate::identities::Credentials;
use crate::settings::Settings;

const UI: &str = include_str!("ui.html");

/// Refuse to serve an unauthenticated board beyond loopback. The `Host`
/// check is not client authentication: any remote client can send
/// `Host: localhost` and claim any role, so a reachable board needs
/// credentials (or an explicit, clearly named override).
///
/// # Errors
/// A message naming the fix, when `addr` is not loopback, `auth` is
/// [`Auth::Open`] and `accept_unauthenticated` is false.
pub fn check_bind(
    addr: std::net::SocketAddr,
    auth: &Auth,
    accept_unauthenticated: bool,
) -> Result<(), String> {
    if addr.ip().is_loopback() || !matches!(auth, Auth::Open) || accept_unauthenticated {
        return Ok(());
    }
    Err(format!(
        "refusing to serve an unauthenticated board on {addr}: anyone who can reach it can act as any role \
         (the Host check is not authentication). Use --identities FILE (a token per model) or --auth-token, \
         bind to loopback, or pass --accept-unauthenticated-public if you really mean it"
    ))
}

/// How callers of `/api` and `/mcp` are authenticated.
pub enum Auth {
    /// No credential required (loopback use).
    Open,
    /// One shared bearer token; callers state their own actor and role.
    Shared(String),
    /// A token per model, each bound to an actor and role. Shared with the settings, which
    /// add and remove agents while the board serves.
    Credentials(Arc<RwLock<Credentials>>),
}

impl From<Option<&str>> for Auth {
    fn from(token: Option<&str>) -> Self {
        token.map_or(Self::Open, |t| Self::Shared(t.to_owned()))
    }
}

/// The combined handler.
pub struct Web {
    board: Arc<Shared>,
    ops: Vec<Op>,
    mcp: HttpHandler,
    mcp_path: String,
    hosts: Vec<String>,
    auth: Auth,
    settings: Arc<Settings>,
}

impl Web {
    /// `hosts` are accepted `Host` values (port ignored); `auth` governs
    /// `/api/*` and `/mcp`.
    pub fn new(
        board: Arc<Shared>,
        mcp: HttpHandler,
        mcp_path: String,
        hosts: Vec<String>,
        auth: Auth,
    ) -> Self {
        Self {
            settings: Arc::new(Settings::read_only(Arc::clone(&board))),
            board,
            ops: crate::api::ops(),
            mcp,
            mcp_path,
            hosts,
            auth,
        }
    }

    /// Serve `/api/settings_*` from `settings` (read-only settings by default).
    #[must_use]
    pub fn with_settings(mut self, settings: Arc<Settings>) -> Self {
        self.settings = settings;
        self
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

    /// `Err` when the request must be refused; `Ok(None)` when no identity applies.
    fn identify(&self, request: &Request<'_>) -> Result<Option<Identity>, ()> {
        let presented = request
            .authorization
            .and_then(rusty_oauth::bearer::token_from_authorization);
        match &self.auth {
            Auth::Open => Ok(None),
            Auth::Shared(expected) => presented
                .filter(|got| constant_time_eq(got.as_bytes(), expected.as_bytes()))
                .map(|_| None)
                .ok_or(()),
            Auth::Credentials(all) => presented
                .and_then(|got| {
                    all.read()
                        .unwrap_or_else(PoisonError::into_inner)
                        .identify(got)
                        .cloned()
                })
                .map(Some)
                .ok_or(()),
        }
    }

    fn api(&self, name: &str, identity: Option<&Identity>, request: &Request<'_>) -> Response {
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
        let args: Value = serde_json::from_slice(request.body).unwrap_or(Value::Null);
        if let Some(result) = self.settings.call(name, identity, args.clone()) {
            return answer(result);
        }
        let Some(op) = self.ops.iter().find(|o| o.name == name) else {
            return reply(StatusCode::NOT_FOUND, json!({"error": "no such operation"}));
        };
        answer(op.call(&self.board, identity, args))
    }
}

fn answer(result: Result<Value, CallError>) -> Response {
    match result {
        Ok(out) => reply(StatusCode::OK, out),
        Err(CallError::BadArgs(why)) => reply(StatusCode::BAD_REQUEST, json!({"error": why})),
        Err(CallError::Board(e)) => reply(status_of(&e), json!({"error": e.to_string()})),
    }
}

/// The identity as the MCP layer's principal, which tools read from the call context.
fn principal(identity: &Identity) -> Option<WireValue> {
    WireValue::from_json_str(&serde_json::to_string(identity).ok()?).ok()
}

fn status_of(e: &BoardError) -> StatusCode {
    match e {
        BoardError::NotFound(_) => StatusCode::NOT_FOUND,
        BoardError::Conflict | BoardError::Claimed(_) => StatusCode::CONFLICT,
        BoardError::Forbidden(_) | BoardError::NotClaimant | BoardError::OutOfScope(_) => {
            StatusCode::FORBIDDEN
        }
        BoardError::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
        BoardError::Terminal
        | BoardError::NoSuchMove { .. }
        | BoardError::Invalid(_)
        | BoardError::Refused(_) => StatusCode::UNPROCESSABLE_ENTITY,
    }
}

fn reply(status: StatusCode, body: Value) -> Response {
    Response::json(status, body.to_string().into_bytes())
}

impl SharedHandler for Web {
    fn handle(&self, request: &Request<'_>) -> Response {
        let path = request.target.split('?').next().unwrap_or("");
        if path == self.mcp_path {
            return match self.identify(request) {
                Ok(identity) => self
                    .mcp
                    .handle_with(request, identity.and_then(|i| principal(&i))),
                Err(()) => reply(StatusCode::UNAUTHORIZED, json!({"error": "unauthorized"})),
            };
        }
        if path == "/health" && request.method == &Method::Get {
            // Open to probes on any Host: it says only whether the store can still commit.
            return match self.board.health() {
                Ok(()) => reply(StatusCode::OK, json!({"ok": true})),
                Err(e) => reply(
                    StatusCode::SERVICE_UNAVAILABLE,
                    json!({"ok": false, "error": e.to_string()}),
                ),
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
        match self.identify(request) {
            Ok(identity) => self.api(name, identity.as_ref(), request),
            Err(()) => reply(StatusCode::UNAUTHORIZED, json!({"error": "unauthorized"})),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(s: &str) -> std::net::SocketAddr {
        s.parse().expect("addr")
    }

    #[test]
    fn open_boards_stay_on_loopback() {
        for ok in ["127.0.0.1:8787", "[::1]:8787", "127.9.9.9:1"] {
            assert!(check_bind(addr(ok), &Auth::Open, false).is_ok(), "{ok}");
        }
        for bad in [
            "0.0.0.0:8787",
            "[::]:8787",
            "192.168.1.5:8787",
            "10.0.0.1:80",
        ] {
            let e = check_bind(addr(bad), &Auth::Open, false).expect_err(bad);
            assert!(
                e.contains("--identities") && e.contains("--accept-unauthenticated-public"),
                "{e}"
            );
        }
    }

    #[test]
    fn credentials_or_an_explicit_override_allow_a_public_bind() {
        let public = addr("0.0.0.0:8787");
        assert!(check_bind(public, &Auth::Shared("t".into()), false).is_ok());
        assert!(check_bind(public, &Auth::Open, true).is_ok());
    }
}
