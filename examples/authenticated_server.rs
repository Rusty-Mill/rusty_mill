//! An ACP server behind a bearer token, and a client that carries one.
//!
//! The crate takes **no position on authentication**. Building a scheme in
//! would mean picking one for everybody, and the router this crate produces is
//! a plain `axum::Router`, so ordinary tower middleware layers on top. This
//! example is an illustration of that seam rather than a recommendation of the
//! scheme — swap the token check for OIDC, mTLS, HMAC-signed requests or
//! whatever your deployment already uses.
//!
//! What is *not* generic, and what this example is really about, is **which
//! endpoints must stay open** and **what protecting the session URLs does to
//! distributed sessions**. Both are specific to ACP, and neither is obvious
//! until it has broken something.
//!
//! The `well-known` feature is required, because half of what this example is
//! about is the endpoint that feature adds.
//!
//! ```sh
//! cargo run --example authenticated_server --features well-known
//!
//! # Open, on purpose:
//! curl -s localhost:8000/ping                             # liveness
//! curl -s localhost:8000/ready                            # readiness
//! curl -s localhost:8000/.well-known/agent.yml            # open discovery
//!
//! # Closed:
//! curl -si localhost:8000/agents | head -1                # => 401
//! curl -s localhost:8000/agents -H "authorization: Bearer $ACP_TOKEN" | jq
//!
//! curl -s -X POST localhost:8000/runs \
//!   -H "authorization: Bearer $ACP_TOKEN" \
//!   -H 'content-type: application/json' -d '{
//!   "agent_name": "greeter",
//!   "input": [{"role": "user", "parts": [{"content": "hi"}]}]
//! }' | jq
//! ```
//!
//! The token defaults to `demo-token`; set `ACP_TOKEN` to change it.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::{from_fn_with_state, Next};
use axum::response::{IntoResponse, Response};
use rusty_acp::client::AcpClient;
// Through the crate rather than as a dependency of its own. An example gets the
// right `reqwest` for free from the dev-dependencies and so cannot demonstrate
// the mistake it would otherwise be modelling: a caller whose own `reqwest`
// resolves to a different copy hands `with_http_client` a type that merely
// shares its name.
use rusty_acp::reqwest;
use rusty_acp::server::{agent_fn, AcpServer, RunContext};
use rusty_acp::types::{AgentManifest, AgentName, Message, RunCreateRequest, SessionId};

/// Paths reachable without a token, and why each one has to be.
///
/// This list is the part worth copying. Everything else here is ordinary axum.
///
/// - **`/ping`** is the liveness check and **`/ready`** the readiness one. A
///   load balancer probes both, and a load balancer has no credentials — put
///   either behind the token and every replica is marked unhealthy, which looks
///   like an outage rather than a misconfigured exemption list. `/ready` is the
///   more dangerous of the two to forget: a 401 there is indistinguishable from
///   "do not send me traffic", so the whole fleet quietly drops out of
///   rotation while every process stays perfectly healthy.
/// - **`/.well-known/agent.yml`** is *open discovery*. Its entire purpose is
///   that an unauthenticated crawler or another agent can find out what this
///   domain hosts without knowing anything about it first. A token in front of
///   it does not secure it; it deletes it.
///
/// `GET /agents` is deliberately **not** on the list even though it serves the
/// same manifests. The well-known document is the public advertisement; the
/// `/agents` endpoint is the API. Keeping the advertisement open while the API
/// is closed is the intended shape, and it is why ACP defines both.
fn is_public(path: &str) -> bool {
    matches!(path, "/ping" | "/ready" | "/.well-known/agent.yml")
}

/// Reject anything that does not carry the expected bearer token.
async fn require_bearer(State(token): State<Arc<str>>, request: Request, next: Next) -> Response {
    if is_public(request.uri().path()) {
        return next.run(request).await;
    }

    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));

    // A real deployment should compare in constant time — `subtle::ConstantTimeEq`
    // or equivalent — or, better, not be comparing bearer strings at all. The
    // plain comparison here keeps the example about the wiring.
    match presented {
        Some(presented) if presented == &*token => next.run(request).await,
        _ => unauthorized(),
    }
}

/// A 401 with `WWW-Authenticate`, and a body that is *not* an ACP error object.
///
/// That is deliberate, and it is a detail specific to this protocol: ACP defines
/// exactly three error codes — `server_error`, `invalid_input` and `not_found` —
/// and none of them means "unauthenticated". Dressing a 401 up as one of them
/// would put a code on the wire that lies about what happened.
///
/// So this returns ordinary HTTP. `AcpClient` surfaces it as
/// `AcpError::Http { status: 401, .. }` rather than `AcpError::Protocol`, which
/// is the honest distinction: the request never reached ACP at all. It is also
/// not retried — a 401 is a verdict, not a blip.
fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"))],
        Body::from("missing or invalid bearer token"),
    )
        .into_response()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let token: Arc<str> =
        std::env::var("ACP_TOKEN").unwrap_or_else(|_| "demo-token".to_string()).into();

    let greeter = agent_fn(
        AgentManifest::new(AgentName::new("greeter").unwrap(), "Greets whoever asks"),
        |ctx: RunContext| async move {
            ctx.reply_text(format!("Hello, {}!", ctx.input_text())).await?;
            Ok(())
        },
    );

    let router = AcpServer::builder()
        .agent(greeter)
        .build()?
        .into_router()
        // Auth wraps the whole router rather than being applied per route, so
        // an endpoint added later is closed by default. The exemptions are an
        // explicit list in one place, which is the safer direction to be wrong
        // in: a new route that should have been public is a visible 401, where
        // a new route that should have been private is silent.
        .layer(from_fn_with_state(token.clone(), require_bearer));
    // Browser callers need CORS as well — `tower_http::cors::CorsLayer` layered
    // the same way. It is left out here because it is ordinary axum with no ACP
    // subtlety to it, unlike the exemption list above.

    let listener = tokio::net::TcpListener::bind("127.0.0.1:8000").await?;
    let addr = listener.local_addr()?;
    println!("serving on http://{addr} — token: {token}");
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    demonstrate_client(&format!("http://{addr}"), &token).await?;

    println!("\nserver still running; curl it or press ctrl-c");
    std::future::pending::<()>().await;
    Ok(())
}

/// The client half: credentials belong on the `reqwest::Client`, not on ACP.
///
/// `AcpClientBuilder::http_client` exists for exactly this. Attaching the header
/// once as a default beats threading it through every call, and it means the
/// requests `AcpClient` makes on your behalf — following a session's history
/// URLs, reconnecting a dropped stream — carry it too, which per-call plumbing
/// would miss.
async fn demonstrate_client(base_url: &str, token: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Unauthenticated first, so the failure is visible rather than assumed.
    let anonymous = AcpClient::new(base_url)?;
    println!("\n== without a token ==");
    println!("  ping:   {:?}", anonymous.ping().await.map(|()| "ok"));
    match anonymous.list_all_agents().await {
        Ok(agents) => println!("  agents: {} (unexpected!)", agents.len()),
        Err(error) => println!("  agents: {error}"),
    }

    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))?,
    );
    let http = reqwest::Client::builder().default_headers(headers).build()?;
    let client = AcpClient::with_http_client(base_url, http)?;

    println!("\n== with a token ==");
    println!("  agents: {}", client.list_all_agents().await?.len());

    // In a session, so the next part has history to dereference.
    let session_id = SessionId::new();
    let run = client
        .create_run(
            RunCreateRequest::new(AgentName::new("greeter")?, [Message::user("Ada")])
                .with_session_id(session_id),
        )
        .await?;
    println!("  run:    {} — {}", run.status, run.output_text());

    // The part worth watching. A session's history is a list of **URLs**, not
    // messages, and reading it means following them — one authenticated request
    // per entry, made by the client rather than by you.
    //
    // It works here because the credential is on the HTTP client, so it travels
    // with whatever that client fetches. A scheme scoped to the *caller* rather
    // than to the *resource* — one-time nonces, per-replica secrets, tokens
    // audience-bound to a single host — breaks the moment a session's URLs point
    // at a server the follower cannot authenticate to, which is the ordinary
    // case once sessions are shared across replicas. ACP's premise is that those
    // URLs are dereferenceable by whoever holds them; whatever guards them has
    // to be satisfiable by whoever follows them.
    let session = client.get_session(session_id).await?;
    println!("  session: {} history urls", session.history.len());
    let history = client.fetch_session_history(&session).await?;
    println!("  followed them all: {:?}", history.iter().map(Message::text).collect::<Vec<_>>());

    Ok(())
}

/// Tests for the exemption list, which is the part of this example worth
/// getting right and the part that would rot without noticing.
///
/// They run against the real router rather than a description of it, so a
/// renamed or added ACP endpoint shows up here instead of in production.
#[cfg(test)]
mod tests {
    use super::*;

    /// Serve the example's router and return the base URL.
    async fn serve() -> String {
        let greeter = agent_fn(
            AgentManifest::new(AgentName::new("greeter").unwrap(), "Greets whoever asks"),
            |ctx: RunContext| async move { ctx.reply_text(ctx.input_text()).await.map(|_| ()) },
        );
        let token: Arc<str> = "test-token".into();
        let router = AcpServer::builder()
            .agent(greeter)
            .build()
            .unwrap()
            .into_router()
            .layer(from_fn_with_state(token, require_bearer));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }

    async fn status(url: &str, token: Option<&str>) -> u16 {
        let mut request = reqwest::Client::new().get(url);
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        request.send().await.unwrap().status().as_u16()
    }

    /// A load balancer has no credentials, and a crawler reading open discovery
    /// is the entire point of the well-known document.
    #[tokio::test]
    async fn the_health_checks_and_open_discovery_stay_open() {
        let base = serve().await;
        assert_eq!(status(&format!("{base}/ping"), None).await, 200);
        assert_eq!(status(&format!("{base}/ready"), None).await, 200);
        assert_eq!(status(&format!("{base}/.well-known/agent.yml"), None).await, 200);
    }

    /// Every other endpoint, including the `/agents` that serves the same
    /// manifests the open document does.
    #[tokio::test]
    async fn everything_else_needs_a_token() {
        let base = serve().await;
        for path in ["/agents", "/agents/greeter", "/runs", "/session/x", "/session/x/state"] {
            assert_eq!(status(&format!("{base}{path}"), None).await, 401, "{path} was open");
        }
    }

    #[tokio::test]
    async fn a_token_gets_through() {
        let base = serve().await;
        assert_eq!(status(&format!("{base}/agents"), Some("test-token")).await, 200);
        assert_eq!(status(&format!("{base}/agents"), Some("wrong")).await, 401);
    }

    /// A 401 is ordinary HTTP, not an ACP error object — ACP has no code that
    /// means "unauthenticated", so the client must see `Http`, not `Protocol`.
    #[tokio::test]
    async fn a_rejection_is_not_dressed_up_as_an_acp_error() {
        let base = serve().await;
        let error = AcpClient::new(&base).unwrap().list_all_agents().await.unwrap_err();
        assert!(matches!(error, rusty_acp::AcpError::Http { status: 401, .. }), "{error}");
    }
}
