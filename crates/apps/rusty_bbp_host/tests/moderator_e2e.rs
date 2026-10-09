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
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command as Proc;
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
                human::perform(&dir, &task, "approve-plan", &["2"]).expect("approve plan");
            }
            State::MergeGate => {
                let cand = c.candidate.expect("candidate");
                let (run, _, _) = c.run.expect("run");
                human::perform(
                    &dir,
                    &task,
                    "approve-merge",
                    &[&cand.0.to_string(), &run.0.to_string()],
                )
                .expect("approve merge");
            }
            State::Approved => {
                let cand = c.candidate.expect("candidate");
                human::perform(&dir, &task, "receipt", &[&cand.0.to_string(), "deadbeef"])
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
    let count = |sub: &str| std::fs::read_dir(dir.join(sub)).expect(sub).count();
    assert_eq!(count("agents"), 4, "one harness log per launched turn");
    assert_eq!(count("mcp"), 4, "one MCP config per launched turn");
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
