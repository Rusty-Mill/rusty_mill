//! Draining a replica that is being deployed over.
//!
//! Leases handle a replica that *dies*: nothing can be done from inside a
//! process that is already gone, so the run is failed by whoever notices the
//! lapsed lease. A replica being deployed over is the same situation wearing
//! the same clothes with one difference — it knows. These tests are about
//! spending that knowledge.
//!
//! Nothing here sleeps and hopes. The agent blocks on a channel the test holds,
//! so "the drain waited for a run in flight" is asserted by observing the drain
//! still pending while the run is provably unable to have finished, rather than
//! by racing a timer against it.

#![cfg(all(feature = "server", feature = "client"))]

use std::sync::Arc;
use std::time::Duration;

use rusty_acp::client::AcpClient;
use rusty_acp::server::store::{InMemoryStore, Store};
use rusty_acp::server::{agent_fn, AcpServer, RunContext};
use rusty_acp::types::{
    AgentManifest, AgentName, AwaitResume, Message, Run, RunId, RunMode, RunResumeRequest,
    RunStatus,
};
use tokio::sync::{mpsc, oneshot, Mutex};

/// Long enough that a drain waiting on it has visibly waited, short enough not
/// to slow the suite when it is the deadline being exceeded.
const SHORT_DEADLINE: Duration = Duration::from_millis(150);

/// A replica, and the controls for the agent running on it.
struct Replica {
    server: Arc<AcpServer>,
    client: AcpClient,
    store: Arc<dyn Store>,
    /// Signalled by the agent once it is running, so a test never has to guess
    /// whether the run has started.
    started: Mutex<mpsc::UnboundedReceiver<()>>,
    /// Releases the agent. Until this is sent, the run cannot finish — which is
    /// what makes "the drain is still waiting" an observation rather than a
    /// race.
    release: Mutex<Option<oneshot::Sender<()>>>,
}

impl Replica {
    async fn new(store: Arc<dyn Store>) -> Self {
        let (started_tx, started_rx) = mpsc::unbounded_channel();
        let (release_tx, release_rx) = oneshot::channel();
        let release_rx = Arc::new(Mutex::new(Some(release_rx)));

        let blocking = agent_fn(
            AgentManifest::new(AgentName::new("blocking").unwrap(), "Runs until released"),
            move |ctx: RunContext| {
                let started = started_tx.clone();
                let release = Arc::clone(&release_rx);
                async move {
                    let _ = started.send(());
                    if let Some(release) = release.lock().await.take() {
                        let _ = release.await;
                    }
                    ctx.reply_text("released").await?;
                    Ok(())
                }
            },
        );

        // Parks awaiting a client answer, to check that a drain does not sever
        // a conversation already in progress.
        let asker = agent_fn(
            AgentManifest::new(AgentName::new("asker").unwrap(), "Pauses to ask a question"),
            |ctx: RunContext| async move {
                let resume = ctx.await_json(serde_json::json!({ "question": "name?" })).await?;
                let answer = resume.as_value()["answer"].as_str().unwrap_or("stranger").to_string();
                ctx.reply_text(format!("hello {answer}")).await?;
                Ok(())
            },
        );

        let (server, router) = AcpServer::builder()
            .agent(blocking)
            .agent(asker)
            .store(Arc::clone(&store))
            .base_url("http://acp.example")
            .build()
            .unwrap()
            .into_shared_router();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

        Self {
            server,
            client: AcpClient::new(format!("http://{addr}")).unwrap(),
            store,
            started: Mutex::new(started_rx),
            release: Mutex::new(Some(release_tx)),
        }
    }

    async fn memory() -> Self {
        Self::new(Arc::new(InMemoryStore::default())).await
    }

    /// Start a run and return once the agent is provably executing it.
    async fn start_blocking_run(&self) -> Run {
        let run = self.client.run_async("blocking", [Message::user("go")]).await.unwrap();
        self.started.lock().await.recv().await.expect("the agent starts");
        run
    }

    async fn release_agent(&self) {
        if let Some(release) = self.release.lock().await.take() {
            let _ = release.send(());
        }
    }
}

/// The whole point: a run in flight finishes rather than being killed by the
/// deploy that is replacing its replica.
#[tokio::test]
async fn a_drain_waits_for_a_run_in_flight() {
    let replica = Replica::memory().await;
    let run = replica.start_blocking_run().await;

    let server = Arc::clone(&replica.server);
    let mut drain = tokio::spawn(async move { server.shutdown(Duration::from_secs(30)).await });

    // The agent cannot have finished — it is blocked on a channel nothing has
    // sent to — so a drain that has returned has returned early. This is the
    // assertion that fails against a replica which does not track its work.
    let early = tokio::time::timeout(SHORT_DEADLINE, &mut drain).await;
    assert!(early.is_err(), "the drain returned while a run was still executing");

    replica.release_agent().await;
    let abandoned = drain.await.unwrap();

    assert!(abandoned.is_empty(), "nothing should have been left behind: {abandoned:?}");
    let finished = replica.client.get_run(run.run_id).await.unwrap();
    assert_eq!(finished.status, RunStatus::Completed);
    assert_eq!(finished.output_text(), "released");
}

/// A run that outlasts the deadline is handed back rather than failed here.
///
/// Releasing the lease is what turns "this replica vanished, wait out the TTL
/// to find out" into "this run needs a new owner now".
#[tokio::test]
async fn a_run_outlasting_the_deadline_has_its_lease_released() {
    let replica = Replica::memory().await;
    let run = replica.start_blocking_run().await;

    let abandoned = replica.server.shutdown(SHORT_DEADLINE).await;

    assert_eq!(abandoned, vec![run.run_id], "the straggler should be reported");
    assert_eq!(
        replica.store.lease_owner(run.run_id).await.unwrap(),
        None,
        "a drained run must not still look owned, or a client waits out the whole lease ttl \
         before anyone will touch it"
    );

    // Not failed by the departing replica: it is leaving, and is in no position
    // to judge a run that might have been a second from finishing. The next
    // replica to read it decides.
    let snapshot = replica.store.get_run(run.run_id).await.unwrap().unwrap();
    assert!(!snapshot.status.is_terminal(), "left as {}", snapshot.status);
}

/// New work is refused at the door, with the status the client knows to wait on.
#[tokio::test]
async fn a_draining_replica_refuses_new_runs() {
    let replica = Replica::memory().await;
    replica.server.stop_accepting();

    let response = reqwest::Client::new()
        .post(format!("{}/runs", replica.client.base_url()))
        .json(&serde_json::json!({
            "agent_name": "blocking",
            "input": [{ "role": "user", "parts": [{ "content": "go" }] }],
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 503);
    assert_eq!(
        response.headers().get("retry-after").and_then(|value| value.to_str().ok()),
        Some("5"),
        "a 503 without a Retry-After tells a client nothing about when to come back"
    );

    // Not an ACP error object, on purpose: `AcpError::Protocol` is not
    // transient, so a client would never retry the rejection the 503 exists to
    // invite.
    let error = replica.client.get_agent("blocking").await;
    assert!(error.is_ok(), "reads are still served while draining");
}

/// Refusing new runs must not sever a conversation already under way.
///
/// An `awaiting` run belongs to a client that is about to answer. Rejecting the
/// answer would strand a run this replica is still holding, which is the
/// opposite of draining it.
#[tokio::test]
async fn a_parked_run_can_still_be_resumed_while_draining() {
    let replica = Replica::memory().await;
    let parked = replica.client.run_sync("asker", [Message::user("hi")]).await.unwrap();
    assert_eq!(parked.status, RunStatus::Awaiting);

    replica.server.stop_accepting();

    let resumed = replica
        .client
        .resume_run(RunResumeRequest::new(
            parked.run_id,
            AwaitResume::new(serde_json::json!({ "answer": "Ada" })),
            RunMode::Sync,
        ))
        .await
        .unwrap();

    assert_eq!(resumed.status, RunStatus::Completed);
    assert_eq!(resumed.output_text(), "hello Ada");
}

/// A replica on its way out does not adopt an abandoned run.
///
/// Reaping means failing the run or starting a replacement *here*, and taking
/// on work mid-drain is the one thing a drain exists to prevent. The run is
/// left exactly as found, for a replica that is staying.
#[tokio::test]
async fn a_draining_replica_leaves_an_abandoned_run_alone() {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::default());
    let replica = Replica::new(Arc::clone(&store)).await;

    // A run with no owner and no live lease: exactly what a replica that died
    // leaves behind.
    let mut orphan = Run::new(AgentName::new("blocking").unwrap(), None);
    orphan.status = RunStatus::InProgress;
    store.put_run(&orphan).await.unwrap();

    replica.server.stop_accepting();
    let seen = replica.client.get_run(orphan.run_id).await.unwrap();
    assert_eq!(seen.status, RunStatus::InProgress, "a draining replica reaped a run anyway");

    // The same read on a replica that is staying does fail it, which is what
    // makes the assertion above about draining rather than about reaping being
    // broken.
    let staying = Replica::new(Arc::clone(&store)).await;
    let reaped = staying.client.get_run(orphan.run_id).await.unwrap();
    assert_eq!(reaped.status, RunStatus::Failed);
}

/// `in_flight` is what a readiness probe would report, so it has to be right.
#[tokio::test]
async fn in_flight_counts_what_is_executing() {
    let replica = Replica::memory().await;
    assert_eq!(replica.server.in_flight(), 0);
    assert!(replica.server.is_accepting());

    let _run: Run = replica.start_blocking_run().await;
    assert_eq!(replica.server.in_flight(), 1);

    replica.server.stop_accepting();
    assert!(!replica.server.is_accepting());

    replica.release_agent().await;
    let abandoned = replica.server.drain(Duration::from_secs(30)).await;
    assert!(abandoned.is_empty());
    assert_eq!(replica.server.in_flight(), 0);
}

/// Draining an idle replica returns immediately rather than waiting out the
/// deadline — the common case for a replica that happened to be doing nothing.
#[tokio::test]
async fn draining_an_idle_replica_is_immediate() {
    let replica = Replica::memory().await;
    let started = std::time::Instant::now();

    let abandoned = replica.server.shutdown(Duration::from_secs(30)).await;

    assert!(abandoned.is_empty());
    assert!(started.elapsed() < Duration::from_secs(5), "waited for a change that never comes");
}

/// Repeated calls are safe: an orchestrator that signals twice, or a handler
/// wired to more than one signal, must not deadlock or double-release.
#[tokio::test]
async fn shutting_down_twice_is_harmless() {
    let replica = Replica::memory().await;
    let run: Run = replica.start_blocking_run().await;

    let first = replica.server.shutdown(SHORT_DEADLINE).await;
    let second = replica.server.shutdown(SHORT_DEADLINE).await;

    assert_eq!(first, vec![run.run_id]);
    assert_eq!(second, vec![run.run_id], "the run is still going, so it is still reported");
    let ids: Vec<RunId> = second;
    assert_eq!(ids.len(), 1);
}
