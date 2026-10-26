//! A live provider through the AG-UI handler: `AnthropicModel` (reqwest) is
//! pointed at a loopback stand-in for the Messages API, so the run crosses a
//! real HTTP transport without credentials. `MockModel` never reaches reqwest,
//! so only this test checks that `AdkAgent` drives runs on a runtime the live
//! models can use.

use adk_agents::LlmAgent;
use adk_agui::AdkAgent;
use adk_core::Services;
use adk_models::AnthropicModel;
use adk_runner::Runner;
use adk_sessions::InMemorySessionService;
use rusty_agui::client::HttpAgent;
use rusty_agui::serve::AgentHandler;
use rusty_agui::{EventKind, Message, Reducer, RunAgentInput, RunOutcome};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use tokio::runtime::Runtime;

const ANSWER: &str =
    r#"{"content":[{"type":"text","text":"Hello over the wire."}],"stop_reason":"end_turn"}"#;

/// Answers one HTTP request on `listener` with the canned Messages reply.
fn answer_once(listener: TcpListener) {
    let (stream, _) = listener.accept().unwrap();
    let mut reader = BufReader::new(stream);
    let mut length = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().unwrap();
            }
        }
    }
    reader.read_exact(&mut vec![0; length]).unwrap();
    let reply = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{ANSWER}",
        ANSWER.len()
    );
    reader.get_mut().write_all(reply.as_bytes()).unwrap();
}

#[test]
fn a_live_model_answers_through_the_handler() {
    let api = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/v1/messages", api.local_addr().unwrap());
    let api = std::thread::spawn(move || answer_once(api));

    let runtime = Runtime::new().unwrap();
    let model = AnthropicModel::new("claude-test", "test-key").with_endpoint(endpoint);
    let agent = LlmAgent::builder("greeter")
        .model(Arc::new(model))
        .description("Greets people.")
        .build()
        .unwrap()
        .shared();
    let services = Services::new(Arc::new(InMemorySessionService::new()));
    let runner = Arc::new(Runner::new("app", agent, services));
    let handler = AgentHandler::new(AdkAgent::new(runner, runtime.handle().clone()));
    let server = rusty_serve::Server::bind("127.0.0.1:0".parse().unwrap(), handler).unwrap();
    let addr = server.local_addr().unwrap();
    let _stop = server.shutdown_handle().unwrap();
    std::thread::spawn(move || server.run().unwrap());

    let client = HttpAgent::new(&format!("http://{addr}/api/agent")).unwrap();
    let input = RunAgentInput::new("t", "r1", vec![Message::user("u1", "hi")]);
    let events: Vec<_> = client.run(&input).unwrap().map(|e| e.unwrap()).collect();
    api.join().unwrap();

    let mut view = Reducer::from_input(&input);
    view.apply_all(&events).unwrap();
    assert!(matches!(
        view.messages.last(),
        Some(Message::Assistant { content: Some(text), .. }) if text == "Hello over the wire."
    ));
    assert!(events.iter().any(|e| matches!(
        e.kind,
        EventKind::RunFinished {
            outcome: Some(RunOutcome::Success),
            ..
        }
    )));
}
