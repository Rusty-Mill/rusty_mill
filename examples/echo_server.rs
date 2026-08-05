//! An ACP server hosting two agents.
//!
//! ```sh
//! cargo run --example echo_server
//!
//! curl localhost:8000/agents | jq
//! curl -X POST localhost:8000/runs -H 'content-type: application/json' -d '{
//!   "agent_name": "echo",
//!   "input": [{"role": "user", "parts": [{"content_type": "text/plain", "content": "hello"}]}]
//! }' | jq
//!
//! # Stream a run
//! curl -N -X POST localhost:8000/runs -H 'content-type: application/json' -d '{
//!   "agent_name": "slow-writer",
//!   "mode": "stream",
//!   "input": [{"role": "user", "parts": [{"content_type": "text/plain", "content": "hello there"}]}]
//! }'
//! ```

use std::time::Duration;

use rusty_acp::{
    server::{agent_fn, AcpServer, Agent, RunContext},
    types::{AgentManifest, AgentName, Error, Link, LinkType, Metadata, Tag, TrajectoryMetadata},
};

/// Echoes the input straight back as a single message.
struct Echo;

#[async_trait::async_trait]
impl Agent for Echo {
    fn manifest(&self) -> AgentManifest {
        AgentManifest::new(
            AgentName::new("echo").expect("valid agent name"),
            "Echoes the input back verbatim",
        )
        .with_input_content_types(["text/plain"])
        .with_output_content_types(["text/plain"])
        .with_metadata(
            Metadata::new()
                .with_license("Apache-2.0")
                .with_programming_language("Rust")
                .with_framework("rusty-acp")
                .with_tags([Tag::CHAT])
                .with_links([Link::new(
                    LinkType::Homepage,
                    "https://agentcommunicationprotocol.dev",
                )]),
        )
    }

    async fn run(&self, ctx: RunContext) -> Result<(), Error> {
        ctx.reply_text(ctx.input_text());
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,rusty_acp=debug".into()),
        )
        .init();

    // A streaming agent, defined inline: it emits one part per word, with a
    // trajectory step in front so clients can show what it is doing.
    let slow_writer = agent_fn(
        AgentManifest::new(
            AgentName::new("slow-writer")?,
            "Streams the input back one word at a time",
        ),
        |ctx: RunContext| async move {
            ctx.reply_part(rusty_acp::types::MessagePart::trajectory(TrajectoryMetadata {
                message: Some("Splitting the input into words".to_string()),
                ..Default::default()
            }));

            let mut writer = ctx.begin_message();
            for word in ctx.input_text().split_whitespace() {
                if ctx.is_cancelled() {
                    return Err(Error::server_error("cancelled"));
                }
                writer.push_text(format!("{word} "));
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            writer.finish();
            Ok(())
        },
    );

    let router = AcpServer::builder()
        .agent(Echo)
        .agent(slow_writer)
        .base_url("http://localhost:8000")
        .build()?
        .into_router();

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8000").await?;
    tracing::info!("ACP server listening on http://localhost:8000");
    axum::serve(listener, router).await?;
    Ok(())
}
