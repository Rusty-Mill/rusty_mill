//! End-to-end: `rusty_agui`'s own client talking to a Rusty Keys session served
//! by `AgentHandler` on `rusty_serve`, over a real socket, with the scripted
//! `FakeLanguageModel` driving the real harness in a temporary workspace.

use rk_agui::{KeyAgent, APPROVE_TOOL, PLAN_TOOL};
use rk_config::Config;
use rk_constrain::ApprovalTrigger;
use rk_kernel::fake::{FakeLanguageModel, Scripted};
use rusty_agui::client::HttpAgent;
use rusty_agui::serve::AgentHandler;
use rusty_agui::{Event, EventKind, Message, Reducer, RunAgentInput};
use serde_json::json;
use tokio::runtime::Runtime;

struct Served {
    client: HttpAgent,
    workspace: std::path::PathBuf,
    _runtime: Runtime,
    _stop: rusty_serve::ShutdownHandle,
}

fn workspace(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rk-agui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn serve(name: &str, turns: Vec<Vec<Scripted>>, triggers: Vec<ApprovalTrigger>) -> Served {
    let workspace = workspace(name);
    let ws = workspace.to_string_lossy().into_owned();
    let config = Config::resolve(|key| match key {
        "RUSTYKEYS_MODEL" => Some("fake".into()),
        "RUSTYKEYS_WORKSPACE" => Some(ws.clone()),
        _ => None,
    })
    .unwrap();
    let runtime = Runtime::new().unwrap();
    let agent = KeyAgent::new(
        runtime.handle().clone(),
        config,
        FakeLanguageModel::new(turns),
    )
    .with_approval(triggers);
    let server =
        rusty_serve::Server::bind("127.0.0.1:0".parse().unwrap(), AgentHandler::new(agent))
            .unwrap();
    let addr = server.local_addr().unwrap();
    let stop = server.shutdown_handle().unwrap();
    std::thread::spawn(move || server.run().unwrap());
    Served {
        client: HttpAgent::new(&format!("http://{addr}/api/agent")).unwrap(),
        workspace,
        _runtime: runtime,
        _stop: stop,
    }
}

fn run(served: &Served, input: &RunAgentInput) -> Vec<Event> {
    served
        .client
        .run(input)
        .unwrap()
        .map(|e| e.unwrap())
        .collect()
}

fn view(input: &RunAgentInput, events: &[Event]) -> Reducer {
    let mut view = Reducer::from_input(input);
    view.apply_all(events).unwrap();
    view
}

fn run_error(events: &[Event]) -> Option<&str> {
    events.iter().find_map(|e| match &e.kind {
        EventKind::RunError { message, .. } => Some(message.as_str()),
        _ => None,
    })
}

fn result(events: &[Event]) -> Option<rusty_json::Value> {
    events.iter().find_map(|e| match &e.kind {
        EventKind::RunFinished { result, .. } => result.clone(),
        _ => None,
    })
}

fn first_call(view: &Reducer) -> rusty_agui::ToolCall {
    view.messages
        .iter()
        .find_map(|m| match m {
            Message::Assistant { tool_calls, .. } => tool_calls.first().cloned(),
            _ => None,
        })
        .expect("a tool call")
}

fn answer(view: &Reducer, call_id: &str, text: &str) -> Vec<Message> {
    let mut messages = view.messages.clone();
    messages.push(Message::Tool {
        id: format!("tm-{call_id}"),
        content: text.into(),
        tool_call_id: call_id.into(),
        error: None,
    });
    messages
}

#[test]
fn a_turn_streams_the_reply_and_reports_tool_events() {
    let served = serve(
        "turn",
        vec![
            vec![Scripted::ToolCall {
                name: "read_file".into(),
                args: json!({"path": "note.txt"}),
            }],
            vec![Scripted::Text("I read the note.".into())],
        ],
        vec![],
    );
    std::fs::write(
        served.workspace.join("note.txt"),
        "hello from the workspace",
    )
    .unwrap();
    let input = RunAgentInput::new("t", "r1", vec![Message::user("u1", "Read note.txt")]);
    let events = run(&served, &input);
    let view = view(&input, &events);
    assert!(
        matches!(view.messages.iter().find(|m| matches!(m, Message::Assistant { content: Some(_), .. })), Some(Message::Assistant { content: Some(text), .. }) if text == "I read the note.")
    );
    let call = first_call(&view);
    assert_eq!(call.function.name, "read_file");
    assert_eq!(call.function.arguments, r#"{"path":"note.txt"}"#);
    assert!(view.messages.iter().any(|m| matches!(m, Message::Tool { content, tool_call_id, .. } if tool_call_id == &call.id && content.text().contains("hello from the workspace"))));
    let result = result(&events).expect("a result");
    assert_eq!(
        result.get("reply").and_then(|v| v.as_str()),
        Some("I read the note.")
    );
    assert!(result.get("verified").is_some());
}

#[test]
fn an_approval_is_a_frontend_tool_call_answered_on_the_next_run() {
    let served = serve(
        "approval",
        vec![
            vec![Scripted::ToolCall {
                name: "write_file".into(),
                args: json!({"path": "new.txt", "content": "made"}),
            }],
            vec![Scripted::Text("Written.".into())],
        ],
        vec![ApprovalTrigger::NewFilePath],
    );
    let first = RunAgentInput::new("t", "r1", vec![Message::user("u1", "Create new.txt")]);
    let events = run(&served, &first);
    let view1 = view(&first, &events);
    let call = first_call(&view1);
    assert_eq!(call.function.name, APPROVE_TOOL);
    let args: serde_json::Value = serde_json::from_str(&call.function.arguments).unwrap();
    assert_eq!(args["tool"], "write_file");
    assert_eq!(args["args"]["path"], "new.txt");
    assert_eq!(args["trigger"], "NewFilePath");
    assert!(events
        .iter()
        .any(|e| matches!(e.kind, EventKind::RunFinished { .. })));
    assert!(
        !served.workspace.join("new.txt").exists(),
        "the gate holds the write"
    );

    // A new prompt while the turn waits is refused; so is an answer on another thread.
    let busy = RunAgentInput::new("t", "r2", vec![Message::user("u2", "something else")]);
    assert!(run_error(&run(&served, &busy))
        .unwrap()
        .contains("waiting on an approval"));
    let other = RunAgentInput::new("other", "r3", answer(&view1, &call.id, "allow"));
    assert!(run_error(&run(&served, &other))
        .unwrap()
        .contains("no pending approval"));

    let second = RunAgentInput::new("t", "r4", answer(&view1, &call.id, "allow"));
    let events = run(&served, &second);
    let view2 = view(&second, &events);
    assert!(view2.messages.iter().any(
        |m| matches!(m, Message::Assistant { content: Some(text), .. } if text == "Written.")
    ));
    assert_eq!(
        std::fs::read_to_string(served.workspace.join("new.txt")).unwrap(),
        "made"
    );
    assert!(view2.messages.iter().any(
        |m| matches!(m, Message::Tool { content, .. } if !content.text().contains("blocked"))
    ));
    assert_eq!(
        result(&events)
            .unwrap()
            .get("reply")
            .and_then(|v| v.as_str()),
        Some("Written.")
    );
}

#[test]
fn a_blocked_approval_is_a_blocked_tool_result() {
    let served = serve(
        "block",
        vec![
            vec![Scripted::ToolCall {
                name: "write_file".into(),
                args: json!({"path": "nope.txt", "content": "x"}),
            }],
            vec![Scripted::Text("Understood.".into())],
        ],
        vec![ApprovalTrigger::NewFilePath],
    );
    let first = RunAgentInput::new("t", "r1", vec![Message::user("u1", "Create nope.txt")]);
    let view1 = view(&first, &run(&served, &first));
    let call = first_call(&view1);
    let second = RunAgentInput::new("t", "r2", answer(&view1, &call.id, "no"));
    let events = run(&served, &second);
    let view2 = view(&second, &events);
    assert!(!served.workspace.join("nope.txt").exists());
    assert!(view2.messages.iter().any(|m| matches!(m, Message::Tool { content, .. } if content.text().contains("\"status\":\"blocked\""))));
    assert!(view2.messages.iter().any(
        |m| matches!(m, Message::Assistant { content: Some(text), .. } if text == "Understood.")
    ));
}

#[test]
fn a_plan_exit_is_a_frontend_tool_call_and_the_decision_resolves_it() {
    let served = serve(
        "plan",
        vec![
            vec![Scripted::ToolCall {
                name: "enter_plan_mode".into(),
                args: json!({}),
            }],
            vec![Scripted::ToolCall {
                name: "exit_plan_mode".into(),
                args: json!({"plan": "1. write the file"}),
            }],
            vec![Scripted::Text("Plan submitted.".into())],
        ],
        vec![],
    );
    let first = RunAgentInput::new("t", "r1", vec![Message::user("u1", "Plan the work")]);
    let events = run(&served, &first);
    let view1 = view(&first, &events);
    let call = view1
        .messages
        .iter()
        .flat_map(|m| match m {
            Message::Assistant { tool_calls, .. } => tool_calls.clone(),
            _ => vec![],
        })
        .find(|c| c.function.name == PLAN_TOOL)
        .expect("a plan call");
    assert!(call.function.arguments.contains("write the file"));
    let second = RunAgentInput::new("t", "r2", answer(&view1, &call.id, "annotate add tests"));
    let events = run(&served, &second);
    assert!(run_error(&events).is_none());
    let result = result(&events).unwrap();
    assert_eq!(
        result.get("feedback").and_then(|v| v.as_str()),
        Some("add tests")
    );
    // Resolved: answering again is refused.
    assert!(run_error(&run(&served, &second))
        .unwrap()
        .contains("no pending"));
}

#[test]
fn bad_runs_are_errors() {
    let served = serve("bad", vec![], vec![]);
    let empty = RunAgentInput::new("t", "r1", vec![]);
    assert!(run_error(&run(&served, &empty))
        .unwrap()
        .contains("last message"));
    let blank = RunAgentInput::new("t", "r2", vec![Message::user("u1", "  ")]);
    assert!(run_error(&run(&served, &blank)).unwrap().contains("empty"));
}
