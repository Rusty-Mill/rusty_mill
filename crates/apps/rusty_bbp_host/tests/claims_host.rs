#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Claims mode on the host (#593): opening a claims task, the credentialed
//! MCP tools, the sandboxed runner with `tests_changed`, and the inbox with
//! its dry run. CL-44 is `claims_open_refuses_a_host_without_a_sandbox` and
//! `a_claims_run_executes_only_in_the_sandbox`.
#![cfg(unix)]

use rusty_bbp::*;
use rusty_bbp_host::mcp::{Binding, Server};
use rusty_bbp_host::profiles::ProfileSet;
use rusty_bbp_host::runner::{self, Confinement, Unconfined};
use rusty_bbp_host::{admin, human, inbox, open_driver};
use rusty_serde::Value;
use std::path::{Path, PathBuf};
use std::process::Command as Proc;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bbp_claims_{}_{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

fn lens(id: &str, order: u16) -> Role {
    use ArtifactKind::*;
    use MessageKind::*;
    Role::new(id, order)
        .post(&[Claim, Evidence])
        .write(&[Diff, Candidate, Source])
        .read_artifacts(&[Brief, Diff, Candidate, TestReport, Log, Source])
}

fn profiles() -> ProfileSet {
    let mut set = ProfileSet::shell("test", "sh ./test.sh");
    set.test_paths = vec!["tests/**".into()];
    set
}

fn task() -> TaskId {
    TaskId("C1".into())
}

fn open(dir: &Path, set: &ProfileSet, probe: Result<(), String>) -> Response {
    admin::open_claims_task(
        dir,
        &task(),
        "local",
        b"Make the flag say ok.",
        &PrincipalId("human".into()),
        set,
        vec![lens("a", 0), lens("b", 1)],
        ClaimsConfig::default(),
        || probe,
    )
    .expect("open")
}

/// A claims task with lenses `a` and `b` assigned.
fn claims_task(dir: &Path) {
    assert!(matches!(
        open(dir, &profiles(), Ok(())),
        Response::Stored(_)
    ));
    for who in ["a", "b"] {
        admin::assign(
            dir,
            &task(),
            &RoleId::from(who),
            &PrincipalId(who.into()),
            "v",
        )
        .expect("assign");
    }
}

fn state(dir: &Path) -> TaskState {
    open_driver(dir, &task()).expect("driver").state
}

fn rejected(r: Response) -> (Code, String) {
    match r {
        Response::Rejected(rej) => (rej.code, rej.detail),
        other => panic!("expected a rejection, got {other:?}"),
    }
}

/// CL-44 (a): a host that cannot sandbox refuses a claims open, and so does a
/// profile set without test_paths; nothing is written either way.
#[test]
fn claims_open_refuses_a_host_without_a_sandbox() {
    let dir = tempdir("nosandbox");
    let (code, detail) = rejected(open(&dir, &profiles(), Err("no Landlock".into())));
    assert_eq!(code, Code::BadClaimsConfig);
    assert!(detail.starts_with("sandbox"), "{detail}");
    let (code, detail) = rejected(open(&dir, &ProfileSet::shell("t", "true"), Ok(())));
    assert_eq!(code, Code::BadClaimsConfig);
    assert!(detail.starts_with("test_paths"), "{detail}");
    assert!(!log_path_for(&dir, &task()).exists());
    assert!(!rusty_bbp_host::profiles::path(&dir, &task()).exists());

    // The CLI with confinement turned off is refused the same way.
    let files = tempdir("nosandbox-cli");
    let roster = files.join("roster.json");
    std::fs::write(
        &roster,
        rusty_serde::json::to_string(&vec![lens("a", 0)]).unwrap(),
    )
    .unwrap();
    let set = files.join("profiles.json");
    std::fs::write(&set, profiles().to_bytes().unwrap()).unwrap();
    let brief = files.join("brief.md");
    std::fs::write(&brief, "x").unwrap();
    let out = Proc::new(env!("CARGO_BIN_EXE_bbp"))
        .args([
            "open",
            "--mode",
            "claims",
            "--confine",
            "none",
            "--task",
            "C1",
            "--repo",
            "r",
        ])
        .arg("--dir")
        .arg(&dir)
        .arg("--brief")
        .arg(&brief)
        .arg("--roster")
        .arg(&roster)
        .arg("--profiles")
        .arg(&set)
        .output()
        .expect("bbp open");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("BadClaimsConfig") && text.contains("sandbox"),
        "{text}"
    );
    assert!(!log_path_for(&dir, &task()).exists());
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&files);
}

#[test]
fn a_claims_open_writes_a_claims_task() {
    let dir = tempdir("open");
    claims_task(&dir);
    let st = state(&dir);
    assert_eq!(st.mode, Mode::Claims);
    assert_eq!(st.state, State::Open);
    let (code, _) = rejected(open(&dir, &profiles(), Ok(())));
    assert_eq!(code, Code::AlreadyOpen);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- MCP

fn server(dir: &Path, who: &str) -> Server {
    Server::new(Binding {
        dir: dir.to_path_buf(),
        task: task(),
        principal: PrincipalId(who.into()),
        turn: None,
    })
    .expect("server")
}

/// One `tools/call` through the JSON-RPC handler: the text and whether it is an error.
fn call(s: &mut Server, tool: &str, args: &str) -> (String, bool) {
    let req: Value = rusty_serde::json::from_str(&format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"{tool}","arguments":{args}}}}}"#
    ))
    .unwrap();
    let resp = s.handle(&req).expect("reply");
    let result = &resp["result"];
    let text = result["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    (text, result["isError"].as_bool().unwrap_or(true))
}

fn ok(s: &mut Server, tool: &str, args: &str) -> String {
    let (text, err) = call(s, tool, args);
    assert!(!err, "{tool} {args}: {text}");
    text
}

fn id_of(text: &str) -> u64 {
    let v: Value = rusty_serde::json::from_str(text).unwrap();
    for key in ["Posted", "Stored"] {
        if let Some(id) = v[key].as_u64() {
            return id;
        }
    }
    panic!("no id in {text}")
}

fn brief(dir: &Path) -> u64 {
    state(dir).brief.expect("brief").0
}

/// Post a claim as `who`; its message id.
fn claim(dir: &Path, who: &str, text: &str, reversible: bool) -> u64 {
    let b = brief(dir);
    let mut s = server(dir, who);
    id_of(&ok(
        &mut s,
        "post",
        &format!(
            r#"{{"op":"c-{text}","kind":"claim","text":"{text}","domains":["d"],"stakes":"low","reversible":{reversible},"refs":["art:{b}"]}}"#
        ),
    ))
}

fn cited(dir: &Path, who: &str, c: u64, strength: u8) -> String {
    let b = brief(dir);
    let mut s = server(dir, who);
    ok(
        &mut s,
        "post",
        &format!(
            r#"{{"op":"e-{who}-{c}","kind":"evidence","claim":{c},"direction":"support","evidence_kind":"cited","strength":{strength},"refs":["art:{b}"],"evidence":["art:{b}"]}}"#
        ),
    )
}

#[test]
fn a_claims_session_posts_with_its_credential_and_never_shows_it() {
    let dir = tempdir("mcp");
    claims_task(&dir);
    let mut s = server(&dir, "a");
    let list: Value = rusty_serde::json::from_str(
        &s.handle(
            &rusty_serde::json::from_str(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#)
                .unwrap(),
        )
        .unwrap()
        .to_string(),
    )
    .unwrap();
    let names: Vec<&str> = match &list["result"]["tools"] {
        Value::Seq(t) => t.iter().filter_map(|x| x["name"].as_str()).collect(),
        _ => vec![],
    };
    for t in [
        "post",
        "put_artifact",
        "claims_delta",
        "claim_card",
        "claim_line",
    ] {
        assert!(names.contains(&t), "{names:?}");
    }
    assert!(!names.contains(&"wait"), "a claims task has no turns");

    let task_card = ok(&mut s, "task_card", "{}");
    let c = claim(&dir, "a", "flag", true);
    cited(&dir, "b", c, 1);
    let card = ok(&mut s, "claim_card", &format!(r#"{{"claim":{c}}}"#));
    assert!(card.contains("\"text\":\"flag\""), "{card}");
    let line = ok(&mut s, "claim_line", &format!(r#"{{"claim":{c}}}"#));
    assert!(line.contains("OPEN"), "{line}");
    let delta = ok(&mut s, "claims_delta", r#"{"after":0,"domains":["d"]}"#);
    assert!(delta.contains("\"text\":\"flag\""), "{delta}");
    let src = ok(
        &mut s,
        "put_artifact",
        r#"{"op":"s1","kind":"source","origin":"https://example.com","retrieved_at":"2026-11-02","excerpt":"quoted"}"#,
    );
    assert!(src.contains("Stored"), "{src}");
    // No reply ever carries the credential.
    let cred = state(&dir)
        .credential_for(&PrincipalId("a".into()))
        .unwrap();
    for text in [&task_card, &card, &line, &delta, &src] {
        assert!(!text.contains(&cred.0.hex()), "credential leaked");
    }
    // A per-turn binding on a claims task is refused; so is wait.
    assert!(Server::new(Binding {
        dir: dir.clone(),
        task: task(),
        principal: PrincipalId("a".into()),
        turn: Some(TurnId(1)),
    })
    .is_err());
    let (text, err) = call(&mut s, "wait", "{}");
    assert!(err, "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_closed_task_refuses_every_session_call() {
    let dir = tempdir("closed");
    claims_task(&dir);
    let c = claim(&dir, "a", "flag", true);
    let rev = state(&dir).rev;
    let r = human::perform(&dir, &task(), "close", &[], Some(rev)).expect("close");
    assert_eq!(r, Response::Ok);
    let mut s = server(&dir, "a");
    let (text, err) = call(&mut s, "claim_card", &format!(r#"{{"claim":{c}}}"#));
    assert!(err && text.contains("BadCredential"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

fn reassign(dir: &Path, role: &str, who: &str) {
    admin::assign(
        dir,
        &task(),
        &RoleId::from(role),
        &PrincipalId(who.into()),
        "v",
    )
    .expect("assign");
}

#[test]
fn a_reassignment_revokes_a_running_session() {
    let dir = tempdir("reassign");
    claims_task(&dir);
    let b = brief(&dir);
    let post = |op: &str| {
        format!(
            r#"{{"op":"{op}","kind":"claim","text":"{op}","domains":["d"],"stakes":"low","reversible":true,"refs":["art:{b}"]}}"#
        )
    };
    // A to A: the same principal, a new assignment.
    let mut old = server(&dir, "a");
    reassign(&dir, "a", "a");
    let (text, err) = call(&mut old, "post", &post("p1"));
    assert!(err && text.contains("BadCredential"), "{text}");
    // A to B to A: the role comes back, the old session does not.
    reassign(&dir, "a", "x");
    reassign(&dir, "a", "a");
    let (text, err) = call(&mut old, "task_card", "{}");
    assert!(err && text.contains("BadCredential"), "{text}");
    // A session started under the current assignment works.
    ok(&mut server(&dir, "a"), "post", &post("p2"));
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- inbox

#[test]
fn an_ended_task_leaves_the_inbox() {
    let dir = tempdir("ended");
    claims_task(&dir);
    let x = claim(&dir, "a", "ship it", false);
    cited(&dir, "a", x, 3);
    cited(&dir, "b", x, 3);
    assert_eq!(inbox::read(&dir, &[task()]).unwrap().len(), 1);
    let rev = state(&dir).rev;
    let r = human::perform(&dir, &task(), "close", &[], Some(rev)).expect("close");
    assert_eq!(r, Response::Ok);
    assert!(inbox::read(&dir, &[task()]).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_irreversible_claim_reaches_the_inbox_and_a_dry_run_commits_nothing() {
    let dir = tempdir("inbox");
    claims_task(&dir);
    assert!(
        inbox::read(&dir, &[task()]).unwrap().is_empty(),
        "usually empty"
    );
    let x = claim(&dir, "a", "ship it", false);
    cited(&dir, "a", x, 3);
    cited(&dir, "b", x, 3);
    let open = inbox::read(&dir, &[task()]).unwrap();
    assert_eq!(open.len(), 1, "{open:?}");
    let e = &open[0];
    assert_eq!(e.reason, inbox::Reason::Irreversible);
    assert_eq!(e.reversible, Some(false));
    assert_eq!(e.claim.as_ref().map(|c| c.id), Some(MsgId(x)));
    assert_eq!(e.evidence.len(), 2);
    assert!(e.id.starts_with("C1/msg:"), "{}", e.id);

    let arg = format!("msg:{x}");
    let preview = human::preview_fenced(
        &dir,
        &task(),
        "claim",
        &[&arg, "accept"],
        None,
        "owner",
        Some(e.rev),
        None,
    )
    .unwrap();
    assert_eq!(preview.response, Response::Ok);
    assert!(preview
        .events
        .iter()
        .any(|ev| matches!(ev, Event::ClaimAccepted { claim, .. } if claim.0 == x)));
    assert_eq!(state(&dir).rev, e.rev, "a dry run commits nothing");

    let r = human::perform_fenced(
        &dir,
        &task(),
        "claim",
        &[&arg, "accept"],
        None,
        "owner",
        Some(e.rev),
        None,
    )
    .unwrap();
    assert_eq!(r, Response::Ok);
    assert!(inbox::read(&dir, &[task()]).unwrap().is_empty());
    // The same fence again is stale: the card moved on.
    let (code, _) = rejected(
        human::perform_fenced(
            &dir,
            &task(),
            "claim",
            &[&arg, "drop"],
            None,
            "owner",
            Some(e.rev),
            None,
        )
        .unwrap(),
    );
    assert_eq!(code, Code::StaleRev);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- runner

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

/// A repository whose test passes only when `flag.txt` says `ok`, with a
/// test file under `tests/`.
fn repo(dir: &Path) -> (PathBuf, String) {
    let repo = dir.join("repo");
    std::fs::create_dir_all(repo.join("tests")).unwrap();
    git(&repo, &["init", "--quiet", "-b", "main"]);
    git(&repo, &["config", "user.email", "t@example.com"]);
    git(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("flag.txt"), "no\n").unwrap();
    std::fs::write(repo.join("test.sh"), "grep -q '^ok$' flag.txt\n").unwrap();
    std::fs::write(repo.join("tests/core.txt"), "one\n").unwrap();
    std::fs::write(repo.join("tests/old.txt"), "gone\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "--quiet", "-m", "base"]);
    let base = git(&repo, &["rev-parse", "HEAD"]);
    (repo, base)
}

const DIFF: &str = "--- a/flag.txt\n+++ b/flag.txt\n@@ -1 +1 @@\n-no\n+ok\n\
--- a/tests/core.txt\n+++ b/tests/core.txt\n@@ -1 +1 @@\n-one\n+two\n\
--- /dev/null\n+++ b/tests/new.txt\n@@ -0,0 +1 @@\n+new\n\
--- a/tests/old.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-gone\n";

/// A claim and a candidate on it through the MCP tools: a pending claims run.
fn candidate(dir: &Path, base: &str) {
    let c = claim(dir, "a", "flag", true);
    let mut s = server(dir, "a");
    let body = rusty_serde::json::to_string(&DIFF.to_owned()).unwrap();
    let diff = id_of(&ok(
        &mut s,
        "put_artifact",
        &format!(r#"{{"op":"d1","kind":"diff","body":{body}}}"#),
    ));
    ok(
        &mut s,
        "put_artifact",
        &format!(
            r#"{{"op":"k1","kind":"candidate","base":"{base}","diffs":[{diff}],"claim":{c}}}"#
        ),
    );
}

fn report(dir: &Path) -> (Report, String) {
    let d = open_driver(dir, &task()).unwrap();
    let run = d.state.claims.runs.values().next().expect("run");
    let rec = d.state.artifacts.get(&run.report.expect("report")).unwrap();
    let ArtifactPayload::TestReport(r) = &rec.payload else {
        panic!("not a report")
    };
    let log = d.state.artifacts.get(&run.log.expect("log")).unwrap();
    let text = String::from_utf8(d.store.blob_get(&log.blob.sha).unwrap()).unwrap();
    (r.clone(), text)
}

/// CL-44 (b), and the interim rule's input: the run executes inside the
/// sandbox profile and lists every test path the candidate adds, modifies or
/// deletes, and nothing else.
#[test]
fn a_claims_run_executes_only_in_the_sandbox() {
    let dir = tempdir("run");
    let (repo, base) = repo(&dir);
    claims_task(&dir);
    candidate(&dir, &base);
    let work = dir.join("work");
    let err = runner::run_once(
        &dir,
        &task(),
        &repo,
        &work,
        &Unconfined,
        Confinement::Unconfined,
    )
    .unwrap_err();
    assert!(err.contains("sandboxed"), "{err}");
    let exec = runner::Sandboxed::new(
        PathBuf::from(env!("CARGO_BIN_EXE_bbp")),
        &dir.join("sandbox-state"),
    );
    runner::run_once(&dir, &task(), &repo, &work, &exec, Confinement::Sandboxed).expect("run");
    let (rep, log) = report(&dir);
    assert_eq!(rep.sandbox, runner::Sandboxed::LABEL);
    let mut changed = rep.tests_changed.clone();
    changed.sort_by(|x, y| x.path.cmp(&y.path));
    assert_eq!(
        changed,
        vec![
            TestChange::modified("tests/core.txt"),
            TestChange::added("tests/new.txt"),
            TestChange::deleted("tests/old.txt"),
        ]
    );
    match rep.status {
        RunStatus::Passed => {}
        RunStatus::Error => {
            eprintln!("sandbox unavailable here: {log}");
            assert!(log.contains("not run") || log.contains("sandbox"), "{log}");
        }
        RunStatus::Failed => panic!("confined test failed: {log}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_host_probe_passes_where_the_sandbox_works() {
    let base = tempdir("probe");
    let exec = runner::Sandboxed::new(
        PathBuf::from(env!("CARGO_BIN_EXE_bbp")),
        &base.join("state"),
    );
    match runner::probe(&exec, &base.join("work")) {
        Ok(()) => {}
        Err(e) => eprintln!("sandbox unavailable here: {e}"),
    }
    // An executor that cannot start anything always fails the probe.
    struct Broken;
    impl rusty_sandbox::Executor for Broken {
        type Error = String;
        fn exec(
            &self,
            _: &rusty_sandbox::SandboxSpec,
            _: &str,
            _: &[String],
        ) -> Result<rusty_sandbox::ExecOutcome, String> {
            Err("no helper".into())
        }
    }
    assert!(runner::probe(&Broken, &base.join("work")).is_err());
    let _ = std::fs::remove_dir_all(&base);
}
