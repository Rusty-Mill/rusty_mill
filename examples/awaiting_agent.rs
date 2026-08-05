//! An agent that pauses mid-run to ask the client a question.
//!
//! ```sh
//! cargo run --example awaiting_agent
//!
//! # 1. Start the run. It comes back `awaiting` with an `await_request`.
//! curl -s -X POST localhost:8000/runs -H 'content-type: application/json' -d '{
//!   "agent_name": "greeter",
//!   "input": [{"role": "user", "parts": [{"content_type": "text/plain", "content": "hi"}]}]
//! }' | jq
//!
//! # 2. Answer it, using the run_id from step 1.
//! curl -s -X POST localhost:8000/runs/$RUN_ID -H 'content-type: application/json' -d '{
//!   "run_id": "'"$RUN_ID"'",
//!   "mode": "sync",
//!   "await_resume": {"answer": "Ada"}
//! }' | jq
//! ```

use rusty_acp::{
    server::{agent_fn, AcpServer, RunContext},
    types::{AgentManifest, AgentName, Error},
};
use serde::{Deserialize, Serialize};

/// What the agent asks the client for.
#[derive(Debug, Serialize)]
struct Question {
    question: String,
}

/// What the client sends back.
#[derive(Debug, Deserialize)]
struct Answer {
    answer: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("info,rusty_acp=debug").init();

    let greeter = agent_fn(
        AgentManifest::new(AgentName::new("greeter")?, "Asks for your name, then greets you by it"),
        |ctx: RunContext| async move {
            let request = rusty_acp::types::AwaitRequest::from_value(&Question {
                question: "What is your name?".to_string(),
            })?;

            let resume = ctx.await_request(request).await?;
            let answer: Answer = resume.deserialize()?;

            if answer.answer.trim().is_empty() {
                return Err(Error::invalid_input("the answer must not be empty"));
            }

            ctx.reply_text(format!("Hello, {}!", answer.answer.trim())).await?;
            Ok(())
        },
    );

    let router = AcpServer::builder().agent(greeter).build()?.into_router();
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8000").await?;
    tracing::info!("ACP server listening on http://localhost:8000");
    axum::serve(listener, router).await?;
    Ok(())
}
