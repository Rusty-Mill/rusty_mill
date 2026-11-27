//! The `BbpCli` adapter against a stand-in `bbp` that logs its argv and
//! answers from canned JSON: the exact host commands each action issues,
//! the fence chaining, and a refusal stopping the sequence.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use rusty_mill_board::bbp_cli::BbpCli;
use rusty_mill_board::inbox::{Action, Escalations, InboxError, Outcome, Reason, Response};

const INBOX: &str = r#"[{"id":"C1/msg:1/14","task":"C1","rev":16,"seq":14,"reason":"Budget","claim":{"id":1,"text":"ship it","domains":["d"],"status":"Escalated","score":4,"confidence":70,"epoch":0,"spent":8,"budget":8,"evidence":[],"deps":[],"candidates":[],"sole_candidate":false,"current":null,"failing":[],"tests_changed":[],"endorsement":null},"stakes":"Low","reversible":true,"evidence":[],"candidates":[],"mark":null,"candidate":null}]"#;

/// Held by each test for its whole run. Writing an executable while another
/// thread forks can leak the write fd into the child, and exec then fails
/// with ETXTBSY; one test at a time avoids it.
static SERIAL: Mutex<()> = Mutex::new(());

/// A fake `bbp` in a fresh dir. Every call appends its argv to `calls`.
/// A dry run previews two events; a commit answers `Ok`, unless its
/// fence is listed in `stale`, then it answers `StaleRev`.
fn fake(tag: &str, stale: &[u64]) -> (PathBuf, PathBuf, MutexGuard<'static, ()>) {
    let serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    let dir = std::env::temp_dir().join(format!("mill_inbox_{}_{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let calls = dir.join("calls");
    let script = format!(
        r#"#!/bin/sh
echo "$@" >> '{calls}'
case "$1" in inbox) cat <<'J'
{INBOX}
J
exit 0;; esac
case "$1" in claims) cat '{claims}'; exit $?;; esac
case "$1" in decision) cat '{answer}'; exit $?;; esac
line="$*"
case "$line" in *"--dry-run yes"*)
  echo '{{"rev":0,"response":"Ok","events":[{{"A":1}},{{"B":2}}]}}'; exit 0;; esac
for f in {fences}; do case "$line" in *"--fence $f "*) echo '{{"Rejected":{{"code":"StaleRev","detail":"human view changed"}}}}'; exit 0;; esac; done
echo '"Ok"'
"#,
        calls = calls.display(),
        claims = dir.join("claims.json").display(),
        answer = dir.join("decision.json").display(),
        fences = stale
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(" "),
    );
    let bin = dir.join("bbp");
    fs::write(&bin, script).unwrap();
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
    (bin, calls, serial)
}

fn calls(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn open_reads_the_inbox_of_the_given_tasks() {
    let (bin, log, _serial) = fake("open", &[]);
    let cli = BbpCli::new(bin, "/srv/bbp".into());
    let rows = cli.open(&["C1".into(), "C2".into()]).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].reason, Reason::Budget);
    assert_eq!(
        calls(&log),
        ["inbox --dir /srv/bbp --task C1,C2 --json yes"]
    );
}

#[test]
fn no_tasks_runs_nothing_and_a_bad_task_id_is_refused() {
    let (bin, log, _serial) = fake("none", &[]);
    let cli = BbpCli::new(bin, "/srv/bbp".into());
    assert!(cli.open(&[]).unwrap().is_empty());
    assert!(matches!(
        cli.open(&["C1,C2".into()]),
        Err(InboxError::Invalid(_))
    ));
    assert!(calls(&log).is_empty());
}

#[test]
fn approve_on_budget_extends_then_reopens_with_chained_fences() {
    let (bin, log, _serial) = fake("chain", &[]);
    let cli = BbpCli::new(bin, "/srv/bbp".into());
    let e = cli.open(&["C1".into()]).unwrap().remove(0);
    assert_eq!(
        cli.act(&e, &Action::Approve, "nano", None).unwrap(),
        Outcome::Done
    );
    let human: Vec<String> = calls(&log).into_iter().skip(1).collect();
    let base = "human --dir /srv/bbp --task C1";
    assert_eq!(
        human,
        [
            format!("{base} --rev 16 --fence 16 --as nano --dry-run yes extend-claim msg:1"),
            format!("{base} --rev 16 --fence 16 --as nano extend-claim msg:1"),
            format!("{base} --rev 18 --fence 18 --as nano --dry-run yes claim msg:1 reopen"),
            format!("{base} --rev 18 --fence 18 --as nano claim msg:1 reopen"),
        ],
        "the second command is fenced on the first's rev plus its two events"
    );
}

#[test]
fn a_stale_commit_stops_the_sequence_and_says_where() {
    let (bin, log, _serial) = fake("stale", &[18]);
    let cli = BbpCli::new(bin, "/srv/bbp".into());
    let e = cli.open(&["C1".into()]).unwrap().remove(0);
    let out = cli.act(&e, &Action::Approve, "nano", None).unwrap();
    assert_eq!(
        out,
        Outcome::Refused {
            step: 1,
            code: "StaleRev".into(),
            detail: "human view changed".into()
        }
    );
    assert_eq!(calls(&log).len(), 1 + 4, "nothing after the refusal");
}

#[test]
fn preview_dry_runs_only_the_first_command_and_lists_the_rest() {
    let (bin, log, _serial) = fake("preview", &[]);
    let cli = BbpCli::new(bin, "/srv/bbp".into());
    let e = cli.open(&["C1".into()]).unwrap().remove(0);
    let dry = cli.preview(&e, &Action::Approve).unwrap();
    assert_eq!(dry.step.verb, "extend-claim");
    assert!(matches!(dry.response, Response::Accepted(_)));
    assert_eq!(dry.events.len(), 2);
    assert_eq!(dry.then.len(), 1);
    assert!(calls(&log).last().unwrap().contains("--dry-run yes"));
    assert!(calls(&log).iter().all(|c| !c.contains("reopen")));
}

#[test]
fn an_invalid_action_sends_nothing() {
    let (bin, log, _serial) = fake("invalid", &[]);
    let cli = BbpCli::new(bin, "/srv/bbp".into());
    let e = cli.open(&["C1".into()]).unwrap().remove(0);
    let n = calls(&log).len();
    assert!(
        cli.act(&e, &Action::Choose { chosen: 9 }, "nano", None)
            .is_err()
    );
    assert!(
        cli.act(
            &e,
            &Action::Refine {
                note: String::new()
            },
            "nano",
            None
        )
        .is_err()
    );
    assert_eq!(calls(&log).len(), n);
}

#[test]
fn a_failing_host_is_an_error_not_an_outcome() {
    let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    let cli = BbpCli::new("/nonexistent/bbp".into(), "/srv/bbp".into());
    assert!(matches!(cli.open(&["C1".into()]), Err(InboxError::Host(_))));
}

fn answers(bin: &Path, json: &str) {
    fs::write(bin.with_file_name("decision.json"), json).unwrap();
}

#[test]
fn a_decision_id_rides_a_single_command_and_is_looked_up_by_it() {
    let (bin, log, _serial) = fake("decision", &[]);
    let cli = BbpCli::new(bin.clone(), "/srv/bbp".into());
    let mut e = cli.open(&["C1".into()]).unwrap().remove(0);
    e.acted = Some(vec![]); // a host that reports acted-on work
    // Approve on a budget is two commands: a repeat would replay the first without events
    // and fence the second wrongly, so an id is refused before anything is sent.
    let two = cli.act(&e, &Action::Approve, "nano", Some("dec-1"));
    assert!(matches!(two, Err(InboxError::Invalid(_))), "{two:?}");
    assert_eq!(calls(&log).len(), 1, "only the inbox read ran");

    let drop = Action::Reject { reason: None };
    assert_eq!(
        cli.act(&e, &drop, "nano", Some("dec-1")).unwrap(),
        Outcome::Done
    );
    let sent = calls(&log);
    for n in [1, 2] {
        assert!(sent[n].contains("--decision dec-1.0"), "{sent:?}");
    }

    answers(&bin, r#"[{"decision":"dec-1.0","applied":true,"rev":19}]"#);
    assert_eq!(cli.applied("C1", "dec-1").unwrap(), Some(19));
    assert_eq!(
        calls(&log).last().unwrap(),
        "decision --dir /srv/bbp --task C1 --decision dec-1.0"
    );
    answers(
        &bin,
        r#"[{"decision":"dec-1.0","applied":false,"rev":null}]"#,
    );
    assert_eq!(cli.applied("C1", "dec-1").unwrap(), None);
}

/// Absence is evidence only from a host that answered for exactly this id: an empty
/// answer, another id's row, two rows, an inconsistent row, or a host without the verb
/// are errors, so nothing is ever settled from them.
#[test]
fn a_lookup_that_does_not_answer_for_its_id_is_an_error() {
    let (bin, _log, _serial) = fake("decision-bad", &[]);
    let cli = BbpCli::new(bin.clone(), "/srv/bbp".into());
    for bad in [
        "[]",
        r#"[{"decision":"other.0","applied":false,"rev":null}]"#,
        r#"[{"decision":"dec-1.0","applied":false,"rev":null},{"decision":"dec-1.0","applied":false,"rev":null}]"#,
        r#"[{"decision":"dec-1.0","applied":true,"rev":null}]"#,
        r#"[{"decision":"dec-1.0","applied":false,"rev":5}]"#,
        r#"{"decision":"dec-1.0"}"#,
    ] {
        answers(&bin, bad);
        assert!(
            matches!(cli.applied("C1", "dec-1"), Err(InboxError::Host(_))),
            "{bad}"
        );
    }
    // An old host has no `decision` verb: its answer is an error exit.
    fs::remove_file(bin.with_file_name("decision.json")).unwrap();
    assert!(matches!(
        cli.applied("C1", "dec-1"),
        Err(InboxError::Host(_))
    ));
}

#[test]
fn claims_reads_the_host_array_with_the_exact_argv() {
    let (bin, log, _serial) = fake("claims", &[]);
    fs::write(
        bin.with_file_name("claims.json"),
        r#"[{"id":1,"spent":2,"budget":9}]"#,
    )
    .unwrap();
    let cli = BbpCli::new(bin, "/srv/bbp".into());
    let v = cli.claims("C1").unwrap();
    assert_eq!(v[0]["budget"], 9);
    assert_eq!(calls(&log), ["claims --dir /srv/bbp --task C1 --json yes"]);
    assert!(matches!(cli.claims("--x"), Err(InboxError::Invalid(_))));
    assert_eq!(calls(&log).len(), 1, "a bad id runs nothing");
}

#[test]
fn an_oversized_claims_answer_is_refused_not_buffered() {
    let (bin, _log, _serial) = fake("big", &[]);
    // Valid JSON, 9 MiB: over the 8 MiB ceiling.
    let big = format!("[\"{}\"]", "x".repeat(9 << 20));
    fs::write(bin.with_file_name("claims.json"), big).unwrap();
    let cli = BbpCli::new(bin, "/srv/bbp".into());
    let Err(InboxError::Host(why)) = cli.claims("C1") else {
        panic!("expected a refusal")
    };
    assert!(why.contains("over") && why.contains("refused"), "{why}");
}

#[test]
fn a_failing_claims_host_reports_its_stderr() {
    let (bin, _log, _serial) = fake("fail", &[]);
    // No claims.json: the stand-in's `cat` fails with a message on stderr.
    let cli = BbpCli::new(bin, "/srv/bbp".into());
    assert!(matches!(cli.claims("C1"), Err(InboxError::Host(m)) if m.contains("claims.json")));
}

/// A stand-in `bbp` whose `verify` runs `body` (shell), in a fresh dir.
fn verifier(tag: &str, body: &str) -> (PathBuf, MutexGuard<'static, ()>) {
    let serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    let dir = std::env::temp_dir().join(format!("mill_verify_{}_{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let bin = dir.join("bbp");
    fs::write(&bin, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
    (bin, serial)
}

const HEAD: &str =
    r#"echo '{"lines":3,"unchained":0,"head":"ab","partial":false,"expect_found":null}'"#;

/// A stand-in whose `verify` for task C1 runs `stuck`, and for any other task answers a head.
fn stuck_on_c1(tag: &str, stuck: &str) -> (BbpCli, MutexGuard<'static, ()>) {
    let (bin, serial) = verifier(
        tag,
        &format!("case \"$*\" in *'--task C1'*) {stuck};; esac\n{HEAD}"),
    );
    let cli = BbpCli::new(bin, "/srv/bbp".into())
        .with_verify_deadline(std::time::Duration::from_millis(200));
    (cli, serial)
}

/// `C1` fails fast as a host error, never a refusal, and `C2` still verifies right after.
fn assert_bounded_then_next_task_runs(cli: &BbpCli, expect: &str) {
    let started = std::time::Instant::now();
    match cli.verify("C1", None) {
        Err(InboxError::Host(m)) => assert!(m.contains(expect), "{m}"),
        other => panic!("a stuck host is operational, not a refusal: {other:?}"),
    }
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "it did not wait"
    );
    let next = cli.verify("C2", None).unwrap().unwrap();
    assert_eq!(next.head, "ab");
}

#[test]
fn a_stuck_verify_is_killed_at_its_deadline_and_the_next_task_still_verifies() {
    let (cli, _serial) = stuck_on_c1("stuck", "exec sleep 30");
    assert_bounded_then_next_task_runs(&cli, "timed out");
}

#[test]
fn a_grandchild_holding_the_output_past_the_deadline_blocks_nothing() {
    // As under the guard's sudo: killing the direct child leaves a process on the output.
    let (cli, _serial) = stuck_on_c1("grandchild", "sleep 3 & wait");
    assert_bounded_then_next_task_runs(&cli, "timed out");
}

#[test]
fn a_wrapper_that_exits_while_a_grandchild_holds_stdout_is_bounded_by_the_deadline() {
    // A clean exit is not EOF: the background `sleep` keeps stdout open past the deadline.
    let (cli, _serial) = stuck_on_c1("held", "sleep 3 & exit 0");
    assert_bounded_then_next_task_runs(&cli, "timed out");
}

#[test]
fn endless_output_is_refused_at_the_cap_without_storing_it() {
    let (cli, _serial) = stuck_on_c1("flood", "exec yes");
    assert_bounded_then_next_task_runs(&cli, "over");
}

#[test]
fn a_flood_on_stderr_is_drained_and_capped_and_never_stalls_the_answer() {
    let (bin, _serial) = verifier(
        "errflood",
        &format!("head -c 1000000 /dev/zero >&2\n{HEAD}"),
    );
    let cli =
        BbpCli::new(bin, "/srv/bbp".into()).with_verify_deadline(std::time::Duration::from_secs(5));
    assert_eq!(cli.verify("C2", None).unwrap().unwrap().head, "ab");
}

#[test]
fn verify_reads_a_head_and_records_a_refusal() {
    let (bin, _serial) = verifier(
        "answers",
        r#"case "$*" in *--expect*) echo 'bbp: expected head not found' >&2; exit 1;; esac
echo '{"lines":3,"unchained":0,"head":"ab","partial":false,"expect_found":null}'"#,
    );
    let cli = BbpCli::new(bin, "/srv/bbp".into());
    let ok = cli.verify("C1", None).unwrap().unwrap();
    assert_eq!((ok.lines, ok.unchained, ok.head.as_str()), (3, 0, "ab"));
    let refused = cli
        .verify("C1", Some(&"0".repeat(64)))
        .unwrap()
        .unwrap_err();
    assert_eq!(refused, "expected head not found");
}

#[test]
fn a_stderr_stream_that_never_runs_dry_still_meets_the_deadline() {
    let (cli, _serial) = stuck_on_c1("errendless", "exec yes >&2");
    assert_bounded_then_next_task_runs(&cli, "timed out");
}

#[test]
fn endless_stdout_beside_endless_stderr_is_refused_at_the_cap() {
    let (cli, _serial) = stuck_on_c1("bothendless", "yes >&2 & exec yes");
    assert_bounded_then_next_task_runs(&cli, "over");
}
