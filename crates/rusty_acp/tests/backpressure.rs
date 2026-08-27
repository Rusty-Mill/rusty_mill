//! Refusing work a replica has no capacity for.
//!
//! The failure being prevented is a server that accepts everything and then
//! runs out of memory holding it. The interesting part is not the counting but
//! *what counts*: a run parked awaiting a client answer is this replica's to
//! finish, but it is a suspended future waiting on a human who may never come
//! back. Counting it against capacity would let idle conversations starve work
//! that is ready to run.
//!
//! As elsewhere in this suite, nothing races. Agents block on channels the test
//! holds, so "at capacity" is a state the test has established rather than one
//! it hopes to catch.

#![cfg(all(feature = "server", feature = "client"))]

use std::sync::Arc;
use std::time::Duration;

use rusty_acp::client::AcpClient;
use rusty_acp::server::{agent_fn, AcpServer, RunContext};
use rusty_acp::types::{
    AgentManifest, AgentName, AwaitResume, Message, RunMode, RunResumeRequest, RunStatus,
};
use tokio::sync::{mpsc, oneshot, Mutex};

/// A replica with a concurrency ceiling and agents the test can hold open.
struct Replica {
    server: Arc<AcpServer>,
    client: AcpClient,
    /// Signalled once per run that reaches the agent body.
    started: Mutex<mpsc::UnboundedReceiver<()>>,
    /// Releasing this lets every blocked run finish at once.
    release: Mutex<Option<oneshot::Sender<()>>>,
}

impl Replica {
    async fn with_limit(limit: usize) -> Self {
        let (started_tx, started_rx) = mpsc::unbounded_channel();
        let (release_tx, release_rx) = oneshot::channel();
        // A shared receiver every run awaits: one send releases them all, which
        // keeps the test from having to release them one at a time in an order
        // it cannot control.
        let released = Arc::new(tokio::sync::Notify::new());
        {
            let released = Arc::clone(&released);
            tokio::spawn(async move {
                let _ = release_rx.await;
                released.notify_waiters();
            });
        }

        let blocking = agent_fn(
            AgentManifest::new(AgentName::new("blocking").unwrap(), "Runs until released"),
            move |ctx: RunContext| {
                let started = started_tx.clone();
                let released = Arc::clone(&released);
                async move {
                    let waiting = released.notified();
                    let _ = started.send(());
                    waiting.await;
                    ctx.reply_text("released").await?;
                    Ok(())
                }
            },
        );

        let asker = agent_fn(
            AgentManifest::new(AgentName::new("asker").unwrap(), "Pauses to ask a question"),
            |ctx: RunContext| async move {
                let resume = ctx.await_json(serde_json::json!({ "question": "name?" })).await?;
                let answer = resume.as_value()["answer"].as_str().unwrap_or("nobody").to_string();
                ctx.reply_text(format!("hello {answer}")).await?;
                Ok(())
            },
        );

        let (server, router) = AcpServer::builder()
            .agent(blocking)
            .agent(asker)
            .max_concurrent_runs(limit)
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
            started: Mutex::new(started_rx),
            release: Mutex::new(Some(release_tx)),
        }
    }

    /// Fill `count` slots and return once every one of those runs is provably
    /// executing an agent body.
    async fn occupy(&self, count: usize) {
        for _ in 0..count {
            self.client.run_async("blocking", [Message::user("go")]).await.expect("admitted");
        }
        let mut started = self.started.lock().await;
        for _ in 0..count {
            started.recv().await.expect("each run reaches its agent");
        }
    }

    async fn release_all(&self) {
        if let Some(release) = self.release.lock().await.take() {
            let _ = release.send(());
        }
    }

    /// `POST /runs` without the client's own error handling, so the raw status
    /// and headers can be read.
    ///
    /// Explicitly `async`: the default mode is `sync`, which would hold the
    /// request open until the run settles — and the runs here are held open by
    /// the test on purpose, so a `sync` submission would wait out the server's
    /// whole `sync_timeout` rather than answering.
    async fn submit(&self, agent: &str) -> reqwest::Response {
        reqwest::Client::new()
            .post(format!("{}/runs", self.client.base_url()))
            .json(&serde_json::json!({
                "agent_name": agent,
                "mode": "async",
                "input": [{ "role": "user", "parts": [{ "content": "go" }] }],
            }))
            .send()
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn a_full_replica_refuses_with_429_and_a_retry_after() {
    let replica = Replica::with_limit(2).await;
    replica.occupy(2).await;
    assert_eq!(replica.server.executing(), 2);

    let refused = replica.submit("blocking").await;

    assert_eq!(refused.status(), 429);
    assert_eq!(
        refused.headers().get("retry-after").and_then(|value| value.to_str().ok()),
        Some("2"),
        "a 429 without a Retry-After tells a client nothing about when to come back"
    );

    // Not an ACP error object: `AcpError::Protocol` is not transient, so a
    // client would never retry the rejection the 429 exists to invite.
    let error = replica.client.run_sync("blocking", [Message::user("go")]).await.unwrap_err();
    assert!(matches!(error, rusty_acp::AcpError::Http { status: 429, .. }), "{error}");
}

/// A refused submission must leave nothing behind — no run to read, reap or
/// clean up. Admission happens before the first store write for this reason.
#[tokio::test]
async fn a_refused_run_is_never_created() {
    let replica = Replica::with_limit(1).await;
    replica.occupy(1).await;

    replica.submit("blocking").await;

    assert_eq!(replica.server.executing(), 1, "a refused run took a slot anyway");
    assert_eq!(replica.server.in_flight(), 1, "a refused run was registered anyway");
}

/// Capacity comes back as runs finish, which is what makes a 429 worth retrying.
#[tokio::test]
async fn capacity_returns_when_runs_finish() {
    let replica = Replica::with_limit(1).await;
    replica.occupy(1).await;
    assert_eq!(replica.submit("blocking").await.status(), 429);

    replica.release_all().await;
    replica.server.drain(Duration::from_secs(30)).await;
    assert_eq!(replica.server.executing(), 0);

    // A fresh server would pass this trivially; the drain above is what makes
    // it about capacity being *returned*.
    let admitted = Replica::with_limit(1).await;
    assert_eq!(admitted.submit("blocking").await.status(), 202);
}

/// The heart of it: a parked run gives its slot up.
///
/// An `awaiting` run is waiting on a human. Holding a slot for it would mean a
/// replica whose conversations are all mid-question could not start anything,
/// while doing no work at all.
#[tokio::test]
async fn a_run_awaiting_a_client_does_not_hold_capacity() {
    let replica = Replica::with_limit(1).await;

    let parked = replica.client.run_sync("asker", [Message::user("hi")]).await.unwrap();
    assert_eq!(parked.status, RunStatus::Awaiting);
    assert_eq!(replica.server.executing(), 0, "a parked run is still holding a slot");

    // The slot it gave up is available to somebody else.
    assert_eq!(replica.submit("blocking").await.status(), 202);
}

/// And takes it back on the way out, so a resumed run is not free forever.
#[tokio::test]
async fn a_resumed_run_takes_its_slot_back() {
    let replica = Replica::with_limit(4).await;

    let parked = replica.client.run_sync("asker", [Message::user("hi")]).await.unwrap();
    assert_eq!(replica.server.executing(), 0);

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
    // Back to zero because it finished, having briefly held the slot again.
    assert_eq!(replica.server.executing(), 0);
}

/// A resume is never refused for want of capacity, even over the ceiling.
///
/// The run was admitted once already. Refusing it here would strand a
/// conversation mid-sentence to defend a number, which is the wrong trade — so
/// the ceiling bounds what the replica *takes on*, not an instantaneous
/// invariant.
#[tokio::test]
async fn a_resume_is_not_refused_when_the_replica_is_full() {
    let replica = Replica::with_limit(1).await;

    let parked = replica.client.run_sync("asker", [Message::user("hi")]).await.unwrap();
    assert_eq!(parked.status, RunStatus::Awaiting);

    // Somebody else takes the only slot while the conversation is parked.
    replica.occupy(1).await;
    assert_eq!(replica.server.executing(), 1);
    assert_eq!(replica.submit("blocking").await.status(), 429, "the replica really is full");

    let resumed = replica
        .client
        .resume_run(RunResumeRequest::new(
            parked.run_id,
            AwaitResume::new(serde_json::json!({ "answer": "Ada" })),
            RunMode::Sync,
        ))
        .await
        .expect("a parked conversation is not sacrificed to the ceiling");

    assert_eq!(resumed.status, RunStatus::Completed);
}

/// Unset by default, so nothing changes for anyone who has not thought about it.
#[tokio::test]
async fn there_is_no_ceiling_unless_one_is_set() {
    let idle = agent_fn(
        AgentManifest::new(AgentName::new("idle").unwrap(), "Does nothing"),
        |_ctx: RunContext| async move { Ok(()) },
    );
    let server = AcpServer::builder().agent(idle).build().unwrap();
    assert_eq!(server.max_concurrent_runs(), None);
    assert_eq!(server.executing(), 0);
}

/// Capacity and draining are different refusals and say so, because they want
/// different responses from whoever is reading.
#[tokio::test]
async fn draining_and_at_capacity_are_distinguishable() {
    let replica = Replica::with_limit(1).await;
    replica.occupy(1).await;
    assert_eq!(replica.submit("blocking").await.status(), 429);

    replica.server.stop_accepting();
    assert_eq!(replica.submit("blocking").await.status(), 503);
}
