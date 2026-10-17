#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Stage-3c exit criterion: `bbp mod` drives a task from open to closed with
//! scripted agents. Four shell harnesses, one per role, each speaking
//! JSON-RPC to the per-turn `bbp mcp` server they are given; the moderator
//! launches each on its turn, runs the runner on the selected run, and the
//! test plays the human at the two gates and the merge receipt.
//!
//! Unix only, like the runner tests: the harnesses are `/bin/sh`.
#![cfg(unix)]

mod common;
use common::Who;
use rusty_bbp::*;
use rusty_bbp_host::moderator::{self, Config, Launcher, Launchers};
use rusty_bbp_host::profiles::ProfileSet;
use rusty_bbp_host::runner::Confinement;
use rusty_bbp_host::{admin, human, open_driver};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as Proc, Stdio};
use std::time::{Duration, Instant};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bbp_mod_{}_{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

fn git(repo: &Path, args: &[&str]) -> String {
    let out = Proc::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn repo(dir: &Path) -> (PathBuf, String) {
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).expect("mkdir");
    git(&repo, &["init", "--quiet", "-b", "main"]);
    git(&repo, &["config", "user.email", "t@example.com"]);
    git(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("flag.txt"), "no\n").expect("write");
    std::fs::write(repo.join("test.sh"), "grep -q '^ok$' flag.txt\n").expect("write");
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "--quiet", "-m", "base"]);
    let base = git(&repo, &["rev-parse", "HEAD"]);
    (repo, base)
}

/// A harness that sends `lines` to the turn's `bbp mcp` and exits.
fn harness(dir: &Path, name: &str, lines: &[&str]) -> Launcher {
    let bin = dir.join("agents-bin");
    std::fs::create_dir_all(&bin).expect("mkdir");
    let path = bin.join(format!("{name}.sh"));
    let mut script = String::from("#!/bin/sh\nset -e\n\"$BBP_BIN\" mcp <<'EOF'\n");
    script.push_str(r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"sh","version":"0"}}}"#);
    script.push('\n');
    for (i, l) in lines.iter().enumerate() {
        script.push_str(&format!(
            r#"{{"jsonrpc":"2.0","id":{},"method":"tools/call","params":{l}}}"#,
            i + 1
        ));
        script.push('\n');
    }
    script.push_str("EOF\n");
    std::fs::write(&path, script).expect("write script");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    Launcher {
        program: path.to_string_lossy().into_owned(),
        args: vec!["{role}".into(), "{turn}".into()],
        builtin_tools: false,
    }
}

fn card(dir: &Path, task: &TaskId) -> Card {
    admin::card(dir, task).expect("card")
}

#[test]
fn moderator_drives_a_task_from_open_to_closed_with_scripted_agents() {
    let dir = tempdir("happy");
    let (repo, base) = repo(&dir);
    let task = TaskId("T1".into());
    admin::open_task(
        &dir,
        &task,
        "local",
        b"Make the flag say ok.",
        &PrincipalId("human".into()),
        &ProfileSet::shell("test", "sh ./test.sh"),
        common::roster(),
    )
    .expect("open");
    for role in Who::ALL {
        admin::assign(
            &dir,
            &task,
            &role.id(),
            &PrincipalId(role.name().into()),
            "v",
        )
        .expect("assign");
    }
    // Artifact ids are sequential: brief 1, spec 2, diff 3, candidate 4, log 5, report 6.
    let diff = r"--- a/flag.txt\n+++ b/flag.txt\n@@ -1 +1 @@\n-no\n+ok\n";
    let launchers = Launchers::default()
        .with("planner", harness(
            &dir,
            "planner",
            &[
                r##"{"name":"put_artifact","arguments":{"op":"p1","kind":"spec","brief":1,"body":"# Spec\nAC1 flag says ok."}}"##,
                r#"{"name":"post","arguments":{"op":"p2","kind":"request_decision","to":["human"],"refs":["art:2"],"body":"Approve the plan?","gate":true}}"#,
            ],
        ))
        .with("coder", harness(
            &dir,
            "coder",
            &[
                &format!(
                    r#"{{"name":"put_artifact","arguments":{{"op":"c1","kind":"diff","body":"{diff}"}}}}"#
                ),
                &format!(
                    r#"{{"name":"put_artifact","arguments":{{"op":"c2","kind":"candidate","base":"{base}","diffs":[3],"body":""}}}}"#
                ),
            ],
        ))
        .with("tester", harness(
            &dir,
            "tester",
            &[
                r#"{"name":"post","arguments":{"op":"t1","kind":"verdict","to":["*"],"refs":["art:4"],"evidence":["art:6"],"verdict":{"subject":4,"run":1,"verdict":"approve","blocking":[],"non_blocking":[]}}}"#,
            ],
        ))
        .with("reviewer", harness(
            &dir,
            "reviewer",
            &[
                r#"{"name":"post","arguments":{"op":"r1","kind":"verdict","to":["*"],"refs":["art:4"],"evidence":["art:6"],"verdict":{"subject":4,"run":1,"verdict":"approve","blocking":[],"non_blocking":[]}}}"#,
            ],
        ));
    let cfg = Config {
        dir: dir.clone(),
        task: task.clone(),
        bbp: PathBuf::from(env!("CARGO_BIN_EXE_bbp")),
        repo,
        work: dir.join("work"),
        launchers,
        confinement: Confinement::Unconfined,
        poll: Duration::from_millis(50),
        max_wall: Duration::from_secs(90),
    };
    let moderator = std::thread::spawn(move || moderator::run(&cfg));

    // The human: approve the plan, approve the merge, send the receipt.
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut seen = Vec::new();
    loop {
        assert!(
            Instant::now() < deadline,
            "timed out; states seen: {seen:?}"
        );
        let c = card(&dir, &task);
        if seen.last() != Some(&c.state) {
            seen.push(c.state);
        }
        match c.state {
            State::PlanGate => {
                // The card's `spec` is the approved one; the proposal under
                // decision is artifact 2 (see the id sequence above).
                human::perform(&dir, &task, "approve-plan", &["2"], None).expect("approve plan");
            }
            State::MergeGate => {
                let cand = c.candidate.expect("candidate");
                let (run, _, _) = c.run.expect("run");
                human::perform(
                    &dir,
                    &task,
                    "approve-merge",
                    &[&cand.0.to_string(), &run.0.to_string()],
                    None,
                )
                .expect("approve merge");
            }
            State::Approved => {
                let cand = c.candidate.expect("candidate");
                human::perform(
                    &dir,
                    &task,
                    "receipt",
                    &[&cand.0.to_string(), "deadbeef"],
                    None,
                )
                .expect("receipt");
            }
            State::Closed => break,
            State::Escalated | State::Cancelled => panic!("task left the happy path: {seen:?}"),
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let out = moderator.join().expect("join").expect("moderator");
    assert_eq!(out.state, State::Closed);
    assert_eq!(out.turns_launched, 4, "planner, coder, tester, reviewer");
    assert_eq!(out.runs, 1);
    assert_eq!(
        seen,
        vec![
            State::Planning,
            State::PlanGate,
            State::Build,
            State::Test,
            State::Review,
            State::MergeGate,
            State::Approved,
            State::Closed
        ]
        .into_iter()
        .filter(|s| seen.contains(s))
        .collect::<Vec<_>>(),
        "states in protocol order"
    );
    let d = open_driver(&dir, &task).expect("driver");
    let run = d.state.selected_run().expect("run");
    assert_eq!(run.status, Some(RunStatus::Passed));
    // One harness log and one MCP config per launched turn, and the turns
    // are exactly 1..=4: assignment grants nothing (the moderator's first
    // tick does) and the candidate submission grants nothing for the
    // transition to revoke.
    for turn in 1..=4 {
        assert!(
            moderator::log_path(&dir, &task, TurnId(turn)).exists(),
            "log for turn {turn}"
        );
        assert!(
            moderator::mcp_config_path(&dir, &task, TurnId(turn)).exists(),
            "mcp config for turn {turn}"
        );
    }
    let count = |p: PathBuf| {
        std::fs::read_dir(p.parent().expect("dir"))
            .expect("dir")
            .count()
    };
    assert_eq!(
        count(moderator::log_path(&dir, &task, TurnId(1))),
        4,
        "one harness log per launched turn"
    );
    assert_eq!(
        count(moderator::mcp_config_path(&dir, &task, TurnId(1))),
        4,
        "one MCP config per launched turn"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A harness that exits without ending its turn forfeits it: the moderator
/// aborts the turn so the protocol moves on instead of waiting for the deadline.
#[test]
fn a_harness_that_exits_early_forfeits_its_turn() {
    let dir = tempdir("forfeit");
    let (repo, _) = repo(&dir);
    let task = TaskId("T1".into());
    admin::open_task(
        &dir,
        &task,
        "local",
        b"brief",
        &PrincipalId("human".into()),
        &ProfileSet::shell("test", "true"),
        common::roster(),
    )
    .expect("open");
    for role in Who::ALL {
        admin::assign(
            &dir,
            &task,
            &role.id(),
            &PrincipalId(role.name().into()),
            "v",
        )
        .expect("assign");
    }
    // The planner only reads the card and quits.
    let launchers = Launchers::default().with(
        "planner",
        harness(&dir, "planner", &[r#"{"name":"task_card","arguments":{}}"#]),
    );
    let cfg = Config {
        dir: dir.clone(),
        task: task.clone(),
        bbp: PathBuf::from(env!("CARGO_BIN_EXE_bbp")),
        repo,
        work: dir.join("work"),
        launchers,
        confinement: Confinement::Unconfined,
        poll: Duration::from_millis(50),
        max_wall: Duration::from_secs(5),
    };
    let first = card(&dir, &task);
    let out = moderator::run(&cfg);
    // The loop ran to its wall limit (the task never closes), but the first
    // turn was aborted and a new planner turn granted along the way.
    assert!(out.is_err(), "the loop stops at the wall limit: {out:?}");
    let d = open_driver(&dir, &task).expect("driver");
    assert!(
        d.state.next_turn > 2,
        "later turns were granted after the forfeit"
    );
    assert_eq!(first.state, State::Planning);
    let _ = std::fs::remove_dir_all(&dir);
}

/// An agents file may name only some roles.
#[test]
fn launchers_file_may_omit_roles() {
    let dir = tempdir("launchers");
    let file = dir.join("agents.json");
    std::fs::write(
        &file,
        r#"{"planner": {"program": "sh", "args": ["-c", "true"]}}"#,
    )
    .expect("write");
    let l = Launchers::read_file(&file).expect("parse");
    assert_eq!(
        l.for_role(&Who::Planner.id()).map(|x| x.program.as_str()),
        Some("sh")
    );
    assert!(l.for_role(&Who::Coder.id()).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Open `task` with every role assigned and `set` as its profile set.
fn open_assigned(dir: &Path, task: &TaskId, set: &ProfileSet) {
    admin::open_task(
        dir,
        task,
        "local",
        b"brief",
        &PrincipalId("human".into()),
        set,
        common::roster(),
    )
    .expect("open");
    for role in Who::ALL {
        admin::assign(dir, task, &role.id(), &PrincipalId(role.name().into()), "v")
            .expect("assign");
    }
}

fn config(dir: &Path, task: &TaskId, repo: PathBuf, launchers: Launchers, wall: u64) -> Config {
    Config {
        dir: dir.to_path_buf(),
        task: task.clone(),
        bbp: PathBuf::from(env!("CARGO_BIN_EXE_bbp")),
        repo,
        work: dir.join("work"),
        launchers,
        confinement: Confinement::Unconfined,
        poll: Duration::from_millis(50),
        max_wall: Duration::from_secs(wall),
    }
}

/// A `bbp mcp` process the test keeps alive across a moderator restart.
struct Mcp {
    child: Child,
    reader: BufReader<std::process::ChildStdout>,
}

impl Mcp {
    fn spawn(dir: &Path, task: &TaskId, principal: &str, turn: u64) -> Mcp {
        let mut child = Proc::new(env!("CARGO_BIN_EXE_bbp"))
            .arg("mcp")
            .env("BBP_DIR", dir)
            .env("BBP_TASK", &task.0)
            .env("BBP_PRINCIPAL", principal)
            .env("BBP_TURN", turn.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn bbp mcp");
        let reader = BufReader::new(child.stdout.take().expect("stdout"));
        Mcp { child, reader }
    }

    /// Call a tool; returns (text, isError).
    fn call(&mut self, name: &str, args: &str) -> (String, bool) {
        let line = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{args}}}}}\n"
        );
        self.child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(line.as_bytes())
            .expect("write");
        let mut out = String::new();
        self.reader.read_line(&mut out).expect("read");
        let v: rusty_serde::Value = rusty_serde::json::from_str(out.trim()).expect("json");
        let r = &v["result"];
        (
            r["content"][0]["text"].as_str().unwrap_or("").to_owned(),
            r["isError"].as_bool().unwrap_or(false),
        )
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A moderator that starts while a turn is live cannot see that turn's
/// harness, so it aborts the turn: the old `bbp mcp` is fenced and the
/// replacement turn's harness acts.
#[test]
fn a_restarted_moderator_fences_the_invocation_it_cannot_see() {
    let dir = tempdir("restart");
    let (repo, _) = repo(&dir);
    let task = TaskId("T1".into());
    open_assigned(&dir, &task, &ProfileSet::shell("test", "true"));
    // An earlier moderator's first tick granted turn 1 and launched for it.
    admin::tick(&dir, &task).expect("tick");
    let turn = open_driver(&dir, &task)
        .expect("driver")
        .state
        .turn()
        .expect("turn")
        .id;
    assert_eq!(turn, TurnId(1));
    // The invocation an earlier moderator launched, still alive.
    let mut old = Mcp::spawn(&dir, &task, "planner", 1);
    let (_, err) = old.call("task_card", "{}");
    assert!(!err);
    let launchers = Launchers::default()
        .with("planner", harness(
            &dir,
            "planner",
            &[
                r#"{"name":"post","arguments":{"op":"new-1","kind":"ask","to":["coder"],"body":"from the replacement turn"}}"#,
            ],
        ));
    let out = moderator::run(&config(&dir, &task, repo, launchers, 3));
    assert!(out.is_err(), "stops at its wall limit: {out:?}");
    let st = open_driver(&dir, &task).expect("driver").state;
    assert!(
        st.messages
            .iter()
            .any(|m| m.draft.body == Body::Prose("from the replacement turn".into())),
        "the replacement invocation acted: {:?}",
        st.messages
    );
    // The old invocation's fresh operation is refused; its exact replay of
    // nothing exists, and the card stays free.
    let (late, err) = old.call(
        "post",
        r#"{"op":"old-1","kind":"ask","to":["coder"],"body":"from the dead turn"}"#,
    );
    assert!(err && late.contains("has ended"), "{late}");
    let (_, err) = old.call("task_card", "{}");
    assert!(!err);
    assert!(!st
        .messages
        .iter()
        .any(|m| m.draft.body == Body::Prose("from the dead turn".into())));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The moderator lock keeps two loops off one task.
#[test]
fn two_moderators_cannot_hold_one_task() {
    let dir = tempdir("lock");
    let (repo, _) = repo(&dir);
    let task = TaskId("T1".into());
    open_assigned(&dir, &task, &ProfileSet::shell("test", "true"));
    let path = moderator::lock_path(&dir, &task);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    let holder = std::fs::File::create(&path).expect("create");
    holder.lock().expect("lock");
    let err = moderator::run(&config(&dir, &task, repo, Launchers::default(), 2)).unwrap_err();
    assert!(err.contains("another moderator"), "{err}");
    assert!(
        open_driver(&dir, &task)
            .expect("driver")
            .state
            .turn()
            .is_none(),
        "the refused moderator touched nothing"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Two tasks at the same turn id in one store get distinct configs and logs,
/// each naming its own task.
#[test]
fn per_turn_files_are_task_scoped() {
    let dir = tempdir("scoped");
    let bbp = PathBuf::from(env!("CARGO_BIN_EXE_bbp"));
    let (a, b) = (TaskId("T1".into()), TaskId("T2".into()));
    let p = PrincipalId("planner".into());
    let ca = moderator::write_mcp_config(&dir, &a, &bbp, &p, TurnId(1)).expect("write");
    let cb = moderator::write_mcp_config(&dir, &b, &bbp, &p, TurnId(1)).expect("write");
    assert_ne!(ca, cb);
    assert_ne!(
        moderator::log_path(&dir, &a, TurnId(1)),
        moderator::log_path(&dir, &b, TurnId(1))
    );
    let text = |p: &Path| std::fs::read_to_string(p).expect("read");
    assert!(text(&ca).contains(r#""BBP_TASK":"T1""#), "{}", text(&ca));
    assert!(text(&cb).contains(r#""BBP_TASK":"T2""#), "{}", text(&cb));
    // Rewriting one leaves the other untouched.
    moderator::write_mcp_config(&dir, &a, &bbp, &PrincipalId("coder".into()), TurnId(1))
        .expect("write");
    assert!(text(&cb).contains(r#""BBP_PRINCIPAL":"planner""#));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A harness that forks a heartbeat writer: when the moderator leaves on its
/// error path, the whole process group is gone, not just the direct child.
#[test]
fn the_harness_process_tree_dies_with_the_moderator() {
    let dir = tempdir("tree");
    let (repo, _) = repo(&dir);
    let task = TaskId("T1".into());
    open_assigned(&dir, &task, &ProfileSet::shell("test", "true"));
    let hb = dir.join("heartbeat");
    let launchers = Launchers::default().with(
        "planner",
        Launcher {
            program: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                "( while :; do date +%s%N > \"$1\"; sleep 0.02; done ) & exec sleep 300".into(),
                "sh".into(),
                "{dir}/heartbeat".into(),
            ],
            builtin_tools: false,
        },
    );
    let out = moderator::run(&config(&dir, &task, repo, launchers, 2));
    assert!(out.is_err(), "{out:?}");
    assert!(hb.exists(), "the heartbeat writer ran");
    std::thread::sleep(Duration::from_millis(200));
    let a = std::fs::read_to_string(&hb).expect("read");
    std::thread::sleep(Duration::from_millis(300));
    let b = std::fs::read_to_string(&hb).expect("read");
    assert_eq!(a, b, "the forked writer stopped with the moderator");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Cancelling the task while a profile is still running ends the loop; the
/// runner on its own thread cannot block supervision.
#[test]
fn cancellation_is_honoured_while_a_profile_runs() {
    let dir = tempdir("cancel");
    let (repo, base) = repo(&dir);
    let task = TaskId("T1".into());
    open_assigned(&dir, &task, &ProfileSet::shell("slow", "sleep 30"));
    let diff = r"--- a/flag.txt\n+++ b/flag.txt\n@@ -1 +1 @@\n-no\n+ok\n";
    let launchers = Launchers::default()
        .with("planner", harness(
            &dir,
            "planner",
            &[
                r##"{"name":"put_artifact","arguments":{"op":"p1","kind":"spec","brief":1,"body":"# Spec"}}"##,
                r#"{"name":"post","arguments":{"op":"p2","kind":"request_decision","to":["human"],"refs":["art:2"],"body":"Approve?","gate":true}}"#,
            ],
        ))
        .with("coder", harness(
            &dir,
            "coder",
            &[
                &format!(
                    r#"{{"name":"put_artifact","arguments":{{"op":"c1","kind":"diff","body":"{diff}"}}}}"#
                ),
                &format!(
                    r#"{{"name":"put_artifact","arguments":{{"op":"c2","kind":"candidate","base":"{base}","diffs":[3],"body":""}}}}"#
                ),
            ],
        ));
    let cfg = config(&dir, &task, repo, launchers, 60);
    let moderator = std::thread::spawn(move || moderator::run(&cfg));
    let deadline = Instant::now() + Duration::from_secs(30);
    let cancelled_at;
    loop {
        assert!(Instant::now() < deadline, "timed out before cancel");
        let c = card(&dir, &task);
        match c.state {
            State::PlanGate => {
                human::perform(&dir, &task, "approve-plan", &["2"], None).expect("approve");
            }
            State::Test => {
                // Give the runner time to start the slow profile, then cancel.
                std::thread::sleep(Duration::from_millis(500));
                let c = card(&dir, &task);
                let r =
                    human::perform(&dir, &task, "cancel", &["stop"], Some(c.rev)).expect("cancel");
                assert_eq!(r, Response::Ok, "{r:?}");
                cancelled_at = Instant::now();
                break;
            }
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let out = moderator.join().expect("join").expect("moderator");
    assert_eq!(out.state, State::Cancelled);
    let waited = cancelled_at.elapsed();
    assert!(
        waited < Duration::from_secs(10),
        "the loop noticed in {waited:?}, not after the 30s profile"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn launchers_are_keyed_by_any_role_id() {
    let l = Launchers::default().with(
        "scribe",
        Launcher {
            program: "true".into(),
            args: vec![],
            builtin_tools: true,
        },
    );
    assert!(l
        .for_role(&RoleId::from("scribe"))
        .is_some_and(|x| x.builtin_tools));
    assert!(l.for_role(&Who::Planner.id()).is_none());
    assert_eq!(l.roles().collect::<Vec<_>>(), ["scribe"]);
}

#[test]
fn a_roster_without_a_finalizer_is_refused_at_open() {
    let dir = tempdir("badroster");
    let mut roster = common::roster();
    for r in &mut roster {
        r.final_decision = false;
    }
    let r = admin::open_task(
        &dir,
        &TaskId("T1".into()),
        "local",
        b"brief",
        &PrincipalId("human".into()),
        &ProfileSet::shell("test", "true"),
        roster,
    )
    .expect("dispatch");
    match r {
        Response::Rejected(rej) => {
            assert_eq!(rej.code, Code::BadRoster);
            assert!(rej.detail.contains("no finalizer"), "{}", rej.detail);
        }
        other => panic!("expected a rejection, got {other:?}"),
    }
    // A refused open writes nothing, so the corrected roster opens the same task.
    let logs = std::fs::read_dir(dir.join("tasks")).map_or(0, Iterator::count);
    assert_eq!(logs, 0, "no log was written");
    let r = admin::open_task(
        &dir,
        &TaskId("T1".into()),
        "local",
        b"brief",
        &PrincipalId("human".into()),
        &ProfileSet::shell("test", "true"),
        common::roster(),
    )
    .expect("dispatch");
    assert!(matches!(r, Response::Stored(_)), "{r:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A shell harness: `body` runs under `/bin/sh` with `$1` the task directory
/// and `$BBP_BIN` the `bbp` binary. Not a protocol client by itself.
fn script(dir: &Path, name: &str, body: &str) -> Launcher {
    let bin = dir.join("agents-bin");
    std::fs::create_dir_all(&bin).expect("mkdir");
    let path = bin.join(format!("{name}.sh"));
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write script");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    Launcher {
        program: path.to_string_lossy().into_owned(),
        args: vec!["{dir}".into()],
        builtin_tools: false,
    }
}

fn alive(pid: &str) -> bool {
    Proc::new("kill")
        .args(["-0", pid])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Two builders hold live turns at once (`max_live_turns = 2`), each with its
/// own harness, and both edit the same lines of the same file. The coder's
/// harness waits until the second builder has stored its diff, so it deadlocks
/// if turns are serialized. The coder's candidate then revokes the other turn,
/// the moderator kills that harness, and `bbp metrics` reports the lost work
/// and the hunk-level overlap. This checks the measurement, not the criteria.
#[test]
fn two_builders_run_in_parallel_and_the_candidate_kills_the_other_harness() {
    let dir = tempdir("parallel");
    let (repo, base) = repo(&dir);
    let task = TaskId("T1".into());
    let mut roster = common::roster();
    let mut second = roster
        .iter()
        .find(|r| r.id.0 == "coder")
        .expect("coder")
        .clone();
    second.id = RoleId("coder2".into());
    second.turn_order = 9;
    roster.push(second);
    admin::open_task_live(
        &dir,
        &task,
        "local",
        b"Make the flag say ok.",
        &PrincipalId("human".into()),
        &ProfileSet::shell("test", "true"),
        roster,
        2,
        SchedulingPolicy::default(),
    )
    .expect("open");
    for role in ["planner", "coder", "coder2", "tester", "reviewer"] {
        admin::assign(
            &dir,
            &task,
            &RoleId(role.into()),
            &PrincipalId(role.into()),
            "v",
        )
        .expect("assign");
    }
    let diff = r"--- a/flag.txt\n+++ b/flag.txt\n@@ -1 +1 @@\n-no\n+ok\n";
    let mcp = |calls: &[String]| {
        let mut s = String::from("\"$BBP_BIN\" mcp <<'EOF'\n");
        s.push_str(r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"sh","version":"0"}}}"#);
        s.push('\n');
        for (i, c) in calls.iter().enumerate() {
            s.push_str(&format!(
                r#"{{"jsonrpc":"2.0","id":{},"method":"tools/call","params":{c}}}"#,
                i + 1
            ));
            s.push('\n');
        }
        s.push_str("EOF\n");
        s
    };
    let coder = mcp(&[
        format!(
            r#"{{"name":"put_artifact","arguments":{{"op":"c1","kind":"diff","body":"{diff}"}}}}"#
        ),
        format!(
            r#"{{"name":"put_artifact","arguments":{{"op":"c2","kind":"candidate","base":"{base}","diffs":[4],"body":""}}}}"#
        ),
    ]);
    let rival = mcp(&[format!(
        r#"{{"name":"put_artifact","arguments":{{"op":"r1","kind":"diff","body":"{diff}"}}}}"#
    )]);
    let launchers = Launchers::default()
        .with(
            "planner",
            harness(
                &dir,
                "planner",
                &[
                    r##"{"name":"put_artifact","arguments":{"op":"p1","kind":"spec","brief":1,"body":"# Spec\nAC1"}}"##,
                    r#"{"name":"post","arguments":{"op":"p2","kind":"request_decision","to":["human"],"refs":["art:2"],"body":"Approve?","gate":true}}"#,
                ],
            ),
        )
        .with(
            "coder",
            script(
                &dir,
                "coder",
                &format!(
                    "n=0; while [ ! -f \"$1/coder2.pid\" ]; do n=$((n+1)); [ $n -gt 400 ] && exit 1; sleep 0.05; done\n{coder}"
                ),
            ),
        )
        .with(
            "coder2",
            script(
                &dir,
                "coder2",
                &format!("{rival}echo $$ > \"$1/coder2.pid\"\nexec sleep 300"),
            ),
        );
    let cfg = config(&dir, &task, repo, launchers, 60);
    let moderator = std::thread::spawn(move || moderator::run(&cfg));

    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        assert!(
            Instant::now() < deadline,
            "timed out in {:?}",
            card(&dir, &task).state
        );
        let c = card(&dir, &task);
        match c.state {
            State::PlanGate => {
                human::perform_as(&dir, &task, "approve-plan", &["2"], None, "script")
                    .expect("approve plan");
            }
            State::Test => break,
            State::Escalated | State::Cancelled => panic!("left the path: {:?}", c.state),
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let pid = std::fs::read_to_string(dir.join("coder2.pid")).expect("coder2 started");
    let pid = pid.trim();
    let report = rusty_bbp_host::report::of(&dir, &task, None).expect("report");
    // The scripted human answered the plan gate: named in the output, and
    // self-approved, as every scripted gate is.
    assert_eq!(
        report
            .gates
            .iter()
            .map(|g| (g.answered_by.as_str(), g.self_approved))
            .collect::<Vec<_>>(),
        [("script", true)]
    );
    assert!(report.self_approved);
    assert_eq!(report.max_live_turns, 2);
    assert_eq!(report.metrics.peak_live, 2);
    assert_eq!(report.metrics.revocations, 1, "coder2's turn");
    assert_eq!(
        report.metrics.discarded.bytes,
        diff.replace("\\n", "\n").len() as u64
    );
    assert_eq!(
        (
            report.overlap.unused_diffs,
            report.overlap.file_overlap_diffs,
            report.overlap.hunk_overlap_diffs
        ),
        (1, 1, 1),
        "the loser edited the winner's lines"
    );
    let printed = Proc::new(env!("CARGO_BIN_EXE_bbp"))
        .args(["metrics", "--dir"])
        .arg(&dir)
        .args(["--task", "T1"])
        .output()
        .expect("bbp metrics");
    let printed = String::from_utf8_lossy(&printed.stdout);
    assert!(
        printed.contains("\"revocations\":1")
            && printed.contains("\"hunk_overlap_diffs\":1")
            && printed.contains("\"spec_conflicts\""),
        "{printed}"
    );
    let gone = Instant::now() + Duration::from_secs(10);
    while alive(pid) && Instant::now() < gone {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!alive(pid), "the revoked turn's harness was killed");
    // The runner and the moderator keep appending, so the revision a cancel
    // names can go stale (a rejection, not an error): retry on a fresh card.
    while card(&dir, &task).state != State::Cancelled {
        assert!(Instant::now() < deadline, "cancel never landed");
        let c = card(&dir, &task);
        human::perform(&dir, &task, "cancel", &["done"], Some(c.rev)).expect("cancel");
    }
    let out = moderator.join().expect("join").expect("moderator");
    assert_eq!(out.state, State::Cancelled);
    // A tester turn (no launcher) may also be granted once the runner reports,
    // depending on when the cancel lands, so count the builders' turns from the
    // log instead of the moderator's total.
    assert!(
        out.turns_launched >= 3,
        "at least planner, coder and coder2"
    );
    let log = open_driver(&dir, &task)
        .expect("driver")
        .store
        .events(&task, 0)
        .expect("log");
    for role in ["planner", "coder", "coder2"] {
        let n = log
            .iter()
            .filter(|e| matches!(e, Event::TurnGranted { role: r, .. } if r.0 == role))
            .count();
        assert_eq!(n, 1, "{role} was granted exactly one turn");
    }
    assert!(out.elapsed > Duration::ZERO && out.elapsed < Duration::from_secs(60));
    let _ = std::fs::remove_dir_all(&dir);
}

fn pending_slow_run(dir: &Path, command: &str) -> (PathBuf, TaskId) {
    let (repo, base) = repo(dir);
    let task = TaskId("T1".into());
    open_assigned(dir, &task, &ProfileSet::shell("slow", command));
    admin::tick(dir, &task).unwrap();
    let turn = open_driver(dir, &task).unwrap().state.turn().unwrap().id.0;
    let mut planner = Mcp::spawn(dir, &task, "planner", turn);
    assert!(
        !planner
            .call(
                "put_artifact",
                r##"{"op":"p1","kind":"spec","brief":1,"body":"# Spec"}"##
            )
            .1
    );
    assert!(!planner.call("post",r#"{"op":"p2","kind":"request_decision","to":["human"],"refs":["art:2"],"body":"Approve?","gate":true}"#).1);
    human::perform(dir, &task, "approve-plan", &["2"], None).unwrap();
    let turn = open_driver(dir, &task).unwrap().state.turn().unwrap().id.0;
    let mut coder = Mcp::spawn(dir, &task, "coder", turn);
    let diff = r"--- a/flag.txt\n+++ b/flag.txt\n@@ -1 +1 @@\n-no\n+ok\n";
    assert!(
        !coder
            .call(
                "put_artifact",
                &format!(r#"{{"op":"c1","kind":"diff","body":"{diff}"}}"#)
            )
            .1
    );
    assert!(
        !coder
            .call(
                "put_artifact",
                &format!(
                    r#"{{"op":"c2","kind":"candidate","base":"{base}","diffs":[3],"body":""}}"#
                )
            )
            .1
    );
    assert_eq!(card(dir, &task).state, State::Test);
    (repo, task)
}

struct ModProcess(Child);
impl Drop for ModProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn process_mod(dir: &Path, repo: &Path, task: &TaskId, wall: &str, confine: &str) -> ModProcess {
    std::fs::write(dir.join("agents.json"), "{}").unwrap();
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("process-mod.log"))
        .unwrap();
    ModProcess(
        Proc::new(env!("CARGO_BIN_EXE_bbp"))
            .args(["mod", "--dir"])
            .arg(dir)
            .args(["--task", &task.0, "--repo-path"])
            .arg(repo)
            .arg("--work")
            .arg(dir.join("work"))
            .arg("--agents")
            .arg(dir.join("agents.json"))
            .args([
                "--max-wall-secs",
                wall,
                "--poll-ms",
                "10",
                "--confine",
                confine,
            ])
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap(),
    )
}

#[test]
#[cfg(target_os = "linux")]
fn process_exit_cancels_reaps_and_serializes_a_running_workload_before_restart() {
    for confine in ["sandbox", "none"] {
        'cases: for mode in ["timeout", "cancel", "kill"] {
            let dir = tempdir(&format!("process-{confine}-{mode}"));
            let (repo, task) = pending_slow_run(
                &dir,
                "echo $$ > leader.pid; sleep 30 & echo $! > child.pid; wait",
            );
            let mut moderator = process_mod(
                &dir,
                &repo,
                &task,
                if mode == "timeout" { "2" } else { "20" },
                confine,
            );
            let leader = dir.join("work/run-1/leader.pid");
            let descendant = dir.join("work/run-1/child.pid");
            let deadline = Instant::now() + Duration::from_secs(8);
            while !leader.exists() || !descendant.exists() {
                if moderator.0.try_wait().unwrap().is_some() {
                    let driver = open_driver(&dir, &task).unwrap();
                    let run = driver.state.selected_run().unwrap();
                    let log = driver.state.artifacts.get(&run.log.unwrap()).unwrap();
                    let bytes = driver.store.blob_get(&log.blob.sha).unwrap();
                    let log = String::from_utf8_lossy(&bytes);
                    if confine == "sandbox"
                        && log.contains("Landlock filesystem confinement is NotEnforced")
                    {
                        assert_eq!(run.status, Some(RunStatus::Error));
                        assert!(!leader.exists() && !descendant.exists());
                        eprintln!("sandbox lifecycle cases unavailable on this kernel; verified fail-closed refusal: {log}");
                        drop(moderator);
                        std::fs::remove_dir_all(&dir).unwrap();
                        break 'cases;
                    }
                    panic!("moderator exited before workload: {log}");
                }
                assert!(Instant::now() < deadline, "workload not started");
                std::thread::sleep(Duration::from_millis(10));
            }
            let leader = std::fs::read_to_string(leader).unwrap();
            let descendant = std::fs::read_to_string(descendant).unwrap();
            assert!(alive(leader.trim()) && alive(descendant.trim()));
            let stopped = Instant::now();
            if mode == "cancel" {
                let c = card(&dir, &task);
                assert_eq!(
                    human::perform(&dir, &task, "cancel", &["stop"], Some(c.rev)).unwrap(),
                    Response::Ok
                );
            }
            if mode == "kill" {
                moderator.0.kill().unwrap();
            }
            while moderator.0.try_wait().unwrap().is_none() {
                assert!(
                    stopped.elapsed() < Duration::from_secs(6),
                    "moderator failed to exit"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            // Restart immediately: the previous worker may still be observing
            // EOF. Its task lock must prevent any overlapping execution.
            let mut replacement = process_mod(&dir, &repo, &task, "1", confine);
            let deadline = Instant::now() + Duration::from_secs(6);
            while alive(leader.trim()) || alive(descendant.trim()) {
                assert!(
                    Instant::now() < deadline,
                    "old workload survived moderator/restart: {mode}/{confine}"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            while replacement.0.try_wait().unwrap().is_none() {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(10));
            }
            let d = open_driver(&dir, &task).unwrap();
            let logs = d.store.events(&task, 0).unwrap();
            assert!(matches!(d.state.state, State::Cancelled | State::Escalated));
            assert!(d
                .state
                .selected_run()
                .is_none_or(|r| r.status != Some(RunStatus::Passed)));
            assert_eq!(
                logs.iter()
                    .filter(|e| matches!(e, Event::RunStarted { .. }))
                    .count(),
                1
            );
            drop(moderator);
            drop(replacement);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
}
