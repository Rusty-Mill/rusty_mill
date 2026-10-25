use super::*;
use orch_core::board::Entry;
use orch_dispatch::{ClassifiedError, GateContext, Relayed};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);
pub(super) fn temp() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "maintenance-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}
fn report() -> Inspection {
    Inspection {
        schema_version: 1,
        workspace_root: "/repo".into(),
        crates: vec![
            CrateInfo {
                id: "a".into(),
                name: "alpha".into(),
                manifest: "crates/apps/a/Cargo.toml".into(),
                reverse_dependencies: vec!["b".into()],
            },
            CrateInfo {
                id: "b".into(),
                name: "beta".into(),
                manifest: "crates/apps/b/Cargo.toml".into(),
                reverse_dependencies: vec![],
            },
        ],
        findings: vec![Finding {
            id: "layer:1".into(),
            rule: "layer-classification".into(),
            affected_crates: vec!["a".into()],
            evidence: vec![Value::String("missing layer".into())],
            recommendation: "declare apps".into(),
            impact: Impact {
                crates_to_validate: vec!["a".into(), "b".into()],
            },
        }],
    }
}
fn selection() -> Selection {
    select(
        &report(),
        &BTreeMap::new(),
        &BTreeSet::new(),
        &BTreeSet::new(),
    )
    .unwrap()
    .0
    .unwrap()
}
fn attempt() -> Attempt {
    // Configuration deserialization exercises the same public input as production.
    let config = json::from_str(r#"{"binary":"bbp","dir":"/tmp/bbp","repo":"/repo","work":"/tmp/work","profiles":"/tmp/profiles.json","pool":{},"roster":[],"assignments":{},"live_turns":1,"max_wall_secs":60}"#).unwrap();
    Attempt {
        selection: selection(),
        base: "base".into(),
        config,
        profiles: "{}".into(),
        namespace: "durable".into(),
        handle: None,
        status: "opening".into(),
        detail: "".into(),
        pr_url: None,
        last_card: None,
        generation: 0,
    }
}
fn fence() -> Fence {
    Fence {
        assignment: Text::new(MAINTENANCE).unwrap(),
        generation: 1,
        holder: Text::new("test").unwrap(),
        lease: None,
    }
}
#[test]
fn default_is_dry_run_and_flags_never_inherit_run_configuration() {
    let o = parse([], "/repo".into()).unwrap();
    assert!(!o.execute);
    assert!(o.bbp.is_none());
    for args in [
        vec!["--execute", "--execute"],
        vec!["--state", "x"],
        vec!["--confirm-benefit", "id"],
        vec!["--repo"],
    ] {
        assert!(parse(args.into_iter().map(str::to_owned), "/repo".into()).is_err());
    }
}
#[test]
fn deterministic_smallest_impact_first_and_one_card_only() {
    let mut r = report();
    let mut smaller = r.findings[0].clone();
    smaller.id = "layer:2".into();
    smaller.affected_crates = vec!["b".into()];
    smaller.impact.crates_to_validate = vec!["b".into()];
    r.findings.push(smaller);
    let first = select(&r, &BTreeMap::new(), &BTreeSet::new(), &BTreeSet::new()).unwrap();
    r.findings.reverse();
    r.crates.reverse();
    let second = select(&r, &BTreeMap::new(), &BTreeSet::new(), &BTreeSet::new()).unwrap();
    assert_eq!(first.0.unwrap().finding_id, "layer:2");
    assert_eq!(second.0.unwrap().finding_id, "layer:2");
    assert_eq!(first.1.len(), 1);
}
#[test]
fn advisory_needs_human_benefit_and_structural_changes_stay_excluded() {
    let mut r = report();
    r.findings[0].rule = "duplicate-declaration".into();
    assert!(
        select(&r, &BTreeMap::new(), &BTreeSet::new(), &BTreeSet::new())
            .unwrap()
            .0
            .is_none()
    );
    let benefits = BTreeMap::from([(
        "layer:1".into(),
        "Centralize the same dependency used by these crates; reviewed feature semantics".into(),
    )]);
    assert!(select(&r, &benefits, &BTreeSet::new(), &BTreeSet::new())
        .unwrap()
        .0
        .is_some());
    r.findings[0].rule = "dependency-layer".into();
    assert!(select(&r, &benefits, &BTreeSet::new(), &BTreeSet::new())
        .unwrap()
        .0
        .is_none());
}
#[test]
fn unknown_schema_incomplete_impact_and_conflicts_fail_closed() {
    let mut r = report();
    r.schema_version = 2;
    assert!(select(&r, &BTreeMap::new(), &BTreeSet::new(), &BTreeSet::new()).is_err());
    let mut r = report();
    r.findings[0].impact.crates_to_validate.pop();
    assert!(
        select(&r, &BTreeMap::new(), &BTreeSet::new(), &BTreeSet::new())
            .unwrap()
            .0
            .is_none()
    );
    let conflict = BTreeSet::from(["crates/apps/a/Cargo.toml".into()]);
    assert!(
        select(&report(), &BTreeMap::new(), &BTreeSet::new(), &conflict)
            .unwrap()
            .0
            .is_none()
    );
    let seen = BTreeSet::from(["layer:1".into()]);
    assert!(select(&report(), &BTreeMap::new(), &seen, &BTreeSet::new())
        .unwrap()
        .0
        .is_none());
}
#[test]
fn validation_contains_reverse_dependents_and_retains_host_limits() {
    let p = profiles(
        r#"{"profiles":[{"name":"custom","program":"true","args":[]}],"limits":{"wall_secs":10}}"#,
        &selection(),
    )
    .unwrap();
    let p: Value = json::from_str(&p).unwrap();
    assert_eq!(p["limits"]["wall_secs"].as_u64(), Some(10));
    assert_eq!(p["profiles"].as_seq().unwrap().len(), 5);
    let text = json::to_string(&p).unwrap();
    assert!(text.contains("alpha") && text.contains("beta") && text.contains("--frozen"));
    assert!(profiles("{}", &selection()).is_err());
}
#[test]
fn patch_scope_rejects_source_changes_deletion_renames_and_missing_headers() {
    let allowed = selection().allowed_paths;
    assert!(check_patch(
        "--- a/crates/apps/a/Cargo.toml\n+++ b/crates/apps/a/Cargo.toml\n@@ -1 +1 @@\n-a\n+b\n",
        &allowed
    )
    .is_ok());
    for p in [
        "--- a/src/lib.rs\n+++ b/src/lib.rs\n",
        "--- a/crates/apps/a/Cargo.toml\n+++ /dev/null\n",
        "--- a/crates/apps/a/Cargo.toml\n+++ b/Cargo.toml\n",
        "GIT binary patch",
        "--- a/crates/apps/a/Cargo.toml",
    ] {
        assert!(check_patch(p, &allowed).is_err(), "{p}");
    }
}
#[derive(Debug)]
struct Host {
    opens: usize,
    fail_poll: bool,
    gate: bool,
}
impl TaskHost for Host {
    fn open(
        &mut self,
        task: &Task,
        _: &Board,
        _: &Fence,
    ) -> std::result::Result<HostHandle, ClassifiedError> {
        assert_eq!(task.spec().role, Role::Implement);
        self.opens += 1;
        Ok(HostHandle(Text::new("bbp:durable").unwrap()))
    }
    fn poll(
        &mut self,
        _: &HostHandle,
        _: &Fence,
    ) -> std::result::Result<Progress, ClassifiedError> {
        if self.fail_poll {
            return Err(ClassifiedError::Transient(orch_dispatch::AgentError(
                "lost reply".into(),
            )));
        }
        if self.gate {
            return Ok(Progress::Gate(GateContext {
                prompt: Text::new("approve-plan art:2").unwrap(),
                rev: 8,
                ids: vec![],
            }));
        }
        Ok(Progress::Working)
    }
    fn relay(
        &mut self,
        _: &HostHandle,
        _: &Entry,
        _: &Text,
        _: &Fence,
    ) -> std::result::Result<Relayed, ClassifiedError> {
        panic!("maintenance must never relay approval")
    }
    fn cancel(
        &mut self,
        _: &Task,
        _: &Board,
        _: &Text,
        _: &Fence,
    ) -> std::result::Result<Progress, ClassifiedError> {
        panic!("maintenance must never cancel")
    }
}
#[test]
fn crash_recovery_keeps_identity_and_does_not_reopen_known_handle() {
    let mut a = attempt();
    let mut host = Host {
        opens: 0,
        fail_poll: true,
        gate: false,
    };
    assert!(drive(&mut a, &mut host, &fence()).is_err());
    let bytes = json::to_string(&a).unwrap();
    let mut recovered: Attempt = json::from_str(&bytes).unwrap();
    host.fail_poll = false;
    host.gate = true;
    drive(&mut recovered, &mut host, &fence()).unwrap();
    assert_eq!(host.opens, 1);
    assert_eq!(recovered.namespace, a.namespace);
    assert_eq!(recovered.status, "waiting-human");
    drive(&mut recovered, &mut host, &fence()).unwrap();
    assert_eq!(host.opens, 1);
}
#[test]
fn journal_is_durable_and_corruption_is_never_treated_as_empty() {
    let dir = temp();
    let path = dir.join("journal.json");
    let journal = Journal {
        schema_version: 1,
        repo: dir.to_string_lossy().into(),
        attempts: vec![attempt()],
    };
    save(&path, &journal).unwrap();
    assert_eq!(load(&path, &dir).unwrap().attempts[0].namespace, "durable");
    fs::write(&path, "{partial").unwrap();
    assert!(load(&path, &dir).is_err());
    fs::remove_dir_all(dir).unwrap();
}
#[test]
fn repository_lock_survives_file_presence_and_releases_on_process_end() {
    let dir = temp();
    let path = dir.join("lock");
    let first = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .unwrap();
    first.try_lock().unwrap();
    let second = OpenOptions::new().write(true).open(&path).unwrap();
    assert!(second.try_lock().is_err());
    drop(first);
    second.try_lock().unwrap();
    drop(second);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn publication_needs_explicit_execution_and_a_prepared_candidate() {
    assert!(parse(vec!["--publish-pr".into()], "/repo".into()).is_err());
    let dir = temp();
    let mut j = Journal {
        schema_version: 1,
        repo: "/repo".into(),
        attempts: vec![attempt()],
    };
    let options = Options {
        repo: "/repo".into(),
        bbp: None,
        execute: true,
        publish_pr: true,
        benefits: BTreeMap::new(),
    };
    // No report/profile/patch or command exists: nothing can publish at opening.
    publish_if_requested(
        &options,
        &orch_cli::fake::FakeCommand::new(Err(orch_cli::ExecError::Spawn("must not run".into()))),
        &mut j,
        0,
        &dir.join("journal"),
        &dir,
    )
    .unwrap();
    assert!(j.attempts[0].pr_url.is_none());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn lost_publication_reply_adopts_same_commit_and_pr_without_push_or_creation() {
    use std::cell::RefCell;
    struct Replies(RefCell<Vec<(String, String)>>);
    impl CommandRunner for Replies {
        fn run_in(
            &self,
            _: Option<&Path>,
            argv: &[String],
            _: &[u8],
            _: Duration,
            _: &[&str],
        ) -> std::result::Result<orch_cli::Exit, orch_cli::ExecError> {
            let (expected, stdout) = self.0.borrow_mut().remove(0);
            assert_eq!(argv.join(" "), expected);
            Ok(orch_cli::Exit {
                status: 0,
                stdout: stdout.into_bytes(),
                stderr: vec![],
            })
        }
    }
    let dir = temp();
    let mut a = attempt();
    a.config.work = dir.join("work");
    let checkout = a.config.work.join("maintenance-pr").join(&a.namespace);
    fs::create_dir_all(&checkout).unwrap();
    let bundle = dir.join("pr").join(&a.namespace);
    fs::create_dir_all(&bundle).unwrap();
    let report = rusty_bbp::Report {
        candidate: rusty_bbp::ArtId(4),
        run: rusty_bbp::RunId(1),
        profile_digest: Sha256::of(b"profiles"),
        status: rusty_bbp::RunStatus::Passed,
        profiles: vec![],
        tree: Some(Sha256::of(b"tree")),
        sandbox: "landlock+seccomp:no-net".into(),
        log: rusty_bbp::ArtId(5),
    };
    fs::write(
        bundle.join("report.json"),
        json::to_string(&report).unwrap(),
    )
    .unwrap();
    let runner = Replies(RefCell::new(vec![
        ("git remote get-url --push origin".into(), "https://github.com/example/repo.git".into()),
        ("gh repo view https://github.com/example/repo.git --json nameWithOwner,defaultBranchRef".into(), r#"{"nameWithOwner":"example/repo","defaultBranchRef":{"name":"main"}}"#.into()),
        ("git check-ref-format refs/heads/main".into(), "".into()),
        ("git fetch --no-tags -- https://github.com/example/repo.git refs/heads/main".into(), "".into()),
        ("git rev-parse FETCH_HEAD".into(), "tip".into()),
        ("git merge-base --is-ancestor base tip".into(), "".into()),
        (
            "git symbolic-ref --short HEAD".into(),
            "maintenance/durable".into(),
        ),
        ("git rev-parse HEAD".into(), "commit".into()),
        ("git write-tree".into(), "tree".into()),
        ("git write-tree".into(), "tree".into()),
        ("git rev-parse HEAD".into(), "commit".into()),
        ("git rev-parse HEAD^{tree}".into(), "tree".into()),
        (
            "gh pr list --repo example/repo --head maintenance/durable --state all --json url,headRefOid".into(),
            r#"[{"url":"https://github.com/example/repo/pull/1","headRefOid":"commit"}]"#.into(),
        ),
    ]));
    assert_eq!(
        publish(&a, &runner, &dir).unwrap(),
        "https://github.com/example/repo/pull/1"
    );
    assert!(runner.0.borrow().is_empty());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn publication_uses_real_git_worktree_and_recovers_without_duplicate_commit_or_pr() {
    publication_fixture(false);
}

#[test]
fn publication_refuses_unmerged_feature_base_before_worktree_push_or_pr() {
    publication_fixture(true);
}

fn publication_fixture(unmerged_feature: bool) {
    use std::cell::RefCell;
    use std::process::Command;
    struct LocalGithub {
        commit: RefCell<Option<String>>,
        creates: std::cell::Cell<usize>,
        source: PathBuf,
    }
    impl CommandRunner for LocalGithub {
        fn run_in(
            &self,
            cwd: Option<&Path>,
            argv: &[String],
            stdin: &[u8],
            timeout: Duration,
            scrub: &[&str],
        ) -> std::result::Result<orch_cli::Exit, orch_cli::ExecError> {
            if argv[0] != "gh" {
                return StdCommand.run_in(cwd, argv, stdin, timeout, scrub);
            }
            if argv[1] == "pr" {
                assert!(argv.windows(2).any(|a| a == ["--repo", "fixture/repo"]));
            }
            let stdout = match (argv[1].as_str(),argv[2].as_str()) {
                ("pr","list") => self.commit.borrow().as_ref().map(|c| format!(r#"[{{"url":"https://github.com/fixture/repo/pull/1","headRefOid":"{c}"}}]"#)).unwrap_or_else(|| "[]".into()),
                ("repo","view") => r#"{"nameWithOwner":"fixture/repo","defaultBranchRef":{"name":"main"}}"#.into(),
                ("pr","create") => {
                    self.creates.set(self.creates.get()+1);
                    let commit = git(&StdCommand,&self.source,&["rev-parse","refs/heads/maintenance/durable"]).unwrap();
                    *self.commit.borrow_mut() = Some(commit);
                    "https://github.com/fixture/repo/pull/1".into()
                }
                _ => panic!("unexpected GitHub action {argv:?}"),
            };
            Ok(orch_cli::Exit {
                status: 0,
                stdout: stdout.into_bytes(),
                stderr: vec![],
            })
        }
    }
    let dir = temp();
    let repo = dir.join("repo");
    fs::create_dir_all(&repo).unwrap();
    let mut a = attempt();
    a.config.repo = repo.clone();
    a.config.work = dir.join("work");
    for args in [
        vec!["init", "-qb", "main"],
        vec!["config", "user.name", "Test"],
        vec!["config", "user.email", "test@example.invalid"],
        vec!["config", "core.autocrlf", "false"],
    ] {
        assert!(Command::new("git")
            .args(args)
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
    }
    let original = "[package]\nname = \"alpha\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[package.metadata.rusty_mill]\nlayer = \"invalid\"\n";
    let manifest = repo.join("crates/apps/a/Cargo.toml");
    fs::create_dir_all(manifest.parent().unwrap()).unwrap();
    fs::write(&manifest, original).unwrap();
    git(&StdCommand, &repo, &["add", "."]).unwrap();
    git(&StdCommand, &repo, &["commit", "-qm", "base"]).unwrap();
    a.base = git(&StdCommand, &repo, &["rev-parse", "HEAD"]).unwrap();
    let bundle = dir.join("pr/durable");
    fs::create_dir_all(&bundle).unwrap();
    let patch = bundle.join("candidate.patch");
    fs::write(
        &patch,
        "--- a/crates/apps/a/Cargo.toml\n+++ b/crates/apps/a/Cargo.toml\n@@ -4,3 +4,3 @@\n edition = \"2021\"\n [package.metadata.rusty_mill]\n-layer = \"invalid\"\n+layer = \"apps\"\n",
    )
    .unwrap();
    git(
        &StdCommand,
        &repo,
        &["apply", "--index", patch.to_str().unwrap()],
    )
    .unwrap();
    let tree = git(&StdCommand, &repo, &["write-tree"]).unwrap();
    git(
        &StdCommand,
        &repo,
        &["apply", "-R", "--index", patch.to_str().unwrap()],
    )
    .unwrap();
    let report = rusty_bbp::Report {
        candidate: rusty_bbp::ArtId(4),
        run: rusty_bbp::RunId(1),
        profile_digest: Sha256::of(b"profiles"),
        status: rusty_bbp::RunStatus::Passed,
        profiles: vec![],
        tree: Some(Sha256::of(tree.as_bytes())),
        sandbox: "landlock+seccomp:no-net".into(),
        log: rusty_bbp::ArtId(5),
    };
    fs::write(
        bundle.join("report.json"),
        json::to_string(&report).unwrap(),
    )
    .unwrap();
    fs::write(bundle.join("PR.md"), "Validated maintenance fixture").unwrap();
    let remote = dir.join("remote.git");
    assert!(Command::new("git")
        .args(["init", "-q", "--bare"])
        .arg(&remote)
        .status()
        .unwrap()
        .success());
    git(
        &StdCommand,
        &repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    )
    .unwrap();
    git(&StdCommand, &repo, &["push", "origin", "main"]).unwrap();
    if unmerged_feature {
        git(&StdCommand, &repo, &["checkout", "-qb", "feature"]).unwrap();
        fs::write(repo.join("unrelated.rs"), "unrelated feature change\n").unwrap();
        git(&StdCommand, &repo, &["add", "."]).unwrap();
        git(&StdCommand, &repo, &["commit", "-qm", "unrelated feature"]).unwrap();
        a.base = git(&StdCommand, &repo, &["rev-parse", "HEAD"]).unwrap();
        // Record the exact candidate tree on feature B, not on default A.
        git(
            &StdCommand,
            &repo,
            &["apply", "--index", patch.to_str().unwrap()],
        )
        .unwrap();
        let feature_tree = git(&StdCommand, &repo, &["write-tree"]).unwrap();
        let mut feature_report = report.clone();
        feature_report.tree = Some(Sha256::of(feature_tree.as_bytes()));
        fs::write(
            bundle.join("report.json"),
            json::to_string(&feature_report).unwrap(),
        )
        .unwrap();
        git(
            &StdCommand,
            &repo,
            &["apply", "-R", "--index", patch.to_str().unwrap()],
        )
        .unwrap();
    }
    let runner = LocalGithub {
        commit: RefCell::new(None),
        creates: std::cell::Cell::new(0),
        source: repo.clone(),
    };
    if unmerged_feature {
        assert!(publish(&a, &runner, &dir)
            .unwrap_err()
            .to_string()
            .contains("not in publication target history"));
        assert_eq!(runner.creates.get(), 0);
        assert!(!a.config.work.join("maintenance-pr/durable").exists());
        assert!(git(
            &StdCommand,
            &remote,
            &["show-ref", "--verify", "refs/heads/maintenance/durable"]
        )
        .is_err());
        assert!(git(&StdCommand, &repo, &["status", "--porcelain"])
            .unwrap()
            .is_empty());
        fs::remove_dir_all(dir).unwrap();
        return;
    }
    let url = publish(&a, &runner, &dir).unwrap();
    let commit = git(&StdCommand, &repo, &["rev-parse", "maintenance/durable"]).unwrap();
    // Recovery must use the original publication target even if local remote
    // configuration changes after a lost PR reply.
    git(
        &StdCommand,
        &repo,
        &[
            "remote",
            "set-url",
            "origin",
            "unreachable-after-publication",
        ],
    )
    .unwrap();
    assert_eq!(publish(&a, &runner, &dir).unwrap(), url);
    assert_eq!(runner.creates.get(), 1);
    assert_eq!(
        git(&StdCommand, &repo, &["rev-parse", "maintenance/durable"]).unwrap(),
        commit
    );
    assert_eq!(
        git(&StdCommand, &repo, &["rev-parse", "HEAD"]).unwrap(),
        a.base
    );
    assert_eq!(fs::read_to_string(&manifest).unwrap(), original);
    assert_eq!(
        git(
            &StdCommand,
            &remote,
            &["rev-parse", "maintenance/durable^{tree}"]
        )
        .unwrap(),
        tree
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn public_reports_exclude_frozen_configuration_and_environment() {
    let mut attempt = attempt();
    attempt.profiles = "private-profile-marker".into();
    attempt.config.binary = "private-launcher-marker".into();
    let journal = Journal {
        schema_version: 1,
        repo: "/repo".into(),
        attempts: vec![attempt],
    };
    let report = json::to_string(&public_state(&journal).unwrap()).unwrap();
    assert!(!report.contains("private-profile-marker"));
    assert!(!report.contains("private-launcher-marker"));
    assert!(report.contains("opening") && report.contains("durable"));
}

#[test]
fn linked_worktrees_recover_one_journal_and_exclude_concurrent_cycles() {
    use std::process::Command;
    let dir = temp();
    let repo = dir.join("repo");
    let linked = dir.join("linked");
    fs::create_dir_all(&repo).unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.name", "Test"],
        vec!["config", "user.email", "test@example.invalid"],
    ] {
        assert!(Command::new("git")
            .args(args)
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
    }
    fs::write(repo.join("flag"), "base\n").unwrap();
    git(&StdCommand, &repo, &["add", "."]).unwrap();
    git(&StdCommand, &repo, &["commit", "-qm", "base"]).unwrap();
    git(
        &StdCommand,
        &repo,
        &["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
    )
    .unwrap();
    let mut config = attempt().config;
    config.repo = linked.clone();
    config.dir = dir.join("shared-store");
    persist_task(&config.dir, "active", &repo, false);
    assert_eq!(active_bbp_task(&config).unwrap().as_deref(), Some("active"));
    // Terminal tasks must not require a checkout to remain on disk.
    let deleted = dir.join("deleted");
    fs::create_dir_all(&deleted).unwrap();
    persist_task(&config.dir, "old", &deleted, true);
    fs::remove_dir_all(&deleted).unwrap();
    assert_eq!(active_bbp_task(&config).unwrap().as_deref(), Some("active"));
    let common = repo.join(".git").canonicalize().unwrap();
    let state = common.join("rusty-orch-maintenance");
    fs::create_dir_all(&state).unwrap();
    let mut pending = attempt();
    pending.profiles = "private-workload-environment".into();
    let journal = Journal {
        schema_version: 1,
        repo: common.to_string_lossy().into(),
        attempts: vec![pending],
    };
    save(&state.join("journal.json"), &journal).unwrap();
    let mut options = Options {
        repo: repo.clone(),
        bbp: None,
        execute: false,
        publish_pr: false,
        benefits: BTreeMap::new(),
    };
    let first = execute(&options, &StdCommand).unwrap();
    options.repo = linked;
    assert_eq!(execute(&options, &StdCommand).unwrap(), first);
    assert!(first.contains("recovery pending") && first.contains("durable"));
    assert!(!first.contains("private-workload-environment"));
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(state.join("lock"))
        .unwrap();
    lock.try_lock().unwrap();
    options.execute = true;
    let error = execute(&options, &StdCommand).unwrap_err().to_string();
    assert!(error.contains(if cfg!(target_os = "linux") {
        "another maintenance cycle"
    } else {
        "maintenance execution requires sandboxed Linux BBP"
    }));
    drop(lock);
    fs::remove_dir_all(dir).unwrap();
}

fn persist_task(dir: &Path, name: &str, repo: &Path, terminal: bool) {
    use rusty_bbp::{Budget, Event, PrincipalId, Rev, SchedulingPolicy, TaskId, LOG_SCHEMA};
    let task = TaskId(name.into());
    let mut events = vec![Event::TaskOpened {
        schema: LOG_SCHEMA,
        roster: vec![],
        task: task.clone(),
        repo: repo.to_str().unwrap().into(),
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
    }];
    if terminal {
        events.push(Event::StateChanged {
            from: State::Planning,
            to: State::Cancelled,
        });
        events.push(Event::Cancelled {
            reason: "completed historical fixture".into(),
        });
    }
    FsStore::open(dir)
        .unwrap()
        .append(&task, Rev(0), &events)
        .unwrap();
}

#[test]
fn deleted_terminal_checkout_does_not_block_an_unrelated_repository() {
    let dir = temp();
    let mut config = attempt().config;
    config.dir = dir.join("store");
    config.repo = dir.join("current");
    fs::create_dir_all(&config.repo).unwrap();
    git(&StdCommand, &config.repo, &["init", "-q"]).unwrap();
    persist_task(&config.dir, "old", &dir.join("already-deleted"), true);
    assert!(active_bbp_task(&config).unwrap().is_none());
    fs::remove_dir_all(dir).unwrap();
}
#[test]
fn journals_from_before_generations_load_at_generation_zero() {
    let mut value: Value = json::from_str(&json::to_string(&attempt()).unwrap()).unwrap();
    let Value::Map(fields) = &mut value else {
        panic!("attempt is an object")
    };
    fields.retain(|(k, _)| k != "generation");
    let old: Attempt = json::from_str(&json::to_string(&value).unwrap()).unwrap();
    assert_eq!(old.generation, 0);
}
