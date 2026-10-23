#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Stage 3a admission: moderators racing for one slot directory never run
//! more harnesses or runners than it admits, and a moderator killed while
//! its harness holds a slot frees that slot only when the harness is gone.
//!
//! Every moderator is a real `bbp mod` process. Each harness and each test
//! profile appends `start`/`end` timestamps to a probe file; the recorded
//! intervals lie inside the slot holds, so their overlap bounds nothing the
//! slots did not.
//!
//! Linux only: slot ownership is proven with process groups and `date +%s%N`.
#![cfg(target_os = "linux")]

mod common;
use common::Who;
use rusty_bbp::*;
use rusty_bbp_host::profiles::ProfileSet;
use rusty_bbp_host::{admin, human, open_driver};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as Proc, Stdio};
use std::time::{Duration, Instant};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bbp_admit_{}_{tag}", std::process::id()));
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
    assert!(out.status.success(), "git {args:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A repository whose test passes once `flag.txt` says ok; the test is the
/// validator's workload, so it records itself in the probe directory.
fn repo(dir: &Path, probe: &Path) -> (PathBuf, String) {
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).expect("mkdir");
    git(&repo, &["init", "--quiet", "-b", "main"]);
    git(&repo, &["config", "user.email", "t@example.com"]);
    git(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("flag.txt"), "no\n").expect("write");
    let record = probe.join(format!(
        "validator-{}",
        dir.file_name().unwrap().to_string_lossy()
    ));
    std::fs::write(
        repo.join("test.sh"),
        format!(
            "echo start $(date +%s%N) >> {r}\nsleep 0.4\necho end $(date +%s%N) >> {r}\ngrep -q '^ok$' flag.txt\n",
            r = record.display()
        ),
    )
    .expect("write");
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "--quiet", "-m", "base"]);
    let base = git(&repo, &["rev-parse", "HEAD"]);
    (repo, base)
}

/// A harness script that runs `prelude`, records itself under `probe`, waits
/// for `gate` (when given) to exist, holds its slot a little, then sends
/// `lines` to the turn's `bbp mcp`.
fn harness(
    bin: &Path,
    role: &str,
    probe: &Path,
    gate: Option<&Path>,
    prelude: &str,
    lines: &[String],
) -> String {
    let record = format!("{}/agent-$BBP_TASK-$BBP_TURN", probe.display());
    let mut script = format!("#!/bin/sh\n{prelude}echo start $(date +%s%N) >> {record}\n");
    if let Some(gate) = gate {
        script.push_str(&format!(
            "while [ ! -e {} ]; do sleep 0.05; done\n",
            gate.display()
        ));
    }
    script.push_str("sleep 0.3\n\"$BBP_BIN\" mcp <<'EOF' >/dev/null\n");
    script.push_str(r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"sh","version":"0"}}}"#);
    script.push('\n');
    for (i, l) in lines.iter().enumerate() {
        script.push_str(&format!(
            r#"{{"jsonrpc":"2.0","id":{},"method":"tools/call","params":{l}}}"#,
            i + 1
        ));
        script.push('\n');
    }
    script.push_str(&format!("EOF\necho end $(date +%s%N) >> {record}\n"));
    let path = bin.join(format!("{role}.sh"));
    std::fs::write(&path, script).expect("write script");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    path.to_string_lossy().into_owned()
}

/// One task in its own directory, every role assigned, and its agents file.
struct Task {
    dir: PathBuf,
    task: TaskId,
    repo: PathBuf,
    agents: PathBuf,
}

fn task(root: &Path, name: &str, probe: &Path, gate: Option<&Path>, prelude: &str) -> Task {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let (repo, base) = repo(&dir, probe);
    let task = TaskId(name.into());
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
    // Artifact ids are per task: brief 1, spec 2, diff 3, candidate 4, log 5, report 6.
    let diff = r"--- a/flag.txt\n+++ b/flag.txt\n@@ -1 +1 @@\n-no\n+ok\n";
    let verdict = |op: &str| {
        format!(
            r#"{{"name":"post","arguments":{{"op":"{op}","kind":"verdict","to":["*"],"refs":["art:4"],"evidence":["art:6"],"verdict":{{"subject":4,"run":1,"verdict":"approve","blocking":[],"non_blocking":[]}}}}}}"#
        )
    };
    let scripts: [(&str, Vec<String>); 4] = [
        ("planner", vec![
            r##"{"name":"put_artifact","arguments":{"op":"p1","kind":"spec","brief":1,"body":"# Spec\nAC1 flag says ok."}}"##.into(),
            r#"{"name":"post","arguments":{"op":"p2","kind":"request_decision","to":["human"],"refs":["art:2"],"body":"Approve the plan?","gate":true}}"#.into(),
        ]),
        ("coder", vec![
            format!(r#"{{"name":"put_artifact","arguments":{{"op":"c1","kind":"diff","body":"{diff}"}}}}"#),
            format!(r#"{{"name":"put_artifact","arguments":{{"op":"c2","kind":"candidate","base":"{base}","diffs":[3],"body":""}}}}"#),
        ]),
        ("tester", vec![verdict("t1")]),
        ("reviewer", vec![verdict("r1")]),
    ];
    let bin = dir.join("agents-bin");
    std::fs::create_dir_all(&bin).expect("mkdir");
    let entries: Vec<String> = scripts
        .iter()
        .map(|(role, lines)| {
            let program = harness(&bin, role, probe, gate, prelude, lines);
            format!(r#""{role}":{{"program":"{program}","args":[]}}"#)
        })
        .collect();
    let agents = dir.join("agents.json");
    std::fs::write(&agents, format!("{{{}}}", entries.join(","))).expect("agents");
    Task {
        dir,
        task,
        repo,
        agents,
    }
}

/// `bbp mod` for `t`, admitted through `slots`.
fn moderator(t: &Task, slots: &Path, agents: u32) -> Child {
    Proc::new(env!("CARGO_BIN_EXE_bbp"))
        .arg("mod")
        .arg("--dir")
        .arg(&t.dir)
        .args(["--task", &t.task.0, "--repo-path"])
        .arg(&t.repo)
        .arg("--work")
        .arg(t.dir.join("work"))
        .arg("--agents")
        .arg(&t.agents)
        // The grace lets a harness record its end before it would be killed.
        .args([
            "--confine",
            "none",
            "--poll-ms",
            "50",
            "--end-grace-secs",
            "5",
        ])
        .args(["--max-wall-secs", "120", "--slots"])
        .arg(slots)
        .args([
            "--agent-slots",
            &agents.to_string(),
            "--validator-slots",
            "1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("bbp mod")
}

/// Play the human for `t` once: approve the plan and the merge, send the
/// receipt. True when the task is closed.
fn play_human(t: &Task) -> bool {
    let c = admin::card(&t.dir, &t.task).expect("card");
    match c.state {
        State::PlanGate => {
            human::perform(&t.dir, &t.task, "approve-plan", &["2"], None).expect("approve plan");
        }
        State::MergeGate => {
            let cand = c.candidate.expect("candidate");
            let (run, _, _) = c.run.expect("run");
            let args = [cand.0.to_string(), run.0.to_string()];
            human::perform(
                &t.dir,
                &t.task,
                "approve-merge",
                &[&args[0], &args[1]],
                None,
            )
            .expect("approve merge");
        }
        State::Approved => {
            let cand = c.candidate.expect("candidate").0.to_string();
            human::perform(&t.dir, &t.task, "receipt", &[&cand, "deadbeef"], None)
                .expect("receipt");
        }
        State::Escalated | State::Cancelled => panic!("{} left the happy path", t.task.0),
        _ => {}
    }
    c.state == State::Closed
}

/// The `(start, end)` intervals recorded in probe files whose names start
/// with `prefix`. An interval with no end yet ends now.
fn intervals(probe: &Path, prefix: &str) -> Vec<(u128, u128)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(probe).expect("probe") {
        let path = entry.expect("entry").path();
        if !path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(prefix)
        {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("read probe");
        let mut start = None;
        for line in text.lines() {
            let (kind, at) = line.split_once(' ').expect("probe line");
            let at: u128 = at.parse().expect("timestamp");
            match kind {
                "start" => start = Some(at),
                _ => out.push((start.take().expect("start before end"), at)),
            }
        }
        if let Some(start) = start {
            out.push((start, u128::MAX));
        }
    }
    out
}

/// The most intervals open at one instant.
fn peak(intervals: &[(u128, u128)]) -> usize {
    let mut edges: Vec<(u128, i32)> = intervals
        .iter()
        .flat_map(|&(s, e)| [(s, 1), (e, -1)])
        .collect();
    // An end at the same instant as a start closes first.
    edges.sort();
    let (mut open, mut most) = (0i32, 0i32);
    for (_, delta) in edges {
        open += delta;
        most = most.max(open);
    }
    most as usize
}

fn wait_until(what: &str, timeout: Duration, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn racing_moderators_never_exceed_two_agents_and_one_validator() {
    let root = tempdir("race");
    let (probe, slots) = (root.join("probe"), root.join("slots"));
    std::fs::create_dir_all(&probe).expect("mkdir");
    let tasks: Vec<Task> = (1..=4)
        .map(|i| task(&root, &format!("T{i}"), &probe, None, ""))
        .collect();
    let mut mods: Vec<Child> = tasks.iter().map(|t| moderator(t, &slots, 2)).collect();

    let mut closed = vec![false; tasks.len()];
    wait_until("every task to close", Duration::from_secs(120), || {
        for (t, done) in tasks.iter().zip(&mut closed) {
            *done = *done || play_human(t);
        }
        closed.iter().all(|c| *c)
    });
    for m in &mut mods {
        assert!(m.wait().expect("wait").success(), "moderator failed");
    }

    let agents = intervals(&probe, "agent-");
    let validators = intervals(&probe, "validator-");
    assert_eq!(agents.len(), 16, "four turns in each of four tasks");
    assert_eq!(validators.len(), 4, "one run in each task");
    assert_eq!(peak(&agents), 2, "agents ran two at a time, never more");
    assert_eq!(peak(&validators), 1, "runs were serialized");
    let _ = std::fs::remove_dir_all(&root);
}

/// Creates the gate file when dropped.
struct Release(PathBuf);

impl Drop for Release {
    fn drop(&mut self) {
        let _ = std::fs::write(&self.0, "");
    }
}

#[test]
fn a_killed_moderator_frees_its_slot_only_when_its_harness_is_gone() {
    crash("crash", "");
}

#[test]
fn a_harness_that_drops_its_stdin_still_holds_its_slot_after_a_crash() {
    // A launcher may replace the harness's stdin; the slot is kept by a
    // holder process, not by the harness's descriptors.
    crash("crash_stdin", "exec </dev/null\n");
}

/// Kill a moderator whose harness, started with `prelude`, holds the only
/// agent slot; a restarted moderator's next turn waits until it is gone.
fn crash(tag: &str, prelude: &str) {
    let root = tempdir(tag);
    let (probe, slots, go) = (root.join("probe"), root.join("slots"), root.join("go"));
    std::fs::create_dir_all(&probe).expect("mkdir");
    let t = task(&root, "T1", &probe, Some(&go), prelude);
    // However the test ends, the orphan is let go, so it cannot outlive it.
    let _release = Release(go.clone());
    let started = |turn: u32| probe.join(format!("agent-T1-{turn}")).exists();

    // The first moderator launches turn 1, whose harness holds the only
    // agent slot while it waits at the gate. Then the moderator dies hard;
    // its harness, in its own process group, does not.
    let mut first = moderator(&t, &slots, 1);
    wait_until("turn 1's harness", Duration::from_secs(20), || started(1));
    first.kill().expect("kill");
    first.wait().expect("reap");

    // The restarted moderator aborts the orphaned turn and grants turn 2,
    // but the orphan still holds the slot, so turn 2 waits.
    let mut second = moderator(&t, &slots, 1);
    wait_until("turn 2 to be granted", Duration::from_secs(20), || {
        let d = open_driver(&t.dir, &t.task).expect("driver");
        d.state.turns.iter().any(|turn| turn.id == TurnId(2))
    });
    std::thread::sleep(Duration::from_secs(1));
    assert!(!started(2), "turn 2 started while the orphan held the slot");

    // The orphan finishes (its calls are refused: its turn has ended) and
    // exits; the slot is reclaimed and turn 2 runs the plan to the gate.
    std::fs::write(&go, "").expect("go");
    wait_until("the plan gate", Duration::from_secs(20), || {
        admin::card(&t.dir, &t.task).expect("card").state == State::PlanGate
    });
    let agents = intervals(&probe, "agent-");
    assert_eq!(agents.len(), 2);
    assert_eq!(peak(&agents), 1, "the orphan and turn 2 never overlapped");

    second.kill().expect("kill");
    second.wait().expect("reap");
    let _ = std::fs::remove_dir_all(&root);
}

/// A slot holder is up before its harness: a caller that dies before
/// starting anyone frees the slot, and one that dies just after starting a
/// harness that drops its stdin leaves the slot held until the harness is
/// gone. Closing the holder's stdin is what the caller's death does.
#[test]
fn a_slot_holder_covers_its_harness_from_the_start() {
    use rusty_bbp_host::slots::{Capacity, Class, Slots};
    use std::os::unix::process::CommandExt;
    let root = tempdir("holder");
    let one = Capacity {
        agents: 1,
        validators: 1,
    };
    let slots = Slots::open(&root.join("slots"), one).expect("slots");
    let bbp = Path::new(env!("CARGO_BIN_EXE_bbp"));
    let free = || slots.try_acquire(Class::Agent).expect("acquire").is_some();

    let slot = slots
        .try_acquire(Class::Agent)
        .expect("acquire")
        .expect("free");
    let mut hold = slot.holder(bbp).expect("holder");
    drop(slot);
    drop(hold.stdin.take());
    hold.wait().expect("holder exits");
    wait_until("the unused slot", Duration::from_secs(5), free);

    let slot = slots
        .try_acquire(Class::Agent)
        .expect("acquire")
        .expect("free");
    let mut hold = slot.holder(bbp).expect("holder");
    let mut harness = Proc::new("sh")
        .args(["-c", "exec </dev/null; exec sleep 30"])
        .stdin(slot.stdio().expect("stdio"))
        .process_group(i32::try_from(hold.id()).expect("pid"))
        .spawn()
        .expect("harness");
    drop(slot);
    drop(hold.stdin.take());
    std::thread::sleep(Duration::from_millis(600));
    assert!(!free(), "the harness ran without its slot");
    harness.kill().expect("kill");
    harness.wait().expect("reap");
    hold.wait().expect("holder exits");
    wait_until("the released slot", Duration::from_secs(5), free);
    let _ = std::fs::remove_dir_all(&root);
}
