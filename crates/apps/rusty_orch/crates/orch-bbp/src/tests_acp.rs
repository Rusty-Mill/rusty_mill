//! #1318 S3: the ACP client's codec, a whole turn against a scripted fake
//! agent over a socketpair, permission answers, failure endings and the
//! `acp.log` renderer.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::thread::JoinHandle;

use rusty_json::{json, Value};

use crate::acp::{self, Client, Incoming, Render, StopReason, Turn, TurnError, Usage};

#[test]
fn messages_round_trip_through_the_codec() {
    let line = acp::request(3, "session/new", json!({"cwd": "/w"}));
    assert_eq!(
        Value::parse(&line).expect("json"),
        json!({"jsonrpc": "2.0", "id": 3, "method": "session/new", "params": {"cwd": "/w"}})
    );
    assert_eq!(
        acp::parse(r#"{"jsonrpc":"2.0","id":3,"result":{"sessionId":"s"}}"#),
        Ok(Incoming::Response {
            id: 3,
            outcome: Ok(json!({"sessionId": "s"}))
        })
    );
    assert_eq!(
        acp::parse(r#"{"jsonrpc":"2.0","id":4,"error":{"code":-32000,"message":"no"}}"#),
        Ok(Incoming::Response {
            id: 4,
            outcome: Err("-32000 no".into())
        })
    );
    assert_eq!(
        acp::parse(
            r#"{"jsonrpc":"2.0","id":"p1","method":"session/request_permission","params":{}}"#
        ),
        Ok(Incoming::Request {
            id: json!("p1"),
            method: "session/request_permission".into(),
            params: json!({})
        })
    );
    assert_eq!(
        acp::parse(r#"{"jsonrpc":"2.0","method":"session/update","params":{"x":1}}"#),
        Ok(Incoming::Notification {
            method: "session/update".into(),
            params: json!({"x": 1})
        })
    );
    let answer = acp::answer(&json!("p1"), Err((-32601, "not offered")));
    assert_eq!(
        Value::parse(&answer).expect("json"),
        json!({"jsonrpc": "2.0", "id": "p1", "error": {"code": -32601, "message": "not offered"}})
    );
}

#[test]
fn lines_that_are_not_json_rpc_are_refused() {
    for bad in [
        "not json",
        r#"{"id":1,"result":{}}"#,
        r#"{"jsonrpc":"2.0"}"#,
        r#"{"jsonrpc":"2.0","id":"x","result":{}}"#,
    ] {
        assert!(
            matches!(acp::parse(bad), Err(TurnError::Malformed(_))),
            "{bad}"
        );
    }
}

#[test]
fn initialize_offers_no_fs_or_terminal() {
    let caps = acp::initialize_params();
    assert_eq!(caps["clientCapabilities"]["terminal"], json!(false));
    assert_eq!(
        caps["clientCapabilities"]["fs"]["readTextFile"],
        json!(false)
    );
    assert_eq!(
        caps["clientCapabilities"]["fs"]["writeTextFile"],
        json!(false)
    );
    assert_eq!(
        acp::new_session_params(Path::new("/wt/7/1")),
        json!({"cwd": "/wt/7/1", "mcpServers": []})
    );
}

fn options(kinds: &[&str]) -> Value {
    let options: Vec<Value> = kinds
        .iter()
        .map(|k| json!({"optionId": format!("id-{k}"), "name": *k, "kind": *k}))
        .collect();
    json!({"toolCall": {"title": "Edit src/main.rs"}, "options": options})
}

#[test]
fn permission_prefers_allow_once_then_allow_always_else_cancels() {
    let all = options(&["reject_once", "allow_always", "allow_once"]);
    assert_eq!(
        acp::permission_choice(&all).as_deref(),
        Some("id-allow_once")
    );
    let always = options(&["reject_once", "allow_always"]);
    assert_eq!(
        acp::permission_choice(&always).as_deref(),
        Some("id-allow_always")
    );
    assert_eq!(acp::permission_choice(&options(&["reject_once"])), None);
    assert_eq!(
        acp::permission_result(None),
        json!({"outcome": {"outcome": "cancelled"}})
    );
    assert_eq!(
        acp::permission_result(Some("id-allow_once")),
        json!({"outcome": {"outcome": "selected", "optionId": "id-allow_once"}})
    );
}

/// A scripted agent on its end of a socketpair: answers each request in
/// order with what `script` gives for it, after sending `before` lines.
/// Returns every line orch sent.
fn fake_agent(
    end: UnixStream,
    script: impl Fn(&str, &Value, &mut dyn FnMut(&str)) -> Option<Value> + Send + 'static,
) -> JoinHandle<Vec<Value>> {
    std::thread::spawn(move || {
        let mut writer = end.try_clone().expect("clone");
        let mut seen = Vec::new();
        for line in BufReader::new(end).lines() {
            let Ok(line) = line else { break };
            let msg = Value::parse(&line).expect("orch sent json");
            seen.push(msg.clone());
            let (Some(id), Some(method)) =
                (msg.get("id"), msg.get("method").and_then(Value::as_str))
            else {
                continue;
            };
            let mut send = |l: &str| {
                writeln!(writer, "{l}").expect("agent write");
            };
            let Some(result) = script(method, &msg["params"], &mut send) else {
                break;
            };
            let reply = json!({"jsonrpc": "2.0", "id": id.clone(), "result": result});
            writeln!(writer, "{}", reply.to_json_string()).expect("agent reply");
        }
        seen
    })
}

fn update(u: Value) -> String {
    acp::notification("session/update", json!({"sessionId": "s1", "update": u}))
}

fn client(orch: &UnixStream) -> Client<BufReader<UnixStream>, UnixStream, Vec<u8>> {
    Client::new(
        BufReader::new(orch.try_clone().expect("clone")),
        orch.try_clone().expect("clone"),
        Vec::new(),
    )
}

#[test]
fn a_whole_turn_runs_against_a_scripted_agent() {
    let (orch, agent) = UnixStream::pair().expect("pair");
    let agent = fake_agent(agent, |method, _params, send| match method {
        "initialize" => Some(json!({"protocolVersion": 1, "agentCapabilities": {}})),
        "session/new" => Some(json!({"sessionId": "s1"})),
        "session/prompt" => {
            send(&update(json!({
                "sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": "Reading the intent"}
            })));
            send(&update(json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "t1",
                "title": "Edit src/main.rs",
                "status": "pending"
            })));
            send(
                &acp::request(
                    90,
                    "session/request_permission",
                    options(&["allow_once", "reject_once"]),
                )
                .replace("\"id\":90", "\"id\":\"perm-1\""),
            );
            send(&acp::request(
                91,
                "fs/read_text_file",
                json!({"path": "/etc/passwd"}),
            ));
            Some(json!({
                "stopReason": "end_turn",
                "usage": {"inputTokens": 1200, "outputTokens": 340}
            }))
        }
        _ => None,
    });
    let mut c = client(&orch);
    let turn = c
        .turn(Path::new("/wt/7/1"), "Build the intent.")
        .expect("turn");
    assert_eq!(
        turn,
        Turn {
            stop: StopReason("end_turn".into()),
            usage: Some(Usage {
                tokens_in: 1200,
                tokens_out: 340
            })
        }
    );
    assert!(turn.stop.is_end_turn());
    drop(c);
    orch.shutdown(std::net::Shutdown::Both).expect("shutdown");
    let seen = agent.join().expect("agent");
    let methods: Vec<&str> = seen
        .iter()
        .filter_map(|m| m.get("method").and_then(Value::as_str))
        .collect();
    assert_eq!(methods, ["initialize", "session/new", "session/prompt"]);
    assert_eq!(seen[2]["params"]["sessionId"], json!("s1"));
    assert_eq!(
        seen[2]["params"]["prompt"][0]["text"],
        json!("Build the intent.")
    );
    // The permission answer, then the refusal of the unoffered fs call,
    // were sent between the prompt and its answer.
    let answers: Vec<&Value> = seen.iter().filter(|m| m.get("method").is_none()).collect();
    assert_eq!(answers[0]["id"], json!("perm-1"));
    assert_eq!(
        answers[0]["result"]["outcome"]["optionId"],
        json!("id-allow_once")
    );
    assert_eq!(answers[1]["id"], json!(91));
    assert_eq!(answers[1]["error"]["code"], json!(-32601));
}

#[test]
fn the_log_renders_the_turn() {
    let (orch, agent) = UnixStream::pair().expect("pair");
    let agent = fake_agent(agent, |method, _params, send| match method {
        "initialize" => Some(json!({})),
        "session/new" => Some(json!({"sessionId": "s1"})),
        "session/prompt" => {
            send(&update(json!({
                "sessionUpdate": "plan",
                "entries": [
                    {"content": "Scan drives", "status": "completed"},
                    {"content": "Group duplicates", "status": "in_progress"},
                    {"content": "Quarantine", "status": "pending"}
                ]
            })));
            for chunk in ["I'll ", "start."] {
                send(&update(json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": chunk}
                })));
            }
            send(&update(json!({
                "sessionUpdate": "tool_call", "toolCallId": "t1", "title": "Run cargo test"
            })));
            send(&update(json!({
                "sessionUpdate": "tool_call_update", "toolCallId": "t1", "status": "completed"
            })));
            send(&update(
                json!({"sessionUpdate": "available_commands_update"}),
            ));
            send(&acp::request(
                5,
                "session/request_permission",
                options(&["reject_once"]),
            ));
            Some(json!({"stopReason": "max_tokens"}))
        }
        _ => None,
    });
    let mut c = client(&orch);
    let turn = c.turn(Path::new("/w"), "go").expect("turn");
    assert_eq!(turn.usage, None, "not reported is not zero");
    assert_eq!(turn.tokens(), (0, 0));
    let log = String::from_utf8(log_of(c)).expect("utf8");
    assert_eq!(
        log,
        "[plan] [x] Scan drives\n\
         [plan] [~] Group duplicates\n\
         [plan] [ ] Quarantine\n\
         I'll start.\n\
         [tool] Run cargo test (pending)\n\
         [tool] t1 completed\n\
         [permission] Edit src/main.rs: cancelled (no allow option)\n\
         [usage] not reported\n\
         [stop] max_tokens\n"
    );
    orch.shutdown(std::net::Shutdown::Both).expect("shutdown");
    agent.join().expect("agent");
}

fn log_of(c: Client<BufReader<UnixStream>, UnixStream, Vec<u8>>) -> Vec<u8> {
    c.into_log()
}

#[test]
fn a_closed_socket_fails_the_turn_instead_of_hanging() {
    let (orch, agent) = UnixStream::pair().expect("pair");
    // The agent answers initialize, then dies mid-turn.
    let agent = fake_agent(agent, |method, _, _| match method {
        "initialize" => Some(json!({})),
        _ => None,
    });
    let mut c = client(&orch);
    let err = c.turn(Path::new("/w"), "go").expect_err("closed");
    assert_eq!(err, TurnError::Closed);
    agent.join().expect("agent");
}

#[test]
fn a_shutdown_from_another_thread_ends_a_waiting_turn() {
    let (orch, agent) = UnixStream::pair().expect("pair");
    // The agent never answers the prompt: the wall clock must end it.
    let agent = fake_agent(agent, |method, _, _| match method {
        "initialize" => Some(json!({})),
        "session/new" => Some(json!({"sessionId": "s1"})),
        _ => {
            std::thread::sleep(std::time::Duration::from_secs(30));
            None
        }
    });
    let killer = orch.try_clone().expect("clone");
    let wall = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(200));
        killer.shutdown(std::net::Shutdown::Both).expect("shutdown");
    });
    let mut c = client(&orch);
    assert_eq!(c.turn(Path::new("/w"), "go"), Err(TurnError::Closed));
    wall.join().expect("wall");
    drop(agent);
}

#[test]
fn a_malformed_line_or_an_error_answer_fails_the_turn() {
    let (orch, mut agent) = UnixStream::pair().expect("pair");
    writeln!(agent, "this is not json").expect("write");
    let mut c = client(&orch);
    assert!(matches!(
        c.turn(Path::new("/w"), "go"),
        Err(TurnError::Malformed(_))
    ));

    let (orch, agent) = UnixStream::pair().expect("pair");
    let agent = std::thread::spawn(move || {
        let mut lines = BufReader::new(agent.try_clone().expect("clone")).lines();
        let mut w = agent;
        let _init = lines.next();
        writeln!(
            w,
            r#"{{"jsonrpc":"2.0","id":1,"error":{{"code":-32000,"message":"auth required"}}}}"#
        )
        .expect("write");
    });
    let mut c = client(&orch);
    assert_eq!(
        c.turn(Path::new("/w"), "go"),
        Err(TurnError::Protocol(
            "initialize: -32000 auth required".into()
        ))
    );
    agent.join().expect("agent");
}

#[test]
fn the_renderer_breaks_agent_text_before_other_events() {
    let mut r = Render::default();
    let chunk = |t: &str| json!({"sessionUpdate": "agent_message_chunk", "content": {"text": t}});
    assert_eq!(r.update(&chunk("half")), "half");
    assert_eq!(
        r.update(&json!({"sessionUpdate": "tool_call", "title": "ls", "status": "in_progress"})),
        "\n[tool] ls (in_progress)\n"
    );
    assert_eq!(r.update(&chunk("whole line\n")), "whole line\n");
    assert_eq!(
        r.end(&Turn {
            stop: StopReason("end_turn".into()),
            usage: Some(Usage {
                tokens_in: 1,
                tokens_out: 2
            })
        }),
        "[usage] in 1 out 2\n[stop] end_turn\n"
    );
    // An update with no status says nothing.
    assert_eq!(
        r.update(&json!({"sessionUpdate": "tool_call_update", "toolCallId": "t"})),
        ""
    );
}

#[test]
fn a_line_past_the_cap_fails_without_growing_further() {
    let mut long = std::io::Cursor::new(vec![b'x'; 64]);
    assert!(matches!(
        acp::read_line(&mut long, 16),
        Err(TurnError::Malformed(e)) if e.contains("16 bytes")
    ));
    let mut ok = std::io::Cursor::new(b"0123456789abcdef\nnext\n".to_vec());
    assert_eq!(
        acp::read_line(&mut ok, 16).unwrap(),
        Some(b"0123456789abcdef".to_vec())
    );
    assert_eq!(acp::read_line(&mut ok, 16).unwrap(), Some(b"next".to_vec()));
    assert_eq!(acp::read_line(&mut ok, 16).unwrap(), None);
    // A last line with no newline still counts.
    let mut tail = std::io::Cursor::new(b"end".to_vec());
    assert_eq!(
        acp::read_line(&mut tail, 16).unwrap(),
        Some(b"end".to_vec())
    );
}

#[test]
fn agent_text_cannot_drive_the_terminal_or_forge_a_line() {
    let mut r = Render::default();
    let chunk = update(json!({"sessionUpdate": "agent_message_chunk",
        "content": {"type": "text", "text": "a\u{1b}[2Jb\nc\u{9b}d"}}));
    let text = r.update(&Value::parse(&chunk).unwrap()["params"]["update"]);
    assert!(
        !text.contains('\u{1b}') && !text.contains('\u{9b}'),
        "{text:?}"
    );
    assert!(
        text.contains("\\u{1b}[2J") && text.contains("b\nc"),
        "{text:?}"
    );
    let tool = r.update(&json!({"sessionUpdate": "tool_call",
        "title": "ls\n[stop] end_turn", "status": "pending"}));
    assert_eq!(
        tool.matches('\n').count(),
        2,
        "one line after the break: {tool:?}"
    );
    assert!(tool.contains("ls\\n[stop] end_turn"));
}

#[test]
fn an_empty_chunk_keeps_the_break_before_the_next_event() {
    let mut r = Render::default();
    let chunk = |t: &str| {
        json!({"sessionUpdate": "agent_message_chunk",
        "content": {"type": "text", "text": t}})
    };
    assert_eq!(r.update(&chunk("abc")), "abc");
    assert_eq!(r.update(&chunk("")), "");
    let tool = r.update(&json!({"sessionUpdate": "tool_call", "title": "t"}));
    assert_eq!(tool, "\n[tool] t (pending)\n");
}
