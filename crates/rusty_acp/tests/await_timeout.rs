//! Bounding how long a run may wait for an answer.
//!
//! A parked run is not free. It holds a task, a run entry the default store
//! never evicts — active runs are never evicted — and a lease its replica keeps
//! renewing every few seconds. Nothing reclaims any of it: the run is
//! non-terminal with a live lease, which is exactly what a run that is *working*
//! looks like.
//!
//! Which makes it reachable by anyone who can submit a run. Ask a question,
//! never answer, repeat.

#![cfg(all(feature = "server", feature = "client"))]

use std::sync::Arc;
use std::time::Duration;

use rusty_acp::client::AcpClient;
use rusty_acp::server::store::{InMemoryStore, Store};
use rusty_acp::server::{agent_fn, AcpServer, RunContext, DEFAULT_AWAIT_TIMEOUT};
use rusty_acp::types::{
    AgentManifest, AgentName, AwaitResume, Message, RunMode, RunResumeRequest, RunStatus,
};

/// Short enough to keep the suite quick, long enough that a resume racing it
/// has time to land — the tests that resume do so immediately.
const SHORT: Duration = Duration::from_millis(150);

struct Replica {
    client: AcpClient,
    store: Arc<dyn Store>,
}

impl Replica {
    /// A replica whose `asker` agent parks until answered or timed out.
    async fn new(await_timeout: Option<Duration>) -> Self {
        let asker = agent_fn(
            AgentManifest::new(AgentName::new("asker").unwrap(), "Pauses to ask a question"),
            |ctx: RunContext| async move {
                let resume = ctx.await_json(serde_json::json!({ "question": "name?" })).await?;
                let answer = resume.as_value()["answer"].as_str().unwrap_or("nobody").to_string();
                ctx.reply_text(format!("hello {answer}")).await?;
                Ok(())
            },
        );

        let store: Arc<dyn Store> = Arc::new(InMemoryStore::default());
        let mut builder = AcpServer::builder()
            .agent(asker)
            .store(Arc::clone(&store))
            .base_url("http://acp.example");
        builder = match await_timeout {
            Some(timeout) => builder.await_timeout(timeout),
            None => builder.without_await_timeout(),
        };
        let router = builder.build().unwrap().into_router();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

        Self { client: AcpClient::new(format!("http://{addr}")).unwrap(), store }
    }

    /// Poll until the run is terminal.
    ///
    /// Not `wait_for_run`: that returns as soon as a run is *awaiting*, which
    /// is its documented contract and exactly the state under test here.
    async fn wait_until_terminal(&self, run_id: rusty_acp::types::RunId) -> rusty_acp::types::Run {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let run = self.client.get_run(run_id).await.unwrap();
            if run.status.is_terminal() {
                return run;
            }
            assert!(std::time::Instant::now() < deadline, "still {} after 10s", run.status);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn park(&self) -> rusty_acp::types::Run {
        let run = self.client.run_sync("asker", [Message::user("hi")]).await.unwrap();
        assert_eq!(run.status, RunStatus::Awaiting, "the agent should be parked");
        run
    }
}

/// The point: a conversation nobody answers stops costing anything.
#[tokio::test]
async fn a_run_nobody_answers_is_failed() {
    let replica = Replica::new(Some(SHORT)).await;
    let parked = replica.park().await;

    let settled = replica.wait_until_terminal(parked.run_id).await;

    assert_eq!(settled.status, RunStatus::Failed);
}

/// The message has to name what happened. A bare `server_error` sends whoever
/// reads it hunting for a bug in their agent, when nobody answered.
#[tokio::test]
async fn the_failure_says_what_happened() {
    let replica = Replica::new(Some(SHORT)).await;
    let parked = replica.park().await;

    let settled = replica.wait_until_terminal(parked.run_id).await;

    let message = settled.error.as_ref().map(|error| error.message.clone()).unwrap_or_default();
    assert!(message.contains("awaiting client input"), "the failure does not say why: {message:?}");
}

/// Failing is only half the point — the resources have to come back. A terminal
/// run releases its lease, stops renewing, and becomes evictable.
#[tokio::test]
async fn a_timed_out_run_stops_costing_anything() {
    let replica = Replica::new(Some(SHORT)).await;
    let parked = replica.park().await;

    replica.wait_until_terminal(parked.run_id).await;

    // Give the executor its moment to run the release that follows the
    // terminal transition.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        replica.store.lease_owner(parked.run_id).await.unwrap(),
        None,
        "a failed run is still holding its lease, so it is still being renewed"
    );
}

/// A client who answers in time is unaffected — the deadline must not become a
/// race the conversation can lose.
#[tokio::test]
async fn answering_in_time_still_works() {
    let replica = Replica::new(Some(Duration::from_secs(30))).await;
    let parked = replica.park().await;

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

/// Switched off, a conversation stays open — which is the right answer for
/// genuinely open-ended ones, and the wrong one on a public address.
#[tokio::test]
async fn an_unbounded_replica_leaves_it_parked() {
    let replica = Replica::new(None).await;
    let parked = replica.park().await;

    tokio::time::sleep(Duration::from_millis(400)).await;

    let still = replica.client.get_run(parked.run_id).await.unwrap();
    assert_eq!(still.status, RunStatus::Awaiting, "an unbounded wait was bounded anyway");
}

/// Bounded by default, because the population that most needs the bound is the
/// one that has not thought about it.
#[tokio::test]
async fn the_default_is_bounded_and_generous() {
    let asker = agent_fn(
        AgentManifest::new(AgentName::new("asker").unwrap(), "Asks"),
        |_ctx: RunContext| async move { Ok(()) },
    );
    let server = AcpServer::builder().agent(asker).build().unwrap();

    assert_eq!(server.await_timeout(), Some(DEFAULT_AWAIT_TIMEOUT));
    assert_eq!(DEFAULT_AWAIT_TIMEOUT, Duration::from_secs(3600));
}
