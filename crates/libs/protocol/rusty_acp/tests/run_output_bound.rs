//! A ceiling on how much output one run may accumulate.
//!
//! #60 and #68 bounded the event *log* on all three backends, on the argument
//! that a TTL or a retention window bounds how long a log is kept and not how
//! much. `Run::output` is the same content assembled into messages, it is
//! written on every status transition, returned by `GET /runs/{run_id}`, and
//! carried whole inside every `run.*` event — and nothing bounded it.
//!
//! This one **fails the run** rather than dropping the oldest, which is the
//! opposite of what the log does, and the difference is the whole subject.
//! An event log has a vocabulary for a hole: `earliest_event`, the 410, and
//! the `Acp-Events-From` header #72 added. `Run::output` is a plain list in
//! the ACP schema with nowhere to record one — and every `run.*` event carries
//! the whole `Run` over SSE, where there is no header to put a caveat in. A
//! truncated output would therefore read as a short one on the endpoint *and*
//! on the stream, which is the silent loss this crate keeps refusing.

#![cfg(all(feature = "server", feature = "client"))]

use std::sync::Arc;

use rusty_acp::client::AcpClient;
use rusty_acp::server::store::{InMemoryStore, Store};
use rusty_acp::server::{agent_fn, AcpServer, AcpServerBuilder, RunContext};
use rusty_acp::types::{AgentManifest, AgentName, Message, MessagePart, RunStatus};

/// Roughly the size of one emitted message, so a test can count in whole
/// messages rather than in bytes it has to keep in step with the source.
const CHUNK: usize = 4096;

/// Emits `count` separate completed messages of about [`CHUNK`] bytes each.
fn chatty(count: usize) -> impl rusty_acp::server::Agent {
    agent_fn(
        AgentManifest::new(AgentName::new("chatty").unwrap(), "Says a great deal"),
        move |ctx: RunContext| async move {
            for _ in 0..count {
                ctx.reply_text("y".repeat(CHUNK)).await?;
            }
            Ok(())
        },
    )
}

/// A server whose output ceiling is `limit`, or unlimited for `None`.
async fn server_with(limit: Option<usize>, messages: usize) -> (Arc<dyn Store>, AcpClient) {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::default());
    let builder: AcpServerBuilder = AcpServer::builder()
        .agent(chatty(messages))
        .store(Arc::clone(&store))
        .base_url("http://acp.example");
    let builder = match limit {
        Some(limit) => builder.max_run_output_bytes(limit),
        None => builder.without_run_output_limit(),
    };

    let router = builder.build().unwrap().into_router();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    (store, AcpClient::new(format!("http://{addr}")).unwrap())
}

/// The claim: past the ceiling the run fails, and says why.
///
/// Not truncated, not silently short. A caller reading `output` gets either
/// everything the run produced or an explicit failure — never a plausible
/// prefix it cannot tell from the whole thing.
#[tokio::test]
async fn a_run_past_the_ceiling_fails_rather_than_truncating() {
    // Room for about four messages; the agent tries twenty.
    let (_store, client) = server_with(Some(4 * CHUNK + CHUNK / 2), 20).await;

    let run = client.run_sync("chatty", [Message::user("go")]).await.unwrap();

    assert_eq!(run.status, RunStatus::Failed, "an oversized run was not failed");
    let error = run.error.expect("a failed run must carry its error");
    assert!(
        error.message.contains("output exceeded"),
        "the error did not name the ceiling: {}",
        error.message
    );
    // And it points somewhere useful rather than only saying no.
    assert!(
        error.message.contains("content_url"),
        "the error did not suggest the alternative: {}",
        error.message
    );
}

/// A run inside the ceiling is untouched — the limit is not quietly clipping
/// ordinary runs.
#[tokio::test]
async fn a_run_inside_the_ceiling_is_unaffected() {
    let (_store, client) = server_with(Some(64 * CHUNK), 8).await;

    let run = client.run_sync("chatty", [Message::user("go")]).await.unwrap();

    assert_eq!(run.status, RunStatus::Completed);
    assert_eq!(run.output.len(), 8, "the run did not report everything it produced");
}

/// The ceiling can be removed, for a deployment that would rather risk the
/// memory than lose the run.
#[tokio::test]
async fn the_ceiling_can_be_removed() {
    let (_store, client) = server_with(None, 40).await;

    let run = client.run_sync("chatty", [Message::user("go")]).await.unwrap();

    assert_eq!(run.status, RunStatus::Completed);
    assert_eq!(run.output.len(), 40);
}

/// What the run reports and what the store holds agree.
///
/// The failure is written down, not merely returned: a client that comes back
/// later with `GET /runs/{run_id}` sees the same verdict as the one that was
/// waiting. Failing only the response would leave a run looking successful to
/// everyone who did not happen to be holding the connection.
#[tokio::test]
async fn the_failure_is_recorded_not_just_returned() {
    let (_store, client) = server_with(Some(4 * CHUNK + CHUNK / 2), 20).await;
    let run = client.run_sync("chatty", [Message::user("go")]).await.unwrap();

    let read_back = client.get_run(run.run_id).await.unwrap();

    assert_eq!(read_back.status, RunStatus::Failed);
    assert_eq!(read_back.output.len(), run.output.len(), "two readers saw different output");
}

/// The events are still there. The ceiling bounds the *aggregate*, not the log,
/// so a client that watched the stream saw every part the agent emitted right
/// up to the point the run failed.
///
/// This is what makes failing defensible rather than merely strict: the work is
/// observable, it just is not reported twice.
#[tokio::test]
async fn the_event_log_still_holds_what_was_emitted() {
    let (_store, client) = server_with(Some(4 * CHUNK + CHUNK / 2), 20).await;
    let run = client.run_sync("chatty", [Message::user("go")]).await.unwrap();

    let log = client.list_run_events(run.run_id).await.unwrap();

    let parts = log
        .iter()
        .filter(|event| matches!(event, rusty_acp::types::Event::MessagePart { .. }))
        .count();
    assert!(parts > 0, "the log lost the parts the run did emit");
    assert!(
        log.iter().any(|event| matches!(event, rusty_acp::types::Event::RunFailed { .. })),
        "the log did not record the failure"
    );
}

/// The size estimate is the same one the log bound uses, so the two settings
/// are comparable rather than merely both being called bytes.
///
/// A guard: it passes either way today, and exists so that a later change to
/// one estimate has to reckon with the other. The two limits are documented as
/// sitting in a particular order — the log trims well before the run dies —
/// and that ordering is meaningless if they count different things.
#[test]
fn a_message_is_measured_as_the_sum_of_its_parts() {
    let message = Message::new(
        rusty_acp::types::Role::Agent,
        [MessagePart::text("abc"), MessagePart::text("de")],
    );

    let by_message = message.approximate_size();
    let by_parts: usize = message.parts.iter().map(MessagePart::approximate_size).sum();

    assert_eq!(by_message, by_parts);
    assert!(by_message >= 5, "the inline content was not counted");
}
