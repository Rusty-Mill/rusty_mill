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
    reach_test_with(dir, base, diffs, &ProfileSet::shell("test", "sh ./test.sh"))
}

fn reach_test_with(dir: &Path, base: &str, diffs: &[Vec<u8>], set: &ProfileSet) -> Task {
    let t = reach_build_with(dir, set);
    submit_candidate(&t, base, diffs);
    assert_eq!(t.state().state, State::Test);
    t
}

/// Open a task, approve a plan: state `Build`, the Coder holds the turn.
fn reach_build_with(dir: &Path, set: &ProfileSet) -> Task {
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
        set,
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
    admin::tick(dir, &t.id).expect("tick");
    let brief = t.state().brief.expect("brief");
    let spec = t.put(Role::Planner, ArtifactPayload::Spec { brief }, b"# Spec");
    let mut gate = Draft::new(MessageKind::RequestDecision, "Approve?")
        .to(Recipient::Human)
        .refs(vec![Ref::art(spec)]);
    gate.gate = true;
    t.agent(Role::Planner, AgentAction::Post(gate));
    human::perform(dir, &t.id, "approve-plan", &[&spec.0.to_string()], None).expect("approve");
    assert_eq!(t.state().state, State::Build);
    t
}

fn submit_candidate(t: &Task, base: &str, diffs: &[Vec<u8>]) {
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

#[test]
fn reopening_a_task_keeps_its_frozen_profile_set() {
    let dir = tempdir("reopen");
    let (repo, base) = repo(&dir);
    let t = reach_test(&dir, &base, &[flag_diff("ok")]);
    let r = admin::open_task(
        &dir,
        &t.id,
        "local",
        b"again",
        &PrincipalId("human".into()),
        &ProfileSet::shell("test", "false"),
    )
    .expect("call");
    assert!(
        matches!(r, Response::Rejected(ref rej) if rej.code == Code::AlreadyOpen),
        "{r:?}"
    );
    runner::run_once(
        &dir,
        &t.id,
        &repo,
        &dir.join("work"),
        &Unconfined,
        Confinement::Unconfined,
    )
    .expect("the original profile set still runs");
    assert_eq!(report_of(&t).0.status, RunStatus::Passed);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn profile_files_do_not_collide_across_task_ids() {
    let dir = tempdir("collide");
    let a = rusty_bbp_host::profiles::path(&dir, &TaskId("a/b".into()));
    let b = rusty_bbp_host::profiles::path(&dir, &TaskId("a?b".into()));
    assert_ne!(a, b);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn empty_profile_set_is_refused_at_open() {
    let dir = tempdir("empty");
    let mut set = ProfileSet::shell("test", "true");
    set.profiles.clear();
    let r = admin::open_task(
        &dir,
        &TaskId("T1".into()),
        "local",
        b"brief",
        &PrincipalId("human".into()),
        &set,
    );
    assert!(r.is_err_and(|e| e.contains("no profiles")));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_run_whose_log_is_already_stored_is_reported_as_error_without_rerunning() {
    let dir = tempdir("resume");
    let (repo, base) = repo(&dir);
    let t = reach_test(&dir, &base, &[flag_diff("ok")]);
    // A previous runner stored the log and died before the report.
    let mut d = open_driver(&dir, &t.id).expect("driver");
    let run = d.state.selected_run().expect("run").clone();
    let blob = d.store.blob_put(b"partial log\n");
    let r = d
        .dispatch(
            &Command::Runner {
                op: rusty_bbp_host::fresh_op(),
                run: run.id,
                secret: d.state.secret_for(run.id),
                blob,
                payload: ArtifactPayload::Log,
            },
            now(),
        )
        .expect("dispatch");
    assert!(matches!(r, Response::Stored(_)), "{r:?}");
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
    // The result that log described died with the earlier invocation; a
    // fresh execution would be reported against evidence it did not produce.
    assert_eq!(rep.status, RunStatus::Error);
    assert!(rep.profiles.is_empty() && rep.tree.is_none(), "{rep:?}");
    assert_eq!(log, "partial log\n", "report points at the log on record");
    assert!(
        !dir.join("work").join("run-1").exists(),
        "nothing was checked out or executed"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The first diff plants a symlink where a supervisor might write its
/// patch file; the second diff is invalid. Nothing outside the checkout
/// may change, whatever the candidate names.
#[test]
fn patches_never_touch_a_candidate_controlled_path() {
    use std::os::unix::fs::symlink;
    let dir = tempdir("symlink");
    let (repo, base) = repo(&dir);
    let sentinel = dir.join("sentinel");
    std::fs::write(&sentinel, "untouched\n").expect("write");
    // The base tree already carries a symlink to the sentinel too.
    symlink(&sentinel, repo.join(".bbp-diff-0.patch")).expect("symlink");
    git(&repo, &["add", ".bbp-diff-0.patch"]);
    git(&repo, &["commit", "--quiet", "-m", "tracked symlink"]);
    let base2 = git(&repo, &["rev-parse", "HEAD"]);
    assert_ne!(base, base2);
    let link = format!(
        "diff --git a/.bbp-diff-1.patch b/.bbp-diff-1.patch\nnew file mode 120000\n--- /dev/null\n+++ b/.bbp-diff-1.patch\n@@ -0,0 +1 @@\n+{}\n\\ No newline at end of file\n",
        sentinel.display()
    );
    let bad = b"--- a/missing.txt\n+++ b/missing.txt\n@@ -1 +1 @@\n-x\n+CLOBBERED\n".to_vec();
    let t = reach_test(&dir, &base2, &[link.into_bytes(), bad]);
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
    assert_eq!(rep.status, RunStatus::Error, "{log}");
    assert!(
        log.contains("git apply") && log.contains("missing.txt"),
        "{log}"
    );
    assert_eq!(
        std::fs::read_to_string(&sentinel).expect("read"),
        "untouched\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A listener the host owns; a confined workload must not reach it, while a
/// plain command still runs. Needs python3 for the probe and a kernel that
/// grants the sandbox; otherwise the test only checks fail-closed.
/// The recovery route the first proving run never took: the Tester posts a
/// non-gate request about an environment fault, the human fixes the
/// environment and reruns the current candidate while the request is
/// pending (revision-fenced), the runner stores a fresh report, and only
/// then the human settles the request. Nothing is granted while the request
/// is open; settlement regrants the Tester, who judges the fresh run.
#[test]
fn an_environment_fault_is_recovered_by_rerun_then_settlement() {
    let dir = tempdir("recover");
    let (repo, base) = repo(&dir);
    let fixed = dir.join("env-fixed");
    let set = ProfileSet::shell("test", &format!("test -f {}", fixed.display()));
    let t = reach_test_with(&dir, &base, &[flag_diff("ok")], &set);
    let work = dir.join("work");
    let run_runner = || {
        runner::run_once(
            &dir,
            &t.id,
            &repo,
            &work,
            &Unconfined,
            Confinement::Unconfined,
        )
        .expect("run")
    };
    run_runner();
    let (rep, _) = report_of(&t);
    assert_eq!(rep.status, RunStatus::Failed, "the environment is broken");
    let st = t.state();
    assert_eq!(st.turn.as_ref().map(|x| x.role), Some(Role::Tester));
    let cand = st.candidate.expect("candidate");
    let old_run = st.selected_run().expect("run").id;
    let log = st.selected_run().expect("run").log.expect("log");

    // The Tester asks the human instead of posting revise.
    let ask = Draft::new(
        MessageKind::RequestDecision,
        "rustc cannot run in the sandbox",
    )
    .to(Recipient::Human)
    .refs(vec![Ref::art(log)]);
    let req = match t.agent(Role::Tester, AgentAction::Post(ask)) {
        Response::Posted(m) => m,
        other => panic!("expected Posted, got {other:?}"),
    };
    let st = t.state();
    assert!(st.turn.is_none(), "the request ended the turn");
    assert_eq!(st.pending_request.as_ref().map(|p| p.msg), Some(req));
    assert_eq!(st.iteration, 0, "no iteration spent");

    // A stale revision is refused; the current one reruns the candidate.
    let stale = Rev(st.rev.0 - 1);
    let r =
        human::perform(&dir, &t.id, "rerun", &[&cand.0.to_string()], Some(stale)).expect("call");
    assert!(
        matches!(r, Response::Rejected(ref rej) if rej.code == Code::StaleRev),
        "{r:?}"
    );
    std::fs::write(&fixed, b"").expect("fix the environment");
    let r = human::perform(
        &dir,
        &t.id,
        "rerun",
        &[&cand.0.to_string()],
        Some(t.state().rev),
    )
    .expect("call");
    assert!(matches!(r, Response::Ok), "{r:?}");
    let st = t.state();
    let new_run = st.selected_run().expect("run");
    assert_ne!(new_run.id, old_run);
    assert!(new_run.report.is_none(), "fresh run, no report yet");
    assert!(
        st.pending_request.is_some(),
        "the request survives the rerun"
    );

    run_runner();
    let (rep, _) = report_of(&t);
    assert_eq!(rep.status, RunStatus::Passed);
    assert!(
        t.state().turn.is_none(),
        "a fresh report grants nobody while the request is pending"
    );

    // Settlement regrants the Tester, who approves on the fresh run.
    let r = human::perform(
        &dir,
        &t.id,
        "answer",
        &[&req.0.to_string(), "toolchain", "fixed;", "rerun", "passed"],
        None,
    )
    .expect("call");
    assert!(matches!(r, Response::Posted(_)), "{r:?}");
    let st = t.state();
    assert!(st.pending_request.is_none());
    assert_eq!(st.turn.as_ref().map(|x| x.role), Some(Role::Tester));
    let run = st.selected_run().expect("run");
    let report = run.report.expect("report");
    let approve = Draft {
        kind: MessageKind::Verdict,
        to: vec![Recipient::All],
        body: Body::Verdict(Verdict {
            subject: cand,
            run: run.id,
            verdict: VerdictKind::Approve,
            blocking: vec![],
            non_blocking: vec![],
        }),
        refs: vec![Ref::art(cand)],
        evidence: vec![Ref::art(report)],
        reply_to: None,
        yield_to: None,
        gate: false,
    };
    assert!(matches!(
        t.agent(Role::Tester, AgentAction::Post(approve)),
        Response::Posted(_)
    ));
    assert_eq!(t.state().state, State::Review);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A request to the human referencing what it is about: the selected run's
/// log when there is one, else the approved spec.
fn request_human(t: &Task, role: Role, body: &str) -> MsgId {
    let st = t.state();
    let about = st
        .selected_run()
        .and_then(|r| r.log)
        .or(st.approved_spec)
        .expect("something to reference");
    let ask = Draft::new(MessageKind::RequestDecision, body)
        .to(Recipient::Human)
        .refs(vec![Ref::art(about)]);
    match t.agent(role, AgentAction::Post(ask)) {
        Response::Posted(m) => m,
        other => panic!("expected Posted, got {other:?}"),
    }
}

fn human_answer(t: &Task, req: MsgId, text: &str) {
    let r =
        human::perform(&t.dir, &t.id, "answer", &[&req.0.to_string(), text], None).expect("call");
    assert!(matches!(r, Response::Posted(_)), "{r:?}");
}

fn human_rerun(t: &Task, cand: ArtId) {
    let r = human::perform(
        &t.dir,
        &t.id,
        "rerun",
        &[&cand.0.to_string()],
        Some(t.state().rev),
    )
    .expect("call");
    assert!(matches!(r, Response::Ok), "{r:?}");
}

/// The Coder's recovery path: a request from `build` has no run to redo;
/// the human answers and the regranted Coder submits.
#[test]
fn a_coder_request_is_settled_by_an_answer_and_the_coder_resubmits() {
    let dir = tempdir("recover-coder");
    let (_repo, base) = repo(&dir);
    let t = reach_build_with(&dir, &ProfileSet::shell("test", "sh ./test.sh"));
    let req = request_human(&t, Role::Coder, "the sandbox cannot run rustc");
    let st = t.state();
    assert!(st.turn.is_none());
    assert_eq!(st.pending_request.as_ref().map(|p| p.msg), Some(req));
    assert_eq!(st.state, State::Build);
    human_answer(&t, req, "toolchain readable now; resubmit your candidate");
    let st = t.state();
    assert!(st.pending_request.is_none());
    assert_eq!(st.turn.as_ref().map(|x| x.role), Some(Role::Coder));
    submit_candidate(&t, &base, &[flag_diff("ok")]);
    assert_eq!(t.state().state, State::Test);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A rerun that fails again is not a resolution: the request stays open,
/// nobody is granted, and the human reruns once more after a real fix.
#[test]
fn a_failed_rerun_keeps_the_request_open_until_a_run_passes() {
    let dir = tempdir("recover-twice");
    let (repo, base) = repo(&dir);
    let fixed = dir.join("env-fixed");
    let set = ProfileSet::shell("test", &format!("test -f {}", fixed.display()));
    let t = reach_test_with(&dir, &base, &[flag_diff("ok")], &set);
    let work = dir.join("work");
    let run_runner = || {
        runner::run_once(
            &dir,
            &t.id,
            &repo,
            &work,
            &Unconfined,
            Confinement::Unconfined,
        )
        .expect("run")
    };
    run_runner();
    assert_eq!(report_of(&t).0.status, RunStatus::Failed);
    let cand = t.state().candidate.expect("candidate");
    let req = request_human(&t, Role::Tester, "rustc cannot run");

    // The human reruns without fixing anything: failed again, still pending.
    human_rerun(&t, cand);
    run_runner();
    let st = t.state();
    assert_eq!(report_of(&t).0.status, RunStatus::Failed);
    assert_eq!(st.pending_request.as_ref().map(|p| p.msg), Some(req));
    assert!(st.turn.is_none(), "a failed rerun grants nobody");
    assert_eq!(st.iteration, 0);

    // A real fix, another rerun, then settlement.
    std::fs::write(&fixed, b"").expect("fix");
    human_rerun(&t, cand);
    run_runner();
    assert_eq!(report_of(&t).0.status, RunStatus::Passed);
    assert!(t.state().turn.is_none());
    human_answer(&t, req, "fixed, rerun passed");
    assert_eq!(t.state().turn.as_ref().map(|x| x.role), Some(Role::Tester));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A settlement that does not resolve the fault: the human rejects the
/// request; the regranted Tester may open a new one, because settlement
/// clears the pending slot. An answer to anything else settles nothing.
#[test]
fn a_rejected_request_regrants_the_tester_who_may_ask_again() {
    let dir = tempdir("recover-reject");
    let (repo, base) = repo(&dir);
    let set = ProfileSet::shell("test", "false");
    let t = reach_test_with(&dir, &base, &[flag_diff("ok")], &set);
    runner::run_once(
        &dir,
        &t.id,
        &repo,
        &dir.join("work"),
        &Unconfined,
        Confinement::Unconfined,
    )
    .expect("run");
    let req = request_human(&t, Role::Tester, "please look at the sandbox");
    let r = human::perform(
        &t.dir,
        &t.id,
        "decision",
        &[
            &req.0.to_string(),
            "reject",
            "not",
            "an",
            "environment",
            "problem",
        ],
        None,
    )
    .expect("call");
    assert!(matches!(r, Response::Posted(_)), "{r:?}");
    let st = t.state();
    assert!(st.pending_request.is_none());
    assert_eq!(st.turn.as_ref().map(|x| x.role), Some(Role::Tester));
    let again = request_human(&t, Role::Tester, "then the log is wrong: see art:5");
    let st = t.state();
    assert_eq!(st.pending_request.as_ref().map(|p| p.msg), Some(again));
    assert!(st.turn.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A rerun whose report is `error` (here: the sandbox wall limit) escalates
/// the task with the request still pending. Settlement alone cannot bring
/// the Tester back: the human resumes into `test`, which starts a fresh run,
/// and settles only once that run has a real report.
#[test]
fn an_error_rerun_escalates_and_resume_then_settlement_recovers() {
    let dir = tempdir("recover-escalate");
    let (repo, base) = repo(&dir);
    let fixed = dir.join("env-fixed");
    let hang = dir.join("env-hang");
    let mut set = ProfileSet::shell(
        "test",
        &format!(
            "test -f {} && exit 0; test -f {} && sleep 3; exit 1",
            fixed.display(),
            hang.display()
        ),
    );
    set.limits.cpu_secs = 1;
    set.limits.wall_secs = 1;
    let t = reach_test_with(&dir, &base, &[flag_diff("ok")], &set);
    let exec = runner::Sandboxed::new(
        PathBuf::from(env!("CARGO_BIN_EXE_bbp")),
        &dir.join("sandbox-state"),
    );
    let work = dir.join("work");
    let run_runner =
        || runner::run_once(&dir, &t.id, &repo, &work, &exec, Confinement::Sandboxed).expect("run");
    run_runner();
    let (rep, log) = report_of(&t);
    if log.contains("Landlock filesystem confinement is NotEnforced") {
        eprintln!("kernel without Landlock; skipping: {log}");
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    assert_eq!(rep.status, RunStatus::Failed, "{log}");
    let cand = t.state().candidate.expect("candidate");
    let req = request_human(&t, Role::Tester, "rustc cannot run");

    // A "fix" that makes the run hang: the rerun times out, error, escalated.
    std::fs::write(&hang, b"").expect("hang");
    human_rerun(&t, cand);
    run_runner();
    let (rep, log) = report_of(&t);
    assert_eq!(rep.status, RunStatus::Error, "{log}");
    let st = t.state();
    assert_eq!(st.state, State::Escalated);
    assert_eq!(st.pending_request.as_ref().map(|p| p.msg), Some(req));
    assert!(st.turn.is_none());

    // The real fix; resume into test starts a fresh run; settle after it passes.
    std::fs::remove_file(&hang).expect("unhang");
    std::fs::write(&fixed, b"").expect("fix");
    let r = human::perform(&t.dir, &t.id, "resume", &["test"], Some(t.state().rev)).expect("call");
    assert!(matches!(r, Response::Ok), "{r:?}");
    let st = t.state();
    assert_eq!(st.state, State::Test);
    assert!(
        st.selected_run().expect("run").report.is_none(),
        "a fresh run"
    );
    run_runner();
    assert_eq!(report_of(&t).0.status, RunStatus::Passed);
    assert!(t.state().turn.is_none(), "still pending");
    human_answer(&t, req, "fixed, rerun passed");
    assert_eq!(t.state().turn.as_ref().map(|x| x.role), Some(Role::Tester));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sandboxed_workload_reaches_no_socket_endpoint() {
    use std::os::unix::net::UnixListener;
    let dir = tempdir("endpoint");
    let (repo, base) = repo(&dir);
    let sock = dir.join("probe.sock");
    let listener = UnixListener::bind(&sock).expect("bind");
    listener.set_nonblocking(true).expect("nonblocking");
    let mut set = ProfileSet::shell("plain", "echo plain-ok");
    set.profiles.push(rusty_bbp_host::profiles::Profile {
        name: "probe".into(),
        program: "/usr/bin/python3".into(),
        args: vec![
            "-c".into(),
            "import socket,sys; socket.socket(socket.AF_UNIX).connect(sys.argv[1]); print('CONNE'+'CTED')".into(),
            sock.display().to_string(),
        ],
    });
    let t = reach_test_with(&dir, &base, &[flag_diff("ok")], &set);
    let exec = runner::Sandboxed::new(
        PathBuf::from(env!("CARGO_BIN_EXE_bbp")),
        &dir.join("sandbox-state"),
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
    assert_eq!(rep.sandbox, runner::Sandboxed::LABEL);
    assert!(!log.contains("CONNECTED"), "{log}");
    assert!(
        matches!(listener.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock),
        "the host listener saw a connection"
    );
    match rep.status {
        RunStatus::Failed => {
            assert_eq!(rep.profiles[0].exit_code, 0, "{log}");
            assert_ne!(rep.profiles[1].exit_code, 0, "{log}");
            assert!(log.contains("plain-ok"), "{log}");
        }
        RunStatus::Error => {
            eprintln!("sandbox unavailable here: {log}");
            assert!(log.contains("not run") || log.contains("sandbox"), "{log}");
        }
        RunStatus::Passed => panic!("the probe connected: {log}"),
    }
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
    let exec = runner::Sandboxed::new(
        PathBuf::from(env!("CARGO_BIN_EXE_bbp")),
        &dir.join("sandbox-state"),
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
    assert_eq!(rep.sandbox, runner::Sandboxed::LABEL);
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
