//! Actual CLI contracts and scripted MCP task; no provider credentials or merge.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::tests::*;
use super::*;
use orch_core::board::{Author, NewEntry};
use rusty_bbp::{Event, FsStore, Store, TaskView};
use std::cell::Cell;
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::time::Instant;

#[derive(Debug, Clone)]
struct LostReply {
    at: usize,
    calls: Rc<Cell<usize>>,
}
impl CommandRunner for LostReply {
    fn run_in(
        &self,
        cwd: Option<&std::path::Path>,
        argv: &[String],
        stdin: &[u8],
        timeout: Duration,
        scrub: &[&str],
    ) -> Result<orch_cli::Exit, ExecError> {
        let out = StdCommand.run_in(cwd, argv, stdin, timeout, scrub)?;
        self.calls.set(self.calls.get() + 1);
        if self.calls.get() == self.at {
            return Err(ExecError::Timeout(timeout));
        }
        Ok(out)
    }
}
pub(super) fn setup() -> (PathBuf, Config) {
    let dir = temp();
    let mut c = config(&dir);
    c.binary = std::env::var_os("BBP_TEST_BIN")
        .expect("set BBP_TEST_BIN to built bbp")
        .into();
    std::fs::create_dir_all(&c.repo).unwrap();
    fn git(repo: &std::path::Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    git(&c.repo, &["init", "-q"]);
    git(&c.repo, &["config", "user.name", "Test"]);
    git(&c.repo, &["config", "user.email", "test@example.invalid"]);
    std::fs::write(c.repo.join("flag"), "no\n").unwrap();
    git(&c.repo, &["add", "."]);
    git(&c.repo, &["commit", "-qm", "base"]);
    std::fs::write(&c.profiles, r#"{"profiles":[{"name":"test","program":"/bin/sh","args":["-c","test \"$(cat flag)\" = ok && test -z \"$GH_TOKEN$OPENAI_API_KEY$BBP_TASK\""]}],"read_roots":["/bin","/usr","/lib","/lib64"],"env":[["PATH","/usr/bin:/bin"]],"limits":{"cpu_secs":10,"wall_secs":10,"memory_bytes":1073741824,"file_bytes":1048576,"open_files":256,"processes":4096},"tree":"sha256(git write-tree id)"}"#).unwrap();
    (dir, c)
}
fn events(h: &Handle) -> Vec<Event> {
    FsStore::open(&h.dir)
        .unwrap()
        .events(&rusty_bbp::TaskId(h.task.clone()), 0)
        .unwrap()
}

#[test]
#[ignore = "requires built bbp at BBP_TEST_BIN"]
fn actual_cli_adopts_after_every_lost_open_reply_and_freezes_the_pool() {
    for at in 1..=5 {
        let (dir, c) = setup();
        let (task, board) = task();
        let mut host = BbpHost::with_seams(
            c.clone(),
            "recovery".into(),
            "context".into(),
            LostReply {
                at,
                calls: Rc::new(Cell::new(0)),
            },
            FakeSupervisor::default(),
        )
        .unwrap();
        assert!(
            host.open(&task, &board, &fence()).is_err(),
            "lost reply {at}"
        );
        drop(host);
        let mut changed = c.clone();
        changed.pool.get_mut("writer").unwrap().vendor = "codex".into();
        let mut host = BbpHost::with_seams(
            changed,
            "recovery".into(),
            "new context".into(),
            StdCommand,
            FakeSupervisor::default(),
        )
        .unwrap();
        let handle = host.open(&task, &board, &fence()).unwrap();
        assert_eq!(host.open(&task, &board, &fence()).unwrap(), handle);
        let h = host.handle(&handle).unwrap();
        let log = events(&h);
        assert_eq!(
            log.iter()
                .filter(|e| matches!(e, Event::TaskOpened { .. }))
                .count(),
            1
        );
        assert_eq!(
            log.iter()
                .filter(|e| matches!(e, Event::Assigned { .. }))
                .count(),
            2
        );
        assert_eq!(
            BbpHost::<StdCommand, FakeSupervisor>::load(&h)
                .unwrap()
                .config
                .pool["writer"]
                .vendor,
            "ollama"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[derive(Debug)]
struct Moderator(Child);
impl Drop for Moderator {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn start(c: &Config, h: &Handle) -> Moderator {
    let log = std::fs::File::create(c.dir.join("test-mod.log")).unwrap();
    Moderator(
        Command::new(&c.binary)
            .args(["mod", "--dir"])
            .arg(&h.dir)
            .args(["--task", &h.task, "--repo-path"])
            .arg(&c.repo)
            .arg("--work")
            .arg(supervisor::task_work(c, &h.task))
            .arg("--agents")
            .arg(BbpHost::<StdCommand, FakeSupervisor>::manifest(h).join("agents.json"))
            // Explicit test-only unconfined runner: production always uses sandbox.
            .args([
                "--confine",
                "none",
                "--poll-ms",
                "10",
                "--max-wall-secs",
                "30",
            ])
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap(),
    )
}
#[derive(Debug, Clone, Default)]
struct ScriptSupervisor(Rc<std::cell::RefCell<Option<Moderator>>>);
impl Supervisor for ScriptSupervisor {
    fn ensure(
        &mut self,
        config: &Config,
        task: &str,
        _: &std::path::Path,
    ) -> Result<Supervised, String> {
        let mut child = self.0.borrow_mut();
        if let Some(m) = child.as_mut() {
            if m.0.try_wait().map_err(|e| e.to_string())?.is_none() {
                return Ok(Supervised::Running);
            }
        }
        *child = Some(start(
            config,
            &Handle {
                dir: config.dir.clone(),
                task: task.into(),
            },
        ));
        Ok(Supervised::Started)
    }
}
fn wait_gate<S: Supervisor>(
    host: &mut BbpHost<StdCommand, S>,
    handle: &HostHandle,
    expected: &str,
) -> GateContext {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let progress = host.poll(handle, &fence()).unwrap();
        if let Progress::Gate(g) = &progress {
            assert!(
                g.prompt.as_str().contains(expected),
                "unexpected gate: {progress:?}"
            );
            return g.clone();
        }
        assert!(Instant::now() < deadline, "timed out: {progress:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn answer<S: Supervisor>(
    host: &mut BbpHost<StdCommand, S>,
    handle: &HostHandle,
    board: &mut Board,
    gate: GateContext,
    body: &str,
) -> Entry {
    let mut refs = vec![
        Ref::Url(handle.0.clone()),
        Ref::Url(txt(&format!("host:rev:{}", gate.rev))),
    ];
    refs.extend(gate.ids.into_iter().map(Ref::Url));
    let id = board
        .append(NewEntry {
            task: None,
            author: Author::Human,
            kind: EntryKind::Question,
            body: gate.prompt,
            refs,
            supersedes: None,
        })
        .unwrap();
    let question = board.get(id).unwrap().clone();
    assert_eq!(
        host.relay(handle, &question, &txt(body), &fence()).unwrap(),
        Relayed::Applied
    );
    question
}

pub(super) fn scripted_config() -> (PathBuf, Config) {
    let (dir, mut c) = setup();
    let base = Command::new("git")
        .arg("-C")
        .arg(&c.repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    let base = String::from_utf8(base.stdout).unwrap().trim().to_owned();
    // Two capability-defined roles, exercising a roster unlike the four-role example.
    let maker = dir.join("maker.sh");
    let auditor = dir.join("auditor.sh");
    let initialize = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"script","version":"0"}}}"#;
    let planning = r##"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"put_artifact","arguments":{"op":"p1","kind":"spec","brief":1,"body":"# Spec\nMake flag ok."}}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"post","arguments":{"op":"p2","kind":"request_decision","to":["human"],"refs":["art:2"],"body":"Approve?","gate":true}}}"##;
    let build = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"put_artifact","arguments":{{"op":"c1","kind":"diff","body":"--- a/flag\n+++ b/flag\n@@ -1 +1 @@\n-no\n+ok\n"}}}}}}
{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"put_artifact","arguments":{{"op":"c2","kind":"candidate","base":"{base}","diffs":[3],"body":""}}}}}}"#
    );
    let verdict = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"post","arguments":{"op":"verdict","kind":"verdict","to":["*"],"refs":["art:4"],"evidence":["art:6"],"verdict":{"subject":4,"run":1,"verdict":"approve","blocking":[],"non_blocking":[]}}}}"#;
    // Select the script by the public card state, so restarted turns need no fixed turn ids.
    let script=format!("set -e\nstate=$(\"$BBP_BIN\" card --dir \"$BBP_DIR\" --task \"$BBP_TASK\")\ncase \"$state\" in\n*'\"Planning\"'*) \"$BBP_BIN\" mcp <<'RPC'\n{initialize}\n{planning}\nRPC\n;;\n*'\"Build\"'*) \"$BBP_BIN\" mcp <<'RPC'\n{initialize}\n{build}\nRPC\n;;\n*'\"Test\"'*) \"$BBP_BIN\" mcp <<'RPC'\n{initialize}\n{verdict}\nRPC\n;;\nesac\n");
    std::fs::write(&maker, script).unwrap();
    std::fs::write(
        &auditor,
        format!("set -e\n\"$BBP_BIN\" mcp <<'RPC'\n{initialize}\n{verdict}\nRPC\n"),
    )
    .unwrap();
    c.pool.get_mut("writer").unwrap().program = "/bin/sh".into();
    c.pool.get_mut("writer").unwrap().args = vec![maker.to_string_lossy().into()];
    c.pool.get_mut("judge").unwrap().program = "/bin/sh".into();
    c.pool.get_mut("judge").unwrap().args = vec![auditor.to_string_lossy().into()];
    (dir, c)
}

#[test]
#[ignore = "requires built bbp at BBP_TEST_BIN; scripted unconfined runner"]
fn scripted_task_restarts_moderator_then_records_receipt_and_closes() {
    let (dir, c) = scripted_config();
    let (task, mut board) = task();
    let supervisor = ScriptSupervisor::default();
    let mut host = BbpHost::with_seams(
        c.clone(),
        "scripted".into(),
        "flag task".into(),
        StdCommand,
        supervisor.clone(),
    )
    .unwrap();
    let handle = host.open(&task, &board, &fence()).unwrap();
    let h = host.handle(&handle).unwrap();
    let gate = wait_gate(&mut host, &handle, "approve-plan");
    // Simulate a killed moderator at a durable human gate, then rebuild the adapter.
    let mut moderator = supervisor.0.borrow_mut().take().unwrap();
    moderator.0.kill().unwrap();
    moderator.0.wait().unwrap();
    drop(moderator);
    drop(host);
    let mut host = BbpHost::with_seams(
        c.clone(),
        "scripted".into(),
        "flag task".into(),
        StdCommand,
        supervisor.clone(),
    )
    .unwrap();
    assert_eq!(host.open(&task, &board, &fence()).unwrap(), handle);
    let question = answer(&mut host, &handle, &mut board, gate, "approve-plan art:2");
    // Lost checkpoint after BBP accepted: replay must not create a second approval.
    assert_eq!(
        host.relay(&handle, &question, &txt("approve-plan art:2"), &fence())
            .unwrap(),
        Relayed::Applied
    );
    let gate = wait_gate(&mut host, &handle, "approve-merge");
    answer(
        &mut host,
        &handle,
        &mut board,
        gate,
        "approve-merge art:4 run:1",
    );
    let gate = wait_gate(&mut host, &handle, "receipt");
    assert!(!events(&h)
        .iter()
        .any(|e| matches!(e, Event::MergeReceipt { .. })));
    answer(
        &mut host,
        &handle,
        &mut board,
        gate,
        "receipt art:4 external-merge-sha",
    );
    assert!(
        matches!(host.poll(&handle, &fence()).unwrap(),Progress::Closed{outputs} if outputs.len()==2)
    );
    let log = events(&h);
    let view = TaskView::from_events(rusty_bbp::TaskId(h.task.clone()), &log);
    assert_eq!(
        view.receipt,
        Some((rusty_bbp::ArtId(4), "external-merge-sha".into()))
    );
    assert_eq!(
        log.iter()
            .filter(|e| matches!(
                e,
                Event::HumanApproval {
                    gate: rusty_bbp::Gate::Plan,
                    ..
                }
            ))
            .count(),
        1
    );
    assert!(log.iter().any(|e| matches!(
        e,
        Event::TestReportStored {
            status: rusty_bbp::RunStatus::Passed,
            ..
        }
    )));
    drop(host);
    drop(supervisor);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "requires built bbp at BBP_TEST_BIN and Linux moderator"]
fn real_supervisor_observes_the_bbp_lock_and_restarts_at_a_gate() {
    if !cfg!(target_os = "linux") {
        return;
    }
    let (dir, mut c) = setup();
    let (task, board) = task();
    let script = dir.join("plan.sh");
    std::fs::write(&script,r##""$BBP_BIN" mcp <<'RPC'
{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"script","version":"0"}}}
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"put_artifact","arguments":{"op":"p1","kind":"spec","brief":1,"body":"# Spec\nAC1."}}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"post","arguments":{"op":"p2","kind":"request_decision","to":["human"],"refs":["art:2"],"body":"Approve?","gate":true}}}
RPC
"##).unwrap();
    c.pool.get_mut("writer").unwrap().program = "/bin/sh".into();
    c.pool.get_mut("writer").unwrap().args = vec![script.to_string_lossy().into()];
    let mut host = BbpHost::new(c.clone(), "supervisor".into(), "context".into()).unwrap();
    let handle = host.open(&task, &board, &fence()).unwrap();
    let h = host.handle(&handle).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !matches!(host.poll(&handle, &fence()).unwrap(), Progress::Gate(_)) {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let frozen = BbpHost::<StdCommand, StdSupervisor>::load(&h).unwrap();
    let agents = BbpHost::<StdCommand, StdSupervisor>::manifest(&h).join("agents.json");
    let pid_path = c.dir.join("mod").join(format!(
        "{}.pid",
        rusty_bbp::Sha256::of(h.task.as_bytes()).hex()
    ));
    let first = std::fs::read_to_string(&pid_path).unwrap();
    assert_eq!(
        host.supervisor
            .ensure(&frozen.config, &h.task, &agents)
            .unwrap(),
        Supervised::Running
    );
    assert!(Command::new("kill")
        .args(["-KILL", first.trim()])
        .status()
        .unwrap()
        .success());
    std::thread::sleep(Duration::from_millis(50));
    assert!(matches!(
        host.poll(&handle, &fence()).unwrap(),
        Progress::Gate(_)
    ));
    let second = std::fs::read_to_string(pid_path).unwrap();
    assert_ne!(first, second);
    // Cancel through the real human channel; the detached moderator exits itself.
    let view = TaskView::from_events(rusty_bbp::TaskId(h.task.clone()), &events(&h));
    let out = Command::new(&c.binary)
        .args(["human", "--dir"])
        .arg(&h.dir)
        .args([
            "--task",
            &h.task,
            "--rev",
            &view.card.rev.0.to_string(),
            "cancel",
            "test finished",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let deadline = Instant::now() + Duration::from_secs(5);
    while Command::new("kill")
        .args(["-0", second.trim()])
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success()
    {
        assert!(Instant::now() < deadline, "moderator did not exit");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(matches!(
        host.poll(&handle, &fence()).unwrap(),
        Progress::Cancelled { .. }
    ));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "requires built bbp and rusty_orch at BBP_TEST_BIN and ORCH_TEST_BIN"]
fn orch_binary_reopens_its_durable_goal_without_opening_or_charging_twice() {
    let (dir, mut c) = setup();
    // No provider invocation: BBP pauses safely at an informational gate set
    // through the existing host CLI while Orch persists and resumes the handle.
    c.pool.get_mut("writer").unwrap().program = "/bin/sh".into();
    c.pool.get_mut("writer").unwrap().args = vec!["-c".into(), "sleep 10".into()];
    // A process that exits without posting stays bounded by the host's turn deadline.
    let goal = dir.join("goal.json");
    let cfg = dir.join("bbp.json");
    std::fs::write(&cfg, rusty_serde::json::to_string(&c).unwrap()).unwrap();
    std::fs::write(&goal,r#"{"goal":"Implement flag","done_when":["tested"],"out_of_scope":[],"wall_clock_secs":60,"max_calls":1,"stop":"checkpoint","tasks":[{"role":"implement","instruction":"Implement flag","acceptance":["tested"],"max_calls":1}]}"#).unwrap();
    let orch = std::env::var_os("ORCH_TEST_BIN").expect("set ORCH_TEST_BIN");
    let run = || {
        let out = Command::new(&orch)
            .arg("run")
            .arg(&goal)
            .arg("--bbp")
            .arg(&cfg)
            .arg("--state")
            .arg(dir.join("orch-state"))
            .arg("--json")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
        rusty_serde::json::from_str::<rusty_serde::json::Value>(
            &String::from_utf8(out.stdout).unwrap(),
        )
        .unwrap()
    };
    let first = run();
    let second = run();
    assert_eq!(first.get("calls").and_then(|v| v.as_u64()), Some(1));
    assert_eq!(second.get("calls").and_then(|v| v.as_u64()), Some(1));
    assert_eq!(
        second
            .get("ended")
            .unwrap()
            .get("kind")
            .and_then(|v| v.as_str()),
        Some("waiting")
    );
    let entry = second
        .get("entries")
        .unwrap()
        .as_seq()
        .unwrap()
        .iter()
        .find(|e| e.get("body").and_then(|v| v.as_str()) == Some("hosted task"))
        .unwrap();
    let pointer = entry.get("refs").unwrap().as_seq().unwrap()[0]
        .as_str()
        .unwrap()
        .strip_prefix("url:bbp:")
        .unwrap();
    let h: Handle = rusty_serde::json::from_str(pointer).unwrap();
    let log = events(&h);
    assert_eq!(
        log.iter()
            .filter(|e| matches!(e, Event::TaskOpened { .. }))
            .count(),
        1
    );
    let view = TaskView::from_events(rusty_bbp::TaskId(h.task.clone()), &log);
    let cancel = Command::new(&c.binary)
        .args(["human", "--dir"])
        .arg(&h.dir)
        .args([
            "--task",
            &h.task,
            "--rev",
            &view.card.rev.0.to_string(),
            "cancel",
            "test cleanup",
        ])
        .output()
        .unwrap();
    assert!(cancel.status.success());
    std::thread::sleep(Duration::from_millis(600));
    std::fs::remove_dir_all(dir).unwrap();
}

/// The task a report's hosted entry points at.
fn report_handle(report: &rusty_serde::json::Value) -> Handle {
    let entry = report["entries"]
        .as_seq()
        .unwrap()
        .iter()
        .find(|e| e["body"].as_str() == Some("hosted task"))
        .unwrap();
    let pointer = entry["refs"].as_seq().unwrap()[0]
        .as_str()
        .unwrap()
        .strip_prefix("url:bbp:")
        .unwrap();
    rusty_serde::json::from_str(pointer).unwrap()
}

/// Cancel the one task a report's hosted entry points at, so its harness
/// stops before the test directory goes; returns how often it was opened.
fn cancel_hosted(c: &Config, report: &rusty_serde::json::Value) -> usize {
    let h = report_handle(report);
    let log = events(&h);
    let opened = log
        .iter()
        .filter(|e| matches!(e, Event::TaskOpened { .. }))
        .count();
    let view = TaskView::from_events(rusty_bbp::TaskId(h.task.clone()), &log);
    let cancel = Command::new(&c.binary)
        .args(["human", "--dir"])
        .arg(&h.dir)
        .args([
            "--task",
            &h.task,
            "--rev",
            &view.card.rev.0.to_string(),
            "cancel",
            "test cleanup",
        ])
        .output()
        .unwrap();
    assert!(cancel.status.success());
    std::thread::sleep(Duration::from_millis(600));
    opened
}

#[test]
#[ignore = "requires built bbp and rusty_orch at BBP_TEST_BIN and ORCH_TEST_BIN"]
fn orch_binary_from_a_stale_state_copy_is_fenced_out_of_the_task() {
    let (dir, mut c) = setup();
    c.pool.get_mut("writer").unwrap().program = "/bin/sh".into();
    c.pool.get_mut("writer").unwrap().args = vec!["-c".into(), "sleep 10".into()];
    let goal = dir.join("goal.json");
    let cfg = dir.join("bbp.json");
    std::fs::write(&cfg, rusty_serde::json::to_string(&c).unwrap()).unwrap();
    std::fs::write(&goal, r#"{"goal":"Implement flag","done_when":["tested"],"out_of_scope":[],"wall_clock_secs":60,"max_calls":1,"stop":"checkpoint","tasks":[{"role":"implement","instruction":"Implement flag","acceptance":["tested"],"max_calls":1}]}"#).unwrap();
    let orch = std::env::var_os("ORCH_TEST_BIN").expect("set ORCH_TEST_BIN");
    let run = |state: &str| {
        let out = Command::new(&orch)
            .arg("run")
            .arg(&goal)
            .arg("--bbp")
            .arg(&cfg)
            .arg("--state")
            .arg(dir.join(state))
            .arg("--json")
            .output()
            .unwrap();
        let report = rusty_serde::json::from_str::<rusty_serde::json::Value>(
            &String::from_utf8(out.stdout).unwrap(),
        )
        .unwrap();
        (out.status.success(), report)
    };
    let (ok, first) = run("live");
    assert!(ok, "{first:?}");
    // A copy of the state as it stood after the first holder, e.g. a restored backup.
    let copied = Command::new("cp")
        .arg("-R")
        .arg(dir.join("live"))
        .arg(dir.join("copy"))
        .status()
        .unwrap();
    assert!(copied.success());
    let (ok, second) = run("live");
    assert!(ok, "{second:?}");
    // The copy reaches the same generation from its own record, so only the
    // holder distinguishes it. It must not drive the task.
    let (ok, stale) = run("copy");
    assert!(!ok);
    assert_eq!(stale["ended"]["kind"].as_str(), Some("failed"));
    assert!(
        stale["ended"]["error"]
            .as_str()
            .unwrap()
            .contains("held by another process"),
        "{stale:?}"
    );
    assert_eq!(stale["calls"].as_u64(), Some(1));
    assert_eq!(stale["tasks"][0]["status"].as_str(), Some("running"));
    // The live state takes over again past the generation the refused copy
    // recorded, and keeps the one task.
    let (ok, third) = run("live");
    assert!(ok, "{third:?}");
    assert_eq!(third["ended"]["kind"].as_str(), Some("waiting"));
    assert_eq!(cancel_hosted(&c, &third), 1);
    std::fs::remove_dir_all(dir).unwrap();
    println!("BBP_ACCEPTANCE PASS stale-holder");
}

#[test]
#[ignore = "requires built bbp and rusty_orch at BBP_TEST_BIN and ORCH_TEST_BIN; Linux moderator"]
fn orch_binary_admits_two_goals_through_one_agent_slot() {
    let (dir, mut c) = setup();
    let (probe, slots) = (dir.join("probe"), dir.join("slots"));
    std::fs::create_dir_all(&probe).unwrap();
    c.slots = Some(Slots {
        dir: slots.clone(),
        agents: 1,
        validators: 1,
    });
    // The writer records its lifetime, holds its slot a while, then plans to
    // the gate. Its record lies inside the slot hold.
    let script = dir.join("plan.sh");
    let record = format!("{}/$BBP_TASK", probe.display());
    std::fs::write(&script, format!(r##"echo start $(date +%s%N) >> {record}
sleep 1
"$BBP_BIN" mcp >/dev/null <<'RPC'
{{"jsonrpc":"2.0","id":0,"method":"initialize","params":{{"protocolVersion":"2025-06-18","capabilities":{{}},"clientInfo":{{"name":"script","version":"0"}}}}}}
{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"put_artifact","arguments":{{"op":"p1","kind":"spec","brief":1,"body":"# Spec\nAC1."}}}}}}
{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"post","arguments":{{"op":"p2","kind":"request_decision","to":["human"],"refs":["art:2"],"body":"Approve?","gate":true}}}}}}
RPC
echo end $(date +%s%N) >> {record}
"##)).unwrap();
    c.pool.get_mut("writer").unwrap().program = "/bin/sh".into();
    c.pool.get_mut("writer").unwrap().args = vec![script.to_string_lossy().into()];
    let cfg = dir.join("bbp.json");
    std::fs::write(&cfg, rusty_serde::json::to_string(&c).unwrap()).unwrap();
    let orch = std::env::var_os("ORCH_TEST_BIN").expect("set ORCH_TEST_BIN");
    // Two goals, so two tasks and two detached moderators, started together.
    let runs: Vec<Child> = ["one", "two"]
        .iter()
        .map(|name| {
            let goal = dir.join(format!("{name}.json"));
            std::fs::write(&goal, format!(r#"{{"goal":"Implement flag {name}","done_when":["tested"],"out_of_scope":[],"wall_clock_secs":60,"max_calls":1,"stop":"checkpoint","tasks":[{{"role":"implement","instruction":"Implement flag","acceptance":["tested"],"max_calls":1}}]}}"#)).unwrap();
            Command::new(&orch)
                .arg("run")
                .arg(&goal)
                .arg("--bbp")
                .arg(&cfg)
                .arg("--state")
                .arg(dir.join(format!("state-{name}")))
                .arg("--json")
                .stdout(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    let reports: Vec<rusty_serde::json::Value> = runs
        .into_iter()
        .map(|run| {
            let out = run.wait_with_output().unwrap();
            assert!(out.status.success());
            rusty_serde::json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap()
        })
        .collect();
    // Both writers finish, one after the other.
    let ended = || -> Vec<(u128, u128)> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&probe).unwrap() {
            let text = std::fs::read_to_string(entry.unwrap().path()).unwrap();
            let at: Vec<u128> = text
                .lines()
                .map(|l| l.split_once(' ').unwrap().1.parse().unwrap())
                .collect();
            if let [start, end] = at[..] {
                out.push((start, end));
            }
        }
        out
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    while ended().len() < 2 {
        assert!(Instant::now() < deadline, "both writers did not run");
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut spans = ended();
    spans.sort();
    assert!(
        spans[0].1 <= spans[1].0,
        "the second writer started before the first ended: {spans:?}"
    );
    assert_eq!(
        std::fs::read_to_string(slots.join("capacity")).unwrap(),
        "agents=1 validators=1\n"
    );
    for report in &reports {
        assert_eq!(cancel_hosted(&c, report), 1);
    }
    std::fs::remove_dir_all(dir).unwrap();
    println!("BBP_ACCEPTANCE PASS slot-admission");
}

/// A goal whose one Implement card's writer never ends its turn: it adds
/// its pid to `pids` at each launch, then writes a tick every 100 ms until
/// it is killed. One agent slot admits it.
struct Ticking {
    dir: PathBuf,
    c: Config,
    cfg: PathBuf,
    goal: PathBuf,
    pids: PathBuf,
    ticks: PathBuf,
    slots: PathBuf,
}

const TICKING_GOAL: &str = r#"{"goal":"Implement flag","done_when":["tested"],"out_of_scope":[],"wall_clock_secs":60,"max_calls":1,"stop":"checkpoint","tasks":[{"role":"implement","instruction":"Implement flag","acceptance":["tested"],"max_calls":1}]}"#;

fn ticking(end_grace_secs: u64) -> Ticking {
    let (dir, mut c) = setup();
    let slots = dir.join("slots");
    c.slots = Some(Slots {
        dir: slots.clone(),
        agents: 1,
        validators: 1,
    });
    c.end_grace_secs = Some(end_grace_secs);
    let (pids, ticks) = (dir.join("writer.pids"), dir.join("ticks"));
    let script = dir.join("tick.sh");
    std::fs::write(
        &script,
        format!(
            "echo $$ >> {}\nwhile :; do echo t >> {}; sleep 0.1; done\n",
            pids.display(),
            ticks.display()
        ),
    )
    .unwrap();
    c.pool.get_mut("writer").unwrap().program = "/bin/sh".into();
    c.pool.get_mut("writer").unwrap().args = vec![script.to_string_lossy().into()];
    let cfg = dir.join("bbp.json");
    std::fs::write(&cfg, rusty_serde::json::to_string(&c).unwrap()).unwrap();
    let goal = dir.join("goal.json");
    std::fs::write(&goal, TICKING_GOAL).unwrap();
    Ticking {
        dir,
        c,
        cfg,
        goal,
        pids,
        ticks,
        slots,
    }
}

impl Ticking {
    fn command(&self, state: &str, extra: &[&str]) -> Command {
        let mut command =
            Command::new(std::env::var_os("ORCH_TEST_BIN").expect("set ORCH_TEST_BIN"));
        command
            .arg("run")
            .arg(&self.goal)
            .arg("--bbp")
            .arg(&self.cfg)
            .arg("--state")
            .arg(self.dir.join(state))
            .arg("--json")
            .args(extra);
        command
    }
    fn run(&self, extra: &[&str]) -> std::process::Output {
        self.command("state", extra).output().unwrap()
    }
    fn report(out: &std::process::Output) -> rusty_serde::json::Value {
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        rusty_serde::json::from_str(&String::from_utf8(out.stdout.clone()).unwrap()).unwrap()
    }
    /// The writer's pid once it has started ticking.
    fn writer(&self) -> String {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !(self.ticks.exists() && self.pids.exists()) {
            assert!(Instant::now() < deadline, "the writer never started");
            std::thread::sleep(Duration::from_millis(50));
        }
        let pids = std::fs::read_to_string(&self.pids).unwrap();
        pids.lines().last().unwrap().to_owned()
    }
    fn launches(&self) -> usize {
        std::fs::read_to_string(&self.pids).unwrap().lines().count()
    }
    /// Kill the task's detached moderator outright, as a crash would.
    fn kill_moderator(&self, h: &Handle) {
        let key = rusty_bbp::Sha256::of(h.task.as_bytes()).hex();
        let pid =
            std::fs::read_to_string(self.c.dir.join("mod").join(format!("{key}.pid"))).unwrap();
        assert!(Command::new("kill")
            .args(["-KILL", pid.trim()])
            .status()
            .unwrap()
            .success());
        let deadline = Instant::now() + Duration::from_secs(10);
        while running(pid.trim()) {
            assert!(Instant::now() < deadline, "the moderator did not die");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    /// What a finished cancel must mean: the writer is gone, nothing writes
    /// any more, and the agent slot is free.
    fn assert_stopped(&self, writer: &str) {
        assert!(!running(writer), "writer {writer} still runs");
        let count = || {
            std::fs::read_to_string(&self.ticks)
                .unwrap()
                .lines()
                .count()
        };
        let before = count();
        std::thread::sleep(Duration::from_millis(500));
        assert_eq!(count(), before, "nothing writes after the cancel");
        let slot = std::fs::OpenOptions::new()
            .write(true)
            .open(self.slots.join("agent-0.lock"))
            .unwrap();
        assert!(slot.try_lock().is_ok(), "the agent slot is free");
    }
}

/// Whether `pid` runs: a zombie has exited and waits only to be reaped.
fn running(pid: &str) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        !stat
            .rsplit_once(") ")
            .is_some_and(|(_, rest)| rest.starts_with('Z'))
    })
}

#[test]
#[ignore = "requires built bbp and rusty_orch at BBP_TEST_BIN and ORCH_TEST_BIN; Linux moderator"]
fn orch_binary_cancel_reaps_a_live_turn_frees_its_slot_and_retires_the_goal() {
    let t = ticking(1);
    let first = Ticking::report(&t.run(&[]));
    assert_eq!(first["ended"]["kind"].as_str(), Some("waiting"));
    let h = report_handle(&first);
    let writer = t.writer();

    let started = Instant::now();
    let cancelled = Ticking::report(&t.run(&["--cancel", "owner withdrew the card"]));
    assert_eq!(cancelled["ended"]["kind"].as_str(), Some("cancelled"));
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "cancel took {:?}",
        started.elapsed()
    );
    t.assert_stopped(&writer);
    let view = TaskView::from_events(rusty_bbp::TaskId(h.task.clone()), &events(&h));
    assert_eq!(view.card.state, rusty_bbp::State::Cancelled);
    assert_eq!(view.cancelled.as_deref(), Some("owner withdrew the card"));

    // The goal's assignment is retired: a later run is refused before it
    // hosts anything, and cancelling again changes nothing.
    let resumed = t.run(&[]);
    assert!(!resumed.status.success());
    assert!(
        String::from_utf8_lossy(&resumed.stderr).contains("Retired"),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let logged = events(&h).len();
    let again = Ticking::report(&t.run(&["--cancel", "owner withdrew the card"]));
    assert_eq!(again["ended"]["kind"].as_str(), Some("cancelled"));
    assert_eq!(events(&h).len(), logged, "a second cancel records nothing");
    assert_eq!(t.launches(), 1);
    std::fs::remove_dir_all(&t.dir).unwrap();
    println!("BBP_ACCEPTANCE PASS orch-cancel");
}

#[test]
#[ignore = "requires built bbp and rusty_orch at BBP_TEST_BIN and ORCH_TEST_BIN; Linux moderator"]
fn orch_binary_cancel_stops_the_writer_a_dead_moderator_left_behind() {
    let t = ticking(1);
    let h = report_handle(&Ticking::report(&t.run(&[])));
    let writer = t.writer();
    t.kill_moderator(&h);
    assert!(
        running(&writer),
        "the orphaned writer outlives its moderator"
    );

    let cancelled = Ticking::report(&t.run(&["--cancel", "moderator lost"]));
    assert_eq!(cancelled["ended"]["kind"].as_str(), Some("cancelled"));
    t.assert_stopped(&writer);
    assert_eq!(t.launches(), 1, "the cancel started nothing");
    std::fs::remove_dir_all(&t.dir).unwrap();
    println!("BBP_ACCEPTANCE PASS orch-cancel-moderator-lost");
}

#[test]
#[ignore = "requires built bbp and rusty_orch at BBP_TEST_BIN and ORCH_TEST_BIN; Linux moderator"]
fn orch_binary_cancel_completes_when_the_moderator_dies_while_it_waits() {
    // A long end grace keeps the moderator draining while the cancel waits.
    let t = ticking(30);
    let h = report_handle(&Ticking::report(&t.run(&[])));
    let writer = t.writer();
    let started = Instant::now();
    let cancel = t
        .command("state", &["--cancel", "moderator lost mid-cancel"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while TaskView::from_events(rusty_bbp::TaskId(h.task.clone()), &events(&h))
        .card
        .state
        != rusty_bbp::State::Cancelled
    {
        assert!(Instant::now() < deadline, "the cancel was never recorded");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(running(&writer), "the writer is still in its end grace");
    t.kill_moderator(&h);
    let out = cancel.wait_with_output().unwrap();
    assert_eq!(
        Ticking::report(&out)["ended"]["kind"].as_str(),
        Some("cancelled")
    );
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "the cancel waited out the grace: {:?}",
        started.elapsed()
    );
    t.assert_stopped(&writer);
    std::fs::remove_dir_all(&t.dir).unwrap();
    println!("BBP_ACCEPTANCE PASS orch-cancel-mid-cancel");
}

#[test]
#[ignore = "requires built bbp and rusty_orch at BBP_TEST_BIN and ORCH_TEST_BIN; Linux moderator"]
fn orch_binary_cancel_finds_work_started_before_its_handle_was_checkpointed() {
    let t = ticking(1);
    // A run that crashed after its host opened the card, and so started its
    // moderator, but before the handle reached the checkpointed board.
    let (task, board) = task();
    let namespace = rusty_bbp::Sha256::of(TICKING_GOAL.as_bytes()).hex();
    let mut host = BbpHost::new(t.c.clone(), namespace, "crashed run".into()).unwrap();
    let handle = host.open(&task, &board, &fence_at(0, "crashed")).unwrap();
    let h = host.handle(&handle).unwrap();
    let writer = t.writer();

    // The cancel starts from a state with no handle at all.
    let cancelled = Ticking::report(&t.run(&["--cancel", "crashed before checkpoint"]));
    assert_eq!(cancelled["ended"]["kind"].as_str(), Some("cancelled"));
    t.assert_stopped(&writer);
    assert_eq!(t.launches(), 1, "the cancel launched nothing");
    let log = events(&h);
    assert_eq!(
        log.iter()
            .filter(|e| matches!(e, Event::TaskOpened { .. }))
            .count(),
        1,
        "the cancel adopted the crashed run's task"
    );
    let view = TaskView::from_events(rusty_bbp::TaskId(h.task.clone()), &log);
    assert_eq!(view.card.state, rusty_bbp::State::Cancelled);
    std::fs::remove_dir_all(&t.dir).unwrap();
    println!("BBP_ACCEPTANCE PASS orch-cancel-unrecorded-open");
}

#[test]
#[ignore = "requires built bbp and rusty_orch at BBP_TEST_BIN and ORCH_TEST_BIN; enforced Linux sandbox"]
fn orch_binary_confines_a_sandboxed_pool_member_away_from_the_bbp_directory() {
    let (dir, mut c) = setup();
    // The writer lives outside the BBP directory, which is all it may read
    // besides the system and `bbp`; it probes the directory, then plans.
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let script = bin.join("plan.sh");
    std::fs::write(&script, r##"d="$1"
if cat "$d/master" >/dev/null 2>&1; then echo "master: read"; else echo "master: denied"; fi
if "$BBP_BIN" card --dir "$d" --task "$BBP_TASK" >/dev/null 2>&1; then echo "log: read"; else echo "log: denied"; fi
"$BBP_BIN" mcp <<'RPC'
{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"script","version":"0"}}}
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"put_artifact","arguments":{"op":"p1","kind":"spec","brief":1,"body":"# Spec\nAC1."}}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"post","arguments":{"op":"p2","kind":"request_decision","to":["human"],"refs":["art:2"],"body":"Approve?","gate":true}}}
RPC
"##).unwrap();
    let writer = c.pool.get_mut("writer").unwrap();
    writer.program = "/bin/sh".into();
    writer.args = vec![script.to_string_lossy().into(), "{dir}".into()];
    writer.sandbox = Some(Sandbox {
        read: vec![bin.to_string_lossy().into()],
        ..Sandbox::default()
    });
    let cfg = dir.join("bbp.json");
    std::fs::write(&cfg, rusty_serde::json::to_string(&c).unwrap()).unwrap();
    let goal = dir.join("goal.json");
    std::fs::write(&goal, r#"{"goal":"Implement flag","done_when":["tested"],"out_of_scope":[],"wall_clock_secs":60,"max_calls":1,"stop":"checkpoint","tasks":[{"role":"implement","instruction":"Implement flag","acceptance":["tested"],"max_calls":1}]}"#).unwrap();
    let orch = std::env::var_os("ORCH_TEST_BIN").expect("set ORCH_TEST_BIN");
    let out = Command::new(&orch)
        .arg("run")
        .arg(&goal)
        .arg("--bbp")
        .arg(&cfg)
        .arg("--state")
        .arg(dir.join("orch-state"))
        .arg("--json")
        .output()
        .unwrap();
    // A fast writer can reach the plan gate before `run` returns, which then
    // reports the task blocked on that question (exit 3) instead of running.
    assert!(
        matches!(out.status.code(), Some(0 | 3)),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: rusty_serde::json::Value =
        rusty_serde::json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    let entry = report["entries"]
        .as_seq()
        .unwrap()
        .iter()
        .find(|e| e["body"].as_str() == Some("hosted task"))
        .unwrap();
    let pointer = entry["refs"].as_seq().unwrap()[0]
        .as_str()
        .unwrap()
        .strip_prefix("url:bbp:")
        .unwrap();
    let h: Handle = rusty_serde::json::from_str(pointer).unwrap();
    let log = h
        .dir
        .join("agents")
        .join(rusty_bbp::Sha256::of(h.task.as_bytes()).hex())
        .join("turn-1.log");
    // The writer reaches the gate through its relay.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let view = TaskView::from_events(rusty_bbp::TaskId(h.task.clone()), &events(&h));
        if view.card.state == rusty_bbp::State::PlanGate {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "no plan gate; writer said:\n{}",
            std::fs::read_to_string(&log).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let said = std::fs::read_to_string(&log).unwrap();
    println!("{said}");
    assert!(said.contains("master: denied"), "{said}");
    assert!(said.contains("log: denied"), "{said}");
    assert_eq!(cancel_hosted(&c, &report), 1);
    std::fs::remove_dir_all(dir).unwrap();
    println!("BBP_ACCEPTANCE PASS harness-confined");
}

#[test]
#[ignore = "requires built bbp and rusty_orch at BBP_TEST_BIN and ORCH_TEST_BIN; enforced Linux sandbox"]
fn orch_binary_confines_a_pool_member_to_the_goals_lease() {
    let (dir, mut c) = setup();
    // The pool member may read `secret`; the goal's lease reaches only `bin`.
    let (bin, secret) = (dir.join("bin"), dir.join("secret"));
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&secret).unwrap();
    std::fs::write(secret.join("token"), "s3cret").unwrap();
    let script = bin.join("plan.sh");
    std::fs::write(&script, r##"if cat "$1/token" >/dev/null 2>&1; then echo "secret: read"; else echo "secret: denied"; fi
"$BBP_BIN" mcp <<'RPC'
{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"script","version":"0"}}}
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"put_artifact","arguments":{"op":"p1","kind":"spec","brief":1,"body":"# Spec\nAC1."}}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"post","arguments":{"op":"p2","kind":"request_decision","to":["human"],"refs":["art:2"],"body":"Approve?","gate":true}}}
RPC
"##).unwrap();
    let path = |p: &std::path::Path| p.to_string_lossy().into_owned();
    for agent in c.pool.values_mut() {
        agent.sandbox = Some(Sandbox {
            read: vec![path(&bin), path(&secret)],
            ..Sandbox::default()
        });
    }
    let writer = c.pool.get_mut("writer").unwrap();
    writer.program = "/bin/sh".into();
    writer.args = vec![path(&script), path(&secret)];
    let mut hosting = rusty_serde::json::to_value(&c).unwrap();
    let lease = format!(r#"{{"read":[{:?}],"ttl_secs":600}}"#, path(&bin));
    let lease: rusty_serde::json::Value = rusty_serde::json::from_str(&lease).unwrap();
    hosting.insert("lease", lease);
    let cfg = dir.join("bbp.json");
    std::fs::write(&cfg, rusty_serde::json::to_string(&hosting).unwrap()).unwrap();
    let goal = dir.join("goal.json");
    std::fs::write(&goal, r#"{"goal":"Implement flag","done_when":["tested"],"out_of_scope":[],"wall_clock_secs":60,"max_calls":1,"stop":"checkpoint","tasks":[{"role":"implement","instruction":"Implement flag","acceptance":["tested"],"max_calls":1}]}"#).unwrap();
    let orch = std::env::var_os("ORCH_TEST_BIN").expect("set ORCH_TEST_BIN");
    let out = Command::new(&orch)
        .arg("run")
        .arg(&goal)
        .arg("--bbp")
        .arg(&cfg)
        .arg("--state")
        .arg(dir.join("orch-state"))
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        matches!(out.status.code(), Some(0 | 3)),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: rusty_serde::json::Value =
        rusty_serde::json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    let h = report_handle(&report);
    let log = h
        .dir
        .join("agents")
        .join(rusty_bbp::Sha256::of(h.task.as_bytes()).hex())
        .join("turn-1.log");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let view = TaskView::from_events(rusty_bbp::TaskId(h.task.clone()), &events(&h));
        if view.card.state == rusty_bbp::State::PlanGate {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "no plan gate; writer said:\n{}",
            std::fs::read_to_string(&log).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let said = std::fs::read_to_string(&log).unwrap();
    println!("{said}");
    assert!(said.contains("secret: denied"), "{said}");
    // The pool member could read `secret`; what the moderator launched could not.
    let agents: rusty_serde::json::Value = rusty_serde::json::from_str(
        &std::fs::read_to_string(
            BbpHost::<StdCommand, StdSupervisor>::manifest(&h).join("agents.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let read = agents["maker"]["sandbox"]["read"].as_seq().unwrap();
    assert_eq!(read.len(), 1, "{read:?}");
    assert_eq!(
        read[0].as_str(),
        Some(path(&bin.canonicalize().unwrap()).as_str())
    );
    assert_eq!(cancel_hosted(&c, &report), 1);
    std::fs::remove_dir_all(dir).unwrap();
    println!("BBP_ACCEPTANCE PASS lease-confined");
}

/// A real in-memory Millboard on a loopback port with a shared token and the
/// default roles, stopped on drop.
struct Millboard {
    child: Child,
    url: String,
}
impl Millboard {
    const TOKEN: &'static str = "acceptance-token";
    fn start() -> Self {
        let bin = std::env::var_os("MILLBOARD_TEST_BIN").expect("set MILLBOARD_TEST_BIN");
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let child = Command::new(bin)
            .args(["--http", &format!("127.0.0.1:{port}")])
            .args(["--auth-token", Self::TOKEN])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let board = Self {
            child,
            url: format!("http://127.0.0.1:{port}"),
        };
        let started = Instant::now();
        while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(
                started.elapsed() < Duration::from_secs(20),
                "millboard did not start"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        board
    }
    fn call(&self, op: &str, args: &str) -> rusty_serde::json::Value {
        let out = Command::new("curl")
            .args(["-sS", "-X", "POST", "-H", "Content-Type: application/json"])
            .args(["-H", &format!("Authorization: Bearer {}", Self::TOKEN)])
            .args(["--data", args, &format!("{}/api/{op}", self.url)])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        rusty_serde::json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap()
    }
}
impl Drop for Millboard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
#[ignore = "requires built bbp, rusty_orch and rusty_mill_board at BBP_TEST_BIN, ORCH_TEST_BIN and MILLBOARD_TEST_BIN"]
fn orch_binary_records_each_generation_with_its_millboard_card() {
    let (dir, mut c) = setup();
    c.pool.get_mut("writer").unwrap().program = "/bin/sh".into();
    // The writer records whether it can see Orch's board token, then idles.
    let seen = dir.join("writer-saw-token");
    let script = format!(
        "printf %s \"${{{}-unset}}\" > '{}'; sleep 10",
        orch_cli::BOARD_TOKEN_ENV,
        seen.display()
    );
    c.pool.get_mut("writer").unwrap().args = vec!["-c".into(), script];
    let goal = dir.join("goal.json");
    let cfg = dir.join("bbp.json");
    std::fs::write(&cfg, rusty_serde::json::to_string(&c).unwrap()).unwrap();
    let goal_json = r#"{"goal":"Implement flag","done_when":["tested"],"out_of_scope":[],"wall_clock_secs":60,"max_calls":1,"stop":"checkpoint","tasks":[{"role":"implement","instruction":"Implement flag","acceptance":["tested"],"max_calls":1}]}"#;
    std::fs::write(&goal, goal_json).unwrap();
    let board = Millboard::start();
    let added = board.call(
        "card_add",
        r#"{"actor":"nano","role":"human","title":"Implement flag"}"#,
    );
    let card = added["card"]["id"].as_str().unwrap().to_owned();
    let stored = || {
        let got = board.call("card_get", &format!(r#"{{"id":"{card}"}}"#));
        got["card"]["execution"].clone()
    };
    let orch = std::env::var_os("ORCH_TEST_BIN").expect("set ORCH_TEST_BIN");
    let run = |state: &str| {
        Command::new(&orch)
            .arg("run")
            .arg(&goal)
            .arg("--bbp")
            .arg(&cfg)
            .arg("--state")
            .arg(dir.join(state))
            .args(["--card", &card, "--board", &board.url, "--json"])
            .env(orch_cli::BOARD_TOKEN_ENV, Millboard::TOKEN)
            .env("RUSTY_ORCH_BOARD_ACTOR", "rusty_orch")
            .env("RUSTY_ORCH_BOARD_ROLE", "builder")
            .output()
            .unwrap()
    };
    let report = |out: &std::process::Output| {
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        rusty_serde::json::from_str::<rusty_serde::json::Value>(
            &String::from_utf8(out.stdout.clone()).unwrap(),
        )
        .unwrap()
    };
    let first = report(&run("live"));
    assert_eq!(first["ended"]["kind"].as_str(), Some("waiting"));
    // The token reached Orch's own Millboard calls, but no hosted child: the
    // supervisor, moderator and harness all run without it.
    let started = Instant::now();
    while !seen.exists() {
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "writer never ran"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(std::fs::read_to_string(&seen).unwrap(), "unset");
    let link = stored();
    assert_eq!(link["generation"].as_u64(), Some(1));
    assert_eq!(link["assignment"].as_str(), Some("goal-orchestrator"));
    let namespace = rusty_bbp::Sha256::of(goal_json.as_bytes()).hex();
    assert_eq!(link["execution"].as_str(), Some(namespace.as_str()));
    // A copy of the state as it stood at generation 1, e.g. a restored backup.
    let copied = Command::new("cp")
        .arg("-R")
        .arg(dir.join("live"))
        .arg(dir.join("copy"))
        .status()
        .unwrap();
    assert!(copied.success());
    report(&run("live"));
    report(&run("live"));
    assert_eq!(stored()["generation"].as_u64(), Some(3));
    // The copy takes over at generation 2, which the card has moved past: it
    // stops before any host command, and the card is unchanged.
    let stale = run("copy");
    assert!(!stale.status.success());
    let why = String::from_utf8_lossy(&stale.stderr);
    assert!(why.contains("superseded"), "{why}");
    assert!(stale.stdout.is_empty());
    assert_eq!(stored()["generation"].as_u64(), Some(3));
    let last = report(&run("live"));
    assert_eq!(last["ended"]["kind"].as_str(), Some("waiting"));
    assert_eq!(stored()["generation"].as_u64(), Some(4));
    // Resuming the linked state without its card would take a generation
    // the card never sees, so it is refused before anything is saved.
    let unlinked = Command::new(&orch)
        .arg("run")
        .arg(&goal)
        .arg("--bbp")
        .arg(&cfg)
        .arg("--state")
        .arg(dir.join("live"))
        .output()
        .unwrap();
    assert!(!unlinked.status.success());
    let why = String::from_utf8_lossy(&unlinked.stderr);
    assert!(why.contains("resume it with --card"), "{why}");
    let resumed = report(&run("live"));
    assert_eq!(resumed["ended"]["kind"].as_str(), Some("waiting"));
    assert_eq!(
        stored()["generation"].as_u64(),
        Some(5),
        "no generation was skipped"
    );
    assert_eq!(cancel_hosted(&c, &resumed), 1);
    drop(board);
    std::fs::remove_dir_all(dir).unwrap();
    println!("BBP_ACCEPTANCE PASS millboard-link");
}

#[test]
#[ignore = "requires BBP_TEST_BIN; full dispatcher correction path"]
fn dispatcher_allows_invalid_subject_and_missing_receipt_to_be_corrected() {
    use orch_core::goal::{Goal, GoalDraft, StopRule};
    use orch_core::task::Plan;
    use orch_core::GoalId;
    use orch_dispatch::{fake::FakeAgent, Dispatcher, Ledger, Outcome, Routing, RoutingConfig};
    let (dir, c) = scripted_config();
    let (task, _) = task();
    let mut plan = Plan::new(GoalId::from_raw(1));
    let id = plan.add(task.spec().clone()).unwrap();
    let mut board = Board::new(plan.goal());
    let mut ledger = Ledger::new();
    let goal = Goal::try_from(GoalDraft {
        outcome: Some("flag".into()),
        done_when: vec!["tested".into()],
        out_of_scope: Some(vec![]),
        wall_clock_secs: Some(60),
        max_calls: Some(1),
        stop: Some(StopRule::Checkpoint),
        ..GoalDraft::default()
    })
    .unwrap();
    let route = Routing::try_from(RoutingConfig {
        research: orch_core::task::Agent::Codex,
        design: orch_core::task::Agent::Codex,
        implement: orch_core::task::Agent::Codex,
        triage: orch_core::task::Agent::Local,
        reviewers: vec![
            orch_core::task::Agent::Claude,
            orch_core::task::Agent::Codex,
        ],
    })
    .unwrap();
    let supervisor = ScriptSupervisor::default();
    let host = BbpHost::with_seams(
        c,
        "corrections".into(),
        "flag".into(),
        StdCommand,
        supervisor.clone(),
    )
    .unwrap();
    let mut dispatcher = Dispatcher::new(route, FakeAgent::new([])).with_host(host, fence());
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut invalid_plan = false;
    let mut invalid_receipt = false;
    let mut corrected_plan = false;
    let mut corrected_receipt = false;
    loop {
        assert!(
            Instant::now() < deadline,
            "dispatcher stalled: {:?}",
            plan.get(id).unwrap().state()
        );
        let outcome = dispatcher
            .run(&goal, &mut plan, &mut board, &mut ledger)
            .unwrap();
        if outcome == Outcome::Finished {
            break;
        }
        if matches!(outcome, Outcome::Blocked(_)) {
            let question = board.open_questions()[0];
            let qid = question.id();
            let prompt = question.content().body.as_str();
            let body = if prompt.contains("approve-plan") {
                if !invalid_plan {
                    invalid_plan = true;
                    "approve-plan art:99"
                } else {
                    assert!(prompt.contains("Answer rejected"));
                    corrected_plan = true;
                    "approve-plan art:2"
                }
            } else if prompt.contains("approve-merge") {
                "approve-merge art:4 run:1"
            } else if prompt.contains("receipt") {
                if !invalid_receipt {
                    invalid_receipt = true;
                    "receipt art:4"
                } else {
                    assert!(prompt.contains("BadHumanInput"));
                    corrected_receipt = true;
                    "receipt art:4 merged"
                }
            } else {
                panic!("unexpected gate {prompt}")
            };
            board
                .append(NewEntry {
                    task: Some(id),
                    author: Author::Human,
                    kind: EntryKind::Answer { to: qid },
                    body: txt(body),
                    refs: vec![],
                    supersedes: None,
                })
                .unwrap();
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(corrected_plan && corrected_receipt);
    assert_eq!(ledger.calls(), 1);
    assert!(board.open_questions().is_empty());
    drop(dispatcher);
    drop(supervisor);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "requires BBP_TEST_BIN; concurrent workspaces and adoption"]
fn concurrent_tasks_keep_run_one_checkouts_isolated_during_adoption() {
    let (dir, c) = scripted_config();
    let (task, board) = task();
    let mut profile = std::fs::read_to_string(&c.profiles).unwrap();
    profile = profile.replace(
        "test \\\"$(cat flag)",
        "echo $$ > running.pid; sleep 2; test \\\"$(cat flag)",
    );
    std::fs::write(&c.profiles, profile).unwrap();
    let sa = ScriptSupervisor::default();
    let sb = ScriptSupervisor::default();
    let make = |namespace: &str, s: ScriptSupervisor| {
        BbpHost::with_seams(c.clone(), namespace.into(), "flag".into(), StdCommand, s).unwrap()
    };
    let mut a = make("a", sa.clone());
    let mut b = make("b", sb.clone());
    let ha = a.open(&task, &board, &fence()).unwrap();
    let hb = b.open(&task, &board, &fence()).unwrap();
    let da = a.handle(&ha).unwrap();
    let db = b.handle(&hb).unwrap();
    let wa = supervisor::task_work(&c, &da.task).join("run-1");
    let wb = supervisor::task_work(&c, &db.task).join("run-1");
    assert_ne!(wa, wb);
    let mut ba = board.clone();
    let mut bb = board.clone();
    let ga = wait_gate(&mut a, &ha, "approve-plan");
    answer(&mut a, &ha, &mut ba, ga, "approve-plan art:2");
    let gb = wait_gate(&mut b, &hb, "approve-plan");
    answer(&mut b, &hb, &mut bb, gb, "approve-plan art:2");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !wa.join("running.pid").exists() || !wb.join("running.pid").exists() {
        assert!(Instant::now() < deadline, "both workloads did not overlap");
        a.poll(&ha, &fence()).unwrap();
        b.poll(&hb, &fence()).unwrap();
        std::thread::sleep(Duration::from_millis(10));
    }
    std::fs::write(wa.join("sentinel"), "a").unwrap();
    std::fs::write(wb.join("sentinel"), "b").unwrap();
    drop(a);
    let mut a = make("a", sa.clone());
    assert_eq!(a.open(&task, &board, &fence()).unwrap(), ha);
    assert_eq!(std::fs::read_to_string(wa.join("sentinel")).unwrap(), "a");
    assert_eq!(std::fs::read_to_string(wb.join("sentinel")).unwrap(), "b");
    let ga = wait_gate(&mut a, &ha, "approve-merge");
    let gb = wait_gate(&mut b, &hb, "approve-merge");
    answer(&mut a, &ha, &mut ba, ga, "approve-merge art:4 run:1");
    answer(&mut b, &hb, &mut bb, gb, "approve-merge art:4 run:1");
    let ga = wait_gate(&mut a, &ha, "receipt");
    let gb = wait_gate(&mut b, &hb, "receipt");
    answer(&mut a, &ha, &mut ba, ga, "receipt art:4 a");
    answer(&mut b, &hb, &mut bb, gb, "receipt art:4 b");
    assert!(matches!(
        a.poll(&ha, &fence()).unwrap(),
        Progress::Closed { .. }
    ));
    assert!(matches!(
        b.poll(&hb, &fence()).unwrap(),
        Progress::Closed { .. }
    ));
    drop(a);
    drop(b);
    drop(sa);
    drop(sb);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "requires BBP_TEST_BIN; expired request recovery through real host"]
fn expired_build_request_can_be_settled_before_resuming() {
    use rusty_bbp::{Command as BbpCommand, Driver, State as BbpState, TaskState, Time};
    let (dir, c) = scripted_config();
    let (task, mut board) = task();
    let maker = dir.join("maker.sh");
    let script = std::fs::read_to_string(&maker).unwrap();
    let request = r#"if [ ! -f "$BBP_DIR/requested" ]; then
        touch "$BBP_DIR/requested"
        "$BBP_BIN" mcp <<'RPC'
{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"script","version":"0"}}}
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"post","arguments":{"op":"question","kind":"request_decision","to":["human"],"refs":["art:2"],"body":"Which flag value?","gate":false}}}
RPC
        exit 0
    fi
"#;
    let needle = "*'\"Build\"'*) ";
    assert!(script.contains(needle));
    std::fs::write(
        maker,
        script.replace(needle, &format!("{needle}\n{request}")),
    )
    .unwrap();
    let supervisor = ScriptSupervisor::default();
    let mut host = BbpHost::with_seams(
        c,
        "expiry".into(),
        "flag".into(),
        StdCommand,
        supervisor.clone(),
    )
    .unwrap();
    let handle = host.open(&task, &board, &fence()).unwrap();
    let h = host.handle(&handle).unwrap();
    let gate = wait_gate(&mut host, &handle, "approve-plan");
    answer(&mut host, &handle, &mut board, gate, "approve-plan art:2");
    let gate = wait_gate(&mut host, &handle, "Which flag value?");
    let request_id = gate
        .ids
        .iter()
        .find(|s| s.as_str().starts_with("msg:"))
        .unwrap()
        .as_str()
        .to_owned();
    let task_id = rusty_bbp::TaskId(h.task.clone());
    let state = TaskState::fold(task_id.clone(), &events(&h));
    assert_eq!(state.state, BbpState::Build);
    let expires = state.pending_request.unwrap().deadline.unwrap();
    let mut driver = Driver::new(FsStore::open(&h.dir).unwrap(), task_id);
    driver.reload().unwrap();
    driver
        .dispatch(&BbpCommand::Tick, Time(expires.0 + 1))
        .unwrap();
    let gate = wait_gate(&mut host, &handle, "Escalated");
    assert!(gate.prompt.as_str().contains("Which flag value?"));
    assert!(gate.ids.iter().any(|id| id.as_str() == request_id));
    answer(
        &mut host,
        &handle,
        &mut board,
        gate,
        &format!("decision {request_id} accept ok"),
    );
    // Settling a request grants the requester a consultation turn. Let this
    // fixture's empty escalated-state harness forfeit it before displaying
    // the separate resume confirmation, so its fence is not already obsolete.
    let deadline = Instant::now() + Duration::from_secs(3);
    while !TaskState::fold(rusty_bbp::TaskId(h.task.clone()), &events(&h))
        .turns
        .is_empty()
    {
        assert!(Instant::now() < deadline);
        host.poll(&handle, &fence()).unwrap();
        std::thread::sleep(Duration::from_millis(20));
    }
    let gate = wait_gate(&mut host, &handle, "resume STATE");
    answer(&mut host, &handle, &mut board, gate, "resume build");
    let gate = wait_gate(&mut host, &handle, "approve-merge");
    answer(
        &mut host,
        &handle,
        &mut board,
        gate,
        "approve-merge art:4 run:1",
    );
    let gate = wait_gate(&mut host, &handle, "receipt");
    answer(&mut host, &handle, &mut board, gate, "receipt art:4 merged");
    assert!(matches!(
        host.poll(&handle, &fence()).unwrap(),
        Progress::Closed { .. }
    ));
    drop(host);
    drop(supervisor);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "requires built bbp at BBP_TEST_BIN and rusty_orch at ORCH_TEST_BIN"]
fn no_progress_pause_survives_orch_processes_without_duplicate_task_or_charge() {
    let (dir, mut c) = setup();
    c.pool.get_mut("writer").unwrap().program = "/bin/sh".into();
    // Idle a moment before quitting: an event-driven moderator reaps an
    // instant exit at once, and three of them would pause the task before
    // this first `orch run` reaches its checkpoint.
    c.pool.get_mut("writer").unwrap().args = vec!["-c".into(), "sleep 1; exit 0".into()];
    std::fs::write(
        dir.join("bbp.json"),
        rusty_serde::json::to_string(&c).unwrap(),
    )
    .unwrap();
    std::fs::write(dir.join("goal.json"), r#"{"goal":"Implement flag","done_when":["tested"],"out_of_scope":[],"wall_clock_secs":60,"max_calls":1,"stop":"checkpoint","tasks":[{"role":"implement","instruction":"Implement flag","acceptance":["tested"],"max_calls":1}]}"#).unwrap();
    let orch = std::env::var_os("ORCH_TEST_BIN").expect("set ORCH_TEST_BIN");
    let invoke = || {
        Command::new(&orch)
            .arg("run")
            .arg(dir.join("goal.json"))
            .arg("--bbp")
            .arg(dir.join("bbp.json"))
            .arg("--state")
            .arg(dir.join("orch-state"))
            .arg("--json")
            .output()
            .unwrap()
    };
    let first = invoke();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let report: rusty_serde::json::Value =
        rusty_serde::json::from_str(std::str::from_utf8(&first.stdout).unwrap()).unwrap();
    assert_eq!(
        report
            .get("calls")
            .and_then(rusty_serde::json::Value::as_u64),
        Some(1)
    );
    let entries = report.get("entries").unwrap().as_seq().unwrap();
    let pointer = entries[0].get("refs").unwrap().as_seq().unwrap()[0]
        .as_str()
        .unwrap()
        .strip_prefix("url:bbp:")
        .unwrap();
    let h: Handle = rusty_serde::json::from_str(pointer).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let health = Command::new(&c.binary)
            .args(["mod-health", "--dir"])
            .arg(&c.dir)
            .args(["--task", &h.task, "--check", "yes"])
            .output()
            .unwrap();
        if !health.status.success() {
            assert!(String::from_utf8_lossy(&health.stderr).contains("moderator paused"));
            break;
        }
        assert!(Instant::now() < deadline, "moderator did not pause");
        std::thread::sleep(Duration::from_millis(10));
    }
    let before = events(&h);
    for _ in 0..2 {
        let resumed = invoke();
        assert!(!resumed.status.success());
        let report: rusty_serde::json::Value =
            rusty_serde::json::from_str(std::str::from_utf8(&resumed.stdout).unwrap()).unwrap();
        assert_eq!(report["ended"]["kind"].as_str(), Some("failed"));
        assert!(report["ended"]["error"]
            .as_str()
            .unwrap()
            .contains("moderator paused"));
        assert_eq!(report["calls"].as_u64(), Some(1));
        assert_eq!(report["tasks"][0]["status"].as_str(), Some("running"));
        assert_eq!(
            events(&h),
            before,
            "polling a paused task must not spend turns or create a task"
        );
    }
    assert_eq!(
        before
            .iter()
            .filter(|e| matches!(e, Event::TaskOpened { .. }))
            .count(),
        1
    );
    // Retry is explicit and revision-fenced. A later poll adopts the same task;
    // the writer is still broken, so it pauses again within the same budget.
    let rev = TaskView::from_events(rusty_bbp::TaskId(h.task.clone()), &before)
        .card
        .rev
        .0;
    let reset = Command::new(&c.binary)
        .args(["mod-health", "--dir"])
        .arg(&c.dir)
        .args([
            "--task",
            &h.task,
            "--reset",
            "yes",
            "--rev",
            &rev.to_string(),
        ])
        .output()
        .unwrap();
    assert!(
        reset.status.success(),
        "{}",
        String::from_utf8_lossy(&reset.stderr)
    );
    let retry = invoke();
    assert!(
        retry.status.success(),
        "{}",
        String::from_utf8_lossy(&retry.stderr)
    );
    let report: rusty_serde::json::Value =
        rusty_serde::json::from_str(std::str::from_utf8(&retry.stdout).unwrap()).unwrap();
    assert_eq!(
        report
            .get("calls")
            .and_then(rusty_serde::json::Value::as_u64),
        Some(1)
    );
    let key = Sha256::of(h.task.as_bytes()).hex();
    // Let its bounded moderator pause before removing its private fixture.
    let until = Instant::now() + Duration::from_secs(15);
    loop {
        let lock = std::fs::OpenOptions::new()
            .write(true)
            .open(c.dir.join("mod").join(format!("{key}.lock")))
            .unwrap();
        if lock.try_lock().is_ok() {
            break;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(10));
    }
    std::fs::remove_dir_all(dir).unwrap();
    println!("BBP_ACCEPTANCE PASS pause-adoption");
}
