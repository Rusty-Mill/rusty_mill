#![allow(clippy::unwrap_used)]

//! The server end to end over the stdio loop, on in-memory pipes.

use rusty_json::Value;
use rusty_mcp_proto::{CallToolResult, ContentBlock, ErrorCode, ErrorData, ProtocolVersion, Tool};
mod support;

use rusty_mcp_server::{serve_lines, BuildError, Server, StdioConfig};
use std::io::{self, BufReader, Cursor, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::{Duration, Instant};
use support::{code, reply, result, run_with, Feed, Out};

fn schema() -> Value {
    let mut s = Value::object();
    s.insert("type", "object");
    s
}

fn text(s: &str) -> CallToolResult {
    CallToolResult {
        content: vec![ContentBlock::text(s)],
        ..CallToolResult::default()
    }
}

#[derive(Default, Clone)]
struct Flags {
    saw_cancel: Arc<AtomicBool>,
}

fn server(flags: &Flags) -> Arc<Server> {
    let saw_cancel = flags.saw_cancel.clone();
    Arc::new(
        Server::builder("fixture", "1.2.3")
            .instructions("be kind")
            .tool(Tool::new("add", schema()), |_ctx, call| {
                let args = call.arguments.as_ref();
                let n = |k: &str| args.and_then(|a| a.get(k)).and_then(Value::as_i64);
                match (n("a"), n("b")) {
                    (Some(a), Some(b)) => Ok(text(&(a + b).to_string())),
                    _ => Err(ErrorData::new(
                        ErrorCode::INVALID_PARAMS,
                        "a and b are required",
                    )),
                }
            })
            .tool(Tool::new("fail", schema()), |_ctx, _call| {
                Ok(CallToolResult {
                    is_error: Some(true),
                    ..text("it broke")
                })
            })
            .tool(Tool::new("boom", schema()), |_ctx, _call| {
                panic!("tool panicked")
            })
            .tool(Tool::new("slow", schema()), |_ctx, _call| {
                std::thread::sleep(Duration::from_millis(300));
                Ok(text("done"))
            })
            .tool(Tool::new("progress", schema()), |ctx, _call| {
                for i in 1..=3 {
                    ctx.progress(f64::from(i), Some(3.0), Some("working"));
                }
                Ok(text("finished"))
            })
            .tool(Tool::new("wait", schema()), move |ctx, _call| {
                let deadline = Instant::now() + Duration::from_secs(10);
                while !ctx.is_cancelled() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(5));
                }
                saw_cancel.store(ctx.is_cancelled(), Ordering::SeqCst);
                Ok(text("stopped"))
            })
            .build()
            .unwrap(),
    )
}

fn run(input: &str) -> Vec<Value> {
    run_with(server(&Flags::default()), input)
}

fn first_text(m: &Value) -> &str {
    result(m)
        .get("content")
        .unwrap()
        .get_index(0)
        .unwrap()
        .get("text")
        .unwrap()
        .as_str()
        .unwrap()
}

const INIT: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"c","version":"1"}}}"#;

fn call(id: i64, name: &str, args: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"{name}","arguments":{args}}}}}"#
    )
}

#[test]
fn a_classic_session_initializes_lists_and_calls() {
    let input = [
        INIT.to_string(),
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_string(),
        r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#.to_string(),
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#.to_string(),
        call(4, "add", r#"{"a":2,"b":3}"#),
        call(5, "fail", "{}"),
    ]
    .join("\n");
    let out = run(&input);
    assert_eq!(out.len(), 5, "the notification gets no reply: {out:?}");

    let init = result(reply(&out, 1));
    assert_eq!(
        init.get("protocolVersion").and_then(Value::as_str),
        Some("2025-06-18")
    );
    assert_eq!(
        init.get("serverInfo")
            .unwrap()
            .get("name")
            .and_then(Value::as_str),
        Some("fixture")
    );
    assert_eq!(
        init.get("instructions").and_then(Value::as_str),
        Some("be kind")
    );
    assert!(init.get("capabilities").unwrap().get("tools").is_some());

    assert_eq!(result(reply(&out, 2)), &Value::object());
    let tools = result(reply(&out, 3))
        .get("tools")
        .unwrap()
        .as_array()
        .unwrap()
        .len();
    assert_eq!(tools, 6);
    // Classic revisions carry no cache hints.
    assert!(result(reply(&out, 3)).get("ttlMs").is_none());

    assert_eq!(first_text(reply(&out, 4)), "5");
    let failed = result(reply(&out, 5));
    assert_eq!(
        failed.get("isError").and_then(Value::as_bool),
        Some(true),
        "a tool failure is a result"
    );
}

#[test]
fn an_unknown_version_gets_the_newest_classic_and_a_served_one_gets_itself() {
    let init = INIT.replace("2025-06-18", "2099-01-01");
    let out = run(&init);
    // The newest *classic* revision: 2026-07-28 has no initialize.
    assert_eq!(
        result(reply(&out, 1))
            .get("protocolVersion")
            .and_then(Value::as_str),
        Some("2025-11-25")
    );
    // A client that names a revision the server speaks gets it, 2026-07-28
    // included (the reference client opens with `initialize` when pinned to it).
    let asks_stateless = INIT.replace("2025-06-18", "2026-07-28");
    let out = run(&asks_stateless);
    assert_eq!(
        result(reply(&out, 1))
            .get("protocolVersion")
            .and_then(Value::as_str),
        Some("2026-07-28")
    );
}

#[test]
fn a_stateless_client_needs_no_handshake_and_gets_cache_hints() {
    let meta = r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientInfo":{"name":"c","version":"1"},"io.modelcontextprotocol/clientCapabilities":{}}"#;
    let input = [
        r#"{"jsonrpc":"2.0","id":1,"method":"server/discover"}"#.to_string(),
        format!(r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{{{meta}}}}}"#),
        format!(r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"add","arguments":{{"a":1,"b":1}},{meta}}}}}"#),
    ]
    .join("\n");
    let out = run(&input);

    let discover = result(reply(&out, 1));
    let versions: Vec<&str> = discover
        .get("supportedVersions")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(versions.contains(&"2026-07-28") && versions.contains(&"2025-06-18"));
    assert_eq!(
        discover.get("resultType").and_then(Value::as_str),
        Some("complete")
    );
    let info = discover
        .get("_meta")
        .unwrap()
        .get("io.modelcontextprotocol/serverInfo")
        .unwrap();
    assert_eq!(info.get("name").and_then(Value::as_str), Some("fixture"));

    let list = result(reply(&out, 2));
    assert_eq!(list.get("ttlMs").and_then(Value::as_i64), Some(0));
    assert_eq!(
        list.get("cacheScope").and_then(Value::as_str),
        Some("public")
    );

    let called = result(reply(&out, 3));
    assert_eq!(first_text(reply(&out, 3)), "2");
    assert_eq!(
        called.get("resultType").and_then(Value::as_str),
        Some("complete"),
        "filled in for 2026-07-28"
    );
}

#[test]
fn protocol_errors_carry_the_right_codes() {
    let stateless =
        |v: &str| format!(r#""_meta":{{"io.modelcontextprotocol/protocolVersion":"{v}"}}"#);
    let input = [
        // 1: tools/list before any handshake or version.
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#.to_string(),
        // 2: unknown method.
        r#"{"jsonrpc":"2.0","id":2,"method":"nope"}"#.to_string(),
        // 3: a revision the server does not speak.
        format!(r#"{{"jsonrpc":"2.0","id":3,"method":"tools/list","params":{{{}}}}}"#, stateless("2031-01-01")),
        // 4, 5, 6: after naming a revision: unknown tool, bad params, missing params.
        format!(r#"{{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{{"name":"zzz",{}}}}}"#, stateless("2026-07-28")),
        format!(r#"{{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{{"name":"add","arguments":{{}},{}}}}}"#, stateless("2026-07-28")),
        format!(r#"{{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{{{}}}}}"#, stateless("2026-07-28")),
        // 7: a mistyped known _meta key.
        r#"{"jsonrpc":"2.0","id":7,"method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":5}}}"#.to_string(),
        // 8: arguments that are not an object.
        format!(r#"{{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{{"name":"add","arguments":[1],{}}}}}"#, stateless("2026-07-28")),
    ]
    .join("\n");
    let out = run(&input);
    assert_eq!(code(reply(&out, 1)), ErrorCode::INVALID_REQUEST.0);
    assert_eq!(code(reply(&out, 2)), ErrorCode::METHOD_NOT_FOUND.0);
    assert_eq!(
        code(reply(&out, 3)),
        ErrorCode::UNSUPPORTED_PROTOCOL_VERSION.0
    );
    let data = reply(&out, 3).get("error").unwrap().get("data").unwrap();
    assert_eq!(
        data.get("requested").and_then(Value::as_str),
        Some("2031-01-01")
    );
    assert!(data.get("supported").unwrap().as_array().unwrap().len() >= 2);
    for id in [4, 5, 6, 7, 8] {
        assert_eq!(
            code(reply(&out, id)),
            ErrorCode::INVALID_PARAMS.0,
            "id {id}"
        );
    }
}

#[test]
fn bad_input_lines_are_answered_and_the_session_continues() {
    let input = format!(
        "{{not json\n42\n[]\n\n   \n{{\"jsonrpc\":\"1.0\",\"id\":9,\"method\":\"ping\"}}\n{INIT}\n",
    );
    let out = run(&input);
    // Three null-id errors plus the wrong-version message, then the handshake works.
    let nulls: Vec<i32> = out
        .iter()
        .filter(|m| m.get("id") == Some(&Value::Null))
        .map(code)
        .collect();
    assert_eq!(nulls[0], ErrorCode::PARSE_ERROR.0);
    assert!(
        nulls[1..]
            .iter()
            .all(|c| *c == ErrorCode::INVALID_REQUEST.0),
        "{nulls:?}"
    );
    assert_eq!(nulls.len(), 4, "{out:?}");
    assert!(result(reply(&out, 1)).get("serverInfo").is_some());
}

#[test]
fn an_oversized_line_is_refused_and_skipped() {
    let long = format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"ping","params":{{"pad":"{}"}}}}"#,
        "x".repeat(300)
    );
    let input = format!("{long}\n{INIT}\n");
    let out = Out::default();
    serve_lines(
        server(&Flags::default()),
        Cursor::new(input.into_bytes()),
        out.clone(),
        StdioConfig {
            max_line_bytes: 200,
            ..StdioConfig::default()
        },
    )
    .unwrap();
    let out = out.messages();
    assert_eq!(
        code(
            out.iter()
                .find(|m| m.get("id") == Some(&Value::Null))
                .unwrap()
        ),
        ErrorCode::INVALID_REQUEST.0
    );
    assert!(
        out.iter()
            .all(|m| m.get("id").and_then(Value::as_i64) != Some(2)),
        "the long request was not served"
    );
    assert!(
        result(reply(&out, 1)).get("serverInfo").is_some(),
        "the next line still works"
    );
}

#[test]
fn tools_list_pages_with_opaque_cursors() {
    let mut b = Server::builder("p", "1").page_size(2);
    for name in ["a", "b", "c", "d", "e"] {
        b = b.tool(Tool::new(name, schema()), |_c, _p| Ok(text("x")));
    }
    let srv = Arc::new(b.build().unwrap());
    let meta = r#""io.modelcontextprotocol/protocolVersion":"2026-07-28""#;
    let list = |id: i64, cursor: Option<&str>| {
        let c = cursor
            .map(|c| format!(r#""cursor":"{c}","#))
            .unwrap_or_default();
        format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/list","params":{{{c}"_meta":{{{meta}}}}}}}"#
        )
    };
    let mut names = Vec::new();
    let mut cursor: Option<String> = None;
    for id in 1..=10 {
        let out = run_with(srv.clone(), &list(id, cursor.as_deref()));
        let page = result(reply(&out, id));
        names.extend(
            page.get("tools")
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t.get("name").unwrap().as_str().unwrap().to_owned()),
        );
        cursor = page
            .get("nextCursor")
            .and_then(Value::as_str)
            .map(String::from);
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(names, ["a", "b", "c", "d", "e"]);
    for bad in ["garbage", "MS50b29scy45OQ"] {
        let out = run_with(srv.clone(), &list(1, Some(bad)));
        assert_eq!(code(reply(&out, 1)), ErrorCode::INVALID_PARAMS.0, "{bad}");
    }
}

#[test]
fn progress_is_reported_only_when_asked_for() {
    let with = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"progress","_meta":{"progressToken":"tok","io.modelcontextprotocol/protocolVersion":"2026-07-28"}}}"#;
    let without = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"progress","_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}}}"#;
    let out = run(&format!("{with}\n"));
    let notes: Vec<&Value> = out
        .iter()
        .filter(|m| m.get("method").and_then(Value::as_str) == Some("notifications/progress"))
        .collect();
    assert_eq!(notes.len(), 3);
    let params = notes[2].get("params").unwrap();
    assert_eq!(
        params.get("progressToken").and_then(Value::as_str),
        Some("tok")
    );
    assert_eq!(params.get("progress").and_then(Value::as_f64), Some(3.0));
    assert_eq!(params.get("total").and_then(Value::as_f64), Some(3.0));
    // The reply comes after the progress it describes.
    let last = out.last().unwrap();
    assert_eq!(last.get("id").and_then(Value::as_i64), Some(2));

    let out = run(&format!("{without}\n"));
    assert_eq!(out.len(), 1, "no token, no progress: {out:?}");
}

#[test]
fn a_cancelled_request_stops_and_gets_no_answer() {
    let flags = Flags::default();
    let (tx, rx) = channel();
    let out = Out::default();
    let srv = server(&flags);
    let sink = out.clone();
    let serving = std::thread::spawn(move || {
        serve_lines(
            srv,
            BufReader::new(Feed(rx, Vec::new())),
            sink,
            StdioConfig::default(),
        )
        .unwrap();
    });
    let meta = r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}"#;
    tx.send(format!(r#"{{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{{"name":"wait",{meta}}}}}{}"#, "\n").into_bytes()).unwrap();
    std::thread::sleep(Duration::from_millis(150));
    tx.send(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{\"requestId\":7,\"reason\":\"user\"}}\n".to_vec()).unwrap();
    // A cancel for an id nobody is running is ignored.
    tx.send(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{\"requestId\":99}}\n".to_vec()).unwrap();
    tx.send(format!(r#"{{"jsonrpc":"2.0","id":8,"method":"ping"}}{}"#, "\n").into_bytes())
        .unwrap();
    drop(tx);
    serving.join().unwrap();
    assert!(
        flags.saw_cancel.load(Ordering::SeqCst),
        "the tool saw the cancellation"
    );
    let out = out.messages();
    assert!(
        out.iter()
            .all(|m| m.get("id").and_then(Value::as_i64) != Some(7)),
        "no answer to a cancelled request: {out:?}"
    );
    assert!(
        out.iter()
            .any(|m| m.get("id").and_then(Value::as_i64) == Some(8)),
        "the connection carried on"
    );
}

#[test]
fn a_panicking_tool_is_an_internal_error_and_the_server_survives() {
    let meta = r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}"#;
    let input = format!(
        "{}\n{}\n",
        format_args!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"boom",{meta}}}}}"#
        ),
        format_args!(
            r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"add","arguments":{{"a":1,"b":2}},{meta}}}}}"#
        ),
    );
    let out = run(&input);
    assert_eq!(code(reply(&out, 1)), ErrorCode::INTERNAL_ERROR.0);
    assert_eq!(first_text(reply(&out, 2)), "3");
}

#[test]
fn end_of_input_waits_for_running_requests() {
    let meta = r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}"#;
    let out = run(&format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"slow",{meta}}}}}"#
    ));
    assert_eq!(
        first_text(reply(&out, 1)),
        "done",
        "the answer is written before serve_lines returns"
    );
}

#[test]
fn requests_are_limited_and_ids_cannot_be_reused_while_running() {
    let meta = r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}"#;
    let slow = |id: i64| {
        format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"slow",{meta}}}}}"#
        )
    };
    let srv = Arc::new(
        Server::builder("l", "1")
            .max_in_flight(2)
            .tool(Tool::new("slow", schema()), |_c, _p| {
                std::thread::sleep(Duration::from_millis(400));
                Ok(text("done"))
            })
            .build()
            .unwrap(),
    );
    let out = run_with(srv, &[slow(1), slow(2), slow(3), slow(1)].join("\n"));
    assert_eq!(first_text(reply(&out, 1)), "done");
    assert_eq!(first_text(reply(&out, 2)), "done");
    assert_eq!(
        code(reply(&out, 3)),
        ErrorCode::INTERNAL_ERROR.0,
        "over the limit"
    );
    let nulls = out
        .iter()
        .filter(|m| m.get("id") == Some(&Value::Null))
        .count();
    assert_eq!(
        nulls, 1,
        "the reused id is refused without an id of its own: {out:?}"
    );
}

#[test]
fn a_server_without_tools_has_no_tool_methods_or_capability() {
    let srv = Arc::new(Server::builder("bare", "1").build().unwrap());
    let input = format!(
        "{INIT}\n{}\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#
    );
    let out = run_with(srv, &input);
    assert!(result(reply(&out, 1))
        .get("capabilities")
        .unwrap()
        .get("tools")
        .is_none());
    assert_eq!(code(reply(&out, 2)), ErrorCode::METHOD_NOT_FOUND.0);
}

#[test]
fn a_stateless_only_server_refuses_initialize_but_discovers() {
    let srv = Arc::new(
        Server::builder("s", "1")
            .versions(vec![ProtocolVersion::new(ProtocolVersion::V_2026_07_28)])
            .build()
            .unwrap(),
    );
    let input = format!(
        "{INIT}\n{}\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"server/discover"}"#
    );
    let out = run_with(srv, &input);
    assert_eq!(
        code(reply(&out, 1)),
        ErrorCode::UNSUPPORTED_PROTOCOL_VERSION.0
    );
    assert!(result(reply(&out, 2)).get("supportedVersions").is_some());
}

#[test]
fn the_builder_refuses_nonsense() {
    let dup = Server::builder("d", "1")
        .tool(Tool::new("t", schema()), |_c, _p| Ok(text("a")))
        .tool(Tool::new("t", schema()), |_c, _p| Ok(text("b")))
        .build();
    assert!(matches!(dup, Err(BuildError::DuplicateTool(n)) if n == "t"));
    assert_eq!(
        Server::builder("d", "1").page_size(0).build().err(),
        Some(BuildError::ZeroLimit("page size"))
    );
    assert_eq!(
        Server::builder("d", "1").max_in_flight(0).build().err(),
        Some(BuildError::ZeroLimit("in-flight limit"))
    );
    assert_eq!(
        Server::builder("d", "1").versions(Vec::new()).build().err(),
        Some(BuildError::NoVersions)
    );
}

#[test]
fn a_dead_output_ends_the_session_with_an_error() {
    struct Dead;
    impl Write for Dead {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let input = format!(
        "{INIT}\n{}\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#
    );
    let err = serve_lines(
        server(&Flags::default()),
        Cursor::new(input.into_bytes()),
        Dead,
        StdioConfig::default(),
    )
    .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
}

/// A tool that ignores cancellation must not keep the server from exiting.
#[test]
fn end_of_input_cancels_cooperative_tools_and_abandons_stuck_ones() {
    let flags = Flags::default();
    let stuck = Arc::new(AtomicBool::new(false));
    let stuck_flag = stuck.clone();
    let saw_cancel = flags.saw_cancel.clone();
    let srv = Arc::new(
        Server::builder("d", "1")
            .tool(Tool::new("wait", schema()), move |ctx, _p| {
                while !ctx.is_cancelled() {
                    std::thread::sleep(Duration::from_millis(5));
                }
                saw_cancel.store(true, Ordering::SeqCst);
                Ok(text("stopped"))
            })
            .tool(Tool::new("stuck", schema()), move |_c, _p| {
                // Ignores the cancellation flag entirely.
                std::thread::sleep(Duration::from_secs(4));
                stuck_flag.store(true, Ordering::SeqCst);
                Ok(text("late"))
            })
            .build()
            .unwrap(),
    );
    let meta = r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}"#;
    let input = [
        format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"wait",{meta}}}}}"#
        ),
        format!(
            r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"stuck",{meta}}}}}"#
        ),
    ]
    .join("\n");
    let out = Out::default();
    let started = Instant::now();
    serve_lines(
        srv,
        Cursor::new(input.into_bytes()),
        out.clone(),
        StdioConfig {
            drain_timeout: Duration::from_millis(300),
            ..StdioConfig::default()
        },
    )
    .unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "returned after {:?}, not after the stuck tool",
        started.elapsed()
    );
    assert!(
        flags.saw_cancel.load(Ordering::SeqCst),
        "the cooperative tool was told to stop"
    );
    assert!(
        !stuck.load(Ordering::SeqCst),
        "the stuck tool was abandoned, not awaited"
    );
    assert!(
        out.messages().is_empty(),
        "cancelled requests get no answer"
    );
}

#[test]
fn a_finished_request_frees_its_id_for_reuse() {
    let meta = r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}"#;
    let ping = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"add","arguments":{{"a":1,"b":1}},{meta}}}}}"#
    );
    // The second use of id 1 arrives after the first answer: sequential, same connection.
    let (tx, rx) = channel();
    let out = Out::default();
    let sink = out.clone();
    let srv = server(&Flags::default());
    let serving = std::thread::spawn(move || {
        serve_lines(
            srv,
            BufReader::new(Feed(rx, Vec::new())),
            sink,
            StdioConfig::default(),
        )
        .unwrap();
    });
    tx.send(format!("{ping}\n").into_bytes()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while out.messages().is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    tx.send(format!("{ping}\n").into_bytes()).unwrap();
    drop(tx);
    serving.join().unwrap();
    let out = out.messages();
    assert_eq!(out.len(), 2, "{out:?}");
    assert!(
        out.iter().all(|m| m.get("result").is_some()),
        "no duplicate-id error: {out:?}"
    );
}

#[test]
fn a_classic_only_server_has_no_discover() {
    let server = Arc::new(
        Server::builder("classic", "1")
            .versions(vec![ProtocolVersion::new("2025-06-18")])
            .tool(Tool::new("t", schema()), |_c, _p| {
                Ok(CallToolResult::default())
            })
            .build()
            .unwrap(),
    );
    let out = run_with(
        server,
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"server/discover\"}\n",
    );
    assert_eq!(code(reply(&out, 1)), -32601);
}
