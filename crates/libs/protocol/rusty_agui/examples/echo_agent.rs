//! A small AG-UI agent on `rusty_serve`, for the conformance test and for
//! trying a client by hand. Binds `127.0.0.1:0` (or `AGUI_ECHO_ADDR`),
//! prints `LISTENING http://<addr>/api/agent` on stdout, and serves until
//! killed.
//!
//! Each run: a state snapshot, a state delta, a step, a tool call when the
//! client offered a tool, and an assistant message echoing the last user
//! message. A last user message of `fail` ends the run with `RUN_ERROR`.
//!
//! ```sh
//! cargo run -p rusty_agui --features serve --example echo_agent
//! ```

use rusty_agui::{Agent, AgentHandler, Emitter, Error, EventKind, Message, RunAgentInput};
use rusty_json::{json, Value};
use std::io::Write;

struct Echo;

impl Agent for Echo {
    fn run(
        &mut self,
        input: &RunAgentInput,
        out: &mut Emitter<'_>,
    ) -> rusty_agui::Result<Option<Value>> {
        let last = input
            .messages
            .iter()
            .rev()
            .find_map(|m| match m {
                Message::User { content, .. } => Some(content.text()),
                _ => None,
            })
            .unwrap_or_default();
        if last == "fail" {
            return Err(Error::Agent("asked to fail".into()));
        }

        out.state(json!({"turns": 0, "echoed": null}))?;
        out.emit(EventKind::StepStarted {
            step_name: "echo".into(),
        })?;

        if let Some(tool) = input.tools.first() {
            let tool_call_id = out.next_id();
            let message_id = out.next_id();
            out.emit(EventKind::ToolCallStart {
                tool_call_id: tool_call_id.clone(),
                tool_call_name: tool.name.clone(),
                parent_message_id: Some(message_id),
            })?;
            out.emit(EventKind::ToolCallArgs {
                tool_call_id: tool_call_id.clone(),
                delta: json!({"text": last.as_str()}).to_json_string(),
            })?;
            out.emit(EventKind::ToolCallEnd { tool_call_id })?;
        }

        out.emit(EventKind::StateDelta {
            delta: json!([
                {"op": "replace", "path": "/turns", "value": 1},
                {"op": "replace", "path": "/echoed", "value": last.as_str()}
            ]),
        })?;
        out.text(&format!("you said: {last}"))?;
        out.emit(EventKind::StepFinished {
            step_name: "echo".into(),
        })?;
        Ok(Some(json!({"echoed": last})))
    }
}

fn main() -> std::io::Result<()> {
    let addr = std::env::var("AGUI_ECHO_ADDR").unwrap_or_else(|_| "127.0.0.1:0".into());
    let server = rusty_serve::Server::bind(
        addr.parse().expect("AGUI_ECHO_ADDR is host:port"),
        AgentHandler::new(Echo),
    )?;
    let local = server.local_addr()?;
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "LISTENING http://{local}/api/agent")?;
    stdout.flush()?;
    server.run()
}
