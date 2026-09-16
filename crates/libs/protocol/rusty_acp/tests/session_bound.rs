//! A ceiling on how much history one session may hold.
//!
//! The limit none of the others reached. `max_sessions` bounds how *many*
//! sessions the default store keeps, a Redis TTL and a Postgres retention
//! window bound how *old* they get, and #60/#68 bounded one run's event log —
//! nothing bounded how long a single conversation grows, on any backend.
//!
//! It is latency as much as memory: an agent is handed its whole history on
//! every turn, so the cost climbs with the length and never levels off. That is
//! what `load_state`/`store_state` exist to let an agent avoid, and they are
//! still the real answer; this is the backstop.
//!
//! The shape is a **gate, not a cap**. A run whose session is already full is
//! refused before it starts, so the caller learns while it can still do
//! something — where failing the output append at the other end would deliver
//! the same verdict after the work, to a caller with no remaining choice.

#![cfg(all(feature = "server", feature = "client"))]

use std::sync::Arc;

use rusty_acp::client::AcpClient;
use rusty_acp::server::store::{InMemoryStore, Store};
use rusty_acp::server::{agent_fn, AcpServer, AcpServerBuilder, RunContext};
use rusty_acp::types::{AgentManifest, AgentName, ErrorCode, Message, MessagePart, SessionId};
use rusty_acp::AcpError;

/// Roughly the size of one turn, so a test can count in whole turns.
const TURN: usize = 4096;

fn one_turn() -> Message {
    Message::user("u".repeat(TURN))
}

/// A server whose session ceiling is `limit`, or unlimited for `None`.
async fn server_with(limit: Option<usize>) -> (Arc<dyn Store>, AcpClient) {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::default());
    let quiet = agent_fn(
        AgentManifest::new(AgentName::new("quiet").unwrap(), "Says very little"),
        |ctx: RunContext| async move { ctx.reply_text("ok").await.map(|_| ()) },
    );

    let builder: AcpServerBuilder =
        AcpServer::builder().agent(quiet).store(Arc::clone(&store)).base_url("http://acp.example");
    let builder = match limit {
        Some(limit) => builder.max_session_bytes(limit),
        None => builder.without_session_limit(),
    };

    let router = builder.build().unwrap().into_router();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    (store, AcpClient::new(format!("http://{addr}")).unwrap())
}

/// Fill `session` until the server starts refusing, returning how many turns
/// it took and the refusal.
async fn fill_until_refused(client: &AcpClient, session: SessionId) -> (usize, AcpError) {
    for turn in 0..64 {
        let request =
            rusty_acp::types::RunCreateRequest::new(AgentName::new("quiet").unwrap(), [one_turn()])
                .with_session_id(session);
        match client.create_run(request).await {
            Ok(_) => continue,
            Err(error) => return (turn, error),
        }
    }
    panic!("the session was never refused");
}

/// The claim: a session past its ceiling stops accepting runs, and says why.
#[tokio::test]
async fn a_full_session_is_refused_rather_than_grown() {
    // Room for a handful of turns.
    let (_store, client) = server_with(Some(6 * TURN)).await;
    let session = SessionId::new();

    let (turns, error) = fill_until_refused(&client, session).await;

    assert!(
        turns > 0,
        "the very first run was refused; the ceiling is too tight to prove anything"
    );
    let AcpError::Protocol(protocol) = &error else {
        panic!("expected a protocol error, got {error:?}");
    };
    let (code, message) = (protocol.code, &protocol.message);
    // `invalid_input`, not `server_error`: nothing broke here, and the caller is
    // the only one who can resolve it. It is also a 400, so it is not retried —
    // retrying is exactly the wrong answer to a session that is full.
    assert_eq!(code, ErrorCode::InvalidInput, "a full session was reported as a server fault");
    assert!(message.contains("session history"), "the error did not name the cause: {message}");
    assert!(
        message.contains("store_state"),
        "the error did not point at the alternative: {message}"
    );
}

/// The refusal happens *before* the run, so there is no run to find.
///
/// This is the whole reason the check sits at admission rather than on the
/// output append: a caller refused up front has lost nothing and can choose
/// again, where one failed at the end has already paid for the work.
#[tokio::test]
async fn a_refused_run_never_started() {
    let (store, client) = server_with(Some(6 * TURN)).await;
    let session = SessionId::new();

    let (turns, _) = fill_until_refused(&client, session).await;

    // Exactly the runs that were accepted, and not one more.
    let record = store.get_session(session).await.unwrap().expect("the session exists");
    let user_turns =
        record.messages.iter().filter(|m| matches!(m.role, rusty_acp::types::Role::User)).count();
    assert_eq!(user_turns, turns, "a refused run still wrote its input to the session");
}

/// A session inside the ceiling is untouched — ordinary conversations are not
/// being quietly clipped.
#[tokio::test]
async fn a_session_inside_the_ceiling_is_unaffected() {
    let (_store, client) = server_with(Some(1024 * 1024)).await;
    let session = SessionId::new();

    for _ in 0..5 {
        let request = rusty_acp::types::RunCreateRequest::new(
            AgentName::new("quiet").unwrap(),
            [Message::user("hi")],
        )
        .with_session_id(session);
        client.create_run(request).await.expect("an ordinary conversation was refused");
    }

    let session = client.get_session(session).await.unwrap();
    assert!(session.history.len() >= 5, "the conversation lost turns");
}

/// A run with no session is never gated, whatever the ceiling.
///
/// The check is about what a session accumulates, and a sessionless run
/// accumulates nothing. Worth its own case because the ceiling is enforced on
/// the creation path that both kinds of run share.
#[tokio::test]
async fn a_sessionless_run_is_never_refused() {
    let (_store, client) = server_with(Some(1)).await;

    for _ in 0..3 {
        client
            .run_sync("quiet", [one_turn()])
            .await
            .expect("a run with no session was gated by the session ceiling");
    }
}

/// The ceiling can be removed.
#[tokio::test]
async fn the_ceiling_can_be_removed() {
    let (_store, client) = server_with(None).await;
    let session = SessionId::new();

    for _ in 0..12 {
        let request =
            rusty_acp::types::RunCreateRequest::new(AgentName::new("quiet").unwrap(), [one_turn()])
                .with_session_id(session);
        client.create_run(request).await.expect("an unlimited session refused a run");
    }
}

/// History and the incoming input are both counted.
///
/// A single oversized turn is refused against an empty session, which is the
/// case a check that only looked at stored history would wave through — and
/// then store, leaving the session over its ceiling on the very first run.
#[tokio::test]
async fn the_incoming_input_counts_too() {
    let (_store, client) = server_with(Some(2 * TURN)).await;
    let session = SessionId::new();

    let huge =
        Message::new(rusty_acp::types::Role::User, [MessagePart::text("x".repeat(8 * TURN))]);
    let request = rusty_acp::types::RunCreateRequest::new(AgentName::new("quiet").unwrap(), [huge])
        .with_session_id(session);

    let error = client.create_run(request).await.expect_err("an oversized first turn was accepted");
    let AcpError::Protocol(protocol) = &error else {
        panic!("expected a protocol error, got {error:?}");
    };
    assert_eq!(protocol.code, ErrorCode::InvalidInput);
}
