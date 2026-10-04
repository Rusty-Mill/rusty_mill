//! The outer loop end to end (ADR-0005 P4): a 10-step run in a throwaway
//! git repository holding the harness, on the real toy suite, with a
//! scripted inner model and scripted proposals, then calibration, the
//! report and its replay.
//!
//! - Invariant 3: a candidate is accepted only when a fresh, disjoint seed
//!   set beats the incumbent by more than the margin; equal or merely
//!   better-within-noise candidates are rejected.
//! - Invariant 4: the lineage is a verified hash chain, and the run
//!   replays: every private score bit for bit, every inner run from its
//!   transcript. Tampering with either is caught.
//! - The allowlist: edits outside the harness source, and symlinks, are
//!   rejected as path violations and never built.
//!
//! Linux only (the sandbox); `git`, `rustc` and `python3` must be on PATH.
#![cfg(target_os = "linux")]

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::{executor, suite, suite_dir, Scratch, RSI};
use rsi_core::{
    Budget, CandidateId, Decision, LineageEntry, LineageStore, Margin, ModelId, Seed, Verdict,
};
use rsi_runtime::git::{Repo, HARNESS_DIR, HARNESS_SRC};
use rsi_runtime::grading::{GraderCommand, SYSTEM_READ_ROOTS};
use rsi_runtime::lineage_store::{Blobs, JsonlLineage, BLOB_DIR, LINEAGE_FILE};
use rsi_runtime::outer::{calibrate, read_run, run, Grading, Lab, RunConfig};
use rsi_runtime::proposer::{ScriptedEdit, ScriptedProposer};
use rsi_runtime::report::{replay, summary};
use rsi_runtime::{
    ProcessExecutor, RuntimeError, ScriptedModel, SolutionRunner, TaskDir, Toolchain,
};

const WEAK: &str = include_str!("solutions/weak.py");
const MID: &str = include_str!("solutions/mid.py");
const STRONG: &str = include_str!("solutions/strong.py");

fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A git repository holding a copy of the harness crate at its real path,
/// plus a decoy file elsewhere.
fn harness_repo(scratch: &Scratch) -> Repo {
    let root = scratch.path("repo");
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../harness");
    let target = root.join(HARNESS_DIR);
    std::fs::create_dir_all(target.join("src")).expect("mkdir");
    std::fs::copy(source.join("Cargo.toml"), target.join("Cargo.toml")).expect("manifest");
    for entry in std::fs::read_dir(source.join("src")).expect("src") {
        let entry = entry.expect("entry");
        std::fs::copy(entry.path(), target.join("src").join(entry.file_name())).expect("copy");
    }
    std::fs::write(root.join("Cargo.toml"), "[workspace]\n").expect("decoy");
    git(&root, &["init", "--quiet"]);
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "harness"]);
    Repo::open(&root).expect("repo")
}

/// A harness that ignores the model and submits `solution` at once.
fn fixed_agent(solution: &str) -> String {
    format!(
        r####"use std::io::{{Read, Write}};
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;

const SOLUTION: &str = r###"{solution}"###;

pub fn main() {{
    let socket = std::io::stdin().as_fd().try_clone_to_owned().unwrap();
    let mut broker = UnixStream::from(socket);
    let mut body = vec![3u8];
    body.extend_from_slice(&(SOLUTION.len() as u32).to_be_bytes());
    body.extend_from_slice(SOLUTION.as_bytes());
    broker.write_all(&(body.len() as u32).to_be_bytes()).unwrap();
    broker.write_all(&body).unwrap();
    let mut len = [0u8; 4];
    broker.read_exact(&mut len).unwrap();
    let mut reply = vec![0u8; u32::from_be_bytes(len) as usize];
    broker.read_exact(&mut reply).unwrap();
}}
"####
    )
}

fn reply(code: &str) -> String {
    format!("Plan.\n```python\n{code}```\n")
}

/// The ten proposals, in order, with the verdict each must get.
fn script() -> (Vec<ScriptedEdit>, Vec<Verdict>) {
    let src = |name: &str| format!("{HARNESS_SRC}{name}");
    let symlink = ScriptedEdit {
        summary: "link a file outside the harness".into(),
        apply: Box::new(move |workspace: &Path| {
            let link = workspace.join(src("leak.rs"));
            std::os::unix::fs::symlink("/etc/hostname", link)
                .map_err(|e| RuntimeError::io("symlink", e))
        }),
    };
    let steps = vec![
        (ScriptedEdit::harness("weak, as good as a0", &fixed_agent(WEAK)), Verdict::NotBetter),
        (ScriptedEdit::harness("nearest-neighbour tours", &fixed_agent(MID)), Verdict::WithinNoise),
        (
            ScriptedEdit::files(
                "edit the harness manifest",
                vec![(format!("{HARNESS_DIR}/Cargo.toml"), "[package]\nname = \"x\"\n".into())],
            ),
            Verdict::PathViolation,
        ),
        (ScriptedEdit::harness("does not compile", "pub fn main() { let x: u8 = \"no\"; }\n"), Verdict::Buggy),
        (ScriptedEdit::harness("solve every task well", &fixed_agent(STRONG)), Verdict::Accepted),
        (ScriptedEdit::harness("back to tours only", &fixed_agent(MID)), Verdict::NotBetter),
        (symlink, Verdict::PathViolation),
        (
            ScriptedEdit::files("change nothing", vec![]),
            Verdict::NotBetter,
        ),
        (
            ScriptedEdit::files(
                "rewrite the private labels",
                vec![(
                    "crates/apps/rusty_rsi/crates/rsi-runtime/tasks/tsp-heuristic/private/labels.txt".into(),
                    "1\n".into(),
                )],
            ),
            Verdict::PathViolation,
        ),
        (ScriptedEdit::harness("the same strong agent", &fixed_agent(STRONG)), Verdict::NotBetter),
    ];
    steps.into_iter().unzip()
}

fn toolchain() -> Toolchain {
    let rustc = std::env::var_os("RSI_RUSTC").unwrap_or_else(|| "rustc".into());
    Toolchain::detect(&rustc).expect("a Rust toolchain")
}

fn runner(scratch: &Scratch) -> SolutionRunner<ProcessExecutor> {
    let roots: Vec<PathBuf> = SYSTEM_READ_ROOTS.iter().map(PathBuf::from).collect();
    SolutionRunner::new(executor(scratch), &roots, &scratch.path("solutions")).expect("runner")
}

fn grading() -> Grading {
    Grading {
        run_seed: Seed::new(2026),
        seeds_per_round: 1,
        budget: Budget::new(900, Duration::from_secs(60), None).expect("budget"),
        max_completion_tokens: 64,
        grace: Duration::from_secs(2),
    }
}

struct World {
    scratch: Scratch,
    repo: Repo,
    tasks: Vec<TaskDir>,
    executor: ProcessExecutor,
    toolchain: Toolchain,
    runner: SolutionRunner<ProcessExecutor>,
    model: ScriptedModel,
}

impl World {
    fn new(name: &str) -> Self {
        let scratch = Scratch::new(name);
        for dir in ["state", "run"] {
            std::fs::create_dir_all(scratch.path(dir)).expect("mkdir");
        }
        Self {
            repo: harness_repo(&scratch),
            tasks: suite(),
            executor: executor(&scratch),
            toolchain: toolchain(),
            runner: runner(&scratch),
            // a0 is shown only the weak program, so it submits that.
            model: ScriptedModel::new(
                ModelId::parse("scripted").expect("id"),
                vec![reply(WEAK)],
                100,
                50,
            ),
            scratch,
        }
    }

    fn lab(&self) -> Lab<'_, ScriptedModel> {
        Lab {
            repo: &self.repo,
            tasks: &self.tasks,
            executor: &self.executor,
            toolchain: &self.toolchain,
            runner: &self.runner,
            grader: GraderCommand {
                program: RSI.into(),
                args: vec!["__grade".into()],
            },
            model: &self.model,
            scratch: self.scratch.path("work"),
            protected: vec![
                suite_dir(),
                self.repo.root().to_path_buf(),
                self.scratch.path("run"),
                self.scratch.path("state"),
            ],
        }
    }
}

fn seeds(entry: &LineageEntry, round: usize) -> Vec<u64> {
    entry.fields().evaluations[round]
        .results
        .iter()
        .map(|r| r.seed.get())
        .collect()
}

#[test]
fn a_ten_step_run_gates_rejects_records_and_replays() {
    let world = World::new("outer-run");
    let lab = world.lab();
    let (edits, expected) = script();
    let proposer = ScriptedProposer::new(edits);
    let run_dir = world.scratch.path("run");
    let config = RunConfig {
        name: "ten-steps".into(),
        base: world.repo.resolve("HEAD").expect("head"),
        steps: 10,
        margin: Margin::new(0.3).expect("margin"),
        grading: grading(),
    };
    let entries = run(&lab, &proposer, &config, &run_dir).expect("the run completes");

    // Every candidate got the verdict its edit deserves.
    let verdicts: Vec<Verdict> = entries
        .iter()
        .map(|e| Verdict::of(&e.fields().decision))
        .collect();
    let mut wanted = vec![Verdict::Baseline];
    wanted.extend(expected);
    assert_eq!(verdicts, wanted);
    assert_eq!(
        proposer.history_lengths(),
        (1..=10).collect::<Vec<_>>(),
        "each step sees the lineage so far"
    );

    // Invariant 3: acceptance came from a fresh, disjoint seed set, and
    // the parent of every later candidate is the new incumbent.
    let accepted = &entries[5];
    let (first, fresh) = (seeds(accepted, 0), seeds(accepted, 1));
    assert!(
        first.iter().all(|s| !fresh.contains(s)),
        "{first:?} {fresh:?}"
    );
    for later in &entries[6..] {
        assert_eq!(later.fields().parent, Some(CandidateId::new(5)));
    }
    for early in &entries[1..=5] {
        assert_eq!(early.fields().parent, Some(CandidateId::BASELINE));
    }
    match &entries[2].fields().decision {
        Decision::Rejected(rsi_core::Rejection::WithinNoise {
            incumbent,
            fresh,
            margin,
        }) => {
            assert!(
                fresh.get() > incumbent.get(),
                "better, but not by the margin"
            );
            assert!(fresh.get() - incumbent.get() <= margin.get());
        }
        other => panic!("{other:?}"),
    }

    // Path violations and the broken build were never graded; every
    // proposal's commit is kept under refs/rsi/<run>/<step>.
    for index in [3, 4, 7, 9] {
        assert!(
            entries[index].fields().evaluations.is_empty(),
            "candidate {index}"
        );
    }
    let refs = git(
        world.repo.root(),
        &["for-each-ref", "--format=%(refname)", "refs/rsi/ten-steps/"],
    );
    assert_eq!(refs.lines().count(), 10, "{refs}");
    assert_eq!(
        git(world.repo.root(), &["branch", "--list"])
            .lines()
            .count(),
        1,
        "no new branches"
    );

    // Invariant 4: the stored lineage is the returned one, and it replays.
    let store = JsonlLineage::open(&run_dir).expect("verified chain");
    assert_eq!(store.entries().expect("entries"), entries);
    let info = read_run(&run_dir).expect("run.json");
    let blobs = Blobs::open(&run_dir).expect("blobs");
    let report = summary(&info, &entries).expect("summary");
    println!("{report}");
    assert!(report.contains("| 5 | 0 | accepted |"), "{report}");
    assert!(
        report.contains("11 (baseline + 10 proposals), 1 accepted"),
        "{report}"
    );
    let replayed = replay(&lab, &info, &entries, &blobs).expect("replays");
    assert!(replayed.mismatches.is_empty(), "{:?}", replayed.mismatches);
    // Graded: baseline (1 round), 1 (1), 2 (2), 5 (2), 6 (1), 8 (1), 10 (1),
    // each over the three tasks.
    assert_eq!(replayed.grades, 9 * 3);
    assert_eq!(replayed.trajectories, 9 * 3);

    // Tampering with a stored submission is caught by its address...
    let solution = entries[5].fields().evaluations[1].results[0]
        .solution
        .expect("a submission");
    let path = run_dir.join(BLOB_DIR).join(solution.to_string());
    let original = std::fs::read(&path).expect("blob");
    std::fs::write(&path, b"print('forged')\n").expect("tamper");
    assert!(matches!(
        replay(&lab, &info, &entries, &blobs),
        Err(RuntimeError::Lineage(_))
    ));
    std::fs::write(&path, original).expect("restore");

    // ...and editing a recorded grade breaks the chain.
    let lineage = run_dir.join(LINEAGE_FILE);
    let text = std::fs::read_to_string(&lineage).expect("lineage");
    let forged = text.replacen("\"kind\":\"within_noise\"", "\"kind\":\"accepted\"", 1);
    assert_ne!(forged, text);
    std::fs::write(&lineage, forged).expect("tamper");
    assert!(JsonlLineage::open(&run_dir).is_err());
    std::fs::write(&lineage, text).expect("restore");

    // A finished run cannot be overwritten.
    assert!(matches!(
        run(&lab, &proposer, &config, &run_dir),
        Err(RuntimeError::Lineage(_))
    ));
}

#[test]
fn calibration_grades_the_harness_on_disjoint_seed_sets() {
    let world = World::new("outer-calibrate");
    let mut lab = world.lab();
    let tsp: Vec<TaskDir> = world
        .tasks
        .iter()
        .filter(|t| t.manifest().id.as_str() == "tsp-heuristic")
        .cloned()
        .collect();
    lab.tasks = &tsp;
    let blobs = Blobs::open(&world.scratch.path("calibration")).expect("blobs");
    let base = world.repo.resolve("HEAD").expect("head");
    let calibration = calibrate(&lab, &base, &grading(), 3, 1.645, &blobs).expect("calibrates");
    assert_eq!(calibration.grades.len(), 3);
    // The scripted model and the toy task are deterministic, so the three
    // rounds agree and the margin is zero; a real model gives a spread.
    assert_eq!(calibration.band.std_dev(), 0.0);
    assert_eq!(calibration.margin.get(), 0.0);
    assert!(
        calibrate(&lab, &base, &grading(), 1, 1.645, &blobs).is_err(),
        "two rounds at least"
    );
}
