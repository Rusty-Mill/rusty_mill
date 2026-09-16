//! Taking a replica out of service without killing its work.
//!
//! Two replicas share one store. One is deployed over while the other keeps
//! serving, which is the only setting where a drain means anything — a single
//! host has nowhere to hand its work to.
//!
//! **The order is the whole subject.** Three steps, and getting them wrong
//! costs runs:
//!
//! 1. `stop_accepting` — new submissions are refused with 503 and `/ready`
//!    starts answering 503, so a load balancer stops routing here. First,
//!    because anything that arrives later is work this replica will not finish.
//! 2. axum's own graceful shutdown — stop accepting *connections*, and let the
//!    in-flight requests finish.
//! 3. `drain` — wait for the runs. Last, because a run's task is not tied to
//!    the connection that started it: a `POST /runs` in `async` mode returns
//!    immediately and the agent keeps going. Draining before step 2 would let
//!    new connections arrive during the wait.
//!
//! ```sh
//! cargo run --example graceful_shutdown
//!
//! # While it runs, from another shell:
//! curl -s localhost:8001/ready | jq      # the draining replica
//! curl -s localhost:8002/ready | jq      # the one still serving
//!
//! curl -s -X POST localhost:8001/runs -H 'content-type: application/json' -d '{
//!   "agent_name": "slow", "mode": "async",
//!   "input": [{"role": "user", "parts": [{"content": "go"}]}]
//! }' -i | head -1                        # => 503 once it is draining
//! ```
//!
//! Press ctrl-c and replica B drains too, which is the part to copy into a real
//! deployment: the same sequence, wired to a signal.

use std::sync::Arc;
use std::time::Duration;

use rusty_acp::server::store::{InMemoryStore, Store};
use rusty_acp::server::{agent_fn, AcpServer, Drained, RunContext};
use rusty_acp::types::{AgentManifest, AgentName, AwaitRequest, Error, Message};

/// How long a drain waits before handing what is left back to the fleet.
///
/// Short here so the example finishes; [`DEFAULT_DRAIN_DEADLINE`] is a minute,
/// which is the more sensible starting point for a real deployment.
///
/// [`DEFAULT_DRAIN_DEADLINE`]: rusty_acp::server::DEFAULT_DRAIN_DEADLINE
const DRAIN_DEADLINE: Duration = Duration::from_secs(5);

/// A running replica: its server handle, and where it listens.
struct Replica {
    name: &'static str,
    server: Arc<AcpServer>,
    base_url: String,
    /// Tells axum to stop accepting connections.
    stop_serving: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Replica {
    async fn start(name: &'static str, port: u16, store: Arc<dyn Store>) -> Result<Self, Error> {
        // Every replica is identical and advertises the same public address, so
        // a session's history links stay valid whichever one wrote them.
        let (server, router) = AcpServer::builder()
            .agent(slow_agent()?)
            .agent(asking_agent()?)
            .store(store)
            .replica_id(name)
            .base_url("http://localhost:8000")
            .build()?
            .into_shared_router();

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.unwrap();
        // From the listener, not from `port` — the tests bind port 0 and need
        // the one the OS actually handed out.
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let (stop_serving, stopped) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });

        Ok(Self { name, server, base_url, stop_serving: Some(stop_serving) })
    }

    /// The sequence. This is the part worth copying.
    async fn shut_down(&mut self) -> Drained {
        // 1. Refuse new work, and start reporting unready. A load balancer
        //    polling `/ready` sees 503 on its next probe and stops routing
        //    here — which is what makes the 503s below rare rather than the
        //    normal way a client finds out.
        self.server.stop_accepting();
        // Read back rather than asserted: this is the only moment the flip is
        // observable, since step 2 takes the listener away.
        println!("[{}] not accepting; /ready says {}", self.name, ready(&self.base_url).await);

        // 2. Stop accepting connections, and let in-flight requests finish.
        //    After `stop_accepting` so that anything still arriving is refused
        //    rather than started.
        if let Some(stop) = self.stop_serving.take() {
            let _ = stop.send(());
        }

        // 3. Wait for the runs. Last, because a run outlives the request that
        //    created it — `async` and `stream` both return while the agent is
        //    still going.
        let drained = self.server.drain(DRAIN_DEADLINE).await;
        println!(
            "[{}] drained: {} unfinished, {} parked mid-conversation",
            self.name,
            drained.unfinished.len(),
            drained.parked.len(),
        );
        drained
    }
}

/// Takes long enough that a drain has something to wait for.
fn slow_agent() -> Result<impl rusty_acp::server::Agent, Error> {
    Ok(agent_fn(
        AgentManifest::new(AgentName::new("slow")?, "Takes a couple of seconds"),
        |ctx: RunContext| async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            ctx.reply_text("finished").await?;
            Ok(())
        },
    ))
}

/// Parks awaiting an answer, so the drain has a conversation to hand back.
fn asking_agent() -> Result<impl rusty_acp::server::Agent, Error> {
    Ok(agent_fn(
        AgentManifest::new(AgentName::new("asker")?, "Pauses to ask a question"),
        |ctx: RunContext| async move {
            let resume = ctx
                .await_request(AwaitRequest::new(serde_json::json!({
                    "question": "What is your name?"
                })))
                .await?;
            let name = resume.as_value()["answer"].as_str().unwrap_or("stranger").to_string();
            ctx.reply_text(format!("Hello, {name}!")).await?;
            Ok(())
        },
    ))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("info,rusty_acp=debug").init();

    // One store, shared. Without it a drain would have nobody to hand back to.
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::default());
    let mut a = Replica::start("replica-a", 8001, Arc::clone(&store)).await?;
    let b = Replica::start("replica-b", 8002, Arc::clone(&store)).await?;
    println!("replica-a on {} — replica-b on {}\n", a.base_url, b.base_url);

    let client_a = rusty_acp::client::AcpClient::new(&a.base_url)?;
    let client_b = rusty_acp::client::AcpClient::new(&b.base_url)?;

    // Give replica A something to lose: one run under way, one conversation
    // parked on a client who is not going to answer.
    let running = client_a.run_async("slow", [Message::user("go")]).await?;
    let parked = client_a.run_sync("asker", [Message::user("hi")]).await?;
    println!(
        "[replica-a] running {} and holding a conversation on {}",
        running.run_id, parked.run_id
    );
    println!("[replica-a] ready: {}\n", ready(&a.base_url).await);

    let drained = a.shut_down().await;

    // A's port is gone now, which is the honest end state — the flip to 503 is
    // visible during the drain, printed by `shut_down` above, because after
    // step 2 there is no longer a listener to answer.
    println!("\n[replica-a] ready: {}", ready(&a.base_url).await);
    println!("[replica-b] ready: {}", ready(&b.base_url).await);

    // The run that was under way got its two seconds and finished, and B can
    // say so because the store is shared. The store is the authority on how a
    // run ended; `drained` is only this replica's account of what it was still
    // holding when the deadline arrived.
    let finished = client_b.get_run(running.run_id).await?;
    println!("\n[replica-b] the run A was executing: {}", finished.status);

    // The parked conversation could not survive A: an agent that paused to ask
    // a question is suspended part-way through its own function, and that
    // position lived in A's process. It is handed back rather than left owned,
    // so B decides its fate immediately instead of waiting out A's lease.
    if let Some(parked_id) = drained.parked.first() {
        let handed_back = client_b.get_run(*parked_id).await?;
        println!("[replica-b] the conversation A was holding: {}", handed_back.status);
    }

    println!("\nreplica-b is still serving. ctrl-c to drain it too.");
    tokio::signal::ctrl_c().await?;

    // The same three steps, wired to a signal — which is how a real deployment
    // uses them. `tokio::signal` needs tokio's `signal` feature; on Unix,
    // `unix::signal(SignalKind::terminate())` catches SIGTERM, which is what an
    // orchestrator actually sends.
    let mut b = b;
    b.shut_down().await;
    Ok(())
}

/// Whatever `/ready` says, for printing.
async fn ready(base_url: &str) -> String {
    match reqwest::get(format!("{base_url}/ready")).await {
        Ok(response) => {
            let status = response.status();
            let body: serde_json::Value = response.json().await.unwrap_or_default();
            format!("{status} {}", body.get("reason").and_then(|r| r.as_str()).unwrap_or("ok"))
        }
        Err(error) => format!("unreachable: {error}"),
    }
}

/// Tests for the sequence, which is the part of this example worth getting
/// right and the part a copied bug would spread.
#[cfg(test)]
mod tests {
    use super::*;

    async fn replicas() -> (Replica, Replica) {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::default());
        // Port 0 so the tests do not fight each other or the running example.
        let a = Replica::start("replica-a", 0, Arc::clone(&store)).await.unwrap();
        let b = Replica::start("replica-b", 0, store).await.unwrap();
        (a, b)
    }

    async fn status(url: &str) -> u16 {
        reqwest::get(url).await.unwrap().status().as_u16()
    }

    fn submission() -> serde_json::Value {
        serde_json::json!({
            "agent_name": "slow",
            "mode": "async",
            "input": [{ "role": "user", "parts": [{ "content": "go" }] }],
        })
    }

    /// Step 1 has to take effect before the drain, not after it — otherwise the
    /// balancer keeps routing for the whole drain and every one of those
    /// requests is refused.
    #[tokio::test]
    async fn refusing_work_and_reporting_unready_happen_together() {
        let (a, _b) = replicas().await;
        assert_eq!(status(&format!("{}/ready", a.base_url)).await, 200);

        a.server.stop_accepting();

        assert_eq!(status(&format!("{}/ready", a.base_url)).await, 503);
        let refused = reqwest::Client::new()
            .post(format!("{}/runs", a.base_url))
            .json(&submission())
            .send()
            .await
            .unwrap();
        assert_eq!(refused.status(), 503);
    }

    /// The test above proves the *server* refuses once told to; this one proves
    /// `shut_down` tells it first. They are separate claims, and it is the
    /// second one that a copied sequence gets wrong: run the drain before
    /// `stop_accepting` and the replica keeps admitting work for the whole
    /// deadline, each admission landing on a replica that is about to leave.
    ///
    /// Probing throughout the drain rather than at one instant, and failing on
    /// zero probes, so a loaded runner reports a vacuous pass instead of
    /// scoring one.
    #[tokio::test]
    async fn nothing_is_admitted_once_the_shutdown_starts() {
        let (mut a, _b) = replicas().await;
        let client = rusty_acp::client::AcpClient::new(&a.base_url).unwrap();
        // Something for the drain to wait for, so the window is a real one.
        client.run_async("slow", [Message::user("go")]).await.unwrap();

        let (finished, mut is_finished) = tokio::sync::watch::channel(false);
        let base_url = a.base_url.clone();
        let probe = tokio::spawn(async move {
            let http = reqwest::Client::new();
            let (mut attempts, mut admitted) = (0usize, 0usize);
            while !*is_finished.borrow_and_update() {
                let outcome =
                    http.post(format!("{base_url}/runs")).json(&submission()).send().await;
                // A 503, or a connection refused once the listener has closed:
                // both are the replica declining, which is all this asserts.
                if outcome.is_ok_and(|response| response.status().is_success()) {
                    admitted += 1;
                }
                attempts += 1;
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            (attempts, admitted)
        });

        a.shut_down().await;
        finished.send(true).unwrap();

        let (attempts, admitted) = probe.await.unwrap();
        assert!(attempts > 0, "the drain returned before a single probe ran");
        assert_eq!(
            admitted, 0,
            "{admitted} of {attempts} submissions were taken while shutting down"
        );
    }

    /// Draining one replica must not disturb the other. If it did, a rolling
    /// deploy would be an outage.
    #[tokio::test]
    async fn the_other_replica_keeps_serving() {
        let (mut a, b) = replicas().await;

        a.shut_down().await;

        assert_eq!(status(&format!("{}/ready", b.base_url)).await, 200);
        let client = rusty_acp::client::AcpClient::new(&b.base_url).unwrap();
        let run = client.run_sync("slow", [Message::user("go")]).await.unwrap();
        assert_eq!(run.output_text(), "finished");
    }

    /// A run under way finishes rather than being killed by the deploy.
    #[tokio::test]
    async fn a_run_in_flight_survives_the_drain() {
        let (mut a, b) = replicas().await;
        let client_a = rusty_acp::client::AcpClient::new(&a.base_url).unwrap();
        let started = client_a.run_async("slow", [Message::user("go")]).await.unwrap();

        let drained = a.shut_down().await;

        assert!(drained.unfinished.is_empty(), "the deadline was too short for the work");
        let client_b = rusty_acp::client::AcpClient::new(&b.base_url).unwrap();
        let finished = client_b.get_run(started.run_id).await.unwrap();
        assert_eq!(finished.output_text(), "finished");
    }

    /// A parked conversation is reported apart from unfinished work, and does
    /// not make the drain wait for a client who is not answering.
    #[tokio::test]
    async fn a_parked_conversation_is_handed_back_without_waiting() {
        let (mut a, _b) = replicas().await;
        let client = rusty_acp::client::AcpClient::new(&a.base_url).unwrap();
        client.run_sync("asker", [Message::user("hi")]).await.unwrap();

        let started = std::time::Instant::now();
        let drained = a.shut_down().await;

        assert_eq!(drained.parked.len(), 1);
        assert!(drained.unfinished.is_empty(), "a parked run is not unfinished work");
        assert!(
            started.elapsed() < DRAIN_DEADLINE,
            "waited {:?} for a conversation that cannot finish",
            started.elapsed()
        );
    }
}
