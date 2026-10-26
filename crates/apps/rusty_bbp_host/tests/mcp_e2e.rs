#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Stage-3a exit criterion: a real `bbp mcp` process, driven over stdio with
//! JSON-RPC, serves one turn and refuses calls once that turn has ended.

mod common;
use common::Who;
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
        Mcp::spawn_as(dir, task, principal, ("BBP_TURN", &turn.to_string()))
    }

    /// A session server, which acts only in turns `wait` has shown.
    fn session(dir: &Path, task: &str, principal: &str) -> Mcp {
        Mcp::spawn_as(dir, task, principal, ("BBP_SESSION", "yes"))
    }

    fn spawn_as(dir: &Path, task: &str, principal: &str, bind: (&str, &str)) -> Mcp {
        let child = Proc::new(env!("CARGO_BIN_EXE_bbp"))
            .arg("mcp")
            .env("BBP_DIR", dir)
            .env("BBP_TASK", task)
            .env("BBP_PRINCIPAL", principal)
            .env_remove("BBP_TURN")
            .env_remove("BBP_SESSION")
            .env(bind.0, bind.1)
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
        let id = self.send(method, params);
        self.recv(id)
    }

    /// Write a request without reading its reply; returns its id.
    fn send(&mut self, method: &str, params: &str) -> u64 {
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
        id
    }

    fn recv(&mut self, id: u64) -> Value {
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
        let id = self.send_call(name, args);
        self.recv_call(id)
    }

    fn send_call(&mut self, name: &str, args: &str) -> u64 {
        self.send(
            "tools/call",
            &format!("{{\"name\":\"{name}\",\"arguments\":{args}}}"),
        )
    }

    /// Write several tool calls in one write, as a pipelining client would;
    /// returns their ids.
    fn send_calls(&mut self, calls: &[(&str, &str)]) -> Vec<u64> {
        let mut lines = String::new();
        let ids = calls
            .iter()
            .map(|(name, args)| {
                let id = self.next_id;
                self.next_id += 1;
                lines += &format!("{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{args}}}}}\n");
                id
            })
            .collect();
        let stdin = self.child.stdin.as_mut().expect("stdin");
        stdin.write_all(lines.as_bytes()).expect("write");
        stdin.flush().expect("flush");
        ids
    }

    /// Write raw bytes, such as half a request.
    fn write_raw(&mut self, bytes: &str) {
        let stdin = self.child.stdin.as_mut().expect("stdin");
        stdin.write_all(bytes.as_bytes()).expect("write");
        stdin.flush().expect("flush");
    }

    fn recv_call(&mut self, id: u64) -> (Value, bool) {
        let v = self.recv(id);
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
        &rusty_bbp_host::profiles::ProfileSet::shell("test", "true"),
        common::roster(),
    )
    .expect("open");
    for (role, vendor) in [
        (Who::Planner, "anthropic"),
        (Who::Coder, "openai"),
        (Who::Tester, "google"),
        (Who::Reviewer, "anthropic"),
    ] {
        admin::assign(
            dir,
            &task,
            &role.id(),
            &PrincipalId(role.name().into()),
            vendor,
        )
        .expect("assign");
    }
    admin::tick(dir, &task).expect("tick");
    task
}

#[test]
fn one_process_serves_one_turn_and_refuses_the_next() {
    let dir = tempdir("turn");
    let task = setup(&dir);
    let d = open_driver(&dir, &task).expect("driver");
    let turn = d.state.turn().expect("planner turn").id;
    assert_eq!(
        d.state.turn().map(|t| t.role.clone()),
        Some(Who::Planner.id())
    );

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
    let r = human::perform(&dir, &task, "extend", &["reads_per_turn", "1"], None).expect("extend");
    assert_eq!(r, Response::Ok);
    let mut d = open_driver(&dir, &task).expect("driver");
    d.dispatch(&Command::AbortTurn { turn: None }, rusty_bbp_host::now())
        .expect("abort");
    assert_ne!(
        d.state.turn().map(|t| t.id),
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
    // A new process for the new turn works. Event-driven scheduling gives it
    // to the coder, whom the planner's question woke.
    let next = d.state.turn().expect("turn");
    assert_eq!(next.role, Who::Coder.id());
    let mut mcp2 = Mcp::spawn(&dir, "T1", "coder", next.id.0);
    let (ok, err) = mcp2.call(
        "post",
        r#"{"op":"p3","kind":"ask","to":["planner"],"body":"fresh turn"}"#,
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
        .turn()
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
        .turn()
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
    assert_eq!(
        card.turns.first().map(|t| t.0.clone()),
        Some(Who::Coder.id())
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn turn_of(dir: &Path, task: &TaskId) -> u64 {
    open_driver(dir, task)
        .expect("driver")
        .state
        .turn()
        .as_ref()
        .expect("turn")
        .id
        .0
}

fn stored(resp: &Value) -> u64 {
    resp["Stored"].as_u64().unwrap_or_else(|| panic!("{resp}"))
}

const DIFF: &str = "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-x\n+y\n";

/// Planner: spec and gate. Human: approve. Coder: diff and candidate. The
/// task is then in `test` with run 1 selected. Returns (spec, diff, candidate).
fn drive_to_test(dir: &Path, task: &TaskId) -> (u64, u64, u64) {
    let mut planner = Mcp::spawn(dir, "T1", "planner", turn_of(dir, task));
    let (spec, _) = planner.call(
        "put_artifact",
        r##"{"op":"s","kind":"spec","brief":1,"body":"# Spec\nAC1 retries on 429."}"##,
    );
    let spec = stored(&spec);
    let (_, err) = planner.call("post", &format!(r#"{{"op":"g","kind":"request_decision","to":["human"],"refs":["art:{spec}"],"body":"Approve?","gate":true}}"#));
    assert!(!err);
    drop(planner);
    human::perform(dir, task, "approve-plan", &[&spec.to_string()], None).expect("approve");
    let mut coder = Mcp::spawn(dir, "T1", "coder", turn_of(dir, task));
    let body = DIFF.replace('\n', "\\n");
    let (diff, err) = coder.call(
        "put_artifact",
        &format!(r#"{{"op":"d","kind":"diff","body":"{body}"}}"#),
    );
    assert!(!err, "{diff}");
    let diff = stored(&diff);
    let (cand, err) = coder.call(
        "put_artifact",
        &format!(
            r#"{{"op":"c","kind":"candidate","base":"{}","diffs":[{diff}]}}"#,
            "a".repeat(40)
        ),
    );
    assert!(!err, "{cand}");
    let card = admin::card(dir, task).expect("card");
    assert_eq!(card.state, State::Test);
    (spec, diff, stored(&cand))
}

/// The runner stores a log then a passing report for the selected run.
/// Returns (log, report).
fn runner_passes(dir: &Path, task: &TaskId, log: &[u8]) -> (ArtId, ArtId) {
    let mut d = open_driver(dir, task).expect("driver");
    let run = d.state.run.clone().expect("selected run");
    let secret = d.state.secret_for(run.id);
    let blob = d.store.blob_put(log);
    let store_as = |d: &mut Driver<FsStore>, blob, payload, op: &str| match d
        .dispatch(
            &Command::Runner {
                op: OpId(op.into()),
                run: run.id,
                secret,
                blob,
                payload,
            },
            rusty_bbp_host::now(),
        )
        .expect("dispatch")
    {
        Response::Stored(id) => id,
        other => panic!("{other:?}"),
    };
    let log_id = store_as(&mut d, blob, ArtifactPayload::Log, "log");
    let report = ArtifactPayload::TestReport(Report {
        candidate: run.candidate,
        run: run.id,
        profile_digest: d.state.profile_digest,
        status: RunStatus::Passed,
        profiles: vec![ProfileResult {
            name: "test".into(),
            exit_code: 0,
            failed: 0,
        }],
        tree: Some(Sha256([9; 32])),
        sandbox: "unconfined".into(),
        log: log_id,
    });
    let bytes = encode_artifact(&report, b"").expect("encode");
    let blob = d.store.blob_put(&bytes);
    let report_id = store_as(&mut d, blob, report, "report");
    (log_id, report_id)
}

#[test]
fn get_artifact_returns_content_but_never_a_log_to_the_reviewer() {
    let dir = tempdir("content");
    let task = setup(&dir);
    let mut planner = Mcp::spawn(&dir, "T1", "planner", turn_of(&dir, &task));
    let (brief, err) = planner.call("get_artifact", r#"{"op":"b","id":1}"#);
    assert!(!err, "{brief}");
    assert_eq!(brief["content"].as_str(), Some("Add retry with backoff."));
    assert_eq!(brief["Artifact"]["kind"].as_str(), Some("Brief"));
    drop(planner);

    let (spec, diff, cand) = drive_to_test(&dir, &task);
    let (log, report) = runner_passes(&dir, &task, b"SECRET-LOG-LINE ok");
    // The Tester may read the log; its approval moves the task to review.
    let mut tester = Mcp::spawn(&dir, "T1", "tester", turn_of(&dir, &task));
    let (seen, err) = tester.call("get_artifact", &format!(r#"{{"op":"tl","id":{}}}"#, log.0));
    assert!(!err, "{seen}");
    assert_eq!(seen["content"].as_str(), Some("SECRET-LOG-LINE ok"));
    let (v, err) = tester.call("post", &format!(r#"{{"op":"tv","kind":"verdict","to":["*"],"refs":["art:{cand}"],"evidence":["art:{}"],"verdict":{{"subject":{cand},"run":1,"verdict":"approve"}}}}"#, report.0));
    assert!(!err, "{v}");
    drop(tester);
    assert_eq!(admin::card(&dir, &task).expect("card").state, State::Review);
    let mut reviewer = Mcp::spawn(&dir, "T1", "reviewer", turn_of(&dir, &task));
    let (spec, err) = reviewer.call("get_artifact", &format!(r#"{{"op":"rs","id":{spec}}}"#));
    assert!(!err, "{spec}");
    assert_eq!(
        spec["content"].as_str(),
        Some("# Spec\nAC1 retries on 429.")
    );
    let (diff, err) = reviewer.call("get_artifact", &format!(r#"{{"op":"rd","id":{diff}}}"#));
    assert!(!err, "{diff}");
    assert_eq!(diff["content"].as_str(), Some(DIFF));
    let (denied, err) = reviewer.call("get_artifact", &format!(r#"{{"op":"rl","id":{}}}"#, log.0));
    assert!(err, "{denied}");
    assert!(denied.get("Rejected").is_some(), "{denied}");
    assert!(!denied.to_string().contains("SECRET-LOG-LINE"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn accepted_op_replays_after_the_turn_it_ended() {
    let dir = tempdir("replay");
    let task = setup(&dir);
    let mut planner = Mcp::spawn(&dir, "T1", "planner", turn_of(&dir, &task));
    let (spec, _) = planner.call(
        "put_artifact",
        r##"{"op":"s","kind":"spec","brief":1,"body":"# Spec"}"##,
    );
    let gate = format!(
        r#"{{"op":"g","kind":"request_decision","to":["human"],"refs":["art:{}"],"body":"Approve?","gate":true}}"#,
        stored(&spec)
    );
    let (first, err) = planner.call("post", &gate);
    assert!(!err, "{first}");
    assert_eq!(
        admin::card(&dir, &task).expect("card").state,
        State::PlanGate
    );
    let rev = admin::card(&dir, &task).expect("card").rev;
    // The response was lost; the identical retry gets the identical answer
    // and changes nothing.
    let (again, err) = planner.call("post", &gate);
    assert!(!err, "{again}");
    assert_eq!(first, again);
    assert_eq!(admin::card(&dir, &task).expect("card").rev, rev);
    // The same op with another payload, and any fresh op, are refused.
    let (conflict, err) = planner.call("post", &gate.replace("Approve?", "Changed"));
    assert!(err, "{conflict}");
    assert!(conflict.get("Rejected").is_some(), "{conflict}");
    let (fresh, err) = planner.call(
        "post",
        r#"{"op":"g2","kind":"ask","to":["coder"],"body":"late"}"#,
    );
    assert!(err);
    assert!(
        fresh.as_str().unwrap_or("").contains("has ended"),
        "{fresh}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_delayed_rerun_meets_stale_rev_and_the_newer_run_survives() {
    let dir = tempdir("rerun");
    let task = setup(&dir);
    let (_, _, cand) = drive_to_test(&dir, &task);
    let cand = cand.to_string();
    let seen = admin::card(&dir, &task).expect("card").rev;
    let refused = human::perform(&dir, &task, "rerun", &[&cand], None).unwrap_err();
    assert!(refused.contains("--rev"), "{refused}");
    let r = human::perform(&dir, &task, "rerun", &[&cand], Some(seen)).expect("rerun");
    assert_eq!(r, Response::Ok);
    let run2 = open_driver(&dir, &task)
        .expect("driver")
        .state
        .run
        .expect("run")
        .id;
    assert_eq!(run2, RunId(2));
    // A rerun the human decided on before run 2 existed is refused, not applied.
    let stale = human::perform(&dir, &task, "rerun", &[&cand], Some(seen)).expect("dispatch");
    assert!(
        matches!(
            stale,
            Response::Rejected(Rejection {
                code: Code::StaleRev,
                ..
            })
        ),
        "{stale:?}"
    );
    assert_eq!(
        open_driver(&dir, &task)
            .expect("driver")
            .state
            .run
            .expect("run")
            .id,
        run2
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Racing first opens agree on one master: the key on disk.
#[test]
fn master_initialisation_has_one_winner() {
    use rusty_bbp_host::master;
    use std::sync::{Arc, Barrier};
    let dir = tempdir("master");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let n = 4;
    let barrier = Arc::new(Barrier::new(n));
    let keys: Vec<[u8; 32]> = (0..n)
        .map(|_| {
            let (dir, barrier) = (dir.clone(), barrier.clone());
            std::thread::spawn(move || {
                barrier.wait();
                master::ensure(&dir).expect("ensure")
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|h| h.join().expect("join"))
        .collect();
    let on_disk = master::load(&dir).expect("load");
    assert!(
        keys.iter().all(|k| *k == on_disk),
        "every initialiser returned the published key"
    );
    assert!(
        std::fs::read_dir(&dir).expect("dir").count() == 1,
        "no candidate file left behind"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// End the live turn from outside, as the moderator does at a deadline.
fn abort_turn(dir: &Path, task: &TaskId) {
    open_driver(dir, task)
        .expect("driver")
        .dispatch(&Command::AbortTurn { turn: None }, rusty_bbp_host::now())
        .expect("abort");
}

fn woke(reply: &Value) -> (&str, Option<u64>) {
    (
        reply["woke"].as_str().unwrap_or_else(|| panic!("{reply}")),
        reply["turn"].as_u64(),
    )
}

fn ticket(reply: &Value) -> String {
    reply["ticket"]
        .as_str()
        .unwrap_or_else(|| panic!("{reply}"))
        .to_owned()
}

fn post(op: &str, ticket: &str) -> String {
    format!(r#"{{"op":"{op}","kind":"ask","to":["coder"],"body":"{op}","ticket":"{ticket}"}}"#)
}

fn refusal(reply: &(Value, bool)) -> &str {
    assert!(reply.1, "expected a refusal: {}", reply.0);
    reply.0.as_str().unwrap_or("")
}

#[test]
fn a_session_acts_only_in_turns_wait_has_shown() {
    let dir = tempdir("session");
    let task = setup(&dir);
    let first = turn_of(&dir, &task);
    let mut planner = Mcp::session(&dir, "T1", "planner");
    let tools = planner.request("tools/list", "{}");
    let Value::Seq(tools) = &tools["result"]["tools"] else {
        panic!("{tools}")
    };
    assert_eq!(tools.last().and_then(|t| t["name"].as_str()), Some("wait"));
    let post_tool = tools.iter().find(|t| t["name"].as_str() == Some("post"));
    let required = &post_tool.expect("post")["inputSchema"]["required"];
    assert!(required.to_string().contains("ticket"), "{required}");

    // Pipelined with the wait that shows the grant, a call issued before it
    // holds no ticket and cannot act, whatever the timing.
    let ids = planner.send_calls(&[
        ("wait", r#"{"timeout_s":5}"#),
        (
            "post",
            r#"{"op":"q0","kind":"ask","to":["coder"],"body":"queued"}"#,
        ),
    ]);
    let (w, _) = planner.recv_call(ids[0]);
    assert_eq!(woke(&w), ("turn", Some(first)));
    let t1 = ticket(&w);
    assert!(refusal(&planner.recv_call(ids[1])).contains("ticket"));
    let forged = format!("{first}.{}", "0".repeat(32));
    assert!(refusal(&planner.call("post", &post("f", &forged))).contains("unknown ticket"));
    let (_, err) = planner.call("task_card", "{}");
    assert!(!err, "the card stays free");
    let (w, _) = planner.call("wait", "{}");
    assert_eq!(
        woke(&w),
        ("held", Some(first)),
        "a held turn returns at once"
    );
    assert_eq!(ticket(&w), t1);
    let accepted = planner.call("post", &post("p1", &t1));
    assert!(!accepted.1, "{}", accepted.0);

    // The turn ends: its ticket acts no more, but an accepted op replays.
    abort_turn(&dir, &task);
    assert!(refusal(&planner.call("post", &post("p2", &t1)))
        .contains(&format!("turn {first} has ended; call wait")));
    assert_eq!(
        planner.call("post", &post("p1", &t1)),
        accepted,
        "R15 replay"
    );
    let started = std::time::Instant::now();
    let (w, _) = planner.call("wait", r#"{"timeout_s":1}"#);
    assert_eq!(woke(&w), ("timeout", None));
    assert!(started.elapsed() < std::time::Duration::from_secs(5));

    // Calls from the old inference queued behind a blocked wait keep their
    // binding when the coder's question grants the planner a new turn.
    let mut coder = Mcp::session(&dir, "T1", "coder");
    let (w, _) = coder.call("wait", "{}");
    assert_eq!(woke(&w).0, "turn");
    let tc = ticket(&w);
    let pending = planner.send_call("wait", r#"{"timeout_s":60}"#);
    let stale = planner.send_call("post", &post("p-old", &t1));
    let bare = planner.send_call(
        "post",
        r#"{"op":"p-bare","kind":"ask","to":["coder"],"body":"?"}"#,
    );
    let asked =
        format!(r#"{{"op":"c1","kind":"ask","to":["planner"],"body":"Sync.","ticket":"{tc}"}}"#);
    let (ok, err) = coder.call("post", &asked);
    assert!(!err, "{ok}");
    let started = std::time::Instant::now();
    abort_turn(&dir, &task);
    let next = turn_of(&dir, &task);
    let (w, _) = planner.recv_call(pending);
    assert_eq!(woke(&w), ("turn", Some(next)));
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    let t2 = ticket(&w);
    assert_ne!(t1, t2);
    assert!(refusal(&planner.recv_call(stale)).contains(&format!("turn {first} has ended")));
    assert!(refusal(&planner.recv_call(bare)).contains("ticket"));
    assert!(refusal(&planner.call("post", &post("p-coder", &tc))).contains("unknown ticket"));
    let (ok, err) = planner.call("post", &post("p3", &t2));
    assert!(!err, "{ok}");

    // A finished task ends every wait.
    let rev = admin::card(&dir, &task).expect("card").rev;
    human::perform(&dir, &task, "cancel", &["done"], Some(rev)).expect("cancel");
    let (w, _) = coder.call("wait", "{}");
    assert_eq!(woke(&w), ("ended", None));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_partial_request_never_holds_back_a_wait_reply() {
    let dir = tempdir("partial");
    let task = setup(&dir);
    let mut planner = Mcp::session(&dir, "T1", "planner");
    let id = planner.send_call("wait", r#"{"timeout_s":5}"#);
    planner.write_raw(r#"{"jsonrpc":"2.0","id":99,"method":"#);
    let (w, _) = planner.recv_call(id);
    assert_eq!(woke(&w), ("turn", Some(turn_of(&dir, &task))));
    planner.write_raw("\"ping\"}\n");
    assert!(planner.recv(99).get("result").is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_server_is_bound_to_one_turn_or_to_a_session_never_both() {
    let dir = tempdir("binding");
    let task = setup(&dir);
    let turn = turn_of(&dir, &task).to_string();
    for env in [
        vec![("BBP_TURN", turn.as_str()), ("BBP_SESSION", "yes")],
        vec![("BBP_SESSION", "no")],
        vec![],
    ] {
        let out = Proc::new(env!("CARGO_BIN_EXE_bbp"))
            .arg("mcp")
            .env("BBP_DIR", &dir)
            .env("BBP_TASK", "T1")
            .env("BBP_PRINCIPAL", "planner")
            .env_remove("BBP_TURN")
            .env_remove("BBP_SESSION")
            .envs(env.iter().copied())
            .stdin(Stdio::null())
            .output()
            .expect("run bbp mcp");
        assert!(!out.status.success(), "{env:?}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains("exactly one"), "{env:?}: {err}");
    }
    // A per-turn server has no wait.
    let mut mcp = Mcp::spawn(&dir, "T1", "planner", turn.parse().expect("turn"));
    let (refused, err) = mcp.call("wait", "{}");
    assert!(err);
    assert!(
        refused.as_str().unwrap_or("").contains("session"),
        "{refused}"
    );
    drop(mcp);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_co_edited_spec_is_written_and_read_by_line_id() {
    let dir = tempdir("coedit");
    let task = setup(&dir);
    let mut planner = Mcp::spawn(&dir, "T1", "planner", turn_of(&dir, &task));
    let (v1, err) = planner.call(
        "put_artifact",
        r##"{"op":"e1","kind":"spec","brief":1,"edit":{"ops":[{"insert":{"lines":["# A","a"]}}]}}"##,
    );
    assert!(!err, "{v1}");
    let v1 = stored(&v1);
    let (got, err) = planner.call("get_artifact", &format!(r#"{{"op":"g1","id":{v1}}}"#));
    assert!(!err, "{got}");
    assert_eq!(got["content"].as_str(), Some("# A\na"));
    assert_eq!(got["head"].as_u64(), Some(v1));
    let a = format!("art:{v1}#1");
    assert_eq!(got["lines"][1]["id"].as_str(), Some(a.as_str()));
    assert_eq!(got["lines"][1]["text"].as_str(), Some("a"));
    let snap1 = got["snapshot"].as_str().map(str::to_owned);
    assert_eq!(snap1.as_deref().map(str::len), Some(64));

    let edit = format!(
        r##"{{"op":"e2","kind":"spec","brief":1,"edit":{{"base":{v1},"ops":[{{"insert":{{"after":"{a}","lines":["b"]}}}},{{"delete":"art:{v1}#0"}}]}}}}"##
    );
    let (v2, err) = planner.call("put_artifact", &edit);
    assert!(!err, "{v2}");
    let v2 = stored(&v2);
    let (got, err) = planner.call("get_artifact", &format!(r#"{{"op":"g2","id":{v2}}}"#));
    assert!(!err, "{got}");
    assert_eq!(got["content"].as_str(), Some("a\nb"));
    assert_ne!(got["snapshot"].as_str().map(str::to_owned), snap1);
    let (old, err) = planner.call("get_artifact", &format!(r#"{{"op":"g3","id":{v1}}}"#));
    assert!(!err, "{old}");
    assert_eq!(
        old["head"].as_u64(),
        Some(v2),
        "an old version names its head"
    );

    let (bad, err) = planner.call(
        "put_artifact",
        r##"{"op":"e3","kind":"spec","brief":1,"edit":{"ops":[{"delete":"line 3"}]}}"##,
    );
    assert!(err, "{bad}");
    assert!(bad.to_string().contains("art:N#M"), "{bad}");
    let mixed = format!(
        r##"{{"op":"e4","kind":"spec","brief":1,"edit":{{"base":{v2},"ops":[{{"delete":"{a}","insert":{{"lines":["replacement"]}}}}]}}}}"##
    );
    let (refused, err) = planner.call("put_artifact", &mixed);
    assert!(err, "an op naming two variants is refused: {refused}");
    let (after, err) = planner.call("get_artifact", &format!(r#"{{"op":"g4","id":{v2}}}"#));
    assert!(!err, "{after}");
    assert_eq!(after["head"].as_u64(), Some(v2), "nothing was stored");
    assert_eq!(after["content"].as_str(), Some("a\nb"));
    let _ = std::fs::remove_dir_all(&dir);
}
