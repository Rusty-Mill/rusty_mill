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
use rusty_bbp_host::{admin, human, inbox, now, open_driver};
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
    open_as(dir, &task(), set, probe)
}

fn open_as(dir: &Path, t: &TaskId, set: &ProfileSet, probe: Result<(), String>) -> Response {
    admin::open_claims_task(
        dir,
        t,
        "local",
        b"Make the flag say ok.",
        &PrincipalId("human".into()),
        set,
        None,
        vec![lens("a", 0), lens("b", 1)],
        ClaimsConfig::default(),
        None,
        || probe,
    )
    .expect("open")
}

/// A claims task with lenses `a` and `b` assigned.
fn claims_task(dir: &Path) {
    claims_task_as(dir, &task());
}

fn claims_task_as(dir: &Path, t: &TaskId) {
    assert!(matches!(
        open_as(dir, t, &profiles(), Ok(())),
        Response::Stored(_)
    ));
    for who in ["a", "b"] {
        admin::assign(dir, t, &RoleId::from(who), &PrincipalId(who.into()), "v").expect("assign");
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
    server_as(dir, &task(), who)
}

fn server_as(dir: &Path, t: &TaskId, who: &str) -> Server {
    Server::new(Binding {
        dir: dir.to_path_buf(),
        task: t.clone(),
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
    for (verb, args) in [("close", vec![]), ("cancel", vec!["done here"])] {
        let dir = tempdir(&format!("ended-{verb}"));
        claims_task(&dir);
        let x = claim(&dir, "a", "ship it", false);
        cited(&dir, "a", x, 3);
        cited(&dir, "b", x, 3);
        assert_eq!(inbox::read(&dir, &[task()]).unwrap().len(), 1);
        let rev = state(&dir).rev;
        let r = human::perform(&dir, &task(), verb, &args, Some(rev)).expect(verb);
        assert_eq!(r, Response::Ok, "{verb}");
        assert!(state(&dir).state.terminal(), "{verb}");
        assert!(inbox::read(&dir, &[task()]).unwrap().is_empty(), "{verb}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// An irreversible claim on `t`, escalated by two cited +6 items.
fn escalate(dir: &Path, t: &TaskId, text: &str) -> u64 {
    let b = open_driver(dir, t).unwrap().state.brief.unwrap().0;
    let x = id_of(&ok(
        &mut server_as(dir, t, "a"),
        "post",
        &format!(
            r#"{{"op":"c-{text}","kind":"claim","text":"{text}","domains":["d"],"stakes":"low","reversible":false,"refs":["art:{b}"]}}"#
        ),
    ));
    for who in ["a", "b"] {
        ok(
            &mut server_as(dir, t, who),
            "post",
            &format!(
                r#"{{"op":"e-{who}-{x}","kind":"evidence","claim":{x},"direction":"support","evidence_kind":"cited","strength":3,"refs":["art:{b}"],"evidence":["art:{b}"]}}"#
            ),
        );
    }
    x
}

#[test]
fn the_inbox_never_compares_log_positions_across_tasks() {
    let dir = tempdir("order");
    let (late, early) = (TaskId("C1".into()), TaskId("C2".into()));
    claims_task_as(&dir, &late);
    claims_task_as(&dir, &early);
    escalate(&dir, &late, "filler");
    escalate(&dir, &late, "late");
    escalate(&dir, &early, "early");
    let rows = inbox::read(&dir, &[late.clone(), early.clone()]).unwrap();
    let order: Vec<(&str, usize)> = rows.iter().map(|e| (e.task.0.as_str(), e.seq)).collect();
    assert_eq!(order.len(), 3, "{order:?}");
    assert!(
        order[2].1 < order[1].1,
        "the later task's log is shorter: {order:?}"
    );
    assert_eq!(
        order.iter().map(|o| o.0).collect::<Vec<_>>(),
        ["C1", "C1", "C2"],
        "tasks in the order given"
    );
    let rows = inbox::read(&dir, &[early, late]).unwrap();
    assert_eq!(
        rows.iter().map(|e| e.task.0.as_str()).collect::<Vec<_>>(),
        ["C2", "C1", "C1"]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_reopened_claim_shows_only_its_current_evidence() {
    let dir = tempdir("epoch");
    claims_task(&dir);
    let x = escalate(&dir, &task(), "ship it");
    let arg = format!("msg:{x}");
    for verb in ["accept", "reopen"] {
        let rev = state(&dir).rev;
        let r = human::perform(&dir, &task(), "claim", &[&arg, verb], Some(rev)).expect(verb);
        assert_eq!(r, Response::Ok, "{verb}");
    }
    // The new epoch reaches +12 again on +4 items, weaker than the old +6s.
    let b = brief(&dir);
    for (n, who) in ["a", "b", "a"].iter().enumerate() {
        ok(
            &mut server(&dir, who),
            "post",
            &format!(
                r#"{{"op":"r-{n}","kind":"evidence","claim":{x},"direction":"support","evidence_kind":"cited","strength":2,"refs":["art:{b}"],"evidence":["art:{b}"]}}"#
            ),
        );
    }
    let rows = inbox::read(&dir, &[task()]).unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    let card = rows[0].claim.as_ref().expect("card");
    assert!(card.epoch > 0);
    assert_eq!(rows[0].evidence.len(), 2, "{:?}", rows[0].evidence);
    for item in &rows[0].evidence {
        assert_eq!(item.epoch, card.epoch, "{item:?}");
        assert_eq!(item.delta, 4, "{item:?}");
    }
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

#[test]
fn tests_changed_matches_unicode_paths_and_refuses_non_utf8_ones() {
    use std::os::unix::ffi::OsStrExt;
    let dir = tempdir("unicode");
    let (repo, base) = repo(&dir);
    for (name, body) in [("ü.rs", "1"), ("ñ.rs", "1")] {
        std::fs::write(repo.join("tests").join(name), body).unwrap();
    }
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "--quiet", "-m", "unicode"]);
    let base2 = git(&repo, &["rev-parse", "HEAD"]);
    std::fs::write(repo.join("tests/é.rs"), "new").unwrap();
    std::fs::write(repo.join("tests/éé.rs"), "two chars").unwrap();
    std::fs::write(repo.join("tests/ü.rs"), "2").unwrap();
    std::fs::remove_file(repo.join("tests/ñ.rs")).unwrap();
    git(&repo, &["add", "-A"]);
    let globs = vec!["tests/?.rs".to_owned()];
    let mut got = runner::tests_changed(&repo, &base2, &globs).expect("tests_changed");
    got.sort_by(|a, b| a.path.cmp(&b.path));
    let want = [
        ("tests/é.rs", FileChange::Added),
        ("tests/ñ.rs", FileChange::Deleted),
        ("tests/ü.rs", FileChange::Modified),
    ];
    let got: Vec<(&str, FileChange)> = got.iter().map(|c| (c.path.as_str(), c.change)).collect();
    assert_eq!(got, want);
    // A path git reports as raw non-UTF-8 bytes fails closed.
    let odd = std::ffi::OsStr::from_bytes(b"tests/\xff.rs");
    std::fs::write(repo.join(odd), "x").unwrap();
    git(&repo, &["add", "-A"]);
    let err = runner::tests_changed(&repo, &base, &globs).unwrap_err();
    assert!(err.contains("not UTF-8"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

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

// ---------------------------------------------------------------- inbox follow-up (#595)

fn post_dep(dir: &Path, text: &str, dep: u64) -> u64 {
    let b = brief(dir);
    id_of(&ok(
        &mut server(dir, "a"),
        "post",
        &format!(
            r#"{{"op":"c-{text}","kind":"claim","text":"{text}","domains":["d"],"stakes":"low","reversible":true,"deps":[{dep}],"refs":["art:{b}"]}}"#
        ),
    ))
}

fn human_ok(dir: &Path, verb: &str, args: &[&str]) {
    let rev = state(dir).rev;
    let r = human::perform_fenced(
        dir,
        &task(),
        verb,
        args,
        Some(rev),
        "owner",
        Some(rev),
        None,
    )
    .expect(verb);
    assert!(
        !matches!(r, Response::Rejected(_)),
        "{verb} {args:?}: {r:?}"
    );
}

fn log_len(dir: &Path) -> usize {
    open_driver(dir, &task())
        .expect("driver")
        .store
        .events(&task(), 0)
        .expect("events")
        .len()
}

/// Owner decision 2026-11-04: `extend-claim` gives an evidence run more
/// budget just as `extend` does, so it is refused on an evidence task at the
/// call and appends nothing; on any other task it still applies.
#[test]
fn extend_claim_is_refused_on_an_evidence_task_and_applies_elsewhere() {
    let dir = tempdir("extend-claim-evidence");
    claims_task(&dir);
    let c = claim(&dir, "a", "one", true);
    let arg = format!("msg:{c}");
    human_ok(&dir, "extend-claim", &[&arg]);
    assert_eq!(
        state(&dir).claims.get(MsgId(c)).expect("claim").extensions,
        1
    );

    rusty_bbp_host::moderator::write_evidence_marker(&dir, &task(), 5.0, "sha").expect("marker");
    let before = log_len(&dir);
    let rev = state(&dir).rev;
    let fenced = human::perform_fenced(
        &dir,
        &task(),
        "extend-claim",
        &[&arg],
        Some(rev),
        "owner",
        Some(rev),
        None,
    )
    .expect("fenced");
    let (code, detail) = rejected(fenced);
    assert_eq!(code, Code::BadHumanInput);
    assert!(detail.contains("evidence"), "{detail}");
    let cli = human::perform(&dir, &task(), "extend-claim", &[&arg], None).unwrap_err();
    assert!(cli.contains("evidence"), "{cli}");
    let preview = human::preview_fenced(
        &dir,
        &task(),
        "extend-claim",
        &[&arg],
        Some(rev),
        "owner",
        Some(rev),
        None,
    )
    .expect("preview");
    assert!(
        matches!(&preview.response, Response::Rejected(r) if r.code == Code::BadHumanInput),
        "{preview:?}"
    );
    assert_eq!(
        log_len(&dir),
        before,
        "a refused extend-claim appends nothing"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_acted_on_escalation_names_its_marks_and_refine_answers_it() {
    let dir = tempdir("acted");
    claims_task(&dir);
    let d = claim(&dir, "a", "foundation", true);
    let x = post_dep(&dir, "built-on-it", d);
    for c in [d, x] {
        human_ok(&dir, "claim", &[&format!("msg:{c}"), "accept"]);
    }
    let epoch = state(&dir).claims.get(MsgId(x)).expect("x").epoch;
    let mark = Command::Host {
        op: OpId("mark-x".into()),
        action: HostAction::Acting {
            claim: MsgId(x),
            epoch,
            reference: "pr-7".into(),
        },
    };
    let r = open_driver(&dir, &task())
        .unwrap()
        .dispatch(&mark, now())
        .unwrap();
    assert!(matches!(r, Response::Marked(_)), "{r:?}");
    human_ok(&dir, "claim", &[&format!("msg:{d}"), "drop"]);

    let rows = inbox::read(&dir, &[task()]).unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].reason, inbox::Reason::DepCollapsed);
    assert_eq!(rows[0].brief, state(&dir).brief);
    assert_eq!(
        rows[0].acted,
        [inbox::Acted {
            claim: MsgId(x),
            marks: vec!["pr-7".into()]
        }]
    );

    let b = format!("art:{}", brief(&dir));
    human_ok(
        &dir,
        "refine",
        &[&format!("msg:{x}"), &b, "keep", "it", "small"],
    );
    let st = state(&dir);
    let last = st.messages.last().expect("answer");
    assert_eq!(last.draft.kind, MessageKind::Answer);
    assert_eq!(last.draft.reply_to, Some(MsgId(x)));
    assert_eq!(last.draft.refs, [Ref::art(st.brief.expect("brief"))]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_task_total_row_names_the_field_its_limit_and_one_allowance() {
    let dir = tempdir("total");
    claims_task(&dir);
    let opened = state(&dir).budget.messages;
    human_ok(&dir, "extend", &["messages", "1"]);
    claim(&dir, "a", "one", true);
    claim(&dir, "a", "two", true);
    assert_eq!(state(&dir).state, State::Escalated);
    let rows = inbox::read(&dir, &[task()]).unwrap();
    let row = rows
        .iter()
        .find(|r| r.reason == inbox::Reason::TaskTotal)
        .expect("task row");
    assert_eq!(row.totals.len(), 1, "{row:?}");
    let t = row.totals[0];
    assert_eq!(
        (t.field, t.limit, t.allowance),
        (BudgetField::Messages, 1, opened)
    );
    assert!(t.spent > t.limit);
    // Refine on the whole task, then one allowance and resume, as the inbox does.
    let b = format!("art:{}", brief(&dir));
    human_ok(&dir, "refine", &["task", &b, "smaller", "steps"]);
    human_ok(
        &dir,
        "extend",
        &["messages", &(t.limit + t.allowance).to_string()],
    );
    human_ok(&dir, "resume", &["open"]);
    assert_eq!(state(&dir).state, State::Open);
    assert!(inbox::read(&dir, &[task()])
        .unwrap()
        .iter()
        .all(|r| r.reason != inbox::Reason::TaskTotal));
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- intent versions and pauses (#598)

#[test]
fn a_host_pause_shows_in_the_inbox_and_resume_open_clears_it() {
    let dir = tempdir("pause");
    claims_task(&dir);
    let pause = || {
        admin::pause(
            &dir,
            &task(),
            admin::pause_reason("call_ceiling").unwrap(),
            "400 of 400 calls",
            OpId("pause-1".into()),
        )
        .unwrap()
    };
    assert_eq!(pause(), Response::Ok);
    assert_eq!(pause(), Response::Ok, "the same op replays");
    assert_eq!(state(&dir).state, State::Escalated);
    let rows = inbox::read(&dir, &[task()]).unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].reason, inbox::Reason::TaskPaused);
    assert!(rows[0].totals.is_empty(), "no total is exhausted");
    assert_eq!(
        rows[0].pause,
        Some(inbox::Pause {
            reason: PauseReason::CallCeiling,
            detail: "400 of 400 calls".into()
        })
    );
    human_ok(&dir, "resume", &["open"]);
    assert_eq!(state(&dir).state, State::Open);
    assert!(inbox::read(&dir, &[task()]).unwrap().is_empty());
    assert!(admin::pause_reason("bored").is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn intent_stores_the_new_brief_and_reopens_or_escalates_what_it_affects() {
    let dir = tempdir("intent");
    claims_task(&dir);
    let x = claim(&dir, "a", "plain", true);
    let y = claim(&dir, "a", "shipped", true);
    for c in [x, y] {
        human_ok(&dir, "claim", &[&format!("msg:{c}"), "accept"]);
    }
    let epoch = state(&dir).claims.get(MsgId(y)).unwrap().epoch;
    let mark = Command::Host {
        op: OpId("mark-y".into()),
        action: HostAction::Acting {
            claim: MsgId(y),
            epoch,
            reference: "pr-9".into(),
        },
    };
    open_driver(&dir, &task())
        .unwrap()
        .dispatch(&mark, now())
        .unwrap();
    let old = brief(&dir);
    let file = dir.join("brief-v2.md");
    std::fs::write(&file, "Make the flag say ok, in green.").unwrap();
    let path = file.to_str().unwrap();
    // Lists are comma-separated, one argument each; a repeat is refused.
    let rev = state(&dir).rev;
    let twice = human::perform_fenced(
        &dir,
        &task(),
        "intent",
        &[path, "claims=msg:1", "claims=msg:2"],
        Some(rev),
        "owner",
        Some(rev),
        None,
    );
    assert!(matches!(twice, Ok(Response::Rejected(r)) if r.code == Code::BadHumanInput));
    human_ok(&dir, "intent", &[path, &format!("claims=msg:{x},msg:{y}")]);
    let st = state(&dir);
    assert_ne!(st.brief.unwrap().0, old, "the new brief is current");
    assert_eq!(st.claims.intent_version(), 2);
    assert_eq!(st.claims.get(MsgId(x)).unwrap().status, ClaimStatus::Open);
    assert_eq!(
        st.claims.get(MsgId(y)).unwrap().status,
        ClaimStatus::Escalated
    );
    let rows = inbox::read(&dir, &[task()]).unwrap();
    assert!(
        rows.iter().any(|r| r.reason == inbox::Reason::IntentChanged
            && r.claim.as_ref().map(|c| c.id) == Some(MsgId(y))),
        "{rows:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- brownfield baseline (#598)

/// Open the claims task on `repo`'s base with the suite already run there.
fn open_on_base(dir: &Path, b: &rusty_bbp_host::baseline::Baseline) -> Response {
    admin::open_claims_task(
        dir,
        &task(),
        "local",
        b"Make the flag say ok.",
        &PrincipalId("human".into()),
        &profiles(),
        None,
        vec![lens("a", 0), lens("b", 1)],
        ClaimsConfig::default(),
        Some(b),
        || Ok(()),
    )
    .expect("open")
}

#[test]
fn a_red_base_refuses_the_open_and_writes_nothing() {
    let dir = tempdir("base-red");
    let (repo, base) = repo(&dir);
    let b = runner::baseline(&Unconfined, &profiles(), &repo, &dir.join("work"), &base);
    assert_eq!(b.status, RunStatus::Failed, "flag.txt says no at the base");
    let (code, detail) = rejected(open_on_base(&dir, &b));
    assert_eq!(code, Code::BaselineRed);
    assert!(
        detail.contains("test (exit 1)") && detail.contains("edit profile `test`"),
        "{detail}"
    );
    assert!(!dir.join("master").exists(), "nothing written");
    assert!(!rusty_bbp_host::profiles::path(&dir, &task()).exists());
    assert!(!rusty_bbp_host::baseline::path(&dir, &task()).exists());
    assert!(rusty_bbp_host::baseline::resolve(&repo, "no-such-ref").is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_green_base_pins_candidates_and_guards_its_tests() {
    let dir = tempdir("base-green");
    let (repo, _) = repo(&dir);
    std::fs::write(repo.join("flag.txt"), "ok\n").unwrap();
    git(&repo, &["commit", "--quiet", "-am", "green"]);
    let base = rusty_bbp_host::baseline::resolve(&repo, "main").unwrap();
    let b = runner::baseline(&Unconfined, &profiles(), &repo, &dir.join("work"), &base);
    assert!(b.green(), "{b:?}");
    assert_eq!(b.tests, ["tests/core.txt", "tests/old.txt"]);
    assert!(matches!(open_on_base(&dir, &b), Response::Stored(_)));
    for who in ["a", "b"] {
        admin::assign(
            &dir,
            &task(),
            &RoleId::from(who),
            &PrincipalId(who.into()),
            "v",
        )
        .unwrap();
    }
    let st = state(&dir);
    assert!(st.claims.baseline_digest.is_some());
    assert_eq!(
        rusty_bbp_host::profiles::pinned_base(&dir, &task()).unwrap(),
        Some(base.clone())
    );
    let reversible = claim(&dir, "a", "keep", true);
    let irreversible = claim(&dir, "a", "rewrite", false);
    let mut s = server(&dir, "a");
    let body = rusty_serde::json::to_string(&DIFF.to_owned()).unwrap();
    let diff = id_of(&ok(
        &mut s,
        "put_artifact",
        &format!(r#"{{"op":"d1","kind":"diff","body":{body}}}"#),
    ));
    let put = |s: &mut Server, op: &str, base: &str, c: u64| {
        call(
            s,
            "put_artifact",
            &format!(
                r#"{{"op":"{op}","kind":"candidate","base":"{base}","diffs":[{diff}],"claim":{c}}}"#
            ),
        )
        .0
    };
    let refused = put(&mut s, "k1", &base, reversible);
    assert!(refused.contains("BaseTestChanged"), "{refused}");
    assert!(
        refused.contains("tests/core.txt") && refused.contains("tests/old.txt"),
        "{refused}"
    );
    assert!(
        !refused.contains("tests/new.txt"),
        "an added test is no change to the base: {refused}"
    );
    let off = put(&mut s, "k2", &"b".repeat(40), irreversible);
    assert!(
        off.contains("baseline"),
        "a pinned base holds without protected paths: {off}"
    );
    let stored = put(&mut s, "k3", &base, irreversible);
    assert!(
        stored.contains("Stored"),
        "an irreversible claim goes to Nano: {stored}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Every file under `dir` with its bytes, except `skip`.
fn files_under(dir: &Path, skip: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    let mut out = std::collections::BTreeMap::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                todo.push(p);
            } else if p != skip {
                out.insert(p.clone(), std::fs::read(&p).unwrap());
            }
        }
    }
    out
}

#[test]
fn an_intent_dry_run_stores_nothing_and_matches_the_apply() {
    let dir = tempdir("intent_dry");
    claims_task(&dir);
    let x = claim(&dir, "a", "plain", true);
    human_ok(&dir, "claim", &[&format!("msg:{x}"), "accept"]);
    let file = dir.join("brief-v2.md");
    std::fs::write(&file, "A brief only previewed.").unwrap();
    let path = file.to_str().unwrap();
    let sha = Sha256::of(b"A brief only previewed.");
    let before = files_under(&dir, &file);
    let rev = state(&dir).rev;
    let op = || Some(OpId("intent-1".into()));
    let preview = |args: &[&str], fence: Rev| {
        human::preview_fenced(
            &dir,
            &task(),
            "intent",
            args,
            Some(rev),
            "owner",
            Some(fence),
            op(),
        )
        .unwrap()
    };
    let fresh = preview(&[path], rev);
    assert!(matches!(fresh.response, Response::Stored(_)), "{fresh:?}");
    let unknown = preview(&[path, "claims=msg:999"], rev);
    assert!(
        matches!(&unknown.response, Response::Rejected(r) if r.code == Code::BadAffects),
        "{unknown:?}"
    );
    let stale = preview(&[path], Rev(rev.0 - 1));
    assert!(matches!(stale.response, Response::Rejected(_)), "{stale:?}");
    assert_eq!(files_under(&dir, &file), before, "no preview wrote a file");
    assert!(open_driver(&dir, &task())
        .unwrap()
        .store
        .blob_get(&sha)
        .is_err());

    let applied = human::perform_fenced(
        &dir,
        &task(),
        "intent",
        &[path],
        Some(rev),
        "owner",
        Some(rev),
        op(),
    )
    .unwrap();
    assert_eq!(
        applied, fresh.response,
        "the apply answers as its preview did"
    );
    assert_eq!(state(&dir).rev.0, rev.0 + fresh.events.len() as u64);
    let stored = open_driver(&dir, &task())
        .unwrap()
        .store
        .blob_get(&sha)
        .unwrap();
    assert_eq!(stored, b"A brief only previewed.");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- protected paths (#597)

/// Store `diff` as `who` and a candidate of it naming claim `c`.
fn put_candidate(s: &mut Server, op: &str, base: &str, diff: &str, c: u64) -> (String, bool) {
    let body = rusty_serde::json::to_string(&diff.to_owned()).unwrap();
    let d = id_of(&ok(
        s,
        "put_artifact",
        &format!(r#"{{"op":"{op}-d","kind":"diff","body":{body}}}"#),
    ));
    call(
        s,
        "put_artifact",
        &format!(
            r#"{{"op":"{op}-k","kind":"candidate","base":"{base}","diffs":[{d}],"claim":{c}}}"#
        ),
    )
}

const TESTS_ONLY: &str = "--- a/tests/core.txt\n+++ b/tests/core.txt\n@@ -1 +1 @@\n-one\n+two\n";
const FLAG_ONLY: &str = "--- a/flag.txt\n+++ b/flag.txt\n@@ -1 +1 @@\n-no\n+ok\n";

/// #597 G2: a candidate touching a protected path is refused before the core
/// sees it, naming each path, with nothing stored or debited; a `tests`
/// claim's candidate may touch only protected paths, and once that claim is
/// accepted its diff may be reused while any other change is still refused.
#[test]
fn protected_paths_are_refused_outside_an_accepted_tests_claim() {
    let dir = tempdir("protected");
    let base = "a".repeat(40);
    let mut set = profiles();
    set.protected = vec!["tests/**".into()];
    // Protected paths need a baseline, and globs a matcher reads literally
    // (protecting nothing) are refused.
    for (globs, baseline) in [
        (vec!["tests/**"], None),
        (vec!["tests/[ab].rs"], Some(base.clone())),
        (vec!["/tests/**"], Some(base.clone())),
    ] {
        let mut bad = set.clone();
        bad.protected = globs.into_iter().map(str::to_owned).collect();
        bad.baseline = baseline;
        let (code, detail) = rejected(open(&dir, &bad, Ok(())));
        assert_eq!(code, Code::BadClaimsConfig, "{detail}");
        assert!(detail.starts_with("protected:"), "{detail}");
    }
    set.baseline = Some(base.clone());
    assert!(matches!(open(&dir, &set, Ok(())), Response::Stored(_)));
    for who in ["a", "b"] {
        admin::assign(
            &dir,
            &task(),
            &RoleId::from(who),
            &PrincipalId(who.into()),
            "v",
        )
        .expect("assign");
    }
    let c = claim(&dir, "a", "flag", true);
    let mut s = server(&dir, "a");
    // Another base could carry weaker tests that no diff shows.
    let (text, err) = put_candidate(&mut s, "old", &"b".repeat(40), FLAG_ONLY, c);
    assert!(err && text.contains("baseline"), "{text}");

    let (text, err) = put_candidate(&mut s, "all", &base, DIFF, c);
    assert!(err && text.contains("ProtectedPath"), "{text}");
    for p in ["tests/core.txt", "tests/new.txt", "tests/old.txt"] {
        assert!(text.contains(p), "{p} not named: {text}");
    }
    assert!(!text.contains("flag.txt"), "{text}");
    let st = state(&dir);
    assert!(st.claims.runs.is_empty(), "nothing debited");
    assert_eq!(st.claims.get(MsgId(c)).unwrap().current, None);

    let (text, err) = put_candidate(&mut s, "flag", &base, FLAG_ONLY, c);
    assert!(!err, "{text}");

    // A `tests` claim may carry a test-only candidate, not a mixed one.
    let b = brief(&dir);
    let t = id_of(&ok(
        &mut s,
        "post",
        &format!(
            r#"{{"op":"tests","kind":"claim","text":"fix core","domains":["tests"],"stakes":"low","reversible":true,"refs":["art:{b}"]}}"#
        ),
    ));
    let (text, err) = put_candidate(&mut s, "mixed", &base, DIFF, t);
    assert!(err && text.contains("ProtectedPath"), "{text}");
    let (text, err) = put_candidate(&mut s, "fix", &base, TESTS_ONLY, t);
    assert!(!err, "{text}");
    let fix = match &state(&dir).artifacts[&ArtId(id_of(&text))].payload {
        ArtifactPayload::Candidate(m) => m.diffs[0].0,
        _ => panic!("not a candidate"),
    };

    // Before acceptance its diff cannot be reused.
    let reuse = format!(
        r#"{{"op":"reuse","kind":"candidate","base":"{base}","diffs":[{fix}],"claim":{c}}}"#
    );
    let (text, err) = call(&mut s, "put_artifact", &reuse);
    assert!(err && text.contains("ProtectedPath"), "{text}");
    let rev = state(&dir).rev;
    let arg = format!("msg:{t}");
    let r = human::perform(&dir, &task(), "claim", &[&arg, "accept"], Some(rev)).expect("accept");
    assert_eq!(r, Response::Ok);
    let reuse = reuse.replace("\"reuse\"", "\"reuse2\"");
    let (text, err) = call(&mut s, "put_artifact", &reuse);
    assert!(!err, "{text}");
    // Once T is dropped its diff is no longer exempt, but the committed op
    // still replays its stored answer.
    let rev = state(&dir).rev;
    let r = human::perform(&dir, &task(), "claim", &[&arg, "drop"], Some(rev)).expect("drop");
    assert_eq!(r, Response::Ok);
    let (again, err) = call(&mut s, "put_artifact", &reuse);
    assert!(!err && again == text, "{again}");
    let fresh = reuse.replace("\"reuse2\"", "\"reuse3\"");
    let (text, err) = call(&mut s, "put_artifact", &fresh);
    assert!(err && text.contains("ProtectedPath"), "{text}");
    // A fresh diff to the same path is still refused.
    let (text, err) = put_candidate(&mut s, "again", &base, TESTS_ONLY, c);
    assert!(err && text.contains("ProtectedPath"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- holdout suites (#597)

fn holdout_lens() -> Role {
    lens("h", 2).holdout()
}

/// A suite whose one test passes when the candidate's `flag.txt` says `ok`
/// and prints something builders must never see.
fn holdout_suite(dir: &Path) -> rusty_bbp_host::holdout::HoldoutSet {
    let root = dir.join("suite");
    std::fs::create_dir_all(root.join("api")).unwrap();
    std::fs::write(
        root.join("api/test.sh"),
        "echo HOLDOUT-SECRET-OUTPUT\ngrep -q '^ok$' flag.txt\n",
    )
    .unwrap();
    let set = ProfileSet::shell("holdout:api", "sh \"$BBP_HOLDOUT/api/test.sh\"");
    rusty_bbp_host::holdout::HoldoutSet::scan(set, &root).expect("scan")
}

fn holdout_task(dir: &Path, hs: &rusty_bbp_host::holdout::HoldoutSet) {
    holdout_task_with(dir, &profiles(), hs);
}

fn holdout_task_with(dir: &Path, set: &ProfileSet, hs: &rusty_bbp_host::holdout::HoldoutSet) {
    let r = open_holdout(dir, set, hs);
    assert!(matches!(r, Response::Stored(_)), "{r:?}");
    for who in ["a", "b", "h"] {
        reassign(dir, who, who);
    }
}

fn open_holdout(
    dir: &Path,
    set: &ProfileSet,
    hs: &rusty_bbp_host::holdout::HoldoutSet,
) -> Response {
    admin::open_claims_task(
        dir,
        &task(),
        "local",
        b"Make the flag say ok.",
        &PrincipalId("human".into()),
        set,
        Some(hs),
        vec![lens("a", 0), lens("b", 1), holdout_lens()],
        ClaimsConfig::default(),
        None,
        || Ok(()),
    )
    .expect("open")
}

fn sandbox(dir: &Path) -> runner::Sandboxed {
    runner::Sandboxed::new(
        PathBuf::from(env!("CARGO_BIN_EXE_bbp")),
        &dir.join("sandbox-state"),
    )
}

/// Store a build candidate for the flag fix and a holdout candidate of the
/// same code; their ids and the holdout candidate's JSON.
fn build_and_holdout(dir: &Path, base: &str) -> (u64, u64, String) {
    let c = claim(dir, "a", "flag", true);
    let mut s = server(dir, "a");
    let (text, err) = put_candidate(&mut s, "k", base, FLAG_ONLY, c);
    assert!(!err, "{text}");
    let k = id_of(&text);
    let diffs = match &state(dir).artifacts[&ArtId(k)].payload {
        ArtifactPayload::Candidate(m) => m.diffs[0].0,
        _ => panic!("not a candidate"),
    };
    let holdout = format!(
        r#"{{"op":"h1","kind":"candidate","base":"{base}","diffs":[{diffs}],"claim":{c},"suite":"holdout"}}"#
    );
    // A builder cannot store a holdout candidate; the holdout lens can.
    let (text, err) = call(&mut s, "put_artifact", &holdout);
    assert!(err && text.contains("KindForbidden"), "{text}");
    let mut h = server(dir, "h");
    let h1 = id_of(&ok(&mut h, "put_artifact", &holdout));
    (k, h1, holdout)
}

/// The run of candidate `k`, its report and the log artifact's text.
fn run_of(dir: &Path, k: u64) -> (RunId, Report, String) {
    let d = open_driver(dir, &task()).unwrap();
    let (id, run) = d
        .state
        .claims
        .runs
        .iter()
        .find(|(_, r)| r.candidate == ArtId(k))
        .map(|(id, r)| (*id, r.clone()))
        .expect("run");
    let ArtifactPayload::TestReport(rep) = &d.state.artifacts[&run.report.expect("report")].payload
    else {
        panic!("not a report")
    };
    let log = &d.state.artifacts[&run.log.expect("log")];
    let shown = String::from_utf8(d.store.blob_get(&log.blob.sha).unwrap()).unwrap();
    (id, rep.clone(), shown)
}

/// #597 2.2: a holdout run reports under the holdout digest and stores only
/// a summary as its log, whatever the sandbox here did; a changed suite file
/// fails the next holdout run before anything executes. That the suite
/// really runs, confined, is `confined_holdout_run_passes_and_builds_cannot_read_the_suite`.
#[test]
fn a_holdout_run_shows_builders_only_a_summary() {
    let dir = tempdir("holdout");
    let (repo, base) = repo(&dir);
    let hs = holdout_suite(&dir);
    holdout_task(&dir, &hs);
    let digest = state(&dir).claims.holdout_digest.expect("holdout digest");
    let work = dir.join("work");
    let exec = sandbox(&dir);
    let (_, h1, holdout) = build_and_holdout(&dir, &base);
    runner::run_once(&dir, &task(), &repo, &work, &exec, Confinement::Sandboxed).expect("build");
    runner::run_once(&dir, &task(), &repo, &work, &exec, Confinement::Sandboxed).expect("holdout");
    let (_, rep, shown) = run_of(&dir, h1);
    assert_eq!(rep.profile_digest, digest);
    assert!(shown.starts_with("holdout run: "), "{shown}");
    assert!(!shown.contains("HOLDOUT-SECRET-OUTPUT"), "{shown}");

    // A planted file in the suite fails the next holdout run before it runs.
    std::fs::write(Path::new(&hs.root).join("conftest.py"), "skip").unwrap();
    let again = holdout.replace("\"h1\"", "\"h2\"");
    let h2 = id_of(&ok(&mut server(&dir, "h"), "put_artifact", &again));
    runner::run_once(&dir, &task(), &repo, &work, &exec, Confinement::Sandboxed).expect("planted");
    let (_, rep, shown) = run_of(&dir, h2);
    assert_eq!(rep.status, RunStatus::Error);
    assert!(!shown.contains("conftest"), "{shown}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// #597 2.2, Linux acceptance lane: on a host whose sandbox works, a build
/// run cannot read the suite and the holdout run executes it, passing, with
/// its output on the host only. Refuses to pass where the sandbox does not.
#[test]
#[ignore = "needs a working sandbox; run by the confined acceptance lane"]
fn confined_holdout_run_passes_and_builds_cannot_read_the_suite() {
    let dir = tempdir("holdout_confined");
    runner::probe(&sandbox(&dir), &dir.join("probe")).expect("a working sandbox");
    let (repo, base) = repo(&dir);
    let hs = holdout_suite(&dir);
    let marker = Path::new(&hs.root).join("api/test.sh");
    let mut set = profiles();
    set.profiles[0].args[1] = format!(
        "sh ./test.sh && ! cat {} && echo SUITE-UNREADABLE",
        marker.display()
    );
    holdout_task_with(&dir, &set, &hs);
    let work = dir.join("work");
    let exec = sandbox(&dir);
    let (k, h1, _) = build_and_holdout(&dir, &base);
    runner::run_once(&dir, &task(), &repo, &work, &exec, Confinement::Sandboxed).expect("build");
    runner::run_once(&dir, &task(), &repo, &work, &exec, Confinement::Sandboxed).expect("holdout");

    let (_, rep, log) = run_of(&dir, k);
    assert_eq!(rep.status, RunStatus::Passed, "{log}");
    assert_eq!(rep.sandbox, runner::Sandboxed::LABEL);
    assert!(log.contains("SUITE-UNREADABLE"), "{log}");
    assert!(!log.contains("HOLDOUT-SECRET-OUTPUT"), "{log}");

    let (run, rep, shown) = run_of(&dir, h1);
    let full = std::fs::read_to_string(rusty_bbp_host::holdout::log_path(&dir, &task(), run))
        .expect("host-only log");
    assert_eq!(rep.status, RunStatus::Passed, "{full}");
    assert!(shown.contains("holdout:api: exit 0"), "{shown}");
    assert!(!shown.contains("HOLDOUT-SECRET-OUTPUT"), "{shown}");
    assert!(full.contains("HOLDOUT-SECRET-OUTPUT"), "{full}");
    println!("BBP_ACCEPTANCE PASS holdout-confined");
    let _ = std::fs::remove_dir_all(&dir);
}

/// #597 2.2: a suite any build run could read is refused at open, and a run
/// whose repository or work root overlaps the suite never starts.
#[test]
fn a_holdout_suite_build_runs_could_read_is_refused() {
    let dir = tempdir("holdout_overlap");
    let (repo, _) = repo(&dir);
    let hs = holdout_suite(&dir);
    let mut wide = profiles();
    wide.read_roots.push(dir.to_str().unwrap().into());
    match open_holdout(&dir, &wide, &hs) {
        Response::Rejected(r) => {
            assert_eq!(r.code, Code::BadClaimsConfig);
            assert!(r.detail.contains("overlaps"), "{}", r.detail);
        }
        other => panic!("{other:?}"),
    }
    holdout_task(&dir, &hs);
    let base = git(&repo, &["rev-parse", "HEAD"]);
    build_and_holdout(&dir, &base);
    let inside = Path::new(&hs.root).join("work");
    let err = runner::run_once(
        &dir,
        &task(),
        &repo,
        &inside,
        &sandbox(&dir),
        Confinement::Sandboxed,
    )
    .unwrap_err();
    assert!(err.contains("overlaps"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// #597 2.2: a holdout run whose log was stored before the runner died is
/// reported `error` against that log, and the host-only full output of the
/// run that died is kept, not overwritten by the recovery.
#[test]
fn a_recovered_holdout_run_keeps_its_host_only_log() {
    let dir = tempdir("holdout_recover");
    let (repo, base) = repo(&dir);
    let hs = holdout_suite(&dir);
    holdout_task(&dir, &hs);
    let work = dir.join("work");
    let exec = sandbox(&dir);
    let (_, h1, _) = build_and_holdout(&dir, &base);
    runner::run_once(&dir, &task(), &repo, &work, &exec, Confinement::Sandboxed).expect("build");
    // The holdout runner wrote both logs and died before the report.
    let mut d = open_driver(&dir, &task()).unwrap();
    let (run, _) = d
        .state
        .claims
        .runs
        .iter()
        .find(|(_, r)| r.candidate == ArtId(h1))
        .map(|(id, r)| (*id, r.clone()))
        .expect("holdout run");
    let full = rusty_bbp_host::holdout::log_path(&dir, &task(), run);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(&full, "HOLDOUT-SECRET-OUTPUT from the run that died\n").unwrap();
    let blob = d
        .store
        .blob_put(b"holdout run: Passed (full output is host-only)\n");
    let r = d
        .dispatch(
            &Command::Runner {
                op: rusty_bbp_host::fresh_op(),
                run,
                secret: d.state.secret_for(run),
                blob,
                payload: ArtifactPayload::Log,
            },
            rusty_bbp_host::now(),
        )
        .expect("dispatch");
    assert!(matches!(r, Response::Stored(_)), "{r:?}");
    drop(d);
    runner::run_once(&dir, &task(), &repo, &work, &exec, Confinement::Sandboxed).expect("recover");
    let (_, rep, shown) = run_of(&dir, h1);
    assert_eq!(rep.status, RunStatus::Error);
    assert!(shown.starts_with("holdout run: Passed"), "{shown}");
    assert_eq!(
        std::fs::read_to_string(&full).unwrap(),
        "HOLDOUT-SECRET-OUTPUT from the run that died\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// #597 G3: a suite that only adds is adopted by the host; one that changes
/// a test is staged and needs the human, who adopts it by digest.
#[test]
fn holdout_versions_add_freely_and_change_only_with_the_human() {
    let dir = tempdir("holdout-versions");
    let hs = holdout_suite(&dir);
    holdout_task(&dir, &hs);
    let v1 = state(&dir).claims.holdout_digest.expect("v1");
    let root = PathBuf::from(&hs.root);
    std::fs::write(root.join("api/more.sh"), "true\n").unwrap();
    let v2_set = rusty_bbp_host::holdout::HoldoutSet::scan(hs.set.clone(), &root).unwrap();
    assert_eq!(
        admin::adopt_holdout(&dir, &task(), &v2_set).unwrap(),
        Response::Ok
    );
    let v2 = state(&dir).claims.holdout_digest.expect("v2");
    assert_ne!(v1, v2);

    std::fs::write(root.join("api/test.sh"), "true\n").unwrap();
    let v3_set = rusty_bbp_host::holdout::HoldoutSet::scan(hs.set.clone(), &root).unwrap();
    let (code, detail) = rejected(admin::adopt_holdout(&dir, &task(), &v3_set).unwrap());
    assert_eq!(code, Code::HumanRequired);
    assert_eq!(state(&dir).claims.holdout_digest, Some(v2));
    let v3 = detail
        .split_whitespace()
        .find(|w| w.len() == 64)
        .expect("digest named")
        .to_owned();
    let rev = state(&dir).rev;
    // A digest staged for no suite of this task is refused before the core.
    let typo = "ab".repeat(32);
    let (code, _) =
        rejected(human::perform(&dir, &task(), "holdout-updated", &[&typo], Some(rev)).unwrap());
    assert_eq!(code, Code::HoldoutMismatch);
    assert_eq!(state(&dir).claims.holdout_digest, Some(v2));
    let r = human::perform(&dir, &task(), "holdout-updated", &[&v3], Some(rev)).expect("update");
    assert_eq!(r, Response::Ok);
    assert_eq!(state(&dir).claims.holdout_digest.map(|d| d.hex()), Some(v3));
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- golden sets (#597)

/// A golden set whose one case checks that the candidate's flag says ok.
fn golden_set(dir: &Path, sign: bool) -> PathBuf {
    golden_set_with(dir, sign, &[])
}

/// A golden set whose case `says-ok` reads the candidate's `flag.txt`, plus
/// `extra` cases (name, case.toml) and the private absolute fixtures listed
/// in its MANIFEST.
fn golden_set_with(dir: &Path, sign: bool, extra: &[(&str, String, Option<&Path>)]) -> PathBuf {
    use rusty_bbp_host::golden;
    let g = dir.join("golden").join("flag");
    std::fs::create_dir_all(g.join("cases/says-ok")).unwrap();
    std::fs::write(
        g.join("cases/says-ok/case.toml"),
        "cmd = \"/bin/cat\"\nargs = [\"flag.txt\"]\n[expect]\nexit = 0\nstdout = \"ok\\n\"\n",
    )
    .unwrap();
    for (name, toml, _) in extra {
        std::fs::create_dir_all(g.join("cases").join(name)).unwrap();
        std::fs::write(g.join("cases").join(name).join("case.toml"), toml).unwrap();
    }
    let mut files = golden::local_files(&g).unwrap();
    for fixture in extra.iter().filter_map(|(_, _, f)| *f) {
        let bytes = std::fs::read(fixture).unwrap();
        files.insert(fixture.to_str().unwrap().into(), Sha256::of(&bytes));
    }
    let text = golden::manifest_text(&files);
    let (_, set) = golden::parse_manifest(&text).unwrap();
    std::fs::write(g.join(golden::MANIFEST), text).unwrap();
    if sign {
        std::fs::write(
            g.join(golden::SIGNOFF),
            format!(
                "digest = \"{}\"\nsigner = \"nano\"\ndate = \"2026-11-03\"\n",
                set.hex()
            ),
        )
        .unwrap();
    }
    g
}

/// #597 section 3: an unsigned golden set cannot open a task (`bad_golden`);
/// a signed one becomes a `golden:` profile of the holdout suite, which the
/// runner executes with `bbp golden-run` in the candidate's tree.
#[test]
fn a_signed_golden_set_runs_as_a_holdout_profile() {
    let dir = tempdir("golden");
    let (repo, base) = repo(&dir);
    // Unsigned: the CLI refuses with bad_golden and opens nothing.
    let unsigned = golden_set(&dir.join("unsigned"), false);
    let files = tempdir("golden-cli");
    let roster = files.join("roster.json");
    std::fs::write(
        &roster,
        rusty_serde::json::to_string(&vec![lens("a", 0), holdout_lens()]).unwrap(),
    )
    .unwrap();
    let set = files.join("profiles.json");
    std::fs::write(&set, profiles().to_bytes().unwrap()).unwrap();
    let brief = files.join("brief.md");
    std::fs::write(&brief, "x").unwrap();
    let out = Proc::new(env!("CARGO_BIN_EXE_bbp"))
        .args(["open", "--mode", "claims", "--confine", "sandbox", "--dir"])
        .arg(&dir)
        .args(["--task", "C1", "--repo", "local", "--brief"])
        .arg(&brief)
        .arg("--roster")
        .arg(&roster)
        .arg("--profiles")
        .arg(&set)
        .arg("--golden")
        .arg(&unsigned)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("BadGolden"),
        "{text} {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!log_path_for(&dir, &task()).exists());

    // Signed: the golden set is the holdout suite.
    let g = golden_set(&dir, true);
    let hs = rusty_bbp_host::holdout::with_golden(None, &g, Path::new(env!("CARGO_BIN_EXE_bbp")))
        .expect("golden suite");
    assert_eq!(hs.set.profiles[0].name, "golden:flag");
    holdout_task(&dir, &hs);
    let c = claim(&dir, "a", "flag", true);
    let mut s = server(&dir, "a");
    let (text, err) = put_candidate(&mut s, "k", &base, FLAG_ONLY, c);
    assert!(!err, "{text}");
    let diffs = match &state(&dir).artifacts[&ArtId(id_of(&text))].payload {
        ArtifactPayload::Candidate(m) => m.diffs[0].0,
        _ => panic!("not a candidate"),
    };
    let work = dir.join("work");
    let exec = runner::Sandboxed::new(
        PathBuf::from(env!("CARGO_BIN_EXE_bbp")),
        &dir.join("sandbox-state"),
    );
    runner::run_once(&dir, &task(), &repo, &work, &exec, Confinement::Sandboxed).expect("build");
    let mut h = server(&dir, "h");
    let h1 = id_of(&ok(
        &mut h,
        "put_artifact",
        &format!(
            r#"{{"op":"g1","kind":"candidate","base":"{base}","diffs":[{diffs}],"claim":{c},"suite":"holdout"}}"#
        ),
    ));
    runner::run_once(&dir, &task(), &repo, &work, &exec, Confinement::Sandboxed).expect("golden");
    let d = open_driver(&dir, &task()).unwrap();
    let (id, r) = d
        .state
        .claims
        .runs
        .iter()
        .find(|(_, r)| r.candidate == ArtId(h1))
        .expect("golden run");
    let rec = &d.state.artifacts[&r.report.expect("report")];
    let ArtifactPayload::TestReport(rep) = &rec.payload else {
        panic!("not a report")
    };
    // Whatever the sandbox here did, the run reports under the suite's
    // digest; that it passes, confined, is the acceptance case below.
    assert_eq!(
        rep.profile_digest,
        state(&dir).claims.holdout_digest.unwrap()
    );
    assert!(rusty_bbp_host::holdout::log_path(&dir, &task(), *id).exists());
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&files);
}

/// #597 section 3, Linux acceptance lane: a signed golden set with a private
/// host-only fixture runs confined and passes. The fixture itself is
/// readable; an unsigned file beside it is not.
#[test]
#[ignore = "needs a working sandbox; run by the confined acceptance lane"]
fn confined_golden_run_passes_and_reads_only_signed_fixtures() {
    let dir = tempdir("golden_confined");
    runner::probe(&sandbox(&dir), &dir.join("probe")).expect("a working sandbox");
    let (repo, base) = repo(&dir);
    let private = dir.join("private");
    std::fs::create_dir_all(&private).unwrap();
    let fixture = private.join("input.txt");
    std::fs::write(&fixture, "signed input\n").unwrap();
    std::fs::write(private.join("sibling.txt"), "UNSIGNED-SIBLING\n").unwrap();
    let toml = format!(
        "cmd = \"/bin/sh\"\nargs = [\"-c\", \"cat {} && ! cat {}\"]\n[expect]\nexit = 0\nstdout = \"signed input\\n\"\n",
        fixture.display(),
        private.join("sibling.txt").display()
    );
    let g = golden_set_with(&dir, true, &[("private", toml, Some(&fixture))]);
    let hs = rusty_bbp_host::holdout::with_golden(None, &g, Path::new(env!("CARGO_BIN_EXE_bbp")))
        .expect("golden suite");
    assert!(hs
        .set
        .read_roots
        .contains(&fixture.to_str().unwrap().to_owned()));
    assert!(!hs
        .set
        .read_roots
        .contains(&private.to_str().unwrap().to_owned()));
    holdout_task(&dir, &hs);
    let work = dir.join("work");
    let exec = sandbox(&dir);
    let (_, h1, _) = build_and_holdout(&dir, &base);
    runner::run_once(&dir, &task(), &repo, &work, &exec, Confinement::Sandboxed).expect("build");
    runner::run_once(&dir, &task(), &repo, &work, &exec, Confinement::Sandboxed).expect("golden");
    let (run, rep, shown) = run_of(&dir, h1);
    let full = std::fs::read_to_string(rusty_bbp_host::holdout::log_path(&dir, &task(), run))
        .expect("host-only log");
    assert_eq!(rep.status, RunStatus::Passed, "{full}");
    assert!(
        full.contains("pass says-ok") && full.contains("pass private"),
        "{full}"
    );
    assert!(!full.contains("UNSIGNED-SIBLING"), "{full}");
    assert!(shown.contains("golden:flag: exit 0"), "{shown}");
    println!("BBP_ACCEPTANCE PASS golden-confined");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `bbp why --claim` (#599): the claim's replay as JSON, `--at` cuts it, and
/// an unknown claim or a bad flag pair is refused by name.
#[test]
fn why_replays_one_claim_and_names_the_rule() {
    let dir = tempdir("why");
    claims_task(&dir);
    let c = claim(&dir, "a", "flag", true);
    cited(&dir, "b", c, 3);
    cited(&dir, "a", c, 3);
    let d = dir.to_str().unwrap();
    let why = |extra: &[&str]| {
        let out = Proc::new(env!("CARGO_BIN_EXE_bbp"))
            .args(["why", "--dir", d, "--task", "C1"])
            .args(extra)
            .output()
            .expect("run bbp");
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let id = c.to_string();
    let (ok, out, err) = why(&["--claim", &id]);
    assert!(ok, "{err}");
    let v: Value = rusty_serde::json::from_str(&out).unwrap();
    let steps = match &v["steps"] {
        Value::Seq(s) => s.clone(),
        other => panic!("no steps: {other:?}"),
    };
    assert_eq!(steps.len(), 4, "{out}");
    assert_eq!(steps[0]["change"].as_str(), Some("Posted"), "{out}");
    assert!(
        out.contains(r#""AcceptLine":{"accept_at":8,"support_authors":2}"#),
        "{out}"
    );
    assert_eq!(steps[3]["status"].as_str(), Some("Accepted"), "{out}");

    let at = steps[1]["rev"].as_u64().unwrap().to_string();
    let (ok, cut, err) = why(&["--claim", &format!("msg:{id}"), "--at", &at]);
    assert!(ok, "{err}");
    assert_eq!(cut.matches("\"rev\":").count(), 2, "{cut}");

    let (ok, _, err) = why(&["--claim", "999"]);
    assert!(!ok && err.contains("no claim msg:999"), "{err}");
    let (ok, _, err) = why(&["--claim", &id, "--turn", "1"]);
    assert!(!ok && err.contains("exclusive"), "{err}");
    let (ok, _, err) = why(&["--at", "3"]);
    assert!(!ok && err.contains("--at needs --claim"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}
