//! Exercise every client operation against a running ACP server.
//!
//! Start a server first (`cargo run --example echo_server`), then:
//!
//! ```sh
//! cargo run --example client_demo               # defaults to http://localhost:8000
//! cargo run --example client_demo -- http://localhost:9000
//! ```

use futures_util::StreamExt;
use rusty_acp::{
    client::{AcpClient, WaitOptions},
    types::{Event, Message, RunCreateRequest, SessionId},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let base_url = std::env::args().nth(1).unwrap_or_else(|| "http://localhost:8000".to_string());
    let client = AcpClient::new(base_url)?;

    client.ping().await?;
    println!("server is up\n");

    println!("== discovery ==");
    let agents = client.list_all_agents().await?;
    for manifest in &agents {
        println!("  {} — {}", manifest.name, manifest.description);
    }
    println!();

    println!("== sync run ==");
    let run = client.run_sync("echo", [Message::user("Hello, ACP!")]).await?;
    println!("  status: {}", run.status);
    println!("  output: {}\n", run.output_text());

    println!("== async run ==");
    let started = client.run_async("echo", [Message::user("Take your time")]).await?;
    println!("  accepted: {} ({})", started.run_id, started.status);
    let finished = client.wait_for_run(started.run_id, WaitOptions::default()).await?;
    println!("  finished: {} — {}\n", finished.status, finished.output_text());

    if agents.iter().any(|manifest| manifest.name.as_str() == "slow-writer") {
        println!("== streaming run ==");
        let mut stream = client.stream("slow-writer", [Message::user("one two three")]).await?;
        while let Some(event) = stream.next().await {
            match event? {
                Event::MessagePart { part } => {
                    println!("  part: {:?}", part.content.unwrap_or_default());
                }
                Event::RunCompleted { run } => println!("  done: {}", run.output_text()),
                other => println!("  {}", other.event_type()),
            }
        }
        println!();
    }

    println!("== session continuity ==");
    let session_id = SessionId::new();
    for text in ["first message", "second message"] {
        client
            .create_run(
                RunCreateRequest::new("echo".parse()?, [Message::user(text)])
                    .with_session_id(session_id),
            )
            .await?;
    }
    let session = client.get_session(session_id).await?;
    println!("  {} history entries", session.history.len());
    for message in client.fetch_session_history(&session).await? {
        println!("  {}: {}", message.role, message.text());
    }
    println!();

    println!("== event log ==");
    for event in client.list_run_events(run.run_id).await? {
        println!("  {}", event.event_type());
    }

    Ok(())
}
