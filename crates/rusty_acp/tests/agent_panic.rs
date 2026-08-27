//! What happens when an agent panics.
//!
//! A panic used to unwind the task that owned every piece of a run's cleanup,
//! so the run was left `in-progress` with no terminal transition — and, because
//! dropping a `JoinHandle` detaches the task rather than cancelling it, with an
//! orphaned renewal keeping its lease alive. A live lease is exactly what tells
//! a reaper the run still has a writer, so every replica that read it left it
//! alone. The run could not be failed, reaped, recovered or resumed by anything.
//!
//! Four separate mechanisms were failing, so they are asserted separately
//! rather than through one "the run failed" check that would pass again the
//! moment three of them broke.
//!
//! These tests panic on purpose. The backtraces printed during the run are the
//! agents doing what they were written to do, not the suite failing.

#![cfg(all(feature = "server", feature = "client"))]

use std::sync::Arc;
use std::time::Duration;

use rusty_acp::client::AcpClient;
use rusty_acp::server::store::{InMemoryStore, Store};
use rusty_acp::server::{agent_fn, AcpServer, RunContext};
use rusty_acp::types::{
    AgentManifest, AgentName, AwaitRequest, AwaitResume, Message, RunMode, RunResumeRequest,
    RunStatus,
};

/// Short enough that "has the lease lapsed" is answerable inside a test.
const LEASE_TTL: Duration = Duration::from_millis(500);

/// A replica hosting one agent that panics and one that does not.
async fn replica() -> (Arc<AcpServer>, Arc<dyn Store>, AcpClient) {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::default());

    let exploding = agent_fn(
        AgentManifest::new(AgentName::new("boom").unwrap(), "Panics immediately"),
        |_ctx: RunContext| async move { panic!("the agent had a bad day") },
    );
    // Panics *after* yielding, so the panic lands on a later poll rather than
    // the first — a synchronous panic and one from inside an await are
    // different paths through the task machinery.
    let late = agent_fn(
        AgentManifest::new(AgentName::new("late").unwrap(), "Panics after a wait"),
        |_ctx: RunContext| async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            panic!("{}", format!("failed on turn {}", 2))
        },
    );
    // Parks for a client answer, then panics once it arrives — the panic lands
    // after the slot has been unparked and the capacity taken back.
    let asker = agent_fn(
        AgentManifest::new(AgentName::new("asker").unwrap(), "Panics after being resumed"),
        |ctx: RunContext| async move {
            ctx.await_request(AwaitRequest::new(serde_json::json!({ "q": "?" }))).await?;
            panic!("the agent had a bad day")
        },
    );
    let fine = agent_fn(
        AgentManifest::new(AgentName::new("fine").unwrap(), "Works"),
        |ctx: RunContext| async move { ctx.reply_text("ok").await.map(|_| ()) },
    );

    let (server, router) = AcpServer::builder()
        .agent(exploding)
        .agent(late)
        .agent(asker)
        .agent(fine)
        .store(Arc::clone(&store))
        .base_url("http://acp.example")
        .lease_ttl(LEASE_TTL)
        .build()
        .unwrap()
        .into_shared_router();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let client = AcpClient::new(format!("http://{addr}")).unwrap();
    (server, store, client)
}

/// A panic ends the run, the same as returning `Err` would.
#[tokio::test]
async fn a_panicking_agent_fails_its_run() {
    let (_server, store, client) = replica().await;

    let run = client.run_async("boom", [Message::user("go")]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    let after = store.get_run(run.run_id).await.unwrap().expect("the run exists");
    assert_eq!(after.status, RunStatus::Failed, "a panicked run never reached a terminal state");
}

/// Panicking from inside an await, rather than before the first one, is the
/// same. Tested apart because it is a different path through the task: the
/// panic lands on a later poll, after the future has already been suspended.
#[tokio::test]
async fn a_panic_after_yielding_is_the_same() {
    let (_server, store, client) = replica().await;

    let run = client.run_async("late", [Message::user("go")]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;

    let after = store.get_run(run.run_id).await.unwrap().expect("the run exists");
    assert_eq!(after.status, RunStatus::Failed);
}

/// The lease is released, so nothing has to wait out the TTL and no reaper is
/// ever fooled into thinking the run still has a writer.
///
/// This is the assertion that matters most. The run reaching `failed` says the
/// outcome was recorded; this says the *renewal* stopped, which is the piece a
/// panic used to leave running forever.
#[tokio::test]
async fn a_panicking_agent_releases_its_lease() {
    let (_server, store, client) = replica().await;

    let run = client.run_async("boom", [Message::user("go")]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert_eq!(
        store.lease_owner(run.run_id).await.unwrap(),
        None,
        "the renewal outlived the panic, so no replica will ever reap this run"
    );

    // Well past the TTL, in case the release above was really an expiry that
    // happened to land: a still-running renewal would have pushed it out again.
    tokio::time::sleep(LEASE_TTL * 3).await;
    assert_eq!(store.lease_owner(run.run_id).await.unwrap(), None);
}

/// The run leaves the in-flight set, so the gauge is not wrong for the life of
/// the process and a drain does not hold entries it reports as clean.
#[tokio::test]
async fn a_panicking_agent_leaves_the_in_flight_set() {
    let (server, _store, client) = replica().await;

    for _ in 0..3 {
        client.run_async("boom", [Message::user("go")]).await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(400)).await;

    assert_eq!(server.in_flight(), 0, "panicked runs accumulated in the in-flight set");
    assert_eq!(server.executing(), 0);
}

/// A `sync` caller is released rather than left to time out.
///
/// The whole cost of this bug landed on the caller: a run that never reaches a
/// terminal state never publishes the event that releases them.
#[tokio::test]
async fn a_sync_caller_is_released_by_the_panic() {
    let (_server, _store, client) = replica().await;

    let began = std::time::Instant::now();
    let run = client.run_sync("boom", [Message::user("go")]).await.unwrap();

    assert_eq!(run.status, RunStatus::Failed);
    assert!(
        began.elapsed() < Duration::from_secs(5),
        "the caller waited {:?}, which means it was waiting on the timeout",
        began.elapsed()
    );
}

/// The client is told the agent panicked, and is not told what it said.
///
/// A panic payload carries paths, values and whatever else was in scope. The
/// operator gets it in the log; a remote caller gets the fact and no more.
/// Asserted so a later change that helpfully forwards the message has to come
/// here and argue for it.
#[tokio::test]
async fn the_panic_message_does_not_reach_the_client() {
    let (_server, _store, client) = replica().await;

    let run = client.run_sync("boom", [Message::user("go")]).await.unwrap();

    let error = run.error.expect("a failed run carries an error");
    assert!(
        error.message.contains("panicked"),
        "a caller cannot tell this from any other server error: {}",
        error.message
    );
    assert!(
        !error.message.contains("bad day"),
        "the panic payload was forwarded to the client: {}",
        error.message
    );
}

/// The replica keeps working. A panic is one run's problem.
///
/// A guard rather than a discriminator: this passed before the fix too, just
/// 300 seconds slower, because the bug never threatened the replica — only the
/// runs on it. Kept because "a panic takes the host down" is a plausible way to
/// break this later, and nothing else here would catch it.
#[tokio::test]
async fn the_replica_survives() {
    let (server, _store, client) = replica().await;

    client.run_sync("boom", [Message::user("go")]).await.unwrap();
    let healthy = client.run_sync("fine", [Message::user("go")]).await.unwrap();

    assert_eq!(healthy.status, RunStatus::Completed);
    assert_eq!(healthy.output_text(), "ok");
    assert!(server.readiness().await.is_ready());
}

/// A panic *after* the agent came back from `await_request` is the same.
///
/// Worth its own test because the run has been through the slot's park and
/// unpark by then, and the capacity bookkeeping a panic used to skip is the
/// bookkeeping that path had just finished touching.
#[tokio::test]
async fn a_panic_after_a_resume_is_the_same() {
    let (server, store, client) = replica().await;

    let parked = client.run_sync("asker", [Message::user("hi")]).await.unwrap();
    assert_eq!(parked.status, RunStatus::Awaiting, "the agent should be waiting for an answer");

    let resumed = client
        .resume_run(RunResumeRequest::new(
            parked.run_id,
            AwaitResume::new(serde_json::json!({ "answer": "go on" })),
            RunMode::Async,
        ))
        .await
        .unwrap();
    let _ = resumed;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let after = store.get_run(parked.run_id).await.unwrap().expect("the run exists");
    assert_eq!(after.status, RunStatus::Failed);
    assert_eq!(store.lease_owner(parked.run_id).await.unwrap(), None);
    assert_eq!(server.in_flight(), 0, "the resumed run never left the in-flight set");
    assert_eq!(server.executing(), 0, "the slot reacquired on resume was never given back");
}

/// A drain after a panic reports nothing outstanding, and returns promptly.
///
/// The leaked in-flight entry was invisible through `Drained` — the all-clear
/// early return covered for it — so this asserts the count directly as well.
#[tokio::test]
async fn a_drain_after_a_panic_is_clean() {
    let (server, _store, client) = replica().await;

    client.run_sync("boom", [Message::user("go")]).await.unwrap();

    let began = std::time::Instant::now();
    let drained = server.drain(Duration::from_secs(5)).await;

    assert!(drained.is_empty(), "a drain found work left over from a panicked run");
    assert_eq!(server.in_flight(), 0);
    assert!(began.elapsed() < Duration::from_secs(5), "the drain waited on the panicked run");
}
