//! End-to-end: `rusty_agui`'s own client talking to a Nexus agent served by
//! `AgentHandler` on `rusty_serve`, over a real socket, against a scripted
//! runtime that plays the ai-runtime's part.

use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use nexus_agui::{NexusAgent, Runtime, DECIDE_TOOL};
use nexus_ai_runtime::events::AiEvent;
use rusty_agui::client::HttpAgent;
use rusty_agui::serve::AgentHandler;
use rusty_agui::{Event, EventKind, Message, Reducer, RunAgentInput};
use rusty_json::json as agui_json;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime as Tokio;
use tokio::sync::broadcast;

/// What a session does once submitted or decided: the events it emits.
type Script = Arc<dyn Fn(Call) -> Vec<Fn2> + Send + Sync>;
type Fn2 = Box<dyn FnOnce(uuid::Uuid) -> AiEvent + Send>;

#[derive(Debug, Clone, PartialEq)]
enum Call {
    Submit(Value),
    Decide { session_id: String, decision: Value },
}

struct Scripted {
    bus: broadcast::Sender<AiEvent>,
    script: Script,
    calls: Mutex<Vec<Call>>,
    task_id: uuid::Uuid,
}

impl Scripted {
    fn new(script: impl Fn(Call) -> Vec<Fn2> + Send + Sync + 'static) -> Arc<Self> {
        let (bus, _) = broadcast::channel(64);
        Arc::new(Self {
            bus,
            script: Arc::new(script),
            calls: Mutex::new(Vec::new()),
            task_id: uuid::Uuid::new_v4(),
        })
    }

    fn play(&self, call: Call) {
        self.calls.lock().unwrap().push(call.clone());
        let events = (self.script)(call);
        let bus = self.bus.clone();
        let task_id = self.task_id;
        tokio::spawn(async move {
            // Published after the subscriber is in place, as the runtime does.
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            for make in events {
                let _ = bus.send(make(task_id));
            }
        });
    }
}

#[async_trait]
impl Runtime for Scripted {
    fn subscribe(&self) -> BoxStream<'static, AiEvent> {
        let receiver = self.bus.subscribe();
        futures::stream::unfold(receiver, |mut receiver| async move {
            match receiver.recv().await {
                Ok(event) => Some((event, receiver)),
                Err(broadcast::error::RecvError::Lagged(_)) => Some((
                    AiEvent::Resumed {
                        task_id: uuid::Uuid::nil(),
                    },
                    receiver,
                )),
                Err(broadcast::error::RecvError::Closed) => None,
            }
        })
        .boxed()
    }

    async fn submit(&self, args: Value) -> Result<uuid::Uuid, String> {
        if args.get("goal").and_then(Value::as_str) == Some("refuse") {
            return Err("admission limit reached".into());
        }
        self.play(Call::Submit(args));
        Ok(self.task_id)
    }

    async fn decide(&self, session_id: &str, decision: Value) -> Result<(), String> {
        self.play(Call::Decide {
            session_id: session_id.into(),
            decision,
        });
        Ok(())
    }
}

fn chunk(text: &'static str) -> Fn2 {
    Box::new(move |task_id| AiEvent::TokenChunk {
        task_id,
        text: text.into(),
    })
}

fn finished(outcome: Value) -> Fn2 {
    Box::new(move |task_id| AiEvent::Finished { task_id, outcome })
}

struct Served {
    client: HttpAgent,
    runtime: Arc<Scripted>,
    _tokio: Tokio,
    _stop: rusty_serve::ShutdownHandle,
}

fn serve(runtime: Arc<Scripted>, approval: bool) -> Served {
    let tokio = Tokio::new().unwrap();
    let mut agent = NexusAgent::new(runtime.clone(), tokio.handle().clone());
    if approval {
        agent = agent.with_approval(60);
    }
    let server =
        rusty_serve::Server::bind("127.0.0.1:0".parse().unwrap(), AgentHandler::new(agent))
            .unwrap();
    let addr = server.local_addr().unwrap();
    let stop = server.shutdown_handle().unwrap();
    std::thread::spawn(move || server.run().unwrap());
    Served {
        client: HttpAgent::new(&format!("http://{addr}/api/agent")).unwrap(),
        runtime,
        _tokio: tokio,
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

#[test]
fn a_run_streams_tokens_tool_calls_and_the_result() {
    let runtime = Scripted::new(|call| match call {
        Call::Submit(_) => vec![
            Box::new(|task_id| AiEvent::Started {
                task_id,
                attempt: 1,
            }),
            chunk("It is "),
            Box::new(|task_id| AiEvent::ToolCalled {
                task_id,
                tool_use_id: "tu1".into(),
                name: "read_file".into(),
                args_preview: r#"{"path":"weather.md"}"#.into(),
            }),
            Box::new(|task_id| AiEvent::ToolResult {
                task_id,
                tool_use_id: "tu1".into(),
                is_error: false,
                summary: "sunny".into(),
            }),
            chunk("sunny."),
            finished(
                json!({ "id": "s", "outcome": "complete", "tokens_used": 42, "rounds": [{ "round": 1, "text": "It is sunny." }] }),
            ),
        ],
        Call::Decide { .. } => vec![],
    });
    let served = serve(runtime, false);
    let input = RunAgentInput::new("t", "r1", vec![Message::user("u1", "Weather?")]);
    let events = run(&served, &input);
    let view = view(&input, &events);
    let texts: Vec<String> = view
        .messages
        .iter()
        .filter_map(|m| match m {
            Message::Assistant {
                content: Some(text),
                ..
            } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        texts,
        ["It is ", "sunny."],
        "two messages: the tool call split the stream, the final text was not repeated"
    );
    assert!(view.messages.iter().any(|m| matches!(m, Message::Assistant { tool_calls, .. } if tool_calls.iter().any(|c| c.function.name == "read_file" && c.function.arguments == r#"{"path":"weather.md"}"#))));
    assert!(view.messages.iter().any(|m| matches!(m, Message::Tool { content, tool_call_id, .. } if tool_call_id == "tu1" && content.text() == "sunny")));
    let result = events.iter().find_map(|e| match &e.kind {
        EventKind::RunFinished { result, .. } => result.clone(),
        _ => None,
    });
    assert_eq!(
        result,
        Some(
            agui_json!({ "sessionId": result.as_ref().unwrap().get("sessionId").unwrap().clone(), "outcome": "complete", "tokensUsed": 42 })
        )
    );
    let submitted = served.runtime.calls.lock().unwrap().clone();
    assert!(
        matches!(&submitted[0], Call::Submit(args) if args["goal"] == "Weather?" && args["auto_approve"] == true && args["session_id"].is_string())
    );
}

#[test]
fn without_streamed_text_the_last_round_answers() {
    let runtime = Scripted::new(|_| {
        vec![finished(
            json!({ "outcome": "complete", "rounds": [{ "round": 1, "text": "Done." }] }),
        )]
    });
    let served = serve(runtime, false);
    let input = RunAgentInput::new("t", "r1", vec![Message::user("u1", "go")]);
    let view = view(&input, &run(&served, &input));
    assert!(
        matches!(view.messages.last(), Some(Message::Assistant { content: Some(text), .. }) if text == "Done.")
    );
}

#[test]
fn a_proposed_round_is_a_frontend_tool_call_and_the_decision_continues_it() {
    let runtime = Scripted::new(|call| match call {
        Call::Submit(_) => vec![Box::new(|task_id| AiEvent::RoundProposed {
            task_id,
            round: 1,
            narration: "I will read weather.md".into(),
        })],
        Call::Decide { decision, .. } if decision["kind"] == "approve_all" => vec![
            Box::new(|task_id| AiEvent::RoundDecided {
                task_id,
                round: 1,
                decision_kind: "approve".into(),
            }),
            chunk("Sunny."),
            finished(json!({ "outcome": "complete", "rounds": [] })),
        ],
        Call::Decide { .. } => vec![finished(json!({ "outcome": "aborted", "rounds": [] }))],
    });
    let served = serve(runtime, true);
    let first = RunAgentInput::new("t", "r1", vec![Message::user("u1", "Weather?")]);
    let events = run(&served, &first);
    let view1 = view(&first, &events);
    let call = view1
        .messages
        .iter()
        .find_map(|m| match m {
            Message::Assistant { tool_calls, .. } => tool_calls.first(),
            _ => None,
        })
        .expect("a frontend tool call")
        .clone();
    assert_eq!(call.function.name, DECIDE_TOOL);
    assert!(call.function.arguments.contains("weather.md"));
    assert!(view1.messages.iter().any(|m| matches!(m, Message::Assistant { content: Some(text), .. } if text == "I will read weather.md")));
    assert!(events
        .iter()
        .any(|e| matches!(e.kind, EventKind::RunFinished { .. })));
    {
        let calls = served.runtime.calls.lock().unwrap();
        assert!(
            matches!(&calls[0], Call::Submit(args) if args["auto_approve"] == false && args["approval_timeout_secs"] == 60)
        );
    }

    // Answered on a second thread: refused, the session belongs to "t".
    let stray = RunAgentInput::new(
        "other",
        "r2",
        vec![Message::Tool {
            id: "tm0".into(),
            content: "approve".into(),
            tool_call_id: call.id.clone(),
            error: None,
        }],
    );
    assert!(run_error(&run(&served, &stray))
        .unwrap()
        .contains("no proposed round"));

    let mut messages = view1.messages.clone();
    messages.push(Message::Tool {
        id: "tm1".into(),
        content: "approve".into(),
        tool_call_id: call.id.clone(),
        error: None,
    });
    let second = RunAgentInput::new("t", "r3", messages);
    let events = run(&served, &second);
    let view2 = view(&second, &events);
    assert!(
        matches!(view2.messages.last(), Some(Message::Assistant { content: Some(text), .. }) if text == "Sunny.")
    );
    let calls = served.runtime.calls.lock().unwrap().clone();
    assert!(
        matches!(&calls[1], Call::Decide { session_id, decision } if call.id.starts_with(session_id.as_str()) && decision["kind"] == "approve_all")
    );

    // The decision was consumed: answering again is refused.
    let again = run(&served, &second);
    assert!(run_error(&again).unwrap().contains("no proposed round"));
}

#[test]
fn an_abort_decision_carries_the_reason() {
    let runtime = Scripted::new(|call| match call {
        Call::Submit(_) => vec![Box::new(|task_id| AiEvent::RoundProposed {
            task_id,
            round: 1,
            narration: String::new(),
        })],
        Call::Decide { .. } => vec![finished(json!({ "outcome": "aborted", "rounds": [] }))],
    });
    let served = serve(runtime, true);
    let first = RunAgentInput::new("t", "r1", vec![Message::user("u1", "Delete everything")]);
    let view1 = view(&first, &run(&served, &first));
    let call = view1
        .messages
        .iter()
        .find_map(|m| match m {
            Message::Assistant { tool_calls, .. } => tool_calls.first().cloned(),
            _ => None,
        })
        .unwrap();
    let mut messages = view1.messages.clone();
    messages.push(Message::Tool {
        id: "tm1".into(),
        content: "not that".into(),
        tool_call_id: call.id,
        error: None,
    });
    let second = RunAgentInput::new("t", "r2", messages);
    let events = run(&served, &second);
    assert!(events.iter().any(|e| matches!(&e.kind, EventKind::RunFinished { result: Some(result), .. } if result.get("outcome").and_then(|v| v.as_str()) == Some("aborted"))));
    let calls = served.runtime.calls.lock().unwrap().clone();
    assert!(
        matches!(&calls[1], Call::Decide { decision, .. } if decision == &json!({ "kind": "abort", "reason": "not that" }))
    );
}

#[test]
fn failures_are_run_errors() {
    let runtime = Scripted::new(|call| match call {
        Call::Submit(args) if args["goal"] == "fail" => vec![Box::new(|task_id| AiEvent::Failed {
            task_id,
            error: "provider down".into(),
            retriable: false,
        })],
        Call::Submit(args) if args["goal"] == "cancel" => {
            vec![Box::new(|task_id| AiEvent::Cancelled {
                task_id,
                by: "caller".into(),
            })]
        }
        _ => vec![],
    });
    let served = serve(runtime, false);
    for (goal, expected) in [
        ("fail", "provider down"),
        ("cancel", "cancelled by caller"),
        ("refuse", "admission limit"),
    ] {
        let input = RunAgentInput::new("t", goal, vec![Message::user("u1", goal)]);
        assert!(
            run_error(&run(&served, &input)).unwrap().contains(expected),
            "{goal}"
        );
    }
    let empty = RunAgentInput::new("t", "r", vec![]);
    assert!(run_error(&run(&served, &empty))
        .unwrap()
        .contains("last message"));
    let blank = RunAgentInput::new("t", "r", vec![Message::user("u1", "   ")]);
    assert!(run_error(&run(&served, &blank)).unwrap().contains("empty"));
}
