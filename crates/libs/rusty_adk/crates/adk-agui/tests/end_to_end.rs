//! End-to-end: `rusty_agui`'s own client talking to an ADK agent served by
//! `AgentHandler` on `rusty_serve`, over a real socket.

use adk_agents::AgentNode;
use adk_agents::LlmAgent;
use adk_agui::{AdkAgent, INPUT_TOOL};
use adk_core::{RunConfig, Schema, Services, StreamingMode};
use adk_graph::{chain, FunctionNode, Graph, NodeConfig, NodeOutcome};
use adk_models::MockModel;
use adk_runner::Runner;
use adk_sessions::InMemorySessionService;
use rusty_agui::client::HttpAgent;
use rusty_agui::serve::AgentHandler;
use rusty_agui::{Event, EventKind, Message, Reducer, RunAgentInput, RunOutcome};
use rusty_json::json;
use serde_json::json as adk_json;
use std::sync::Arc;
use tokio::runtime::Runtime;

struct Served {
    client: HttpAgent,
    runner: Arc<Runner>,
    runtime: Runtime,
    _stop: rusty_serve::ShutdownHandle,
}

fn serve(build: impl FnOnce() -> Runner) -> Served {
    serve_with(build, None)
}

fn serve_with(build: impl FnOnce() -> Runner, config: Option<RunConfig>) -> Served {
    let runtime = Runtime::new().unwrap();
    let runner = Arc::new(runtime.block_on(async { build() }));
    let mut agent = AdkAgent::new(runner.clone(), runtime.handle().clone());
    if let Some(config) = config {
        agent = agent.with_run_config(config);
    }
    let server =
        rusty_serve::Server::bind("127.0.0.1:0".parse().unwrap(), AgentHandler::new(agent))
            .unwrap();
    let addr = server.local_addr().unwrap();
    let stop = server.shutdown_handle().unwrap();
    std::thread::spawn(move || server.run().unwrap());
    let client = HttpAgent::new(&format!("http://{addr}/api/agent")).unwrap();
    Served {
        client,
        runner,
        runtime,
        _stop: stop,
    }
}

fn run(served: &Served, input: &RunAgentInput) -> Vec<Event> {
    served
        .client
        .run(input)
        .unwrap()
        .map(|event| event.unwrap())
        .collect()
}

fn services() -> Services {
    Services::new(Arc::new(InMemorySessionService::new()))
}

fn weather_agent() -> adk_agents::SharedAgent {
    LlmAgent::builder("weather")
        .model(Arc::new(
            MockModel::new()
                .push_call_json("get_weather", adk_json!({"city": "Paris"}))
                .push_text("It is sunny in Paris."),
        ))
        .description("Answers weather questions.")
        .tool(get_weather())
        .output_key("last_report")
        .build()
        .unwrap()
        .shared()
}

/// A tool without the macro crate: the smallest `FunctionTool` that answers.
fn get_weather() -> Arc<dyn adk_tools::Tool> {
    Arc::new(adk_tools::FunctionTool::new(
        "get_weather",
        "Looks up the weather.",
        Schema::object().property("city", Schema::string()),
        |args, _ctx| {
            Box::pin(async move {
                Ok(
                    adk_json!({"report": format!("sunny in {}", args["city"].as_str().unwrap_or(""))}),
                )
            })
        },
    ))
}

fn finished(events: &[Event]) -> &Event {
    events
        .iter()
        .find(|e| matches!(e.kind, EventKind::RunFinished { .. }))
        .expect("RUN_FINISHED")
}

#[test]
fn a_run_streams_the_answer_tool_calls_and_state() {
    let served = serve(|| Runner::new("app", weather_agent(), services()));
    let input = RunAgentInput::new("t1", "r1", vec![Message::user("u1", "Weather in Paris?")]);
    let events = run(&served, &input);

    let mut view = Reducer::from_input(&input);
    view.apply_all(&events).unwrap();
    let last = view.messages.last().unwrap();
    assert!(
        matches!(last, Message::Assistant { content: Some(text), .. } if text == "It is sunny in Paris.")
    );
    assert!(view.messages.iter().any(|m| matches!(m, Message::Assistant { tool_calls, .. } if tool_calls.iter().any(|c| c.function.name == "get_weather" && c.function.arguments.contains("Paris")))));
    assert!(view.messages.iter().any(
        |m| matches!(m, Message::Tool { content, .. } if content.text().contains("sunny in Paris"))
    ));
    assert_eq!(
        view.state.get("last_report").and_then(|v| v.as_str()),
        Some("It is sunny in Paris.")
    );
    assert!(matches!(
        &finished(&events).kind,
        EventKind::RunFinished {
            outcome: Some(RunOutcome::Success),
            ..
        }
    ));
}

#[test]
fn the_thread_is_the_session_and_the_history_is_the_agents() {
    let served = serve(|| {
        let agent = LlmAgent::builder("echo")
            .model(Arc::new(MockModel::new().push_text("one").push_text("two")))
            .description("Counts.")
            .build()
            .unwrap()
            .shared();
        Runner::new("app", agent, services())
    });
    let first = RunAgentInput::new("t", "r1", vec![Message::user("u1", "hi")]);
    let events = run(&served, &first);
    let mut view = Reducer::from_input(&first);
    view.apply_all(&events).unwrap();
    let second = RunAgentInput::new("t", "r2", {
        let mut messages = view.messages.clone();
        messages.push(Message::user("u2", "again"));
        messages
    });
    let events = run(&served, &second);
    let mut view = Reducer::from_input(&second);
    view.apply_all(&events).unwrap();
    assert!(
        matches!(view.messages.last(), Some(Message::Assistant { content: Some(text), .. }) if text == "two")
    );

    // The thread id is the session id and, with no userId forwarded, the user id.
    let session = served
        .runtime
        .block_on(served.runner.session("t", "t"))
        .unwrap()
        .expect("session exists");
    // Two user turns in the agent's own history: the client's replay was not appended.
    assert_eq!(
        session.events.iter().filter(|e| e.author == "user").count(),
        2
    );
}

#[test]
fn a_streaming_run_sends_deltas_once() {
    let served = serve_with(
        || {
            let agent = LlmAgent::builder("stream")
                .model(Arc::new(
                    MockModel::new().push_stream(["It is ", "sunny", "."]),
                ))
                .description("Streams.")
                .build()
                .unwrap()
                .shared();
            Runner::new("app", agent, services())
        },
        Some(RunConfig {
            streaming_mode: StreamingMode::Sse,
            ..RunConfig::default()
        }),
    );
    let input = RunAgentInput::new("t", "r1", vec![Message::user("u1", "hi")]);
    let events = run(&served, &input);
    let deltas: Vec<&str> = events
        .iter()
        .filter_map(|e| match &e.kind {
            EventKind::TextMessageContent { delta, .. } => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(deltas, ["It is ", "sunny", "."]);
    let mut view = Reducer::from_input(&input);
    view.apply_all(&events).unwrap();
    assert_eq!(
        view.messages.len(),
        2,
        "the aggregated event did not repeat the text"
    );
    assert!(
        matches!(view.messages.last(), Some(Message::Assistant { content: Some(text), .. }) if text == "It is sunny.")
    );
}

fn approval_graph() -> Runner {
    let approve = FunctionNode::new("approve", NodeConfig::default(), |ctx| {
        let ctx = ctx.clone();
        Box::pin(async move {
            let answer = ctx.resume_or_request_input(
                "May I answer?",
                Some(adk_json!({"choices": ["yes", "no"]})),
            )?;
            Ok(NodeOutcome::output(answer))
        })
    })
    .shared();
    let assistant = LlmAgent::builder("assistant")
        .model(Arc::new(MockModel::new().push_text("Answered.")))
        .description("Answers.")
        .build()
        .unwrap()
        .shared();
    let graph = Arc::new(
        Graph::new(
            vec![approve, AgentNode::new(assistant).shared()],
            chain(["approve", "assistant"]),
        )
        .unwrap(),
    );
    Runner::new("app", graph, services())
}

#[test]
fn a_suspension_is_a_frontend_tool_call_and_the_answer_resumes_it() {
    let served = serve(approval_graph);
    let first = RunAgentInput::new("t", "r1", vec![Message::user("u1", "Weather?")]);
    let events = run(&served, &first);
    let mut view = Reducer::from_input(&first);
    view.apply_all(&events).unwrap();
    let call = view
        .messages
        .iter()
        .find_map(|m| match m {
            Message::Assistant { tool_calls, .. } => tool_calls.first(),
            _ => None,
        })
        .expect("a frontend tool call");
    assert_eq!(call.function.name, INPUT_TOOL);
    assert_eq!(
        rusty_json::Value::parse(&call.function.arguments).unwrap(),
        json!({"hint": "May I answer?", "payload": {"choices": ["yes", "no"]}})
    );
    assert!(matches!(
        &finished(&events).kind,
        EventKind::RunFinished { .. }
    ));

    // The person answers: the tool message resumes the graph at the node.
    let mut messages = view.messages.clone();
    messages.push(Message::Tool {
        id: "tm1".into(),
        content: "yes".into(),
        tool_call_id: call.id.clone(),
        error: None,
    });
    let second = RunAgentInput::new("t", "r2", messages);
    let events = run(&served, &second);
    let mut view = Reducer::from_input(&second);
    view.apply_all(&events).unwrap();
    assert!(
        matches!(view.messages.last(), Some(Message::Assistant { content: Some(text), .. }) if text == "Answered.")
    );

    // An answer to a call nobody is waiting on is refused.
    let stray = RunAgentInput::new(
        "t",
        "r3",
        vec![Message::Tool {
            id: "tm2".into(),
            content: "yes".into(),
            tool_call_id: "nope".into(),
            error: None,
        }],
    );
    let events = run(&served, &stray);
    assert!(events.iter().any(|e| matches!(&e.kind, EventKind::RunError { message, .. } if message.contains("no pending interrupt"))));
}

#[test]
fn a_run_without_a_user_message_is_an_error() {
    let served = serve(|| Runner::new("app", weather_agent(), services()));
    let events = run(&served, &RunAgentInput::new("t", "r1", vec![]));
    assert!(events.iter().any(|e| matches!(&e.kind, EventKind::RunError { message, .. } if message.contains("last message"))));
}
