//! Real Cargo discovery and read-only default, independent of provider logins.
use orch_cli::{CommandRunner, ExecError, Exit, StdCommand};
use rusty_orch::maintenance::{execute, Options};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Repo(PathBuf);
impl Repo {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "maintenance-integration-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(path.join("crates/apps/demo/src")).unwrap();
        fs::write(
            path.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/apps/demo\"]\nresolver = \"2\"\n",
        )
        .unwrap();
        fs::write(path.join("crates/apps/demo/Cargo.toml"), "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[package.metadata.rusty_mill]\nlayer = \"invalid\"\n").unwrap();
        fs::write(
            path.join("crates/apps/demo/src/lib.rs"),
            "pub fn demo() {}\n",
        )
        .unwrap();
        for args in [
            vec!["git", "init", "-q"],
            vec!["git", "config", "user.name", "Test"],
            vec!["git", "config", "user.email", "test@example.invalid"],
            vec!["cargo", "generate-lockfile", "--offline"],
            vec!["git", "add", "."],
            vec!["git", "commit", "-qm", "base"],
        ] {
            let out = Command::new(args[0])
                .args(&args[1..])
                .current_dir(&path)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Self(path)
    }
    fn options(&self) -> Options {
        Options {
            repo: self.0.clone(),
            bbp: None,
            execute: false,
            publish_pr: false,
            benefits: Default::default(),
        }
    }
}
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn default_cycle_discovers_real_finding_without_writes_or_provider_configuration() {
    let repo = Repo::new();
    let report = execute(&repo.options(), &StdCommand).unwrap();
    let value: rusty_serde::json::Value = rusty_serde::json::from_str(&report).unwrap();
    assert!(value["mode"].as_str().unwrap().starts_with("dry-run"));
    assert_eq!(
        value["selected"]["rule"].as_str(),
        Some("layer-classification")
    );
    assert_eq!(
        value["selected"]["validation_packages"][0].as_str(),
        Some("demo")
    );
    assert_eq!(
        value["selected"]["allowed_paths"][0].as_str(),
        Some("crates/apps/demo/Cargo.toml")
    );
    assert!(!repo.0.join(".git/rusty-orch-maintenance").exists());
    assert_eq!(execute(&repo.options(), &StdCommand).unwrap(), report);
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&repo.0)
        .output()
        .unwrap();
    assert!(status.stdout.is_empty());
}

struct PrConflict(bool);
impl CommandRunner for PrConflict {
    fn run_in(
        &self,
        cwd: Option<&std::path::Path>,
        argv: &[String],
        stdin: &[u8],
        timeout: Duration,
        scrub: &[&str],
    ) -> Result<Exit, ExecError> {
        if argv[0] == "gh" {
            if !self.0 {
                return Ok(Exit {
                    status: 0,
                    stdout: b"[]".to_vec(),
                    stderr: vec![],
                });
            }
            return Ok(Exit { status: 0, stdout: br#"[{"headRefName":"existing","url":"https://example.invalid/pr/1","files":[{"path":"crates/apps/demo/Cargo.toml"}]}]"#.to_vec(), stderr: vec![] });
        }
        StdCommand.run_in(cwd, argv, stdin, timeout, scrub)
    }
}
#[test]
fn current_pr_conflict_prevents_opening_before_any_bbp_configuration_is_read() {
    let repo = Repo::new();
    let mut options = repo.options();
    options.execute = true;
    options.bbp = Some("must-not-be-read.json".into());
    let result = execute(&options, &PrConflict(true));
    if cfg!(target_os = "linux") {
        assert!(result.unwrap().contains("open PR touches"));
    } else {
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("maintenance execution requires sandboxed Linux BBP"));
    }
    assert!(!repo
        .0
        .join(".git/rusty-orch-maintenance/journal.json")
        .exists());
}

#[cfg(target_os = "linux")]
#[test]
fn shipped_author_policy_checks_linked_tasks_before_opening_and_skips_deleted_terminal_tasks() {
    use rusty_bbp::{
        Budget, Event, FsStore, PrincipalId, Rev, SchedulingPolicy, Sha256, State, Store, TaskId,
        LOG_SCHEMA,
    };
    let repo = Repo::new();
    let linked = repo.0.with_extension("linked");
    let historical = repo.0.with_extension("historical");
    let state = repo.0.with_extension("bbp");
    assert!(Command::new("git")
        .args(["worktree", "add", "-b", "linked"])
        .arg(&linked)
        .current_dir(&repo.0)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .args(["worktree", "add", "-b", "historical"])
        .arg(&historical)
        .current_dir(&repo.0)
        .output()
        .unwrap()
        .status
        .success());
    fs::create_dir_all(&state).unwrap();
    let mut config: orch_bbp::Config =
        rusty_serde::json::from_str(include_str!("../../../docs/examples/bbp.json")).unwrap();
    config.repo = linked.clone();
    config.dir = state.join("store");
    config.work = state.join("work");
    config.profiles = state.join("profiles.json");
    fs::write(
        &config.profiles,
        r#"{"profiles":[{"name":"fixture","program":"true","args":[]}]}"#,
    )
    .unwrap();
    config.validate().unwrap();
    let task = TaskId("ordinary-task".into());
    let mut store = FsStore::open(&config.dir).unwrap();
    store
        .append(
            &task,
            Rev(0),
            &[Event::TaskOpened {
                schema: LOG_SCHEMA,
                roster: config.roster.clone(),
                task: task.clone(),
                repo: historical.to_str().unwrap().into(),
                profile_digest: Sha256::of(b"profiles"),
                budget: Budget {
                    messages: 10,
                    bytes: 1000,
                    reads: 10,
                    reads_per_turn: 10,
                    turns: 10,
                    iterations: 10,
                },
                turn_ms: 1000,
                request_ms: 1000,
                max_live_turns: 1,
                human: PrincipalId("human".into()),
                policy: SchedulingPolicy::default(),
            }],
        )
        .unwrap();
    let file = state.join("bbp.json");
    fs::write(&file, rusty_serde::json::to_string(&config).unwrap()).unwrap();
    let mut options = repo.options();
    options.repo = linked.clone();
    options.execute = true;
    options.bbp = Some(file);
    assert!(execute(&options, &PrConflict(false))
        .unwrap_err()
        .to_string()
        .contains("existing nonterminal BBP task"));
    assert_eq!(FsStore::open(&config.dir).unwrap().task_ids().len(), 1);
    assert!(!repo
        .0
        .join(".git/rusty-orch-maintenance/journal.json")
        .exists());
    // The same store remains usable after a terminal task loses its checkout.
    store
        .append(
            &task,
            Rev(1),
            &[Event::StateChanged {
                from: State::Planning,
                to: State::Cancelled,
            }],
        )
        .unwrap();
    assert!(Command::new("git")
        .args(["worktree", "remove"])
        .arg(&historical)
        .current_dir(&repo.0)
        .output()
        .unwrap()
        .status
        .success());
    // A deliberately absent profiles file proves intake passed the terminal
    // inventory check and did not open a second task or call a provider.
    fs::remove_file(&config.profiles).unwrap();
    assert!(execute(&options, &PrConflict(false))
        .unwrap_err()
        .to_string()
        .contains("No such file"));
    assert_eq!(FsStore::open(&config.dir).unwrap().task_ids().len(), 1);
    fs::remove_dir_all(linked).unwrap();
    fs::remove_dir_all(state).unwrap();
}
