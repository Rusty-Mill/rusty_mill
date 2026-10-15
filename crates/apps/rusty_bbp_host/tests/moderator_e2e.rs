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

/// A `claude` on PATH that runs the role's script from `agents-bin/`, so test
/// launchers can be what an evidence run requires: the program `claude`.
fn fake_claude_on_path() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let bin = std::env::temp_dir().join(format!("bbp_fake_claude_{}", std::process::id()));
        std::fs::create_dir_all(&bin).expect("mkdir");
        let claude = bin.join("claude");
        std::fs::write(
            &claude,
            "#!/bin/sh\nexec sh \"$BBP_DIR/agents-bin/$BBP_ROLE.sh\" \"$@\"\n",
        )
        .expect("write claude");
        std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        let path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{path}", bin.display()));
    });
}

fn tempdir(tag: &str) -> PathBuf {
    fake_claude_on_path();
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

/// Drive one task from open to closed; the run directory is left for the caller.
fn happy_run(tag: &str) -> (PathBuf, TaskId, moderator::Outcome, Vec<State>) {
    let dir = tempdir(tag);
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
        cost_cap: None,
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
    (dir, task, out, seen)
}

#[test]
fn moderator_drives_a_task_from_open_to_closed_with_scripted_agents() {
    let (dir, task, out, seen) = happy_run("happy");
    assert_eq!(out.state, State::Closed);
    assert_eq!(out.turns_launched, 4, "planner, coder, tester, reviewer");
    assert!(out.replay_equals_live, "the log folds to the live state");
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
        cost_cap: None,
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
        cost_cap: None,
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
    // What an evidence run requires of a launcher, less the pin the test adds.
    Launcher {
        program: "claude".into(),
        args: [
            "{dir}",
            "--output-format",
            "stream-json",
            "--verbose",
            "--disallowedTools",
            "Agent,Task",
        ]
        .map(String::from)
        .into(),
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

// ---- live cost cap

/// A cap of `usd` over one priced model, `priced-model`, at $20 per million
/// output tokens.
fn cost_cap(usd: f64) -> moderator::CostCap {
    moderator::CostCap {
        usd,
        criteria: rusty_bbp_host::tokens::Criteria {
            prices_as_of: "test".into(),
            models: [(
                "priced-model".to_owned(),
                rusty_bbp_host::tokens::Prices {
                    input: 1.0,
                    output: 20.0,
                    cache_creation_5m: 1.0,
                    cache_creation_1h: 1.0,
                    cache_read: 1.0,
                },
            )]
            .into(),
            min_commit: None,
        },
        criteria_sha256: "test".into(),
    }
}

/// A planner whose harness prints one assistant message (priced by the model it
/// names) and then idles, under a cost cap.
fn capped_run(
    tag: &str,
    pin: Option<&str>,
    model_in_output: &str,
    cap_usd: f64,
    wall: u64,
) -> Result<moderator::Outcome, String> {
    capped_run_by(tag, pin, model_in_output, cap_usd, wall, moderator::run)
}

fn capped_run_by(
    tag: &str,
    pin: Option<&str>,
    model_in_output: &str,
    cap_usd: f64,
    wall: u64,
    run: fn(&Config) -> Result<moderator::Outcome, String>,
) -> Result<moderator::Outcome, String> {
    let dir = tempdir(tag);
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
    let line = format!(
        r#"{{"type":"assistant","message":{{"id":"m1","model":"{model_in_output}","usage":{{"input_tokens":0,"output_tokens":1000000}}}}}}"#
    );
    let mut planner = script(&dir, "planner", &format!("echo '{line}'\nsleep 60"));
    if let Some(m) = pin {
        planner.args.extend(["--model".to_owned(), m.to_owned()]);
    }
    let launchers = Launchers::default().with("planner", planner);
    let mut cfg = config(&dir, &task, repo, launchers, wall);
    cfg.cost_cap = Some(cost_cap(cap_usd));
    let out = run(&cfg);
    let _ = std::fs::remove_dir_all(&dir);
    out
}

#[test]
fn a_run_over_its_dollar_cap_is_stopped_and_its_harness_killed() {
    let started = Instant::now();
    let err = capped_run("cap-over", Some("priced-model"), "priced-model", 1.0, 30).unwrap_err();
    assert!(err.contains("cost cap reached"), "{err}");
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "stopped by the cap, not by the wall limit or the harness's own sleep"
    );
}

#[test]
fn a_run_under_its_cap_is_left_to_the_wall_limit() {
    let err = capped_run("cap-under", Some("priced-model"), "priced-model", 1000.0, 3).unwrap_err();
    assert!(err.contains("wall limit"), "{err}");
}

#[test]
fn a_model_with_no_price_stops_the_run_because_the_cap_cannot_be_enforced() {
    let err = capped_run(
        "cap-unpriced",
        Some("priced-model"),
        "unpriced-model",
        1000.0,
        30,
    )
    .unwrap_err();
    assert!(
        err.contains("cannot enforce the cost cap") && err.contains("unpriced-model"),
        "{err}"
    );
}

/// A task cancelled before the moderator's first poll, whose planner's granted
/// turn had printed one assistant message of 1M output tokens ($20 at the
/// priced model) before the cancel: the state the moderator first sees is
/// terminal, so no poll of the cap precedes it.
fn cancelled_with_final_output(
    tag: &str,
    model_in_output: &str,
    cap_usd: f64,
) -> Result<moderator::Outcome, String> {
    let dir = tempdir(tag);
    let (repo, _) = repo(&dir);
    let task = TaskId("T1".into());
    open_assigned(&dir, &task, &ProfileSet::shell("test", "true"));
    admin::tick(&dir, &task).expect("tick");
    let line = format!(
        r#"{{"type":"assistant","message":{{"id":"m1","model":"{model_in_output}","usage":{{"input_tokens":0,"output_tokens":1000000}}}}}}"#
    );
    let log = moderator::log_path(&dir, &task, TurnId(1));
    std::fs::create_dir_all(log.parent().unwrap()).unwrap();
    std::fs::write(&log, format!("{line}\n")).unwrap();
    let c = card(&dir, &task);
    let r = human::perform(&dir, &task, "cancel", &["stop"], Some(c.rev)).expect("cancel");
    assert_eq!(r, Response::Ok, "{r:?}");
    let mut cfg = config(&dir, &task, repo, Launchers::default(), 30);
    cfg.cost_cap = Some(cost_cap(cap_usd));
    let out = moderator::run(&cfg);
    let _ = std::fs::remove_dir_all(&dir);
    out
}

#[test]
fn final_output_over_the_cap_on_a_terminal_state_is_not_a_success() {
    let err = cancelled_with_final_output("cap-term-over", "priced-model", 1.0).unwrap_err();
    assert!(err.contains("cost cap reached"), "{err}");
}

#[test]
fn final_output_of_an_unpriced_model_on_a_terminal_state_is_not_a_success() {
    let err =
        cancelled_with_final_output("cap-term-unpriced", "unpriced-model", 1000.0).unwrap_err();
    assert!(
        err.contains("cannot enforce the cost cap") && err.contains("unpriced-model"),
        "{err}"
    );
}

#[test]
fn final_output_under_the_cap_on_a_terminal_state_still_succeeds() {
    let out = cancelled_with_final_output("cap-term-under", "priced-model", 1000.0)
        .expect("under the cap");
    assert_eq!(out.state, State::Cancelled);
}

#[test]
fn an_evidence_run_refuses_to_start_with_an_unpinned_or_unpriced_role() {
    let unpinned = capped_run("cap-unpinned", None, "priced-model", 1000.0, 30).unwrap_err();
    assert!(
        unpinned.contains("refusing to start") && unpinned.contains("planner"),
        "{unpinned}"
    );
    let unpriced = capped_run(
        "cap-pin-unpriced",
        Some("other-model"),
        "priced-model",
        1000.0,
        30,
    )
    .unwrap_err();
    assert!(
        unpriced.contains("refusing to start") && unpriced.contains("other-model"),
        "{unpriced}"
    );
}

// ---- review findings 9 and 13

const SONNET_PRICES: &str = r#"{"prices_as_of":"t","models":{"claude-sonnet-5-5":{"input":2.0,"output":10.0,"cache_creation_5m":2.5,"cache_creation_1h":4.0,"cache_read":0.10}}}"#;

fn sonnet_cap(dir: &Path, usd: f64) -> moderator::CostCap {
    let f = dir.join("criteria.json");
    std::fs::write(&f, SONNET_PRICES).expect("write criteria");
    let (criteria, criteria_sha256) = rusty_bbp_host::tokens::read_criteria(&f).expect("criteria");
    moderator::CostCap {
        usd,
        criteria,
        criteria_sha256,
    }
}

fn opened(tag: &str) -> (PathBuf, TaskId, PathBuf) {
    let dir = tempdir(tag);
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
    (dir, task, repo)
}

#[test]
fn spend_that_lands_before_the_run_ends_is_still_checked_against_the_cap() {
    // A real completed result costing about $1.01, written before the
    // moderator starts, and a task that is already over (cancelled): the loop
    // sees the terminal state on its first pass, before any later check.
    let (dir, task, repo) = opened("final-poll");
    admin::tick(&dir, &task).expect("tick grants the planner its turn");
    let log = rusty_bbp_host::moderator::log_path(&dir, &task, TurnId(1));
    std::fs::create_dir_all(log.parent().expect("parent")).expect("mkdir");
    let text = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/stream-json/completed-default-ttl.jsonl"),
    )
    .expect("fixture")
    .replace("\"outputTokens\":4", "\"outputTokens\":100000")
    .replace(
        "\"total_cost_usd\":0.0086289",
        "\"total_cost_usd\":1.0085889",
    )
    .replace("\"costUSD\":0.0086289", "\"costUSD\":1.0085889");
    std::fs::write(&log, text).expect("write log");
    let rev = card(&dir, &task).rev;
    human::perform(&dir, &task, "cancel", &["stop"], Some(rev)).expect("cancel");
    let mut cfg = config(&dir, &task, repo, Launchers::default(), 10);
    cfg.cost_cap = Some(sonnet_cap(&dir, 0.5));
    let err = moderator::run(&cfg).expect_err("over the cap is not a clean finish");
    assert!(err.contains("cost cap reached"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_task_started_with_a_cap_cannot_be_resumed_without_one() {
    let (dir, task, repo) = opened("resume-uncapped");
    let mut planner = script(&dir, "planner", "sleep 60");
    planner
        .args
        .extend(["--model".to_owned(), "claude-sonnet-5-5".to_owned()]);
    let launchers = Launchers::default().with("planner", planner);
    let mut first = config(&dir, &task, repo.clone(), launchers.clone(), 2);
    first.cost_cap = Some(sonnet_cap(&dir, 1000.0));
    let e1 = moderator::run(&first).expect_err("wall limit");
    assert!(e1.contains("wall limit"), "{e1}");
    let second = config(&dir, &task, repo, launchers, 2);
    let e2 = moderator::run(&second).expect_err("refused");
    assert!(e2.contains("refusing to start"), "{e2}");
    // `bbp pairs` counts only runs that carry the marker of the manifest's cap.
    let sha = first.cost_cap.as_ref().unwrap().criteria_sha256.clone();
    assert!(moderator::require_evidence_marker(&dir, &task, 1000.0, &sha).is_ok());
    assert!(moderator::require_evidence_marker(&dir, &task, 999.0, &sha).is_err());
    assert!(
        moderator::require_evidence_marker(&dir, &TaskId("other".into()), 1000.0, &sha).is_err()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_evidence_runs_harness_gets_both_cache_ttl_variables_pinned_to_one_hour() {
    let (dir, task, repo) = opened("ttl-env");
    let mut planner = script(
        &dir,
        "planner",
        "echo \"$CLAUDE_CODE_PROMPT_CACHE_TTL $CLAUDE_CODE_SUBAGENT_PROMPT_CACHE_TTL\" > \"$1/ttl.txt\"\nsleep 30",
    );
    planner
        .args
        .extend(["--model".to_owned(), "claude-sonnet-5-5".to_owned()]);
    let launchers = Launchers::default().with("planner", planner);
    let mut cfg = config(&dir, &task, repo, launchers, 3);
    cfg.cost_cap = Some(sonnet_cap(&dir, 1000.0));
    // Even if the environment says otherwise.
    std::env::set_var("CLAUDE_CODE_PROMPT_CACHE_TTL", "5m");
    let _ = moderator::run(&cfg);
    std::env::remove_var("CLAUDE_CODE_PROMPT_CACHE_TTL");
    let got = std::fs::read_to_string(dir.join("ttl.txt")).expect("the harness ran");
    assert_eq!(got.trim(), "1h 1h");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `bbp grade pack` and `join` on two real closed run dirs: the approved pack
/// has one entry per arm, the unused pack is separate, and the key opens only
/// when every id is scored.
#[test]
fn grade_pack_and_join_work_on_real_run_dirs() {
    use rusty_bbp_host::grade;
    let (cdir, ctask, _, _) = happy_run("grade-c");
    let (tdir, ttask, _, _) = happy_run("grade-t");
    let out = tempdir("grade-out");
    let rubrics = out.join("rubrics");
    std::fs::create_dir_all(&rubrics).unwrap();
    std::fs::write(
        rubrics.join("p1.json"),
        r#"{"items":["a","b"],"pass_mark":1}"#,
    )
    .unwrap();
    let run = |d: &Path, t: &TaskId| {
        format!(
            r#"{{"dir":"{}","task":"{}","mod_output":"unused"}}"#,
            d.display(),
            t.0
        )
    };
    let manifest = out.join("manifest.json");
    std::fs::write(
        &manifest,
        format!(
            r#"{{"poll_ms":500,"reads_per_turn":8,"cap_usd":5.0,"criteria":"c","repo":"r","pairs":[{{"id":"p1","kind":"separable","control":{},"treatment":{}}}]}}"#,
            run(&cdir, &ctask),
            run(&tdir, &ttask)
        ),
    )
    .unwrap();
    let packed = grade::write_pack(&manifest, &rubrics, "seed", &out.join("pack")).expect("pack");
    assert!(packed.contains("\"approved\":2"), "{packed}");
    assert!(packed.contains("\"unused\":0"), "{packed}");
    let read = |n: &str| std::fs::read_to_string(out.join("pack").join(n)).unwrap();
    let ids: Vec<String> = read("pack.json")
        .split("\"id\":\"")
        .skip(1)
        .map(|r| r[..16].to_owned())
        .collect();
    assert_eq!(ids.len(), 2);
    assert_eq!(read("unused.json"), "[]");
    for leak in ["control", "treatment", "grade-c", "grade-t"] {
        assert!(!read("pack.json").contains(leak), "pack leaks {leak}");
    }
    let scores = out.join("scores.json");
    let met = |ids: &[String]| {
        ids.iter()
            .map(|i| format!(r#""{i}":[true,false]"#))
            .collect::<Vec<_>>()
            .join(",")
    };
    std::fs::write(
        &scores,
        format!(r#"{{"grader":"g","met":{{{}}}}}"#, met(&ids[..1])),
    )
    .unwrap();
    let key = out.join("pack").join("key.json");
    assert!(grade::join_files(&key, &scores, &rubrics)
        .unwrap_err()
        .contains("sealed"));
    std::fs::write(
        &scores,
        format!(r#"{{"grader":"g","met":{{{}}}}}"#, met(&ids)),
    )
    .unwrap();
    let joined = grade::join_files(&key, &scores, &rubrics).expect("join");
    assert!(joined.contains("\"rubric_ok\":true"), "{joined}");
    for d in [cdir, tdir, out] {
        let _ = std::fs::remove_dir_all(d);
    }
}

// ---- second review: findings 1, 2, 4, 5

#[test]
fn a_run_stopped_by_the_cap_is_an_outcome_with_its_stop_and_its_replay() {
    let out = capped_run_by(
        "stop-cap",
        Some("priced-model"),
        "priced-model",
        1.0,
        30,
        moderator::run_to_stop,
    )
    .expect("a cap stop is reported, not lost");
    let stop = out.stop.expect("stop");
    assert_eq!(stop.kind, moderator::StopKind::Cap);
    assert!(stop.reason.contains("cost cap reached"), "{}", stop.reason);
    assert!(out.replay_equals_live, "the replay is still checked");
    assert!(!out.state.terminal());
}

#[test]
fn a_run_stopped_by_the_wall_limit_is_an_outcome_with_its_stop() {
    let out = capped_run_by(
        "stop-wall",
        Some("priced-model"),
        "priced-model",
        1000.0,
        3,
        moderator::run_to_stop,
    )
    .expect("a wall stop is reported, not lost");
    assert_eq!(out.stop.expect("stop").kind, moderator::StopKind::Wall);
}

/// Everything `bbp mod` needs for a capped run of a planner that prints `body`.
fn mod_cli(tag: &str, body: &str) -> (std::process::Output, PathBuf) {
    let (dir, task, repo) = opened(tag);
    let planner = script(&dir, "planner", body);
    let mut l = planner;
    l.args
        .extend(["--model".to_owned(), "claude-sonnet-5-5".to_owned()]);
    let agents = dir.join("agents.json");
    std::fs::write(
        &agents,
        format!(
            r#"{{"planner":{{"program":"claude","args":{},"builtin_tools":false}}}}"#,
            rusty_serde::json::to_string(&l.args).expect("args")
        ),
    )
    .expect("agents");
    let criteria = dir.join("criteria.json");
    std::fs::write(&criteria, SONNET_PRICES).expect("criteria");
    let out = Proc::new(env!("CARGO_BIN_EXE_bbp"))
        .args(["mod", "--dir"])
        .arg(&dir)
        .args(["--task", &task.0, "--repo-path"])
        .arg(&repo)
        .arg("--work")
        .arg(dir.join("work"))
        .arg("--agents")
        .arg(&agents)
        .args([
            "--confine",
            "none",
            "--poll-ms",
            "50",
            "--max-wall-secs",
            "30",
            "--max-usd",
            "1",
        ])
        .arg("--criteria")
        .arg(&criteria)
        .output()
        .expect("run bbp mod");
    (out, dir)
}

const TEN_DOLLARS: &str = r#"echo '{"type":"assistant","message":{"id":"m1","model":"claude-sonnet-5-5","usage":{"input_tokens":0,"output_tokens":1000000}}}'
sleep 60"#;

#[test]
fn bbp_mod_prints_its_json_with_the_stop_reason_when_the_cap_stops_the_run() {
    let (out, dir) = mod_cli("cli-cap", TEN_DOLLARS);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "a stopped run is not a success");
    assert!(stdout.contains("\"stop\":\"cap\""), "stdout: {stdout:?}");
    let facts = rusty_bbp_host::pairs::mod_facts(&stdout).expect("loads");
    assert_eq!(facts.stop.as_deref(), Some("cap"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_task_started_uncapped_cannot_be_resumed_with_a_cap_and_counted() {
    let (dir, task, repo) = opened("uncapped-first");
    let mut planner = script(&dir, "planner", "sleep 60");
    planner
        .args
        .extend(["--model".to_owned(), "claude-sonnet-5-5".to_owned()]);
    let launchers = Launchers::default().with("planner", planner);
    let first = config(&dir, &task, repo.clone(), launchers.clone(), 2);
    let e1 = moderator::run(&first).expect_err("wall limit");
    assert!(e1.contains("wall limit"), "{e1}");
    let mut second = config(&dir, &task, repo, launchers, 2);
    second.cost_cap = Some(sonnet_cap(&dir, 1000.0));
    let e2 = moderator::run(&second).expect_err("a capped resume of an uncapped task is refused");
    assert!(e2.contains("refusing to start"), "{e2}");
    let sha = second.cost_cap.as_ref().unwrap().criteria_sha256.clone();
    assert!(moderator::require_evidence_marker(&dir, &task, 1000.0, &sha).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_harness_that_prints_garbage_stops_a_capped_run() {
    let started = Instant::now();
    let (out, dir) = mod_cli("garbage", "echo 'this is not stream-json'\nsleep 60");
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("cannot enforce the cost cap"), "{err}");
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "stopped by the cap check, not the wall"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_harness_that_has_printed_nothing_yet_is_left_to_the_wall_limit() {
    let (dir, task, repo) = opened("silent");
    let mut planner = script(&dir, "planner", "sleep 60");
    planner
        .args
        .extend(["--model".to_owned(), "claude-sonnet-5-5".to_owned()]);
    let launchers = Launchers::default().with("planner", planner);
    let mut cfg = config(&dir, &task, repo, launchers, 3);
    cfg.cost_cap = Some(sonnet_cap(&dir, 1000.0));
    let err = moderator::run(&cfg).expect_err("wall limit");
    assert!(err.contains("wall limit"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_evidence_runs_harness_gets_the_subagent_model_pinned_to_the_roles_model() {
    let (dir, task, repo) = opened("subagent-env");
    let mut planner = script(
        &dir,
        "planner",
        "echo \"$CLAUDE_CODE_SUBAGENT_MODEL\" > \"$1/subagent.txt\"\nsleep 30",
    );
    planner
        .args
        .extend(["--model".to_owned(), "claude-sonnet-5-5".to_owned()]);
    let launchers = Launchers::default().with("planner", planner);
    let mut cfg = config(&dir, &task, repo, launchers, 3);
    cfg.cost_cap = Some(sonnet_cap(&dir, 1000.0));
    std::env::set_var("CLAUDE_CODE_SUBAGENT_MODEL", "claude-haiku-5-5");
    let _ = moderator::run(&cfg);
    std::env::remove_var("CLAUDE_CODE_SUBAGENT_MODEL");
    let seen = std::fs::read_to_string(dir.join("subagent.txt")).expect("the harness ran");
    assert_eq!(seen.trim(), "claude-sonnet-5-5");
    let _ = std::fs::remove_dir_all(&dir);
}
