#![allow(clippy::unwrap_used)]
//! Tasks (`io.modelcontextprotocol/tasks`): a call answers with a task, the
//! client polls, answers its questions and may cancel. Driven through
//! `Connection` directly so each step is deterministic.

use rusty_json::Value;
use rusty_mcp_proto::{
    CallToolResult, ContentBlock, ElicitAction, ElicitParams, ErrorCode, ErrorData, Message, Tool,
    Wire,
};
use rusty_mcp_server::{BuildError, Connection, Notifier, Server};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const V: &str = r#""io.modelcontextprotocol/protocolVersion":"2026-07-28""#;
const WITH_TASKS: &str = r#""io.modelcontextprotocol/clientCapabilities":{"extensions":{"io.modelcontextprotocol/tasks":{}}}"#;
const WITHOUT: &str = r#""io.modelcontextprotocol/clientCapabilities":{}"#;

struct Mute;
impl Notifier for Mute {
    fn notify(&self, _: Message) {}
}

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

struct Fixture {
    server: Arc<Server>,
    saw_cancel: Arc<AtomicBool>,
}

fn builder(saw: &Arc<AtomicBool>) -> rusty_mcp_server::ServerBuilder {
    let saw = Arc::clone(saw);
    Server::builder("tasks", "1")
        .task_tool(Tool::new("quick", schema()), |ctx, _c| {
            ctx.set_message("almost");
            Ok(text("done"))
        })
        .task_tool(Tool::new("fails", schema()), |_c, _p| {
            Err(ErrorData::new(ErrorCode::INTERNAL_ERROR, "boom"))
        })
        .task_tool(Tool::new("panics", schema()), |_c, _p| panic!("tool bug"))
        .task_tool(Tool::new("wait", schema()), move |ctx, _p| {
            let until = Instant::now() + Duration::from_secs(10);
            while !ctx.is_cancelled() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(5));
            }
            saw.store(ctx.is_cancelled(), Ordering::SeqCst);
            Ok(text("too late"))
        })
        .task_tool(Tool::new("ask", schema()), |ctx, _p| {
            let form = ElicitParams::Form {
                message: "Name?".to_owned(),
                requested_schema: schema(),
                meta: None,
            };
            Ok(match ctx.elicit("name", &form) {
                Ok(r) if r.action == ElicitAction::Accept => text(&format!(
                    "hello {}",
                    r.content
                        .as_ref()
                        .and_then(|c| c.get("n"))
                        .and_then(Value::as_str)
                        .unwrap_or("?")
                )),
                Ok(_) => text("declined"),
                Err(_) => text("cancelled while asking"),
            })
        })
        .tool(Tool::new("plain", schema()), |_c, _p| Ok(text("plain")))
}

fn fixture() -> Fixture {
    let saw_cancel = Arc::new(AtomicBool::new(false));
    let server = Arc::new(builder(&saw_cancel).build().unwrap());
    Fixture { server, saw_cancel }
}

fn conn(server: &Arc<Server>) -> Arc<Connection> {
    Arc::new(Connection::new(Arc::clone(server), Arc::new(Mute)))
}

fn send(conn: &Arc<Connection>, method: &str, params: &str) -> Value {
    let body = format!(r#"{{"jsonrpc":"2.0","id":1,"method":"{method}","params":{params}}}"#);
    let reply = conn
        .handle(Message::from_json(&body).unwrap())
        .expect("a reply");
    Value::from_json_str(&reply.to_json()).unwrap()
}

fn call(conn: &Arc<Connection>, tool: &str, caps: &str) -> Value {
    send(
        conn,
        "tools/call",
        &format!(r#"{{"name":"{tool}","_meta":{{{V},{caps}}}}}"#),
    )
}

fn task_method(conn: &Arc<Connection>, method: &str, id: &str, extra: &str) -> Value {
    send(
        conn,
        method,
        &format!(r#"{{"taskId":"{id}"{extra},"_meta":{{{V},{WITHOUT}}}}}"#),
    )
}

fn started(conn: &Arc<Connection>, tool: &str) -> String {
    let r = call(conn, tool, WITH_TASKS);
    r["result"]["taskId"].as_str().unwrap().to_owned()
}

fn get(conn: &Arc<Connection>, id: &str) -> Value {
    task_method(conn, "tasks/get", id, "")["result"].clone()
}

fn wait_for(conn: &Arc<Connection>, id: &str, status: &str) -> Value {
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        let t = get(conn, id);
        if t["status"].as_str() == Some(status) {
            return t;
        }
        assert!(Instant::now() < until, "task never became {status}: {t:?}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn code(v: &Value) -> i64 {
    v["error"]["code"]
        .as_i64()
        .unwrap_or_else(|| panic!("{v:?}"))
}

#[test]
fn a_call_is_answered_with_a_task_and_the_task_completes() {
    let f = fixture();
    let c = conn(&f.server);
    let r = call(&c, "quick", WITH_TASKS);
    let t = &r["result"];
    assert_eq!(t["resultType"], "task");
    let id = t["taskId"].as_str().unwrap();
    assert_eq!(id.len(), 32);
    assert_eq!(t["ttlMs"].as_i64(), Some(3_600_000));
    assert!(t["pollIntervalMs"].as_i64().is_some());
    assert!(t["createdAt"].as_str().unwrap().ends_with('Z'));

    let done = wait_for(&c, id, "completed");
    assert_eq!(done["resultType"], "complete");
    assert_eq!(done["result"]["content"][0]["text"], "done");
    assert_eq!(done["statusMessage"], "almost");
}

#[test]
fn a_failing_or_panicking_tool_fails_its_task() {
    let f = fixture();
    let c = conn(&f.server);
    let id = started(&c, "fails");
    let failed = wait_for(&c, &id, "failed");
    assert_eq!(failed["error"]["code"], -32603);
    assert_eq!(failed["error"]["message"], "boom");
    let id = started(&c, "panics");
    let failed = wait_for(&c, &id, "failed");
    assert_eq!(failed["error"]["message"], "internal error");
}

#[test]
fn only_a_client_that_declares_the_extension_may_start_a_task() {
    let f = fixture();
    let c = conn(&f.server);
    assert_eq!(code(&call(&c, "quick", WITHOUT)), -32021);
    // A tool that is not a task tool needs no extension.
    assert!(call(&c, "plain", WITHOUT)["result"].is_object());
    // And classic revisions have no tasks at all.
    let classic = send(&c, "tools/call", r#"{"name":"quick"}"#);
    assert_eq!(code(&classic), -32600);
}

#[test]
fn a_task_can_be_cancelled_and_its_tool_sees_it() {
    let f = fixture();
    let c = conn(&f.server);
    let id = started(&c, "wait");
    assert_eq!(get(&c, &id)["status"], "working");
    let ack = task_method(&c, "tasks/cancel", &id, "");
    assert_eq!(ack["result"]["resultType"], "complete");
    assert_eq!(get(&c, &id)["status"], "cancelled");
    let until = Instant::now() + Duration::from_secs(5);
    while !f.saw_cancel.load(Ordering::SeqCst) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(f.saw_cancel.load(Ordering::SeqCst), "the tool never saw it");
    // Its late result does not bring it back, and it cannot be cancelled twice.
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(get(&c, &id)["status"], "cancelled");
    assert_eq!(code(&task_method(&c, "tasks/cancel", &id, "")), -32602);
}

#[test]
fn a_task_can_ask_the_user_and_continue_on_the_answer() {
    let f = fixture();
    let c = conn(&f.server);
    let id = started(&c, "ask");
    let waiting = wait_for(&c, &id, "input_required");
    assert_eq!(
        waiting["inputRequests"]["name"]["method"],
        "elicitation/create"
    );

    // A malformed answer is refused and the task keeps waiting.
    let bad = task_method(
        &c,
        "tasks/update",
        &id,
        r#","inputResponses":{"name":{"action":"maybe"}}"#,
    );
    assert_eq!(code(&bad), -32602);
    // So is one that answers nothing it asked.
    let other = task_method(&c, "tasks/update", &id, r#","inputResponses":{"zzz":{}}"#);
    assert_eq!(code(&other), -32602);
    assert_eq!(get(&c, &id)["status"], "input_required");

    let ok = task_method(
        &c,
        "tasks/update",
        &id,
        r#","inputResponses":{"name":{"action":"accept","content":{"n":"Ann"}}}"#,
    );
    assert_eq!(ok["result"]["resultType"], "complete");
    let done = wait_for(&c, &id, "completed");
    assert_eq!(done["result"]["content"][0]["text"], "hello Ann");

    // Nothing is pending now.
    let late = task_method(
        &c,
        "tasks/update",
        &id,
        r#","inputResponses":{"name":{"action":"accept"}}"#,
    );
    assert_eq!(code(&late), -32602);
}

#[test]
fn cancelling_a_task_that_waits_wakes_its_tool() {
    let f = fixture();
    let c = conn(&f.server);
    let id = started(&c, "ask");
    wait_for(&c, &id, "input_required");
    task_method(&c, "tasks/cancel", &id, "");
    assert_eq!(get(&c, &id)["status"], "cancelled");
}

#[test]
fn any_connection_can_poll_a_task_but_only_with_its_id() {
    let f = fixture();
    let (a, b) = (conn(&f.server), conn(&f.server));
    let id = started(&a, "quick");
    assert_eq!(wait_for(&b, &id, "completed")["status"], "completed");
    let unknown = task_method(&b, "tasks/get", &"0".repeat(32), "");
    assert_eq!(code(&unknown), -32602);
    assert_eq!(code(&task_method(&b, "tasks/get", "nope", "")), -32602);
}

#[test]
fn a_server_without_task_tools_has_no_task_methods_and_no_extension() {
    let server = Arc::new(
        Server::builder("plain", "1")
            .tool(Tool::new("t", schema()), |_c, _p| Ok(text("t")))
            .build()
            .unwrap(),
    );
    let c = conn(&server);
    assert_eq!(code(&task_method(&c, "tasks/get", "x", "")), -32601);
    let d = send(&c, "server/discover", &format!(r#"{{"_meta":{{{V}}}}}"#));
    assert!(d["result"]["capabilities"].get("extensions").is_none());
}

#[test]
fn a_server_with_task_tools_advertises_the_extension() {
    let f = fixture();
    let d = send(
        &conn(&f.server),
        "server/discover",
        &format!(r#"{{"_meta":{{{V}}}}}"#),
    );
    assert!(d["result"]["capabilities"]["extensions"]["io.modelcontextprotocol/tasks"].is_object());
}

#[test]
fn the_number_of_tasks_is_bounded() {
    let saw = Arc::new(AtomicBool::new(false));
    let server = Arc::new(builder(&saw).max_tasks(1).build().unwrap());
    let c = conn(&server);
    started(&c, "wait");
    let second = call(&c, "quick", WITH_TASKS);
    assert_eq!(code(&second), -32603);
    assert_eq!(second["error"]["message"], "too many tasks");
}

#[test]
fn zero_limits_are_refused_at_build() {
    let saw = Arc::new(AtomicBool::new(false));
    assert!(matches!(
        builder(&saw).max_tasks(0).build(),
        Err(BuildError::ZeroLimit(_))
    ));
    assert!(matches!(
        builder(&saw).task_ttl(Duration::from_millis(900)).build(),
        Err(BuildError::ZeroLimit(_))
    ));
}
