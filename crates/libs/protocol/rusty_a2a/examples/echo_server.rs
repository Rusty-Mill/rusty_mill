//! A minimal A2A agent that echoes back whatever text it receives.
//!
//! Run it with:
//!
//! ```sh
//! cargo run --example echo_server --features server
//! ```
//!
//! Then, in another terminal:
//!
//! ```sh
//! curl http://127.0.0.1:8080/.well-known/agent-card.json
//! cargo run --example send_message --features client
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use rusty_a2a::error::Result;
use rusty_a2a::server::{AgentExecutor, AgentServer, EventSink, RequestContext};
use rusty_a2a::types::{AgentCard, AgentInterface, AgentSkill, Message, TaskState};

struct EchoAgent;

#[async_trait]
impl AgentExecutor for EchoAgent {
    async fn execute(&self, ctx: RequestContext, events: EventSink) -> Result<()> {
        events.status(TaskState::Working);

        let reply = format!("you said: {}", ctx.message.text());
        events.status_with_message(TaskState::Completed, Some(Message::agent_text(reply)));

        Ok(())
    }
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let addr = ([127, 0, 0, 1], 8080);

    let card = AgentCard::new(
        "Echo Agent",
        "Echoes back whatever text you send it, as a demonstration of the rusty_a2a crate.",
        env!("CARGO_PKG_VERSION"),
        AgentInterface::json_rpc("http://127.0.0.1:8080"),
    )
    // Both bindings are served on the same port by `AgentServer`; this
    // just makes the second one discoverable via the Agent Card too.
    .with_interface(AgentInterface::http_json("http://127.0.0.1:8080"))
    .with_streaming(true)
    .with_skill(
        AgentSkill::new("echo", "Echo", "Repeats back the text of your message.").with_tags(["demo", "echo"]),
    );

    let server = AgentServer::new(card, Arc::new(EchoAgent));
    println!("Echo agent listening on http://127.0.0.1:8080");
    println!("Agent card: http://127.0.0.1:8080/.well-known/agent-card.json");
    server.serve(addr).await
}
