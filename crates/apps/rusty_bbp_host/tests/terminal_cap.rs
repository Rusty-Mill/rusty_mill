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
        cost_cap,
    }
}

/// A task that reached `closed`, whose first turn printed one assistant message
/// of 1M output tokens ($20 at the priced model) of `model`.
fn closed_task(dir: &Path, model: &str) -> Fx {
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
    // Exactly at the cap is not over it.
    let at = config(&dir, &fx.st().task, Some(cap(20.0)));
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
