#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Stage-3c exit criterion: `bbp mod` drives a task from open to closed with
//! scripted agents. Four shell harnesses, one per role, each speaking
//! JSON-RPC to the per-turn `bbp mcp` server they are given; the moderator
//! launches each on its turn, runs the runner on the selected run, and the
//! test plays the human at the two gates and the merge receipt.
//!
//! Unix only, like the runner tests: the harnesses are `/bin/sh`.
#![cfg(unix)]

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
    )
    .expect("open");
    for role in Role::ALL {
        admin::assign(
            &dir,
            &task,
            role,
            &PrincipalId(admin::role_name(role).into()),
            "v",
        )
        .expect("assign");
    }
    // Artifact ids are sequential: brief 1, spec 2, diff 3, candidate 4, log 5, report 6.
    let diff = r"--- a/flag.txt\n+++ b/flag.txt\n@@ -1 +1 @@\n-no\n+ok\n";
    let launchers = Launchers {
        planner: Some(harness(
            &dir,
            "planner",
            &[
                r##"{"name":"put_artifact","arguments":{"op":"p1","kind":"spec","brief":1,"body":"# Spec\nAC1 flag says ok."}}"##,
                r#"{"name":"post","arguments":{"op":"p2","kind":"request_decision","to":["human"],"refs":["art:2"],"body":"Approve the plan?","gate":true}}"#,
            ],
        )),
        coder: Some(harness(
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
        )),
        tester: Some(harness(
            &dir,
            "tester",
            &[
                r#"{"name":"post","arguments":{"op":"t1","kind":"verdict","to":["*"],"refs":["art:4"],"evidence":["art:6"],"verdict":{"subject":4,"run":1,"verdict":"approve","blocking":[],"non_blocking":[]}}}"#,
            ],
        )),
        reviewer: Some(harness(
            &dir,
            "reviewer",
            &[
                r#"{"name":"post","arguments":{"op":"r1","kind":"verdict","to":["*"],"refs":["art:4"],"evidence":["art:6"],"verdict":{"subject":4,"run":1,"verdict":"approve","blocking":[],"non_blocking":[]}}}"#,
            ],
        )),
    };
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
    // One harness log and one MCP config per launched turn. Turn ids are not
    // 1..=4: the core grants and immediately revokes a Coder turn when the
    // candidate submission moves the task to Test, so ids skip one.
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
    )
    .expect("open");
    for role in Role::ALL {
        admin::assign(
            &dir,
            &task,
            role,
            &PrincipalId(admin::role_name(role).into()),
            "v",
        )
        .expect("assign");
    }
    // The planner only reads the card and quits.
    let launchers = Launchers {
        planner: Some(harness(
            &dir,
            "planner",
            &[r#"{"name":"task_card","arguments":{}}"#],
        )),
        ..Launchers::default()
    };
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
        l.for_role(Role::Planner).map(|x| x.program.as_str()),
        Some("sh")
    );
    assert!(l.for_role(Role::Coder).is_none());
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
    )
    .expect("open");
    for role in Role::ALL {
        admin::assign(
            dir,
            task,
            role,
            &PrincipalId(admin::role_name(role).into()),
            "v",
        )
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
    let turn = open_driver(&dir, &task)
        .expect("driver")
        .state
        .turn
        .expect("turn")
        .id;
    assert_eq!(turn, TurnId(1));
    // The invocation an earlier moderator launched, still alive.
    let mut old = Mcp::spawn(&dir, &task, "planner", 1);
    let (_, err) = old.call("task_card", "{}");
    assert!(!err);
    let launchers = Launchers {
        planner: Some(harness(
            &dir,
            "planner",
            &[
                r#"{"name":"post","arguments":{"op":"new-1","kind":"ask","to":["coder"],"body":"from the replacement turn"}}"#,
            ],
        )),
        ..Launchers::default()
    };
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
    let first = open_driver(&dir, &task)
        .expect("driver")
        .state
        .turn
        .expect("turn")
        .id;
    assert_eq!(first, TurnId(1), "the refused moderator touched nothing");
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
    let launchers = Launchers {
        planner: Some(Launcher {
            program: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                "( while :; do date +%s%N > \"$1\"; sleep 0.02; done ) & exec sleep 300".into(),
                "sh".into(),
                "{dir}/heartbeat".into(),
            ],
        }),
        ..Launchers::default()
    };
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
    let launchers = Launchers {
        planner: Some(harness(
            &dir,
            "planner",
            &[
                r##"{"name":"put_artifact","arguments":{"op":"p1","kind":"spec","brief":1,"body":"# Spec"}}"##,
                r#"{"name":"post","arguments":{"op":"p2","kind":"request_decision","to":["human"],"refs":["art:2"],"body":"Approve?","gate":true}}"#,
            ],
        )),
        coder: Some(harness(
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
        )),
        ..Launchers::default()
    };
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
