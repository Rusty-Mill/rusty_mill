//! Bounding a single run's event log.
//!
//! `max_runs` bounds how many runs are kept and `max_sessions` how many
//! sessions, and neither reaches the third dimension: one run's log. A
//! streaming agent emits one event per token, and a non-terminal run is never
//! evicted — correctly, since evicting a live run would delete something a
//! client is watching — so one long stream grew with no bound applying to it.
//!
//! The log is not a cache. It **is** the run's output, and what `Last-Event-ID`
//! replays from, so dropping the oldest events is only safe if a client that
//! then resumes from before them is told rather than handed a shorter prefix
//! that reads as complete. That refusal is as much the subject here as the
//! bound itself.

#![cfg(all(feature = "server", feature = "client"))]

use std::sync::Arc;

use rusty_acp::server::store::{InMemoryStore, Store};
use rusty_acp::server::{agent_fn, AcpServer, RunContext};
use rusty_acp::types::{AgentManifest, AgentName, Event, Message, MessagePart, Run, RunId};

/// A part of roughly `bytes` bytes, so a test can count in whole events.
fn sized_event(bytes: usize) -> Event {
    Event::MessagePart { part: MessagePart::text("x".repeat(bytes)) }
}

async fn store_with(limit: usize) -> (Arc<InMemoryStore>, RunId) {
    let store = Arc::new(InMemoryStore::default().with_max_run_event_bytes(limit));
    let run = Run::new(AgentName::new("probe").unwrap(), None);
    store.put_run(&run).await.unwrap();
    (store, run.run_id)
}

#[tokio::test]
async fn a_runs_log_is_bounded() {
    let (store, run_id) = store_with(64 * 1024).await;

    for _ in 0..200 {
        store.append_event(run_id, &sized_event(4 * 1024)).await.unwrap();
    }

    let retained = store.events(run_id).await.unwrap();
    assert!(retained.len() < 200, "the log kept everything it was given");
    let held: usize = retained.iter().map(Event::approximate_size).sum();
    assert!(held <= 64 * 1024, "retained {held} bytes against a 64 KiB limit");
}

/// Indices keep counting past a trim.
///
/// They used to be `events.len() - 1`, which stops being the count the moment
/// the front can be dropped. A restarted index would hand two different events
/// the same `Last-Event-ID`, and a resuming client would silently skip or
/// repeat everything between.
#[tokio::test]
async fn indices_do_not_restart_when_the_front_is_dropped() {
    let (store, run_id) = store_with(16 * 1024).await;

    let mut indices = Vec::new();
    for _ in 0..50 {
        indices.push(store.append_event(run_id, &sized_event(2 * 1024)).await.unwrap());
    }

    assert_eq!(indices, (0..50).collect::<Vec<u64>>(), "indices restarted or repeated");
}

/// Reading from an index still held returns exactly that suffix.
///
/// The skip is relative to the earliest retained event rather than to zero;
/// getting that wrong returns real events from the wrong position, which is
/// worse than returning none.
#[tokio::test]
async fn reading_from_a_retained_index_is_correctly_aligned() {
    let (store, run_id) = store_with(16 * 1024).await;

    for index in 0..50u64 {
        let part = MessagePart::text(format!("{index:0>2048}"));
        store.append_event(run_id, &Event::MessagePart { part }).await.unwrap();
    }

    let earliest = store.earliest_event(run_id).await.unwrap();
    assert!(earliest > 0, "nothing was dropped, so this proves nothing");

    let from = earliest + 1;
    let tail = store.events_from(run_id, from).await.unwrap();
    let Event::MessagePart { part } = &tail[0] else { panic!("expected a message part") };
    assert_eq!(
        part.content.as_deref().unwrap().trim_start_matches('0'),
        from.to_string(),
        "events_from returned the wrong position in the log"
    );
}

/// A store that has dropped nothing reports zero, so nothing is refused on a
/// log that is whole.
#[tokio::test]
async fn an_untrimmed_log_reports_no_loss() {
    let (store, run_id) = store_with(1024 * 1024).await;

    for _ in 0..10 {
        store.append_event(run_id, &sized_event(64)).await.unwrap();
    }

    assert_eq!(store.earliest_event(run_id).await.unwrap(), 0);
}

/// The event just emitted is always kept, even if it alone is over the limit.
///
/// A log that dropped what it was being given could not serve even a live tail,
/// and an agent emitting one oversized artifact would produce a run whose
/// stream shows nothing at all.
#[tokio::test]
async fn the_newest_event_is_never_dropped() {
    let (store, run_id) = store_with(1024).await;

    store.append_event(run_id, &sized_event(64 * 1024)).await.unwrap();

    let retained = store.events(run_id).await.unwrap();
    assert_eq!(retained.len(), 1, "the log dropped the event it was just handed");
}

/// A resume from events that are gone is refused, not served short.
///
/// The whole point of the decision: replaying from the earliest retained event
/// would hand the client a log with a hole in it that reads as complete, which
/// is the silent loss the resumable stream exists to avoid.
#[tokio::test]
async fn resuming_from_dropped_events_is_refused() {
    let store = Arc::new(InMemoryStore::default().with_max_run_event_bytes(16 * 1024));
    let store: Arc<dyn Store> = store;

    let chatty = agent_fn(
        AgentManifest::new(AgentName::new("chatty").unwrap(), "Emits a great deal"),
        |ctx: RunContext| async move {
            for _ in 0..60 {
                ctx.reply_part(MessagePart::text("y".repeat(2048))).await?;
            }
            Ok(())
        },
    );
    let router = AcpServer::builder()
        .agent(chatty)
        .store(Arc::clone(&store))
        .base_url("http://acp.example")
        .build()
        .unwrap()
        .into_router();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let client = rusty_acp::client::AcpClient::new(format!("http://{addr}")).unwrap();
    let run = client.run_sync("chatty", [Message::user("go")]).await.unwrap();

    let earliest = store.earliest_event(run.run_id).await.unwrap();
    assert!(earliest > 1, "the log was not trimmed, so this proves nothing");

    // Resume as a dropped stream would: from an event that is gone.
    let response = reqwest::Client::new()
        .get(format!("http://{addr}/runs/{}/events", run.run_id))
        .header("accept", "text/event-stream")
        .header("last-event-id", "0")
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        reqwest::StatusCode::GONE,
        "a resume from dropped events was served rather than refused"
    );
    let body = response.text().await.unwrap();
    assert!(
        body.contains(&earliest.to_string()),
        "the refusal does not say where to pick up: {body}"
    );
}

/// A resume from an index still held is served as it always was.
///
/// The refusal must not become a blanket one — the ordinary reconnection is the
/// case that matters most, and it is untouched.
#[tokio::test]
async fn resuming_from_a_retained_index_still_works() {
    let store: Arc<dyn Store> =
        Arc::new(InMemoryStore::default().with_max_run_event_bytes(16 * 1024));

    let chatty = agent_fn(
        AgentManifest::new(AgentName::new("chatty").unwrap(), "Emits a great deal"),
        |ctx: RunContext| async move {
            for _ in 0..60 {
                ctx.reply_part(MessagePart::text("y".repeat(2048))).await?;
            }
            Ok(())
        },
    );
    let router = AcpServer::builder()
        .agent(chatty)
        .store(Arc::clone(&store))
        .base_url("http://acp.example")
        .build()
        .unwrap()
        .into_router();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let client = rusty_acp::client::AcpClient::new(format!("http://{addr}")).unwrap();
    let run = client.run_sync("chatty", [Message::user("go")]).await.unwrap();
    let earliest = store.earliest_event(run.run_id).await.unwrap();

    let response = reqwest::Client::new()
        .get(format!("http://{addr}/runs/{}/events", run.run_id))
        .header("accept", "text/event-stream")
        // The header names the last event *seen*; resumption starts after it.
        .header("last-event-id", earliest.to_string())
        .send()
        .await
        .unwrap();

    assert!(
        response.status().is_success(),
        "an ordinary resume was refused: {}",
        response.status()
    );
}
