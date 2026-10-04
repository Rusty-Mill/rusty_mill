//! The inner loop end to end (ADR-0005 P3): a0 built from source with
//! plain rustc, run in the sandbox against the broker, a scripted model and
//! a real toy task.
//!
//! - Invariant 2: the token and wall-clock budgets are hard stops.
//! - Invariant 4: a run replays from its transcript to the same submission.
//! - Invariant 1(b): the agent cannot reach private data, the repository or
//!   any socket but the broker's.
//!
//! Everything here needs Linux (Landlock, seccomp), except the fail-closed
//! test at the bottom.
#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]

mod common;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use common::{executor, suite, suite_dir, task, Scratch};
use rsi_core::{Budget, Harness, InnerOutcome, ModelId, PublicTask, Seed, Solution};
use rsi_runtime::harness::{build, BuildFailure, HarnessBinary};
use rsi_runtime::protocol::{decode_transcript, Response};
use rsi_runtime::{
    HarnessProcess, LocalTask, ProcessExecutor, RuntimeError, SandboxedHarness, ScriptedModel,
    SolutionRunner, TaskDir, Toolchain,
};

const SEED: Seed = Seed::new(11);
const GRACE: Duration = Duration::from_secs(1);

fn harness_source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../harness")
}

fn toolchain() -> Toolchain {
    let rustc = std::env::var_os("RSI_RUSTC").unwrap_or_else(|| "rustc".into());
    Toolchain::detect(&rustc).expect("a Rust toolchain")
}

/// Builds the crate at `source` into `scratch`.
fn build_in(scratch: &Scratch, source: &Path) -> Result<HarnessBinary, BuildFailure> {
    build(
        &executor(scratch),
        &toolchain(),
        source,
        &scratch.path("build"),
    )
    .expect("build runs")
}

/// a0, built once per test binary (a release build takes seconds).
fn a0() -> HarnessBinary {
    static A0: OnceLock<HarnessBinary> = OnceLock::new();
    A0.get_or_init(|| {
        let dir =
            Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("rsi-a0-{}", std::process::id()));
        if dir.exists() {
            std::fs::remove_dir_all(&dir).expect("clearing an old build");
        }
        let executor = ProcessExecutor::new(
            common::RSI.into(),
            vec!["__sandbox".into()],
            dir.join("state"),
        );
        build(
            &executor,
            &toolchain(),
            &harness_source(),
            &dir.join("build"),
        )
        .expect("build runs")
        .expect("a0 compiles")
    })
    .clone()
}

fn runner(scratch: &Scratch) -> SolutionRunner<ProcessExecutor> {
    let roots: Vec<PathBuf> = rsi_runtime::grading::SYSTEM_READ_ROOTS
        .iter()
        .map(PathBuf::from)
        .collect();
    SolutionRunner::new(executor(scratch), &roots, &scratch.path("solutions")).expect("runner")
}

fn process<'a>(
    executor: &'a ProcessExecutor,
    binary: HarnessBinary,
    scratch: &Scratch,
) -> HarnessProcess<'a> {
    std::fs::create_dir_all(scratch.path("state")).expect("state dir");
    HarnessProcess::new(
        executor,
        binary,
        &scratch.path("agent"),
        &[suite_dir(), scratch.path("state")],
        GRACE,
    )
    .expect("process")
}

fn reply(code: &str) -> String {
    format!("Here is the plan.\n```python\n{code}```\n")
}

const CRASH: &str = "raise SystemExit(3)\n";

const IDENTITY: &str = r#"import sys
with open(sys.argv[1]) as f, open(sys.argv[2], "w") as out:
    for line in f:
        out.write(" ".join(map(str, range(len(line.split()) // 2))) + "\n")
"#;

const NEAREST: &str = r#"import math, sys
with open(sys.argv[1]) as f, open(sys.argv[2], "w") as out:
    for line in f:
        v = list(map(float, line.split()))
        pts = list(zip(v[0::2], v[1::2]))
        order, left = [0], set(range(1, len(pts)))
        while left:
            here = pts[order[-1]]
            nxt = min(left, key=lambda c: (math.dist(here, pts[c]), c))
            order.append(nxt)
            left.remove(nxt)
        out.write(" ".join(map(str, order)) + "\n")
"#;

/// Cycles a crash, the identity tour and nearest neighbour, at 150 tokens
/// a call.
fn scripted() -> ScriptedModel {
    ScriptedModel::new(
        ModelId::parse("scripted").expect("valid"),
        vec![reply(CRASH), reply(IDENTITY), reply(NEAREST)],
        100,
        50,
    )
}

fn budget(tokens: u64, wall: Duration) -> Budget {
    Budget::new(tokens, wall, None).expect("valid budget")
}

fn run_a0(
    scratch: &Scratch,
    tsp: &TaskDir,
    model: &ScriptedModel,
    budget: &Budget,
) -> InnerOutcome {
    let executor = executor(scratch);
    let runner = runner(scratch);
    let public = LocalTask::new(tsp, &runner);
    let harness = SandboxedHarness::new(process(&executor, a0(), scratch), model, 64);
    harness
        .run(&public, budget, SEED)
        .expect("the inner run completes")
}

// --- Invariant 2: the budget is a hard stop -------------------------------

#[cfg(target_os = "linux")]
#[test]
fn a0_searches_until_the_token_budget_stops_it() {
    let scratch = Scratch::new("a0-tokens");
    let tsp = task(&suite(), "tsp-heuristic");
    let model = scripted();
    let outcome = run_a0(
        &scratch,
        &tsp,
        &model,
        &budget(1_200, Duration::from_secs(120)),
    );

    assert_eq!(model.calls(), 8, "1,200 tokens buy exactly 8 calls at 150");
    assert_eq!(outcome.usage.tokens(), 1_200);
    let exchanges = decode_transcript(&outcome.transcript).expect("transcript");
    let last = exchanges.last().expect("exchanges");
    assert!(
        matches!(last.response, Response::Exhausted(_)),
        "the budget ended the run: {last:?}"
    );
    let evaluated = exchanges
        .iter()
        .filter(|e| matches!(e.response, Response::Evaluated { .. }))
        .count();
    assert_eq!(
        evaluated, 7,
        "the eighth program was refused: no tokens left"
    );

    let submitted = outcome.submission.expect("a0 submitted its best");
    assert_eq!(submitted.source(), NEAREST);
    let runner = runner(&scratch);
    let public = LocalTask::new(&tsp, &runner);
    let score = |s: &Solution| {
        public
            .public_score(s, SEED)
            .expect("scores")
            .score
            .expect("valid")
            .get()
    };
    assert!(score(&submitted) > score(tsp.baseline()) + 0.3);
}

/// An agent that submits once and then never stops.
fn stubborn_agent() -> String {
    rogue_agent(
        r#"submit(&mut broker, "print('kept')\n");
    loop {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }"#,
    )
}

#[cfg(target_os = "linux")]
#[test]
fn an_agent_that_ignores_the_wall_clock_is_killed_and_keeps_its_submission() {
    let scratch = Scratch::new("a0-wall");
    let binary = build_in(&scratch, &source_crate(&scratch, &stubborn_agent())).expect("builds");
    let tsp = task(&suite(), "tsp-heuristic");
    let (executor, runner, model) = (executor(&scratch), runner(&scratch), scripted());
    let public = LocalTask::new(&tsp, &runner);
    let harness = SandboxedHarness::new(process(&executor, binary, &scratch), &model, 64);

    let wall = Duration::from_secs(2);
    let started = Instant::now();
    let outcome = harness
        .run(&public, &budget(1_000, wall), SEED)
        .expect("runs");
    let elapsed = started.elapsed();
    assert!(elapsed >= wall + GRACE, "killed early: {elapsed:?}");
    assert!(
        elapsed < wall + GRACE + Duration::from_secs(5),
        "{elapsed:?}"
    );
    assert!(outcome.usage.wall >= wall);
    assert_eq!(
        outcome.submission.map(|s| s.source().to_owned()),
        Some("print('kept')\n".to_owned())
    );
}

// --- Invariant 4: trajectory replay ---------------------------------------

#[cfg(target_os = "linux")]
#[test]
fn a_run_replays_from_its_transcript_without_the_model() {
    let scratch = Scratch::new("a0-replay");
    let tsp = task(&suite(), "tsp-heuristic");
    let model = scripted();
    let wall = Duration::from_secs(120);
    let outcome = run_a0(&scratch, &tsp, &model, &budget(1_500, wall));
    let calls = model.calls();

    let executor = executor(&scratch);
    let process = process(&executor, a0(), &scratch);
    let replayed = process
        .replay(tsp.description(), SEED, wall, &outcome.transcript)
        .expect("replays");
    assert_eq!(replayed, outcome.submission);
    assert!(replayed.is_some());
    assert_eq!(model.calls(), calls, "replay never calls the model");

    let diverged = process.replay("another task", SEED, wall, &outcome.transcript);
    assert!(
        matches!(diverged, Err(RuntimeError::Broker(_))),
        "{diverged:?}"
    );
}

// --- Invariant 1(b): the agent is isolated --------------------------------

/// A minimal agent crate whose `main` gets a connected `broker` and a
/// `submit` helper, then runs `body`.
fn rogue_agent(body: &str) -> String {
    format!(
        r#"use std::io::{{Read, Write}};
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;

fn submit(broker: &mut UnixStream, text: &str) {{
    let mut body = vec![3u8];
    body.extend_from_slice(&(text.len() as u32).to_be_bytes());
    body.extend_from_slice(text.as_bytes());
    broker.write_all(&(body.len() as u32).to_be_bytes()).unwrap();
    broker.write_all(&body).unwrap();
    let mut reply = [0u8; 5];
    broker.read_exact(&mut reply).unwrap();
}}

pub fn main() {{
    let socket = std::io::stdin().as_fd().try_clone_to_owned().unwrap();
    let mut broker = UnixStream::from(socket);
    {body}
}}
"#
    )
}

/// Writes `lib` as the `src/lib.rs` of a crate under `scratch`.
fn source_crate(scratch: &Scratch, lib: &str) -> PathBuf {
    let dir = scratch.path("candidate");
    std::fs::create_dir_all(dir.join("src")).expect("mkdir");
    std::fs::write(dir.join("src/lib.rs"), lib).expect("write");
    dir
}

#[cfg(target_os = "linux")]
#[test]
fn the_agent_cannot_reach_private_data_the_repository_or_any_socket() {
    let scratch = Scratch::new("a0-isolation");
    let tsp = task(&suite(), "tsp-heuristic");
    let root = tsp.root().display().to_string();
    let repository = suite_dir()
        .canonicalize()
        .expect("suite")
        .ancestors()
        .nth(6)
        .expect("repository root")
        .display()
        .to_string();
    let body = format!(
        r#"let mut report = String::new();
    for path in ["{root}/private/labels.txt", "{root}/public/labels.txt"] {{
        let result = std::fs::read(path).map(|_| ());
        report += &format!("{{path}} {{:?}}\n", result.map_err(|e| e.kind()));
    }}
    for path in ["{root}", "{repository}"] {{
        let result = std::fs::read_dir(path).map(|_| ());
        report += &format!("{{path}} {{:?}}\n", result.map_err(|e| e.kind()));
    }}
    let tcp = std::net::TcpStream::connect("127.0.0.1:9").map(|_| ());
    report += &format!("tcp {{:?}}\n", tcp.map_err(|e| e.kind()));
    let unix = std::os::unix::net::UnixDatagram::unbound().map(|_| ());
    report += &format!("unix {{:?}}\n", unix.map_err(|e| e.kind()));
    submit(&mut broker, &report);"#
    );
    let binary = build_in(&scratch, &source_crate(&scratch, &rogue_agent(&body))).expect("builds");
    let (executor, runner, model) = (executor(&scratch), runner(&scratch), scripted());
    let public = LocalTask::new(&tsp, &runner);
    let harness = SandboxedHarness::new(process(&executor, binary, &scratch), &model, 64);
    let outcome = harness
        .run(&public, &budget(1_000, Duration::from_secs(30)), SEED)
        .expect("runs");
    let report = outcome
        .submission
        .expect("the agent reported")
        .source()
        .to_owned();
    let lines: Vec<&str> = report.lines().collect();
    assert_eq!(lines.len(), 6, "{report}");
    for line in lines {
        assert!(
            line.ends_with("Err(PermissionDenied)"),
            "the agent got through: {line}\n{}",
            outcome.log
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn an_agent_that_could_reach_a_protected_path_is_not_started() {
    let scratch = Scratch::new("a0-protected");
    let binary = a0();
    let executor = executor(&scratch);
    let exposed = HarnessProcess::new(
        &executor,
        binary.clone(),
        &scratch.path("agent"),
        &[binary.path().parent().expect("bin dir").to_path_buf()],
        GRACE,
    )
    .expect("process");
    let tsp = task(&suite(), "tsp-heuristic");
    let replay = exposed.replay(tsp.description(), SEED, Duration::from_secs(5), &[]);
    assert!(
        matches!(replay, Err(RuntimeError::Sandbox(_))),
        "{replay:?}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_candidate_that_does_not_compile_is_a_build_failure() {
    let scratch = Scratch::new("a0-broken");
    let source = source_crate(&scratch, "pub fn main() { let x: u8 = \"no\"; }\n");
    let failure = build_in(&scratch, &source).expect_err("does not compile");
    assert!(failure.0.contains("mismatched types"), "{}", failure.0);
}

#[cfg(not(target_os = "linux"))]
#[test]
fn building_fails_closed_off_linux() {
    let scratch = Scratch::new("a0-off-linux");
    let result = build(
        &executor(&scratch),
        &toolchain(),
        &harness_source(),
        &scratch.path("build"),
    );
    assert!(
        matches!(result, Err(RuntimeError::Sandbox(_))),
        "{result:?}"
    );
}
