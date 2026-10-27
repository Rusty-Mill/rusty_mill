//! OAuth 2.1 for `/mcp`, so a client such as a claude.ai custom connector
//! can sign in instead of being handed a bearer token. The authorization
//! server is [`rusty_oauth_server`]; this file only adapts it to
//! [`rusty_serve`] and to rusty_tick's users.
//!
//! On when `RUSTY_TICK_ISSUER` names the public origin (an https URL, or
//! http on localhost). The consent page asks for one of the server's own
//! tokens, and the grant then acts as that token's user: the subject is the
//! user key (`default` in single-user mode). Each request resolves it to a
//! live user ([`crate::auth::Authenticator::resolve`]), so disabling a user
//! cuts off their OAuth tokens too.
//!
//! Grants are kept in `<data-dir>/oauth.json`. They outlive the token that
//! approved them: revoking one token of a user, or changing
//! `RUSTY_TICK_TOKEN`, does not end them. Disable the user, or delete
//! `oauth.json` and restart, to end every grant.

use crate::backend::Backend;
use rusty_http::{Method, StatusCode};
use rusty_oauth_server::{
    now_unix, Branding, Endpoints, Issuer, Provider, StateStore, Subject, Verify,
};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};

/// The environment variable naming the issuer, which turns OAuth on.
pub const ISSUER_ENV: &str = "RUSTY_TICK_ISSUER";
/// The grants, inside the data directory.
pub const STATE_FILE: &str = "oauth.json";
/// Every path the OAuth endpoints answer, for the web UI to leave alone.
pub use rusty_oauth_server::PATHS;

const BRANDING: Branding = Branding {
    resource_name: "rusty_tick",
    resource_description: "your rusty_tick tasks",
    token_label: "Your rusty_tick API token",
};

/// How OAuth is set up: the public origin and where grants are kept.
pub struct OAuth {
    pub issuer: Issuer,
    pub state_file: PathBuf,
}

/// The endpoints for `oauth`, with `backend` told to accept their tokens.
/// The consent page checks a pasted token against `backend`, held weakly so
/// the backend and its provider do not keep each other alive.
pub(crate) fn endpoints(oauth: OAuth, backend: &Arc<Mutex<Backend>>) -> Endpoints {
    let weak = Arc::downgrade(backend);
    let provider = Arc::new(Provider::new(
        verifier(weak),
        StateStore::new(oauth.state_file),
    ));
    // Nothing else holds this lock yet, so it cannot be poisoned.
    if let Ok(mut backend) = backend.lock() {
        backend.set_oauth(Arc::clone(&provider));
    }
    Endpoints::new(provider, oauth.issuer, BRANDING)
}

fn verifier(backend: Weak<Mutex<Backend>>) -> Verify {
    Box::new(move |token: &str| {
        let backend = backend.upgrade()?;
        let mut backend = backend.lock().ok()?;
        backend
            .verify(token)
            .map(|user| Subject::new(user.as_str()))
    })
}

/// The OAuth answer to `request`, or `None` for any other path.
pub(crate) fn handle(
    endpoints: &Endpoints,
    request: &rusty_serve::Request<'_>,
) -> Option<rusty_serve::Response> {
    let (path, query) = request
        .target
        .split_once('?')
        .unwrap_or((request.target, ""));
    let method = match request.method {
        Method::Get => rusty_oauth_server::Method::Get,
        Method::Post => rusty_oauth_server::Method::Post,
        _ => rusty_oauth_server::Method::Other,
    };
    let oauth_request = rusty_oauth_server::Request {
        method,
        path,
        query,
        body: request.body,
    };
    endpoints.handle(&oauth_request, now_unix()).map(to_serve)
}

/// `response` as [`rusty_serve`] sends it.
pub(crate) fn to_serve(response: rusty_oauth_server::Response) -> rusty_serve::Response {
    let status = StatusCode::from_u16(response.status);
    let headers = response.headers;
    headers
        .iter()
        .try_fold(
            rusty_serve::Response::json(status, response.body),
            |out, (name, value)| out.with_header(name, value),
        )
        .unwrap_or_else(|error| {
            eprintln!("rusty_tick: an OAuth response header was refused: {error}");
            rusty_serve::Response::internal_error()
        })
}
