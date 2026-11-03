//! The `BbpCli` adapter against a stand-in `bbp` that logs its argv and
//! answers from canned JSON: the exact host commands each action issues,
//! the fence chaining, and a refusal stopping the sequence.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use rusty_mill_board::bbp_cli::BbpCli;
use rusty_mill_board::inbox::{Action, Escalations, InboxError, Outcome, Reason, Response};

const INBOX: &str = r#"[{"id":"C1/msg:1/14","task":"C1","rev":16,"seq":14,"reason":"Budget","claim":{"id":1,"text":"ship it","domains":["d"],"status":"Escalated","score":4,"confidence":70,"epoch":0,"spent":8,"budget":8,"evidence":[],"deps":[],"candidates":[],"sole_candidate":false,"current":null,"failing":[],"tests_changed":[],"endorsement":null},"stakes":"Low","reversible":true,"evidence":[],"candidates":[],"mark":null,"candidate":null}]"#;

/// A fake `bbp` in a fresh dir. Every call appends its argv to `calls`.
/// A dry run previews two events; a commit answers `Ok`, unless its
/// fence is listed in `stale`, then it answers `StaleRev`.
fn fake(tag: &str, stale: &[u64]) -> (PathBuf, PathBuf) {
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
line="$*"
case "$line" in *"--dry-run yes"*)
  echo '{{"rev":0,"response":"Ok","events":[{{"A":1}},{{"B":2}}]}}'; exit 0;; esac
for f in {fences}; do case "$line" in *"--fence $f "*) echo '{{"Rejected":{{"code":"StaleRev","detail":"human view changed"}}}}'; exit 0;; esac; done
echo '"Ok"'
"#,
        calls = calls.display(),
        fences = stale
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(" "),
    );
    let bin = dir.join("bbp");
    fs::write(&bin, script).unwrap();
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
    (bin, calls)
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
    let (bin, log) = fake("open", &[]);
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
    let (bin, log) = fake("none", &[]);
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
    let (bin, log) = fake("chain", &[]);
    let cli = BbpCli::new(bin, "/srv/bbp".into());
    let e = cli.open(&["C1".into()]).unwrap().remove(0);
    assert_eq!(
        cli.act(&e, &Action::Approve, "nano").unwrap(),
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
    let (bin, log) = fake("stale", &[18]);
    let cli = BbpCli::new(bin, "/srv/bbp".into());
    let e = cli.open(&["C1".into()]).unwrap().remove(0);
    let out = cli.act(&e, &Action::Approve, "nano").unwrap();
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
    let (bin, log) = fake("preview", &[]);
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
    let (bin, log) = fake("invalid", &[]);
    let cli = BbpCli::new(bin, "/srv/bbp".into());
    let e = cli.open(&["C1".into()]).unwrap().remove(0);
    let n = calls(&log).len();
    assert!(cli.act(&e, &Action::Choose { chosen: 9 }, "nano").is_err());
    assert!(
        cli.act(
            &e,
            &Action::Refine {
                note: String::new()
            },
            "nano"
        )
        .is_err()
    );
    assert_eq!(calls(&log).len(), n);
}

#[test]
fn a_failing_host_is_an_error_not_an_outcome() {
    let cli = BbpCli::new("/nonexistent/bbp".into(), "/srv/bbp".into());
    assert!(matches!(cli.open(&["C1".into()]), Err(InboxError::Host(_))));
}
