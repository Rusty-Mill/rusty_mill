//! Several ACP replicas behind one shared store.
//!
//! This is the deployment shape ACP's [high-availability guide][ha] describes:
//! identical replicas behind a load balancer, sharing centralised storage, with
//! no session affinity. Here both replicas run in one process on two ports so
//! you can watch a run cross between them; in production they would be separate
//! machines behind a real load balancer.
//!
//! ```sh
//! # Uses an in-process shared store by default:
//! cargo run --example ha_server
//!
//! # Or a real Redis, which is what a production deployment would use:
//! cargo run --example ha_server --features redis-store -- redis://127.0.0.1/
//! ```
//!
//! Then drive one replica and control the run through the other:
//!
//! ```sh
//! # Start a run on replica A. It parks, awaiting an answer.
//! RUN=$(curl -s -X POST localhost:8001/runs -H 'content-type: application/json' -d '{
//!   "agent_name": "greeter",
//!   "input": [{"role": "user", "parts": [{"content": "hi"}]}]
//! }' | jq -r .run_id)
//!
//! # Replica B has never seen this run, but can read it...
//! curl -s localhost:8002/runs/$RUN | jq .status        # => "awaiting"
//!
//! # ...and resume it. The payload is routed to the agent inside replica A.
//! curl -s -X POST localhost:8002/runs/$RUN -H 'content-type: application/json' -d '{
//!   "run_id": "'"$RUN"'", "mode": "sync", "await_resume": {"answer": "Ada"}
//! }' | jq .output
//! ```
//!
//! [ha]: https://agentcommunicationprotocol.dev/how-to/high-availability

use std::sync::Arc;

use rusty_acp::{
    server::{agent_fn, store::InMemoryStore, store::Store, AcpServer, RunContext},
    types::{AgentManifest, AgentName, AwaitRequest, Error},
};

/// Build the agents each replica hosts. Every replica is identical — that is
/// the point.
fn build_replica(store: Arc<dyn Store>, base_url: &str) -> Result<axum::Router, Error> {
    let greeter = agent_fn(
        AgentManifest::new(AgentName::new("greeter")?, "Asks for your name, then greets you by it"),
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
    );

    let echo = agent_fn(
        AgentManifest::new(AgentName::new("echo")?, "Echoes the input back"),
        |ctx: RunContext| async move {
            ctx.reply_text(ctx.input_text()).await?;
            Ok(())
        },
    );

    Ok(AcpServer::builder()
        .agent(greeter)
        .agent(echo)
        .store(store)
        // Every replica advertises the same public address, so a session's
        // history links stay valid whichever replica wrote them.
        .base_url(base_url)
        .build()?
        .into_router())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("info,rusty_acp=debug").init();

    // One store, shared by every replica. Swapping this line is the whole
    // difference between a single host and an HA deployment.
    let store: Arc<dyn Store> = match std::env::args().nth(1) {
        #[cfg(feature = "redis-store")]
        Some(url) => {
            let store = rusty_acp::server::store::RedisStore::connect(&url).await?;
            tracing::info!("using Redis at {url}");
            Arc::new(store)
        }
        #[cfg(not(feature = "redis-store"))]
        Some(_) => {
            return Err("rebuild with `--features redis-store` to use a Redis URL".into());
        }
        None => {
            tracing::info!(
                "using a shared in-process store; pass a Redis URL (with --features \
                 redis-store) for a deployment that survives a restart"
            );
            Arc::new(InMemoryStore::default())
        }
    };

    // In production these would be separate processes on separate machines,
    // reached through one load balancer address.
    let mut replicas = Vec::new();
    for port in [8001u16, 8002] {
        let router = build_replica(Arc::clone(&store), "http://localhost:8001")?;
        let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
        tracing::info!("replica listening on http://localhost:{port}");
        replicas.push(tokio::spawn(async move { axum::serve(listener, router).await }));
    }

    for replica in replicas {
        replica.await??;
    }
    Ok(())
}
