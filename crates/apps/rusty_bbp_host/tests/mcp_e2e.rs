#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Stage-3a exit criterion: a real `bbp mcp` process, driven over stdio with
//! JSON-RPC, serves one turn and refuses calls once that turn has ended.

use rusty_bbp::*;
use rusty_bbp_host::{admin, human, open_driver};
use rusty_serde::Value;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as Proc, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bbp_host_{}_{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

struct Mcp {
    child: Child,
    reader: BufReader<std::process::ChildStdout>,
    next_id: u64,
}

impl Mcp {
    fn spawn(dir: &Path, task: &str, principal: &str, turn: u64) -> Mcp {
        let child = Proc::new(env!("CARGO_BIN_EXE_bbp"))
            .arg("mcp")
            .env("BBP_DIR", dir)
            .env("BBP_TASK", task)
            .env("BBP_PRINCIPAL", principal)
            .env("BBP_TURN", turn.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn bbp mcp");
        let mut child = child;
        let reader = BufReader::new(child.stdout.take().expect("stdout"));
        Mcp {
            child,
            reader,
            next_id: 1,
        }
    }

    fn request(&mut self, method: &str, params: &str) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let line = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"{method}\",\"params\":{params}}}\n"
        );
        self.child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(line.as_bytes())
            .expect("write");
        let mut out = String::new();
        self.reader.read_line(&mut out).expect("read");
        let v: Value = rusty_serde::json::from_str(out.trim()).expect("json response");
        assert_eq!(v.get("id").and_then(Value::as_u64), Some(id));
        v
    }

    fn notify(&mut self, method: &str) {
        let line = format!("{{\"jsonrpc\":\"2.0\",\"method\":\"{method}\"}}\n");
        self.child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(line.as_bytes())
            .expect("write");
    }

    fn call(&mut self, name: &str, args: &str) -> (Value, bool) {
        let v = self.request(
            "tools/call",
            &format!("{{\"name\":\"{name}\",\"arguments\":{args}}}"),
        );
        let result = v.get("result").expect("result").clone();
        let is_error = result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let text = result["content"][0]["text"]
            .as_str()
            .expect("text")
            .to_owned();
        let parsed: Value = rusty_serde::json::from_str(&text).unwrap_or(Value::String(text));
        (parsed, is_error)
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn setup(dir: &Path) -> TaskId {
    let task = TaskId("T1".into());
    admin::open_task(
        dir,
        &task,
        "github.com/example/repo",
        b"Add retry with backoff.",
        &PrincipalId("human".into()),
    )
    .expect("open");
    for (role, vendor) in [
        (Role::Planner, "anthropic"),
        (Role::Coder, "openai"),
        (Role::Tester, "google"),
        (Role::Reviewer, "anthropic"),
    ] {
        admin::assign(
            dir,
            &task,
            role,
            &PrincipalId(admin::role_name(role).into()),
            vendor,
        )
        .expect("assign");
    }
    task
}

#[test]
fn one_process_serves_one_turn_and_refuses_the_next() {
    let dir = tempdir("turn");
    let task = setup(&dir);
    let d = open_driver(&dir, &task).expect("driver");
    let turn = d.state.turn.as_ref().expect("planner turn").id;
    assert_eq!(d.state.turn.as_ref().map(|t| t.role), Some(Role::Planner));

    let mut mcp = Mcp::spawn(&dir, "T1", "planner", turn.0);
    let init = mcp.request("initialize", r#"{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"0"}}"#);
    assert_eq!(init["result"]["serverInfo"]["name"].as_str(), Some("bbp"));
    mcp.notify("notifications/initialized");
    let tools = mcp.request("tools/list", "{}");
    let names: Vec<&str> = match &tools["result"]["tools"] {
        Value::Seq(t) => t.iter().filter_map(|x| x["name"].as_str()).collect(),
        _ => vec![],
    };
    assert_eq!(
        names,
        vec!["task_card", "read", "get_artifact", "post", "put_artifact"]
    );

    // Free card, then a post with an explicit op, replayed to the same id.
    let (card, err) = mcp.call("task_card", "{}");
    assert!(!err);
    assert_eq!(card["Card"]["state"].as_str(), Some("Planning"));
    let (first, err) = mcp.call(
        "post",
        r#"{"op":"p1","kind":"ask","to":["coder"],"body":"Is the client async?"}"#,
    );
    assert!(!err, "{first}");
    let (again, _) = mcp.call(
        "post",
        r#"{"op":"p1","kind":"ask","to":["coder"],"body":"Is the client async?"}"#,
    );
    assert_eq!(first, again, "same op, same result");

    // A spec through put_artifact: payload derives from the bytes the server encodes.
    let (spec, err) = mcp.call(
        "put_artifact",
        r##"{"op":"s1","kind":"spec","brief":1,"body":"# Spec\nAC1 retries on 429."}"##,
    );
    assert!(!err, "{spec}");
    let (read, err) = mcp.call("read", r#"{"after":0,"limit":20}"#);
    assert!(!err);
    assert!(read["Read"]["messages"].as_str().is_none());

    // The human ends this turn from outside; the server now refuses.
    let r = human::perform(&dir, &task, "extend", &["reads_per_turn", "1"]).expect("extend");
    assert_eq!(r, Response::Ok);
    let mut d = open_driver(&dir, &task).expect("driver");
    d.dispatch(&Command::AbortTurn, rusty_bbp_host::now())
        .expect("abort");
    assert_ne!(
        d.state.turn.as_ref().map(|t| t.id),
        Some(turn),
        "a new turn was granted"
    );
    let (late, err) = mcp.call(
        "post",
        r#"{"op":"p2","kind":"ask","to":["coder"],"body":"late"}"#,
    );
    assert!(err);
    assert!(late.as_str().unwrap_or("").contains("has ended"), "{late}");
    // The card is still free to read.
    let (_, err) = mcp.call("task_card", "{}");
    assert!(!err);
    // A new process for the new turn works.
    let new_turn = d.state.turn.as_ref().expect("turn").id;
    let mut mcp2 = Mcp::spawn(&dir, "T1", "planner", new_turn.0);
    let (ok, err) = mcp2.call(
        "post",
        r#"{"op":"p3","kind":"ask","to":["coder"],"body":"fresh turn"}"#,
    );
    assert!(!err, "{ok}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn malformed_requests_get_jsonrpc_errors() {
    let dir = tempdir("errors");
    let task = setup(&dir);
    let turn = open_driver(&dir, &task)
        .expect("driver")
        .state
        .turn
        .as_ref()
        .expect("turn")
        .id;
    let mut mcp = Mcp::spawn(&dir, "T1", "planner", turn.0);
    let v = mcp.request("no/such", "{}");
    assert_eq!(v["error"]["code"].as_i64(), Some(-32601));
    let (bad, err) = mcp.call("post", r#"{"op":"x","kind":"verdict"}"#);
    assert!(err);
    assert!(bad.as_str().unwrap_or("").contains("verdict"));
    let (rej, err) = mcp.call(
        "post",
        r#"{"op":"y","kind":"decision","to":["*"],"refs":["art:1"],"body":"no"}"#,
    );
    assert!(err);
    assert_eq!(rej["Rejected"]["code"].as_str(), Some("DecisionForbidden"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn human_cli_drives_a_gate() {
    let dir = tempdir("human");
    let task = setup(&dir);
    let turn = open_driver(&dir, &task)
        .expect("driver")
        .state
        .turn
        .as_ref()
        .expect("turn")
        .id;
    let mut mcp = Mcp::spawn(&dir, "T1", "planner", turn.0);
    let (spec, _) = mcp.call(
        "put_artifact",
        r##"{"op":"s","kind":"spec","brief":1,"body":"# Spec"}"##,
    );
    let spec_id = spec["Stored"].as_u64().expect("spec id");
    let (_, err) = mcp.call("post", &format!(r#"{{"op":"g","kind":"request_decision","to":["human"],"refs":["art:{spec_id}"],"body":"Approve the plan?","gate":true}}"#));
    assert!(!err);
    assert_eq!(
        admin::card(&dir, &task).expect("card").state,
        State::PlanGate
    );
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_bbp"))
        .args([
            "human",
            "--dir",
            dir.to_str().expect("utf8"),
            "--task",
            "T1",
            "approve-plan",
            &spec_id.to_string(),
        ])
        .output()
        .expect("run bbp human");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "\"Ok\"");
    let card = admin::card(&dir, &task).expect("card");
    assert_eq!(card.state, State::Build);
    assert_eq!(card.turn.map(|t| t.0), Some(Role::Coder));
    let _ = std::fs::remove_dir_all(&dir);
}
