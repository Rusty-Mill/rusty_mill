//! Telling a trimmed event log from a short run.
//!
//! #60 made a truncated log observable on the *resume* path: a client asking to
//! start from an event that has been dropped gets 410 Gone. The JSON list
//! beside it got nothing, so it handed back the retained tail with no
//! indication anything was missing — and that is the worse half. A client
//! resuming knows which index it wants and is told no; a client reading the
//! list has nothing to compare against, so a one-event answer for a
//! two-hundred-event run is indistinguishable from a run that emitted one
//! event.
//!
//! The list is still *served* rather than refused, because a caller that wants
//! whatever is left should be able to have it. It just says where it starts.

#![cfg(all(feature = "server", feature = "client"))]

use std::sync::Arc;

use rusty_acp::client::{AcpClient, RunEventLog};
use rusty_acp::server::store::{InMemoryStore, Store};
use rusty_acp::server::{agent_fn, AcpServer, RunContext};
use rusty_acp::types::{AgentManifest, AgentName, Message, MessagePart};
use rusty_acp::EVENTS_FROM_HEADER;

/// A server whose log bound is `limit`, and a client pointed at it.
async fn server_with(limit: usize) -> (Arc<dyn Store>, AcpClient, String) {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::default().with_max_run_event_bytes(limit));

    let chatty = agent_fn(
        AgentManifest::new(AgentName::new("chatty").unwrap(), "Emits a great deal"),
        |ctx: RunContext| async move {
            for _ in 0..60 {
                ctx.reply_part(MessagePart::text("y".repeat(2048))).await?;
            }
            Ok(())
        },
    );
    let brief = agent_fn(
        AgentManifest::new(AgentName::new("brief").unwrap(), "Says one thing"),
        |ctx: RunContext| async move { ctx.reply_text("done").await.map(|_| ()) },
    );

    let router = AcpServer::builder()
        .agent(chatty)
        .agent(brief)
        .store(Arc::clone(&store))
        .base_url("http://acp.example")
        .build()
        .unwrap()
        .into_router();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let base_url = format!("http://{addr}");
    (store, AcpClient::new(&base_url).unwrap(), base_url)
}

/// The whole claim: a trimmed log and a short run are not the same answer.
///
/// Both return a handful of events. Only one of them is the whole story, and
/// before this there was nothing in the response that said which.
#[tokio::test]
async fn a_trimmed_log_is_distinguishable_from_a_short_run() {
    let (_store, client, _base_url) = server_with(16 * 1024).await;

    let long = client.run_sync("chatty", [Message::user("go")]).await.unwrap();
    let short = client.run_sync("brief", [Message::user("go")]).await.unwrap();

    let trimmed = client.list_run_events(long.run_id).await.unwrap();
    let whole = client.list_run_events(short.run_id).await.unwrap();

    assert!(!trimmed.is_complete(), "a trimmed log claimed to be whole");
    assert!(whole.is_complete(), "an untouched log was reported as trimmed");
    assert!(trimmed.first_index.unwrap() > 0, "the trimmed log did not say where it starts");
    assert_eq!(whole.first_index, Some(0));
}

/// The header names the same index the store does, so the two cannot drift.
#[tokio::test]
async fn the_header_agrees_with_the_store() {
    let (store, client, base_url) = server_with(16 * 1024).await;
    let run = client.run_sync("chatty", [Message::user("go")]).await.unwrap();

    let earliest = store.earliest_event(run.run_id).await.unwrap();
    assert!(earliest > 0, "the log was not trimmed, so this proves nothing");

    let response = reqwest::Client::new()
        .get(format!("{base_url}/runs/{}/events", run.run_id))
        .send()
        .await
        .unwrap();
    let header = response
        .headers()
        .get(EVENTS_FROM_HEADER)
        .expect("the list response must say where it starts")
        .to_str()
        .unwrap()
        .to_string();

    assert_eq!(header, earliest.to_string());
}

/// The list is served, not refused. A caller wanting the tail can still have
/// it — which is the difference from the stream, where the client has named an
/// index and can be told that exact index is gone.
///
/// A guard rather than a discriminator: it passes with or without the header,
/// and exists to catch a later change that answers 410 here too. That was one
/// of the four candidates in #67 and it is a defensible one, so if it is ever
/// chosen this test should be the place the argument is made.
#[tokio::test]
async fn a_trimmed_log_is_still_served() {
    let (_store, client, _base_url) = server_with(16 * 1024).await;
    let run = client.run_sync("chatty", [Message::user("go")]).await.unwrap();

    let log = client.list_run_events(run.run_id).await.unwrap();

    assert!(!log.events.is_empty(), "the tail was refused rather than served");
    assert!(!log.is_complete());
}

/// A server that says nothing is reported as "unknown", not as "complete".
///
/// The cautious answer, and the reason `first_index` is an `Option`: a client
/// talking to a server from before the header cannot conclude the log is whole,
/// and defaulting to zero would have it conclude exactly that.
#[test]
fn a_missing_header_is_not_a_claim_of_completeness() {
    let silent = RunEventLog { events: Vec::new(), first_index: None };
    assert!(!silent.is_complete(), "an unknown log was treated as a whole one");
    assert_eq!(silent.dropped(), None);
}

/// The convenience impls, so the type change did not cost callers their
/// ergonomics: iterating and measuring work as they did on a bare `Vec`.
///
/// A guard too. `Deref` and the two `IntoIterator` impls are why every existing
/// caller — the example, four tests — compiled unchanged against the new return
/// type, and dropping one would break them silently at the next edit.
#[tokio::test]
async fn the_log_still_behaves_like_a_list() {
    let (_store, client, _base_url) = server_with(1024 * 1024).await;
    let run = client.run_sync("brief", [Message::user("go")]).await.unwrap();

    let log = client.list_run_events(run.run_id).await.unwrap();

    let by_deref = log.len();
    let by_ref = (&log).into_iter().count();
    let by_value = log.into_iter().count();

    assert_eq!(by_deref, by_ref);
    assert_eq!(by_ref, by_value);
    assert!(by_value > 0);
}
