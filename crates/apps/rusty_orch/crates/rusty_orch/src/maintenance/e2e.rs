//! Real BBP CLI, moderator, MCP providers and sandboxed Cargo validation.
//! Provider scripts are deterministic fixtures; approval is a separate human CLI.
#![cfg(target_os = "linux")]
use super::*;
use orch_cli::{CommandRunner, ExecError, Exit};
use rusty_bbp::{ArtifactKind::*, MessageKind::*, Role as BbpRole, Stage};
use std::process::Command;
use std::time::Instant;

struct LocalPullRequests;
impl CommandRunner for LocalPullRequests {
    fn run_in(
        &self,
        cwd: Option<&Path>,
        argv: &[String],
        stdin: &[u8],
        timeout: Duration,
        scrub: &[&str],
    ) -> std::result::Result<Exit, ExecError> {
        if argv[0] == "gh" {
            return Ok(Exit {
                status: 0,
                stdout: b"[]".to_vec(),
                stderr: vec![],
            });
        }
        StdCommand.run_in(cwd, argv, stdin, timeout, scrub)
    }
}
fn process(repo: &Path, program: &str, args: &[&str]) -> String {
    let out = Command::new(program)
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{program} {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().into()
}
fn current(config: &orch_bbp::Config, attempt: &Attempt) -> rusty_bbp::TaskView {
    let h: Value = json::from_str(
        attempt
            .handle
            .as_ref()
            .unwrap()
            .strip_prefix("bbp:")
            .unwrap(),
    )
    .unwrap();
    let mut store = FsStore::open(&config.dir).unwrap();
    let id = rusty_bbp::TaskId(h["task"].as_str().unwrap().into());
    store.refresh(&id).unwrap();
    rusty_bbp::TaskView::from_events(id.clone(), &store.events(&id, 0).unwrap())
}
fn cycle(options: &Options, expected: &str) -> Journal {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let j: Journal = json::from_str(&execute(options, &LocalPullRequests).unwrap()).unwrap();
        assert_eq!(j.attempts.len(), 1);
        if j.attempts[0].status == expected {
            return j;
        }
        assert!(
            Instant::now() < deadline,
            "maintenance did not reach {expected}: {j:?}"
        );
        assert!(!j.attempts[0].detail.contains("Escalated"), "{j:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}
#[test]
#[ignore = "requires BBP_TEST_BIN; Linux Landlock/seccomp and installed Rust/Cargo/Python"]
fn sandboxed_maintenance_recovers_and_prepares_reviewed_pr_without_merging() {
    let root = super::tests::temp();
    let repo = root.join("repo");
    fs::create_dir_all(repo.join("crates/apps/demo/src")).unwrap();
    fs::write(
        repo.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/apps/demo\"]\nresolver = \"2\"\n",
    )
    .unwrap();
    let manifest = "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[package.metadata.rusty_mill]\nlayer = \"invalid\"\n";
    fs::write(repo.join("crates/apps/demo/Cargo.toml"), manifest).unwrap();
    fs::write(
        repo.join("crates/apps/demo/src/lib.rs"),
        "pub fn demo() {}\n",
    )
    .unwrap();
    process(&repo, "git", &["init", "-q"]);
    process(&repo, "git", &["config", "user.name", "Test"]);
    process(
        &repo,
        "git",
        &["config", "user.email", "test@example.invalid"],
    );
    process(&repo, "cargo", &["generate-lockfile", "--offline"]);
    process(&repo, "git", &["add", "."]);
    process(&repo, "git", &["commit", "-qm", "base"]);
    let base = process(&repo, "git", &["rev-parse", "HEAD"]);
    let binary = PathBuf::from(std::env::var_os("BBP_TEST_BIN").expect("BBP_TEST_BIN"))
        .canonicalize()
        .unwrap();
    let sysroot = process(&repo, "rustc", &["--print", "sysroot"]);
    let input_profiles = root.join("profiles.json");
    let profiles = format!(
        r#"{{"profiles":[{{"name":"fixture","program":"git","args":["status","--porcelain"]}}],"read_roots":["/bin","/usr","/lib","/lib64",{}],"env":[["PATH",{}],["CARGO_HOME",".maintenance-cargo"],["RUSTFLAGS","-C link-arg=-Wl,--threads=1"]],"limits":{{"cpu_secs":60,"wall_secs":60,"memory_bytes":1073741824,"file_bytes":268435456,"open_files":256,"processes":4096}},"tree":"sha256(git write-tree id)"}}"#,
        json::to_string(&sysroot).unwrap(),
        json::to_string(&format!("{sysroot}/bin:/usr/local/bin:/usr/bin:/bin")).unwrap()
    );
    fs::write(&input_profiles, profiles).unwrap();
    let init = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"maintenance-fixture","version":"1"}}}"#;
    let planning = r##"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"put_artifact","arguments":{"op":"plan","kind":"spec","brief":1,"body":"# Spec\nRestore demo's directory layer to apps; no dependency changes. Human approval required."}}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"post","arguments":{"op":"gate","kind":"request_decision","to":["human"],"refs":["art:2"],"body":"Approve the metadata-only repair?","gate":true}}}"##;
    let patch = "--- a/crates/apps/demo/Cargo.toml\n+++ b/crates/apps/demo/Cargo.toml\n@@ -4,3 +4,3 @@\n edition = \"2021\"\n [package.metadata.rusty_mill]\n-layer = \"invalid\"\n+layer = \"apps\"\n";
    let build = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"put_artifact","arguments":{{"op":"diff","kind":"diff","body":{}}}}}}}
{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"put_artifact","arguments":{{"op":"candidate","kind":"candidate","base":"{base}","diffs":[3],"body":""}}}}}}"#,
        json::to_string(patch).unwrap()
    );
    let verdict = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"post","arguments":{"op":"verdict","kind":"verdict","to":["*"],"refs":["art:4"],"evidence":["art:6"],"verdict":{"subject":4,"run":1,"verdict":"approve","blocking":[],"non_blocking":[]}}}}"#;
    let maker = root.join("maker.sh");
    let auditor = root.join("auditor.sh");
    fs::write(&maker, format!("set -e\nstate=$(\"$BBP_BIN\" card --dir \"$BBP_DIR\" --task \"$BBP_TASK\")\ncase \"$state\" in\n*'\"Planning\"'*) \"$BBP_BIN\" mcp <<'RPC'\n{init}\n{planning}\nRPC\n;;\n*'\"Build\"'*) \"$BBP_BIN\" mcp <<'RPC'\n{init}\n{build}\nRPC\n;;\n*'\"Test\"'*) \"$BBP_BIN\" mcp <<'RPC'\n{init}\n{verdict}\nRPC\n;;\nesac\n")).unwrap();
    fs::write(
        &auditor,
        format!("set -e\n\"$BBP_BIN\" mcp <<'RPC'\n{init}\n{verdict}\nRPC\n"),
    )
    .unwrap();
    let config = orch_bbp::Config {
        binary: binary.clone(),
        dir: root.join("bbp"),
        repo: repo.clone(),
        work: root.join("work"),
        profiles: input_profiles,
        pool: BTreeMap::from([
            (
                "worker".into(),
                orch_bbp::Agent {
                    vendor: "fixture-one".into(),
                    program: "/bin/sh".into(),
                    args: vec![maker.to_string_lossy().into()],
                },
            ),
            (
                "checker".into(),
                orch_bbp::Agent {
                    vendor: "fixture-two".into(),
                    program: "/bin/sh".into(),
                    args: vec![auditor.to_string_lossy().into()],
                },
            ),
        ]),
        roster: vec![
            BbpRole::new("repairer", 0)
                .post(&[RequestDecision, Verdict, Pass])
                .write(&[Spec, Diff, Candidate])
                .read_artifacts(&[Brief, Spec, Diff, Candidate, TestReport])
                .verdict_at(&[Stage::Test]),
            BbpRole::new("independent", 1)
                .post(&[Verdict, Pass])
                .read_artifacts(&[Brief, Spec, Diff, Candidate, TestReport])
                .scoped()
                .verdict_at(&[Stage::Review])
                .final_decision(),
        ],
        assignments: BTreeMap::from([
            (
                "repairer".into(),
                orch_bbp::Assignment {
                    agent: "worker".into(),
                    builtin_tools: false,
                },
            ),
            (
                "independent".into(),
                orch_bbp::Assignment {
                    agent: "checker".into(),
                    builtin_tools: false,
                },
            ),
        ]),
        live_turns: 1,
        max_wall_secs: 120,
    };
    let file = root.join("bbp.json");
    fs::write(&file, json::to_string(&config).unwrap()).unwrap();
    let options = Options {
        repo: repo.clone(),
        bbp: Some(file),
        execute: true,
        publish_pr: false,
        benefits: BTreeMap::new(),
    };
    let mut journal = cycle(&options, "waiting-human");
    let view = current(&config, &journal.attempts[0]);
    assert_eq!(view.card.state, State::PlanGate);
    // Lose the Orch checkpoint after the host opened: durable namespace adopts
    // the same task, whose moderator can also be killed/restarted at the gate.
    let task = view.card.task.0.clone();
    let pid = fs::read_to_string(
        config
            .dir
            .join("mod")
            .join(format!("{}.pid", Sha256::of(task.as_bytes()).hex())),
    )
    .unwrap();
    process(&repo, "kill", &["-KILL", pid.trim()]);
    journal.attempts[0].handle = None;
    journal.attempts[0].status = "opening".into();
    let state_dir = repo.join(".git/rusty-orch-maintenance");
    save(&state_dir.join("journal.json"), &journal).unwrap();
    let recovered = cycle(&options, "waiting-human");
    assert_eq!(current(&config, &recovered.attempts[0]).card.task.0, task);
    let view = current(&config, &recovered.attempts[0]);
    process(
        &repo,
        binary.to_str().unwrap(),
        &[
            "human",
            "--dir",
            config.dir.to_str().unwrap(),
            "--task",
            &task,
            "--fence",
            &view.card.rev.0.to_string(),
            "--rev",
            &view.card.rev.0.to_string(),
            "--op",
            "maintenance-test-approval",
            "approve-plan",
            "art:2",
        ],
    );
    let prepared = cycle(&options, "prepared");
    let a = &prepared.attempts[0];
    assert!(state_dir
        .join("pr")
        .join(&a.namespace)
        .join("candidate.patch")
        .is_file());
    assert_eq!(
        fs::read_to_string(repo.join("crates/apps/demo/Cargo.toml")).unwrap(),
        manifest
    );
    let view = current(&config, a);
    assert_eq!(view.card.state, State::MergeGate);
    assert!(view.receipt.is_none());
    let store = FsStore::open(&config.dir).unwrap();
    assert_eq!(store.task_ids().len(), 1);
    let events = store.events(&view.card.task, 0).unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, rusty_bbp::Event::TaskOpened { .. }))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(
                e,
                rusty_bbp::Event::HumanApproval {
                    gate: rusty_bbp::Gate::Plan,
                    ..
                }
            ))
            .count(),
        1
    );
    let repeated = execute(&options, &LocalPullRequests).unwrap();
    let repeated: Journal = json::from_str(&repeated).unwrap();
    assert_eq!(repeated.attempts.len(), 1);
    process(
        &repo,
        binary.to_str().unwrap(),
        &[
            "human",
            "--dir",
            config.dir.to_str().unwrap(),
            "--task",
            &task,
            "--rev",
            &view.card.rev.0.to_string(),
            "cancel",
            "test finished without merge",
        ],
    );
    cycle(&options, "cancelled");
    // The detached moderator owns a bounded lifetime; leave its evidence intact
    // for diagnosis in the test's private temporary directory.
    println!("BBP_ACCEPTANCE PASS maintenance");
}
