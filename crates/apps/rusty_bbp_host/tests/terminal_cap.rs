#![allow(clippy::expect_used, clippy::unwrap_used)]
//! The cost cap at the end of a run, for a task that reached `closed`: the
//! final usage is checked before the run counts as a success. (`cancelled` runs
//! through the whole moderator in `moderator_e2e.rs`; `closed` needs the full
//! pipeline, so it is reached here with the core's fixture and checked directly.)

#[path = "../../../libs/protocol/rusty_bbp/tests/common/mod.rs"]
mod core_fx;

use core_fx::*;
use rusty_bbp::*;
use rusty_bbp_host::moderator::{self, Config, CostCap, Launchers};
use rusty_bbp_host::runner::Confinement;
use rusty_bbp_host::tokens::{Criteria, Prices};
use std::path::{Path, PathBuf};
use std::time::Duration;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bbp_host_terminal_{}_{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn cap(usd: f64) -> CostCap {
    CostCap {
        usd,
        criteria: Criteria {
            prices_as_of: "test".into(),
            models: [(
                "priced-model".to_owned(),
                Prices {
                    input: 1.0,
                    output: 20.0, // 1M output tokens is $20
                    cache_creation_5m: 1.0,
                    cache_creation_1h: 1.0,
                    cache_read: 1.0,
                },
            )]
            .into(),
            min_commit: None,
        },
        criteria_sha256: "test".into(),
    }
}

fn config(dir: &Path, task: &TaskId, cost_cap: Option<CostCap>) -> Config {
    Config {
        dir: dir.to_path_buf(),
        task: task.clone(),
        bbp: PathBuf::from("bbp"),
        repo: dir.to_path_buf(),
        work: dir.join("work"),
        launchers: Launchers::default(),
        confinement: Confinement::Unconfined,
        poll: Duration::from_millis(50),
        max_wall: Duration::from_secs(1),
        end_grace: Duration::ZERO,
        deadline: None,
        cost_cap,
        admission: None,
    }
}

/// [`closed_task_bare`], and every later turn's harness printed a completed
/// turn that cost a millionth of a dollar: a finished run has a log for each
/// turn it granted.
fn closed_task(dir: &Path, model: &str) -> Fx {
    let fx = closed_task_bare(dir, model);
    for e in fx.d.store.events(&fx.st().task, 0).unwrap() {
        if let Event::TurnGranted { turn, .. } = e {
            if turn != TurnId(1) {
                let log = moderator::log_path(dir, &fx.st().task, turn);
                std::fs::write(log, format!("{SMALL_RESULT}\n")).unwrap();
            }
        }
    }
    fx
}

/// A task that reached `closed`, whose first turn printed one assistant message
/// of 1M output tokens ($20 at the priced model) of `model`, and whose later
/// turns left no log.
fn closed_task_bare(dir: &Path, model: &str) -> Fx {
    let mut fx = Fx::new();
    let cand = fx.reach_approved();
    ok(fx.human(HumanAction::MergeReceipt {
        candidate: cand,
        revision: "a".repeat(40),
    }));
    assert_eq!(fx.st().state, State::Closed);
    let line = format!(
        r#"{{"type":"assistant","message":{{"id":"m1","model":"{model}","usage":{{"input_tokens":0,"output_tokens":1000000}}}}}}"#
    );
    let log = moderator::log_path(dir, &fx.st().task, TurnId(1));
    std::fs::create_dir_all(log.parent().unwrap()).unwrap();
    std::fs::write(log, format!("{line}\n")).unwrap();
    fx
}

#[test]
fn a_closed_task_with_final_output_over_the_cap_is_not_a_success() {
    let dir = tempdir("over");
    let fx = closed_task(&dir, "priced-model");
    let cfg = config(&dir, &fx.st().task, Some(cap(1.0)));
    let err = moderator::check_terminal(&cfg, &fx.d).unwrap_err();
    assert!(err.contains("cost cap reached"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_closed_task_with_final_output_of_an_unpriced_model_is_not_a_success() {
    let dir = tempdir("unpriced");
    let fx = closed_task(&dir, "unpriced-model");
    let cfg = config(&dir, &fx.st().task, Some(cap(1000.0)));
    let err = moderator::check_terminal(&cfg, &fx.d).unwrap_err();
    assert!(
        err.contains("cannot enforce the cost cap") && err.contains("unpriced-model"),
        "{err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_closed_task_under_the_cap_or_with_no_cap_succeeds() {
    let dir = tempdir("under");
    let fx = closed_task(&dir, "priced-model");
    let under = config(&dir, &fx.st().task, Some(cap(1000.0)));
    assert_eq!(moderator::check_terminal(&under, &fx.d), Ok(()));
    // At the total ($20 and a millionth for each of the three later turns) is
    // not over it; a hair below is.
    let total = 20.0 + 3e-6;
    let below = config(&dir, &fx.st().task, Some(cap(total - 1e-7)));
    assert!(moderator::check_terminal(&below, &fx.d).is_err());
    let at = config(&dir, &fx.st().task, Some(cap(total + 1e-9)));
    assert_eq!(moderator::check_terminal(&at, &fx.d), Ok(()));
    let none = config(&dir, &fx.st().task, None);
    assert_eq!(moderator::check_terminal(&none, &fx.d), Ok(()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_task_that_is_not_terminal_is_left_to_the_poll() {
    let dir = tempdir("live");
    let mut fx = Fx::new();
    fx.reach_approved();
    assert_eq!(fx.st().state, State::Approved);
    let line = r#"{"type":"assistant","message":{"id":"m1","model":"priced-model","usage":{"input_tokens":0,"output_tokens":1000000}}}"#;
    let log = moderator::log_path(&dir, &fx.st().task, TurnId(1));
    std::fs::create_dir_all(log.parent().unwrap()).unwrap();
    std::fs::write(log, format!("{line}\n")).unwrap();
    let cfg = config(&dir, &fx.st().task, Some(cap(1.0)));
    assert_eq!(moderator::check_terminal(&cfg, &fx.d), Ok(()));
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- second review, finding 5: the cap fails closed on output it cannot read

/// A closed task whose first turn's log is exactly `log`.
fn closed_task_with_log(dir: &Path, log: &str) -> Fx {
    let fx = closed_task(dir, "priced-model");
    std::fs::write(moderator::log_path(dir, &fx.st().task, TurnId(1)), log).unwrap();
    fx
}

fn terminal(tag: &str, log: &str) -> Result<(), String> {
    let dir = tempdir(tag);
    let fx = closed_task_with_log(&dir, log);
    let cfg = config(&dir, &fx.st().task, Some(cap(1.0)));
    let r = moderator::check_terminal(&cfg, &fx.d);
    let _ = std::fs::remove_dir_all(&dir);
    r
}

const TEN_DOLLARS: &str = r#"{"type":"assistant","message":{"id":"m1","model":"priced-model","usage":{"input_tokens":0,"output_tokens":1000000}}}"#;
/// A completed turn that cost one millionth of a dollar.
const SMALL_RESULT: &str = r#"{"type":"result","is_error":false,"terminal_reason":"completed","total_cost_usd":0.000001,"modelUsage":{"priced-model":{"inputTokens":1,"outputTokens":0,"cacheReadInputTokens":0,"cacheCreationInputTokens":0}}}"#;

#[test]
fn a_log_with_no_stream_json_cannot_be_checked_against_the_cap() {
    for (what, log) in [("plain text", "just some words\n"), ("empty", "")] {
        let err = terminal(what, log).unwrap_err();
        assert!(err.contains("cannot enforce the cost cap"), "{what}: {err}");
    }
}

#[test]
fn an_assistant_message_after_the_last_result_still_counts() {
    let log = format!("{SMALL_RESULT}\n{TEN_DOLLARS}\n");
    let err = terminal("after-result", &log).unwrap_err();
    assert!(err.contains("cost cap reached"), "{err}");
}

#[test]
fn a_usage_line_that_does_not_parse_is_not_skipped() {
    let broken = TEN_DOLLARS.replace(
        r#""output_tokens":1000000"#,
        r#""output_tokens":1000000,"cache_creation":null"#,
    );
    let err = terminal("null-split", &format!("{broken}\n")).unwrap_err();
    assert!(err.contains("cannot enforce the cost cap"), "{err}");
    // Noise around good lines (a harness's stderr is in the same log) is fine.
    let noisy = format!("warning: slow\n{SMALL_RESULT}\n");
    assert_eq!(terminal("noisy", &noisy), Ok(()));
}

// ---- review comments on #763 (baileyrd, fa4a8a8a): read failures are not "no output"

#[test]
fn a_log_that_cannot_be_read_is_not_skipped() {
    // Invalid UTF-8 in a log, and a directory where the log should be.
    let dir = tempdir("bad-utf8");
    let fx = closed_task(&dir, "priced-model");
    let log = moderator::log_path(&dir, &fx.st().task, TurnId(1));
    std::fs::write(&log, [b'{', 0xff, 0xfe, b'\n']).unwrap();
    let cfg = config(&dir, &fx.st().task, Some(cap(1000.0)));
    let err = moderator::check_terminal(&cfg, &fx.d).unwrap_err();
    assert!(err.contains("cannot enforce the cost cap"), "utf-8: {err}");
    std::fs::remove_file(&log).unwrap();
    std::fs::create_dir(&log).unwrap();
    let err = moderator::check_terminal(&cfg, &fx.d).unwrap_err();
    assert!(
        err.contains("cannot enforce the cost cap"),
        "directory: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_granted_turn_with_no_log_at_the_end_cannot_be_checked_against_the_cap() {
    let dir = tempdir("missing");
    let fx = closed_task_bare(&dir, "priced-model");
    // `closed_task` logs turn 1 only; the task has later granted turns.
    let granted =
        fx.d.store
            .events(&fx.st().task, 0)
            .unwrap()
            .iter()
            .filter(|e| matches!(e, Event::TurnGranted { .. }))
            .count();
    assert!(granted > 1, "the fixture has later turns");
    let cfg = config(&dir, &fx.st().task, Some(cap(1000.0)));
    let err = moderator::check_terminal(&cfg, &fx.d).unwrap_err();
    assert!(
        err.contains("cannot enforce the cost cap") && err.contains("no log"),
        "{err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- third review, finding 1

/// The lines a real harness prints before its first message (captured).
fn preamble() -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/stream-json/system-lines-before-the-first-message.jsonl"),
    )
    .unwrap()
}

#[test]
fn a_log_with_only_a_preamble_at_the_end_cannot_be_checked_against_the_cap() {
    let err = terminal("preamble", &preamble()).unwrap_err();
    assert!(err.contains("cannot enforce the cost cap"), "{err}");
}

#[test]
fn valid_result_does_not_hide_malformed_complete_record_or_terminal_tail() {
    for (tag, tail) in [
        ("truncated", "{\"type\":\"assistant\","),
        ("malformed", "{\"type\":\"assistant\",bad}\n"),
    ] {
        assert!(terminal(tag, &format!("{SMALL_RESULT}\n{tail}"))
            .unwrap_err()
            .contains("cannot enforce"));
    }
}

#[test]
fn saved_wall_stop_remains_failed_after_task_closes_and_preserves_mixed_costs() {
    use rusty_bbp_host::{pairs, tokens};
    let dir = tempdir("wall-closed-mixed");
    let fx = closed_task(&dir, "priced-model");
    let task = fx.st().task.clone();
    let events = fx.d.store.events(&task, 0).unwrap();
    rusty_bbp_host::master::ensure(&dir).unwrap();
    let mut disk = FsStore::open(&dir).unwrap();
    disk.append(&task, Rev(0), &events).unwrap();
    let first = moderator::log_path(&dir, &task, TurnId(1));
    std::fs::write(&first, r#"{"type":"result","is_error":false,"terminal_reason":"completed","total_cost_usd":10,"modelUsage":{"priced-model":{"inputTokens":10000000}}}"#).unwrap();
    let second = events
        .iter()
        .filter_map(|e| match e {
            Event::TurnGranted { turn, .. } if *turn != TurnId(1) => Some(*turn),
            _ => None,
        })
        .next()
        .unwrap();
    std::fs::write(moderator::log_path(&dir, &task, second), [0xff]).unwrap();
    let out = dir.join("mod.json");
    std::fs::write(
        &out,
        format!(
            r#"{{"state_ms":[],"stop":"wall","bbp_build_sha":"{}"}}"#,
            "a".repeat(40)
        ),
    )
    .unwrap();
    moderator::write_evidence_marker(&dir, &task, 1000.0, "test").unwrap();
    let f = pairs::facts(
        &pairs::RunRef {
            dir: dir.display().to_string(),
            task: task.0.clone(),
            mod_output: out.display().to_string(),
            rubric_ok: true,
        },
        &cap(1000.0).criteria,
        "test",
        1000.0,
    )
    .unwrap();
    assert!(f.closed && f.stopped && !f.over_cap);
    assert!(!f.succeeded());
    assert_eq!(f.killed_turns, 1);
    assert_eq!(f.accounting_errors.len(), 1);
    assert!(f.usd_floor.unwrap() >= 10.0);
    assert_eq!(f.tokens, None);
    let partial = tokens::of_stopped(&dir, &task, &events, &cap(1000.0).criteria);
    assert_eq!(f.usd_floor, partial.usd_floor);
    for stop in ["", "unknown", "Wall"] {
        assert!(pairs::mod_facts(&format!(r#"{{"state_ms":[],"stop":"{stop}"}}"#)).is_err());
    }
    let _ = std::fs::remove_dir_all(dir);
}
