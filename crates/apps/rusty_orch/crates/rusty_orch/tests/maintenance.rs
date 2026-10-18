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
    assert!(!repo.0.join(".git/rusty-orch-maintenance").exists());
    assert_eq!(execute(&repo.options(), &StdCommand).unwrap(), report);
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&repo.0)
        .output()
        .unwrap();
    assert!(status.stdout.is_empty());
}

struct PrConflict;
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
    let report = execute(&options, &PrConflict).unwrap();
    assert!(report.contains("open PR touches"));
    assert!(!repo
        .0
        .join(".git/rusty-orch-maintenance/journal.json")
        .exists());
}
