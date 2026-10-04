//! ADR-0005 invariants 1 (private isolation) and 5 (sandbox limits),
//! exercised end to end: the real `rsi` binary as sandbox helper and
//! out-of-process grader, the real toy tasks, and real Python solutions.
//!
//! Most tests need Linux (Landlock, seccomp, rlimits). Elsewhere only the
//! grader CLI and the fail-closed tests compile in, so the Linux-only
//! helpers are expected to be unused there.
#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]

use std::path::{Path, PathBuf};
use std::time::Duration;

use rsi_core::{
    Executor, Limits, PrivateGrader, PublicTask, SandboxSpec, Seed, Solution, Termination,
};
use rsi_runtime::grading::SYSTEM_READ_ROOTS;
use rsi_runtime::task_dir::{OPEN_FILE_LIMIT, PROCESS_LIMIT};
use rsi_runtime::{
    GraderCommand, LocalTask, ProcessExecutor, RuntimeError, SandboxedGrader, SolutionRunner,
    TaskDir,
};

mod common;
use common::{executor, suite, suite_dir, task, Scratch, RSI};

fn runner(scratch: &Scratch) -> SolutionRunner<ProcessExecutor> {
    let roots: Vec<PathBuf> = SYSTEM_READ_ROOTS.iter().map(PathBuf::from).collect();
    SolutionRunner::new(executor(scratch), &roots, &scratch.path("work")).expect("runner")
}

fn grader_command() -> GraderCommand {
    GraderCommand {
        program: RSI.into(),
        args: vec!["__grade".into()],
    }
}

fn solution(source: &str) -> Solution {
    Solution::new(source).expect("valid solution")
}

const SEED: Seed = Seed::new(7);

// --- Invariant 1: private isolation ---------------------------------------

/// A solution that tries every route to the task's private labels and
/// reports what happened on stdout.
fn snooping_solution(task: &TaskDir) -> Solution {
    let root = task.root().display();
    solution(&format!(
        r#"
import os, sys
for path in ["{root}/private/labels.txt", "{root}/private", "{root}/task.json", "{root}"]:
    try:
        if os.path.isdir(path):
            os.listdir(path)
        else:
            open(path).read()
        print("READ", path)
    except PermissionError:
        print("denied", path)
open(sys.argv[2], "w").write("")
"#
    ))
}

#[cfg(target_os = "linux")]
#[test]
fn a_solution_cannot_read_private_data_during_public_scoring() {
    let scratch = Scratch::new("isolation-public");
    let tasks = suite();
    let task = task(&tasks, "ml-regression");
    let runner = runner(&scratch);
    let attempt = LocalTask::new(&task, &runner)
        .public_score(&snooping_solution(&task), SEED)
        .expect("run happens");
    assert!(!attempt.feedback.contains("READ"), "{}", attempt.feedback);
    assert_eq!(
        attempt.feedback.matches("denied").count(),
        4,
        "{}",
        attempt.feedback
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_solution_cannot_read_private_labels_during_private_grading() {
    let scratch = Scratch::new("isolation-private");
    let tasks = suite();
    let ml = task(&tasks, "ml-regression");
    let labels = ml.root().join("private/labels.txt");
    // Would score 1.0 by echoing the labels, if it could read them.
    let cheat = solution(&format!(
        r#"
import sys
try:
    answer = open("{}").read()
except PermissionError:
    answer = ""
open(sys.argv[2], "w").write(answer)
"#,
        labels.display()
    ));
    let runner = runner(&scratch);
    let grader = SandboxedGrader::new(&tasks, &runner, grader_command());
    let score = grader
        .private_score(&ml.manifest().id, Some(&cheat), SEED)
        .expect("grading happens");
    assert_eq!(score, ml.manifest().floor, "the labels were not readable");
}

#[cfg(target_os = "linux")]
#[test]
fn the_runner_refuses_a_sandbox_that_could_reach_the_task() {
    let scratch = Scratch::new("isolation-roots");
    let tasks = suite();
    let ml = task(&tasks, "ml-regression");
    let parent = ml.root().parent().expect("suite dir").to_path_buf();
    let leaky =
        SolutionRunner::new(executor(&scratch), &[parent], &scratch.path("work")).expect("runner");
    let result = LocalTask::new(&ml, &leaky).public_score(ml.baseline(), SEED);
    assert!(
        matches!(result, Err(RuntimeError::Sandbox(_))),
        "{result:?}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn baselines_score_publicly_and_privately() {
    let scratch = Scratch::new("baselines");
    let tasks = suite();
    let runner = runner(&scratch);
    let grader = SandboxedGrader::new(&tasks, &runner, grader_command());
    // (task, public range, private range) for the deliberately naive baselines.
    let expected = [
        ("ml-regression", 0.0..0.05, 0.0..0.05),
        ("scaffold-oracle", 0.6..0.95, 0.6..0.95),
        ("tsp-heuristic", 0.05..0.5, 0.05..0.5),
    ];
    assert_eq!(tasks.len(), expected.len());
    for (task, (id, public, private)) in tasks.iter().zip(expected) {
        assert_eq!(task.manifest().id.as_str(), id);
        let attempt = LocalTask::new(task, &runner)
            .public_score(task.baseline(), SEED)
            .expect("public run");
        let score = attempt.score.expect(&attempt.feedback).get();
        assert!(public.contains(&score), "{id} public {score}");
        let graded = grader
            .private_score(&task.manifest().id, Some(task.baseline()), SEED)
            .expect("private run")
            .get();
        assert!(private.contains(&graded), "{id} private {graded}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn better_solutions_score_higher_than_baselines() {
    let scratch = Scratch::new("better");
    let tasks = suite();
    let runner = runner(&scratch);
    let cases = [
        ("ml-regression", LEAST_SQUARES, 0.8),
        ("tsp-heuristic", NEAREST_NEIGHBOUR, 0.7),
        ("scaffold-oracle", MAJORITY_VOTE, 0.9),
    ];
    for (id, source, at_least) in cases {
        let task = task(&tasks, id);
        let attempt = LocalTask::new(&task, &runner)
            .public_score(&solution(source), SEED)
            .expect("public run");
        let score = attempt.score.expect(&attempt.feedback).get();
        assert!(
            score >= at_least,
            "{id}: {score} < {at_least}\n{}",
            attempt.feedback
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn missing_solution_and_crashes_score_the_floor() {
    let scratch = Scratch::new("floor");
    let tasks = suite();
    let runner = runner(&scratch);
    let grader = SandboxedGrader::new(&tasks, &runner, grader_command());
    let tsp = task(&tasks, "tsp-heuristic");
    let floor = tsp.manifest().floor;
    let none = grader
        .private_score(&tsp.manifest().id, None, SEED)
        .expect("grading");
    assert_eq!(none, floor);
    let crash = solution("raise SystemExit(3)\n");
    let crashed = grader
        .private_score(&tsp.manifest().id, Some(&crash), SEED)
        .expect("grading");
    assert_eq!(crashed, floor);
    let attempt = LocalTask::new(&tsp, &runner)
        .public_score(&crash, SEED)
        .expect("run");
    assert_eq!(attempt.score, None);
    assert!(
        attempt.feedback.contains("status 3"),
        "{}",
        attempt.feedback
    );
}

#[test]
fn the_grader_cli_scores_output_files() {
    let scratch = Scratch::new("grade-cli");
    let ml = suite_dir().join("ml-regression");
    let labels = ml.join("public/labels.txt");
    let grade = |output: &Path| {
        let result = std::process::Command::new(RSI)
            .args(["__grade", "--task"])
            .arg(&ml)
            .args(["--split", "public", "--output"])
            .arg(output)
            .output()
            .expect("grader runs");
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8(result.stdout).expect("utf-8")
    };
    assert_eq!(
        grade(&labels).trim(),
        "score 1.0",
        "the labels score perfectly"
    );
    assert_eq!(
        grade(&scratch.path("missing.txt")).trim(),
        "score 0.0",
        "no output: floor"
    );
    let bad = std::process::Command::new(RSI)
        .args(["__grade", "--split", "nope"])
        .output()
        .expect("grader runs");
    assert!(!bad.status.success());
}

// --- Untrusted output ingestion (review finding 1) --------------------------

/// A solution whose `output.txt` is a symlink to `target`.
fn symlink_output(target: &Path) -> Solution {
    solution(&format!(
        "import os, sys\nos.symlink({:?}, sys.argv[2])\n",
        target.display().to_string()
    ))
}

#[cfg(target_os = "linux")]
#[test]
fn a_symlinked_output_cannot_forge_a_public_score() {
    let scratch = Scratch::new("symlink-public");
    let tasks = suite();
    let ml = task(&tasks, "ml-regression");
    let runner = runner(&scratch);
    // Followed, this link would score a perfect 1.0 against the public labels.
    let link = symlink_output(&ml.root().join("public/labels.txt"));
    let attempt = LocalTask::new(&ml, &runner)
        .public_score(&link, SEED)
        .expect("run");
    assert_eq!(attempt.score, None, "{}", attempt.feedback);
    assert!(
        attempt.feedback.contains("rejected"),
        "{}",
        attempt.feedback
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_symlinked_output_cannot_read_private_labels_during_grading() {
    let scratch = Scratch::new("symlink-private");
    let tasks = suite();
    let ml = task(&tasks, "ml-regression");
    let runner = runner(&scratch);
    let grader = SandboxedGrader::new(&tasks, &runner, grader_command());
    let link = symlink_output(&ml.root().join("private/labels.txt"));
    let score = grader
        .private_score(&ml.manifest().id, Some(&link), SEED)
        .expect("grading happens");
    assert_eq!(score, ml.manifest().floor);
}

#[cfg(target_os = "linux")]
#[test]
fn a_fifo_output_is_rejected_promptly() {
    let scratch = Scratch::new("fifo");
    let tasks = suite();
    let ml = task(&tasks, "ml-regression");
    let runner = runner(&scratch);
    let fifo = solution("import os, sys\nos.mkfifo(sys.argv[2])\n");
    let started = std::time::Instant::now();
    let attempt = LocalTask::new(&ml, &runner)
        .public_score(&fifo, SEED)
        .expect("run");
    assert_eq!(attempt.score, None, "{}", attempt.feedback);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    let grader = SandboxedGrader::new(&tasks, &runner, grader_command());
    let score = grader
        .private_score(&ml.manifest().id, Some(&fifo), SEED)
        .expect("grading");
    assert_eq!(score, ml.manifest().floor);
}

#[cfg(target_os = "linux")]
#[test]
fn a_replaced_staged_input_cannot_leak_into_public_scoring() {
    let scratch = Scratch::new("input-swap");
    let tasks = suite();
    let tsp = task(&tasks, "tsp-heuristic");
    let runner = runner(&scratch);
    // Writes a valid identity tour, then swaps the staged inputs for a link
    // the parent would follow if it re-read the work directory.
    let swap = solution(&format!(
        "import os, sys\nlines = open(sys.argv[1]).read().split('\\n')\nopen(sys.argv[2], 'w').write(''.join(' '.join(str(i) for i in range(len(l.split()) // 2)) + '\\n' for l in lines if l))\nos.remove(sys.argv[1])\nos.symlink({:?}, sys.argv[1])\n",
        tsp.root().join("private/instances.txt").display().to_string()
    ));
    let baseline = LocalTask::new(&tsp, &runner)
        .public_score(tsp.baseline(), SEED)
        .expect("run");
    let swapped = LocalTask::new(&tsp, &runner)
        .public_score(&swap, SEED)
        .expect("run");
    assert_eq!(swapped.score, baseline.score, "{}", swapped.feedback);
}

// --- Process containment (review finding 2) --------------------------------

/// Whether `pid` is still a live (non-zombie) process.
fn alive(pid: u32) -> bool {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => {
            let state = stat
                .rsplit_once(')')
                .map(|(_, rest)| rest.trim_start().chars().next());
            !matches!(state, Some(Some('Z' | 'X')))
        }
        Err(_) => false,
    }
}

/// A snippet that starts a detached-as-possible grandchild recording its
/// pid, then either exits or sleeps.
fn detaching_snippet(pidfile: &Path, marker: &Path, then: &str) -> String {
    format!(
        r#"
import os, subprocess, sys, time
child = """
import os, time
for attempt in (os.setsid, lambda: os.setpgid(0, 0)):
    try:
        attempt()
        print("escaped", flush=True)
    except PermissionError:
        print("contained", flush=True)
open({pidfile:?}, "w").write(str(os.getpid()))
time.sleep(4)
open({marker:?}, "w").write("survived")
"""
p = subprocess.Popen([sys.executable, "-c", child])
for _ in range(200):
    if os.path.exists({pidfile:?}):
        break
    time.sleep(0.01)
{then}
"#,
        pidfile = pidfile.display().to_string(),
        marker = marker.display().to_string(),
    )
}

#[cfg(target_os = "linux")]
#[test]
fn processes_cannot_leave_the_job_by_changing_session_or_group() {
    let scratch = Scratch::new("setsid");
    let code = r#"
import os
for name, attempt in (("setsid", os.setsid), ("setpgid", lambda: os.setpgid(0, 0))):
    try:
        attempt()
        print(name, "allowed")
    except PermissionError:
        print(name, "denied")
"#;
    let outcome =
        run_python(&scratch, code, limits(2, Duration::from_secs(10), 256)).expect("runs");
    assert_eq!(
        stdout(&outcome),
        "setsid denied
setpgid denied
"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn detached_descendants_die_when_the_job_exits_normally() {
    let scratch = Scratch::new("detach-exit");
    let pidfile = scratch.path("box/grandchild.pid");
    let marker = scratch.path("box/grandchild-survived");
    let code = detaching_snippet(&pidfile, &marker, "sys.exit(0)");
    let outcome =
        run_python(&scratch, &code, limits(5, Duration::from_secs(20), 256)).expect("runs");
    assert_eq!(outcome.termination, Termination::Exited(0));
    assert!(stdout(&outcome).is_empty() || !stdout(&outcome).contains("escaped"));
    let pid: u32 = std::fs::read_to_string(&pidfile)
        .expect("grandchild started")
        .parse()
        .expect("pid");
    assert!(!alive(pid), "grandchild {pid} outlived a normal exit");
    std::thread::sleep(Duration::from_secs(5));
    assert!(!marker.exists(), "a delayed write landed after cleanup");
}

#[cfg(target_os = "linux")]
#[test]
fn detached_descendants_die_at_the_wall_clock_limit() {
    let scratch = Scratch::new("detach-timeout");
    let pidfile = scratch.path("box/grandchild.pid");
    let marker = scratch.path("box/grandchild-survived");
    let code = detaching_snippet(&pidfile, &marker, "time.sleep(60)");
    let outcome =
        run_python(&scratch, &code, limits(5, Duration::from_millis(1500), 256)).expect("runs");
    assert_eq!(outcome.termination, Termination::TimedOut);
    let pid: u32 = std::fs::read_to_string(&pidfile)
        .expect("grandchild started")
        .parse()
        .expect("pid");
    assert!(!alive(pid), "grandchild {pid} outlived the timeout");
    std::thread::sleep(Duration::from_secs(5));
    assert!(!marker.exists(), "a delayed write landed after cleanup");
}

// --- Invariant 5: sandbox limits ------------------------------------------

fn limits(cpu_secs: u64, wall: Duration, memory_mb: u64) -> Limits {
    Limits::new(
        Duration::from_secs(cpu_secs),
        wall,
        memory_mb << 20,
        1 << 20,
        OPEN_FILE_LIMIT,
        PROCESS_LIMIT,
    )
    .expect("valid limits")
}

/// Runs a Python snippet in a bare sandbox (system roots + one work dir).
fn run_python(
    scratch: &Scratch,
    code: &str,
    limits: Limits,
) -> Result<rsi_core::ExecOutcome, RuntimeError> {
    let work = scratch.path("box");
    std::fs::create_dir_all(&work).expect("work dir");
    std::fs::write(work.join("snippet.py"), code).expect("write snippet");
    let roots = SYSTEM_READ_ROOTS
        .iter()
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .collect();
    let env = vec![("PATH".to_owned(), "/usr/local/bin:/usr/bin:/bin".to_owned())];
    let spec = SandboxSpec::new(roots, vec![work.clone()], work, env, limits).expect("spec");
    executor(scratch).exec(&spec, "python3", &["snippet.py".to_owned()])
}

fn stdout(outcome: &rsi_core::ExecOutcome) -> String {
    String::from_utf8_lossy(&outcome.stdout).into_owned()
}

#[cfg(target_os = "linux")]
#[test]
fn internet_sockets_are_refused_but_unix_sockets_work() {
    let scratch = Scratch::new("net");
    let code = r#"
import socket
for family, name in [(socket.AF_INET, "inet"), (socket.AF_INET6, "inet6"), (socket.AF_UNIX, "unix")]:
    try:
        socket.socket(family, socket.SOCK_STREAM).close()
        print(name, "allowed")
    except PermissionError:
        print(name, "denied")
"#;
    let outcome =
        run_python(&scratch, code, limits(2, Duration::from_secs(10), 256)).expect("runs");
    assert_eq!(
        stdout(&outcome),
        "inet denied\ninet6 denied\nunix allowed\n"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn writes_outside_the_work_dir_are_refused() {
    let scratch = Scratch::new("write");
    let outside = scratch.path("outside.txt");
    let code = format!(
        "try:\n    open({:?}, 'w').write('x')\n    print('wrote')\nexcept PermissionError:\n    print('denied')\nopen('inside.txt', 'w').write('ok')\nprint('inside ok')\n",
        outside.display().to_string()
    );
    let outcome =
        run_python(&scratch, &code, limits(2, Duration::from_secs(10), 256)).expect("runs");
    assert_eq!(stdout(&outcome), "denied\ninside ok\n");
    assert!(!outside.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn the_memory_limit_stops_large_allocations() {
    let scratch = Scratch::new("mem");
    let code = "try:\n    b = bytearray(2 * 1024**3)\n    print('allocated')\nexcept MemoryError:\n    print('MemoryError')\n";
    let outcome =
        run_python(&scratch, code, limits(5, Duration::from_secs(20), 256)).expect("runs");
    assert_eq!(stdout(&outcome), "MemoryError\n");
}

#[cfg(target_os = "linux")]
#[test]
fn the_cpu_limit_kills_a_busy_loop() {
    let scratch = Scratch::new("cpu");
    let outcome = run_python(
        &scratch,
        "while True:\n    pass\n",
        limits(1, Duration::from_secs(20), 256),
    )
    .expect("runs");
    // SIGXCPU at the soft limit, SIGKILL at the hard one.
    assert!(
        matches!(outcome.termination, Termination::Signaled(24 | 9)),
        "{:?}",
        outcome.termination
    );
    assert!(outcome.wall < Duration::from_secs(10), "{:?}", outcome.wall);
}

#[cfg(target_os = "linux")]
#[test]
fn the_wall_clock_limit_kills_the_whole_process_group() {
    let scratch = Scratch::new("wall");
    let started = scratch.path("box/child-started");
    let survived = scratch.path("box/child-survived");
    let child = format!(
        "import time; open({:?}, 'w'); time.sleep(3); open({:?}, 'w')",
        started.display().to_string(),
        survived.display().to_string()
    );
    let code = format!(
        "import subprocess, time\nsubprocess.Popen(['python3', '-c', {child:?}])\ntime.sleep(60)\n"
    );
    let outcome =
        run_python(&scratch, &code, limits(5, Duration::from_millis(1500), 256)).expect("runs");
    let stderr = String::from_utf8_lossy(&outcome.stderr);
    assert_eq!(outcome.termination, Termination::TimedOut, "{stderr}");
    assert!(outcome.wall < Duration::from_secs(5), "{:?}", outcome.wall);
    assert!(started.exists(), "the grandchild never started: {stderr}");
    std::thread::sleep(Duration::from_secs(3));
    assert!(!survived.exists(), "a child outlived the timeout");
}

#[cfg(target_os = "linux")]
#[test]
fn sandbox_setup_failure_fails_closed() {
    let scratch = Scratch::new("fail-closed");
    let work = scratch.path("box");
    std::fs::create_dir_all(&work).expect("work dir");
    let marker = work.join("ran");
    // A read root that does not exist cannot be added to the Landlock
    // ruleset, so the helper must refuse to run anything.
    let spec = SandboxSpec::new(
        vec![PathBuf::from("/usr"), scratch.path("does-not-exist")],
        vec![work.clone()],
        work,
        vec![("PATH".to_owned(), "/usr/bin:/bin".to_owned())],
        limits(2, Duration::from_secs(10), 256),
    )
    .expect("spec");
    let result = executor(&scratch).exec(&spec, "touch", &[marker.display().to_string()]);
    assert!(
        matches!(result, Err(RuntimeError::Sandbox(_))),
        "{result:?}"
    );
    assert!(!marker.exists(), "the program ran unconfined");
}

#[cfg(not(target_os = "linux"))]
#[test]
fn execution_fails_closed_off_linux() {
    let scratch = Scratch::new("off-linux");
    let result = run_python(
        &scratch,
        "print('ran')\n",
        limits(2, Duration::from_secs(10), 256),
    );
    assert!(
        matches!(result, Err(RuntimeError::Sandbox(_))),
        "{result:?}"
    );
}

// --- Reference solutions that should beat the baselines -------------------

const LEAST_SQUARES: &str = r#"
import csv, sys

def features(r):
    x1, x2, x3 = float(r["x1"]), float(r["x2"]), float(r["x3"])
    return [1.0, x1, x2, x3, x3 * x3, x1 * x2]

with open("data/train.csv") as f:
    rows = list(csv.DictReader(f))
X = [features(r) for r in rows]
y = [float(r["y"]) for r in rows]
k = len(X[0])
A = [[sum(a[i] * a[j] for a in X) for j in range(k)] + [sum(a[i] * t for a, t in zip(X, y))] for i in range(k)]
for c in range(k):
    p = max(range(c, k), key=lambda r: abs(A[r][c]))
    A[c], A[p] = A[p], A[c]
    for r in range(k):
        if r != c:
            f = A[r][c] / A[c][c]
            A[r] = [v - f * w for v, w in zip(A[r], A[c])]
w = [A[i][k] / A[i][i] for i in range(k)]
with open(sys.argv[1]) as f, open(sys.argv[2], "w") as out:
    for r in csv.DictReader(f):
        out.write(f"{sum(a * b for a, b in zip(w, features(r)))}\n")
"#;

const NEAREST_NEIGHBOUR: &str = r#"
import math, sys
with open(sys.argv[1]) as f, open(sys.argv[2], "w") as out:
    for line in f:
        v = list(map(float, line.split()))
        pts = list(zip(v[0::2], v[1::2]))
        order, left = [0], set(range(1, len(pts)))
        while left:
            here = pts[order[-1]]
            nxt = min(left, key=lambda c: math.dist(here, pts[c]))
            order.append(nxt)
            left.remove(nxt)
        out.write(" ".join(map(str, order)) + "\n")
"#;

const MAJORITY_VOTE: &str = r#"
import sys
from collections import Counter
sys.path.insert(0, "data")
import oracle
with open(sys.argv[1]) as f, open(sys.argv[2], "w") as out:
    for q in f:
        votes = Counter(oracle.ask(q.strip(), attempt) for attempt in range(5))
        out.write(votes.most_common(1)[0][0] + "\n")
"#;
