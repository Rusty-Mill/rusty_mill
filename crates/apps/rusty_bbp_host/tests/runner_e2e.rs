#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Stage-3b exit criterion: the runner serves a selected run end to end.
//! A real git repository, a candidate whose diffs apply (or do not), the
//! frozen profile set executed, log and report stored under the run secret.
//!
//! Unix only: the workload is `/bin/sh` and the sandbox spec takes Unix
//! absolute read roots. On Windows `bbp runner` compiles but every run is
//! reported as `error` before anything executes.
#![cfg(unix)]

use rusty_bbp::*;
use rusty_bbp_host::profiles::ProfileSet;
use rusty_bbp_host::runner::{self, Confinement, Unconfined};
use rusty_bbp_host::{admin, human, now, open_driver};
use std::path::{Path, PathBuf};
use std::process::Command as Proc;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bbp_runner_{}_{tag}", std::process::id()));
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

/// A repository whose test passes only when `flag.txt` says `ok`.
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

fn flag_diff(to: &str) -> Vec<u8> {
    format!("--- a/flag.txt\n+++ b/flag.txt\n@@ -1 +1 @@\n-no\n+{to}\n").into_bytes()
}

struct Task {
    dir: PathBuf,
    id: TaskId,
}

impl Task {
    fn agent(&self, role: Role, action: AgentAction) -> Response {
        let mut d = open_driver(&self.dir, &self.id).expect("driver");
        let turn = d.state.turn.as_ref().expect("turn");
        assert_eq!(turn.role, role, "{role:?} holds the turn");
        let principal = PrincipalId(admin::role_name(role).into());
        let cmd = Command::Agent {
            token: Some(d.state.token_for(&principal, turn.id)),
            principal,
            op: rusty_bbp_host::fresh_op(),
            action,
        };
        d.dispatch(&cmd, now()).expect("dispatch")
    }

    fn put(&self, role: Role, payload: ArtifactPayload, body: &[u8]) -> ArtId {
        let bytes = encode_artifact(&payload, body).expect("encode");
        let mut d = open_driver(&self.dir, &self.id).expect("driver");
        let blob = d.store.blob_put(&bytes);
        match self.agent(role, AgentAction::PutArtifact { blob, payload }) {
            Response::Stored(id) => id,
            other => panic!("expected Stored, got {other:?}"),
        }
    }

    fn state(&self) -> TaskState {
        open_driver(&self.dir, &self.id).expect("driver").state
    }
}

/// Open a task, approve a plan, submit a candidate of `diffs`: state `Test`.
fn reach_test(dir: &Path, base: &str, diffs: &[Vec<u8>]) -> Task {
    let t = Task {
        dir: dir.to_path_buf(),
        id: TaskId("T1".into()),
    };
    admin::open_task(
        dir,
        &t.id,
        "local",
        b"Make the flag say ok.",
        &PrincipalId("human".into()),
        &ProfileSet::shell("test", "sh ./test.sh"),
    )
    .expect("open");
    for role in Role::ALL {
        admin::assign(
            dir,
            &t.id,
            role,
            &PrincipalId(admin::role_name(role).into()),
            "v",
        )
        .expect("assign");
    }
    let brief = t.state().brief.expect("brief");
    let spec = t.put(Role::Planner, ArtifactPayload::Spec { brief }, b"# Spec");
    let mut gate = Draft::new(MessageKind::RequestDecision, "Approve?")
        .to(Recipient::Human)
        .refs(vec![Ref::art(spec)]);
    gate.gate = true;
    t.agent(Role::Planner, AgentAction::Post(gate));
    human::perform(dir, &t.id, "approve-plan", &[&spec.0.to_string()]).expect("approve");
    let ids: Vec<ArtId> = diffs
        .iter()
        .map(|d| t.put(Role::Coder, ArtifactPayload::Diff, d))
        .collect();
    t.put(
        Role::Coder,
        ArtifactPayload::Candidate(Candidate {
            base: base.to_owned(),
            diffs: ids,
        }),
        b"",
    );
    assert_eq!(t.state().state, State::Test);
    t
}

fn report_of(t: &Task) -> (Report, String) {
    let d = open_driver(&t.dir, &t.id).expect("driver");
    let run = d.state.selected_run().expect("run");
    let rep = d
        .state
        .artifacts
        .get(&run.report.expect("report"))
        .expect("record");
    let ArtifactPayload::TestReport(r) = &rep.payload else {
        panic!("not a report")
    };
    let log = d
        .state
        .artifacts
        .get(&run.log.expect("log"))
        .expect("log record");
    let text =
        String::from_utf8(d.store.blob_get(&log.blob.sha).expect("log bytes")).expect("utf8");
    (r.clone(), text)
}

#[test]
fn passing_candidate_yields_a_passed_report_and_a_tester_turn() {
    let dir = tempdir("pass");
    let (repo, base) = repo(&dir);
    let t = reach_test(&dir, &base, &[flag_diff("ok")]);
    let r = runner::run_once(
        &dir,
        &t.id,
        &repo,
        &dir.join("work"),
        &Unconfined,
        Confinement::Unconfined,
    )
    .expect("run");
    assert!(matches!(r, Response::Stored(_)), "{r:?}");
    let (rep, log) = report_of(&t);
    assert_eq!(rep.status, RunStatus::Passed);
    assert_eq!(rep.sandbox, "unconfined");
    assert_eq!(rep.profiles.len(), 1);
    assert_eq!(rep.profiles[0].exit_code, 0);
    assert!(rep.tree.is_some(), "tree id recorded");
    assert!(log.contains("=== test"), "{log}");
    let st = t.state();
    assert_eq!(st.state, State::Test);
    assert_eq!(st.turn.as_ref().map(|t| t.role), Some(Role::Tester));
    // A second invocation finds nothing to serve.
    let again = runner::run_once(
        &dir,
        &t.id,
        &repo,
        &dir.join("work"),
        &Unconfined,
        Confinement::Unconfined,
    );
    assert!(again.is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn failing_test_yields_a_failed_report() {
    let dir = tempdir("fail");
    let (repo, base) = repo(&dir);
    let t = reach_test(&dir, &base, &[flag_diff("nope")]);
    runner::run_once(
        &dir,
        &t.id,
        &repo,
        &dir.join("work"),
        &Unconfined,
        Confinement::Unconfined,
    )
    .expect("run");
    let (rep, _) = report_of(&t);
    assert_eq!(rep.status, RunStatus::Failed);
    assert_eq!(rep.profiles[0].exit_code, 1);
    assert_eq!(rep.profiles[0].failed, 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unapplicable_diff_is_an_error_run_with_no_tree() {
    let dir = tempdir("error");
    let (repo, base) = repo(&dir);
    let t = reach_test(&dir, &base, &[flag_diff("ok"), b"not a diff\n".to_vec()]);
    runner::run_once(
        &dir,
        &t.id,
        &repo,
        &dir.join("work"),
        &Unconfined,
        Confinement::Unconfined,
    )
    .expect("run");
    let (rep, log) = report_of(&t);
    assert_eq!(rep.status, RunStatus::Error);
    assert!(rep.tree.is_none());
    assert!(rep.profiles.is_empty(), "nothing ran");
    assert!(log.contains("prepare:"), "{log}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tampered_profile_set_is_refused() {
    let dir = tempdir("tamper");
    let (repo, base) = repo(&dir);
    let t = reach_test(&dir, &base, &[flag_diff("ok")]);
    let p = rusty_bbp_host::profiles::path(&dir, &t.id);
    let mut set = rusty_bbp_host::profiles::read_file(&p).expect("set");
    set.profiles[0].args = vec!["-c".into(), "true".into()];
    std::fs::write(&p, set.to_bytes().expect("bytes")).expect("write");
    let r = runner::run_once(
        &dir,
        &t.id,
        &repo,
        &dir.join("work"),
        &Unconfined,
        Confinement::Unconfined,
    );
    assert!(
        r.is_err_and(|e| e.contains("frozen digest")),
        "nothing stored"
    );
    assert!(t.state().selected_run().expect("run").status.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The sandboxed executor through the real `bbp __sandbox` helper. Where the
/// kernel refuses Landlock or seccomp the run is reported as `error`, never
/// as an unconfined pass.
#[test]
fn sandboxed_run_passes_or_fails_closed() {
    let dir = tempdir("sandbox");
    let (repo, base) = repo(&dir);
    let t = reach_test(&dir, &base, &[flag_diff("ok")]);
    let exec = rusty_sandbox::ProcessExecutor::new(
        PathBuf::from(env!("CARGO_BIN_EXE_bbp")),
        vec!["__sandbox".into()],
        dir.join("sandbox-state"),
    );
    runner::run_once(
        &dir,
        &t.id,
        &repo,
        &dir.join("work"),
        &exec,
        Confinement::Sandboxed,
    )
    .expect("run");
    let (rep, log) = report_of(&t);
    assert_eq!(rep.sandbox, "landlock+seccomp:no-sockets");
    match rep.status {
        RunStatus::Passed => assert_eq!(rep.profiles[0].exit_code, 0),
        RunStatus::Error => {
            eprintln!("sandbox unavailable here: {log}");
            assert!(log.contains("not run") || log.contains("sandbox"), "{log}");
        }
        RunStatus::Failed => panic!("confined test failed: {log}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}
