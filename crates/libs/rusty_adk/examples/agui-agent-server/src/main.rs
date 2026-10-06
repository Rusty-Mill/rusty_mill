//! Serves a Rust ADK agent over AG-UI.
//!
//! AG-UI is how the workspace's frontends reach an agent: the React, Vue and
//! Angular bindings in `rusty_agui/packages`, the Slack, Teams and SMS
//! channels in `rusty_channel`, `rusty_routine`'s schedules and
//! `rusty_agent_gateway`'s `agui` route all post a `RunAgentInput` and read
//! an event stream. This is the AG-UI counterpart to `a2a-agent-server`.
//!
//! The root is the same ADK 2.0 graph as there: an approval node that
//! suspends for a human, then an agent that answers. The suspension reaches
//! the client as a call to the frontend tool `request_input`; a client that
//! registers an action by that name renders the prompt, and the person's
//! answer, sent back as the tool message, resumes the graph.
//!
//! # Running
//!
//! ```text
//! cargo run -p agui-agent-server              # listens on 127.0.0.1:8080
//! ```
//!
//! Open a thread. The run suspends at the approval node, so the stream ends
//! with a `TOOL_CALL_START` for `request_input` carrying the question:
//!
//! ```text
//! curl -sN http://127.0.0.1:8080/api/agent -H 'content-type: application/json' -d '{
//!   "threadId": "t1", "runId": "r1",
//!   "messages": [{"id": "u1", "role": "user", "content": "What is the weather in Paris?"}]
//! }'
//! ```
//!
//! Answer it with a tool message whose `toolCallId` is that call's id; the
//! graph resumes at the node that asked, and the agent answers:
//!
//! ```text
//! curl -sN http://127.0.0.1:8080/api/agent -H 'content-type: application/json' -d '{
//!   "threadId": "t1", "runId": "r2",
//!   "messages": [
//!     {"id": "u1", "role": "user", "content": "What is the weather in Paris?"},
//!     {"id": "a1", "role": "assistant", "toolCalls": [{"id": "<CALL_ID>", "type": "function",
//!        "function": {"name": "request_input", "arguments": "{}"}}]},
//!     {"id": "t1", "role": "tool", "toolCallId": "<CALL_ID>", "content": "approved"}
//!   ]
//! }'
//! ```
//!
//! From a React, Vue or Angular app the same thing is `useAction({ name:
//! "request_input", render: ({ args, respond }) => ... })`: the prompt shows,
//! the person answers, `respond` sends it, and the follow-up run carries the
//! reply.

use std::sync::Arc;

use rusty_adk::agui::AdkAgent;
use rusty_adk::prelude::*;
use rusty_agui::serve::AgentHandler;
use serde_json::json;

/// Retrieves the current weather for a city.
#[adk_tool(crate = ::rusty_adk::tools)]
async fn get_weather(city: String) -> Result<serde_json::Value> {
    Ok(rusty_adk::tools::success(json!({
        "report": format!("It is sunny in {city}, 22°C."),
    })))
}

const BIND: &str = "127.0.0.1:8080";

fn main() -> std::io::Result<()> {
    // The runner is async; the server is blocking. The agent is driven on
    // this runtime from the handler's thread.
    let runtime = tokio::runtime::Runtime::new()?;

    // An ordinary ADK agent. Nothing about it knows AG-UI exists.
    let assistant = LlmAgent::builder("assistant")
        .model(Arc::new(
            MockModel::new()
                .push_call_json("get_weather", json!({"city": "Paris"}))
                .push_text("It is sunny in Paris, 22°C."),
        ))
        .description("Answers weather questions.")
        .instruction("Answer weather questions using the get_weather tool.")
        .tool(get_weather_tool())
        .build()
        .expect("agent builds")
        .shared();

    // The approval gate: on the first pass it suspends the graph, which the
    // bridge reports as a frontend tool call; the answer resumes it here.
    let approve = FunctionNode::new("approve", NodeConfig::default(), |ctx| {
        let ctx = ctx.clone();
        Box::pin(async move {
            let answer = ctx.resume_or_request_input("May I answer this request?", None)?;
            Ok(NodeOutcome::output(answer))
        })
    })
    .shared();

    let graph = Arc::new(
        Graph::new(
            vec![approve, AgentNode::new(assistant).shared()],
            chain(["approve", "assistant"]),
        )
        .expect("graph is well formed"),
    );

    let runner = Runner::new(
        "weather_app",
        graph,
        Services::new(Arc::new(InMemorySessionService::new())),
    );

    let agent = AdkAgent::new(Arc::new(runner), runtime.handle().clone());
    let server = rusty_serve::Server::bind(
        BIND.parse().expect("a socket address"),
        AgentHandler::new(agent),
    )?;
    println!("AG-UI agent listening on http://{BIND}/api/agent");
    server.run()
}
