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
fn setup() -> (PathBuf, Config) {
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
        assert!(host.open(&task, &board).is_err(), "lost reply {at}");
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
        let handle = host.open(&task, &board).unwrap();
        assert_eq!(host.open(&task, &board).unwrap(), handle);
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
            .arg(&c.work)
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
        let progress = host.poll(handle).unwrap();
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
        host.relay(handle, &question, &txt(body)).unwrap(),
        Relayed::Applied
    );
    question
}

#[test]
#[ignore = "requires built bbp at BBP_TEST_BIN; scripted unconfined runner"]
fn scripted_task_restarts_moderator_then_records_receipt_and_closes() {
    let (dir, mut c) = setup();
    let (task, mut board) = task();
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
    let supervisor = ScriptSupervisor::default();
    let mut host = BbpHost::with_seams(
        c.clone(),
        "scripted".into(),
        "flag task".into(),
        StdCommand,
        supervisor.clone(),
    )
    .unwrap();
    let handle = host.open(&task, &board).unwrap();
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
    assert_eq!(host.open(&task, &board).unwrap(), handle);
    let question = answer(&mut host, &handle, &mut board, gate, "approve-plan art:2");
    // Lost checkpoint after BBP accepted: replay must not create a second approval.
    assert_eq!(
        host.relay(&handle, &question, &txt("approve-plan art:2"))
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
    assert!(matches!(host.poll(&handle).unwrap(),Progress::Closed{outputs} if outputs.len()==2));
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
    let handle = host.open(&task, &board).unwrap();
    let h = host.handle(&handle).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !matches!(host.poll(&handle).unwrap(), Progress::Gate(_)) {
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
    assert!(matches!(host.poll(&handle).unwrap(), Progress::Gate(_)));
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
        host.poll(&handle).unwrap(),
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
