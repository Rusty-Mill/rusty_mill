#![allow(clippy::expect_used, clippy::unwrap_used)]
//! `bbp why`: the reason for each grant, replayed from the log.
//!
//! The fixtures are the logs of three real `bbp mod` runs on the proving task
//! (scripted by nobody: Claude Code harnesses as the four roles): two
//! event-driven (`run_a`, `run_b`) and one turn-based with its shadow log. The
//! replay must reproduce the picks they made. Explicit grants and the pick
//! order are exercised on logs the real engine writes, through the core's own
//! test fixture. The pick rule itself is the core's `pick_from`, called by the
//! engine and by `bbp why`, so there is no second copy to keep in step.

#[path = "../../../libs/protocol/rusty_bbp/tests/common/mod.rs"]
mod core_fx;

use core_fx::*;
use rusty_bbp::*;
use rusty_bbp_host::why::{self, Cause, How, Woken};
use rusty_bbp_host::{master, shadow};
use std::path::{Path, PathBuf};
use std::process::Command as Proc;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/why")
        .join(name)
}

fn events(name: &str) -> Vec<Event> {
    let text = std::fs::read_to_string(fixture(name)).expect("fixture");
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .flat_map(|l| rusty_serde::json::from_str::<Vec<Event>>(l).expect("a log line"))
        .collect()
}

fn logged(name: &str) -> Vec<Shadow> {
    let text = std::fs::read_to_string(fixture(name)).expect("fixture");
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| rusty_serde::json::from_str::<Shadow>(l).expect("a shadow record"))
        .collect()
}

fn role(r: &str) -> RoleId {
    RoleId::from(r)
}

fn woken(role: &str, reason: WakeReason) -> Woken {
    Woken {
        role: RoleId::from(role),
        reason,
    }
}

fn roles_of(report: &why::Report) -> Vec<&str> {
    report.grants.iter().map(|g| g.role.0.as_str()).collect()
}

// ---------------------------------------------------------------- the real runs

fn check_event_driven_run(name: &str, task: &str) {
    let report = why::explain(&TaskId(task.into()), &events(name), &[]);
    assert_eq!(report.policy, SchedulingPolicy::EventDriven);
    assert_eq!(
        roles_of(&report),
        ["planner", "coder", "tester", "reviewer"],
        "the turns the run took"
    );
    let states: Vec<State> = report.grants.iter().map(|g| g.state).collect();
    assert_eq!(
        states,
        [State::Planning, State::Build, State::Test, State::Review]
    );
    // Every grant is the first selectable entry of the set, for State.
    for g in &report.grants {
        assert_eq!(g.how, How::WakeSet(WakeReason::State), "{g:?}");
        assert_eq!(g.kind, TurnKind::Default);
        assert!(g.shadow.is_none(), "no shadow on an event-driven task");
    }
    // The other entries: at `test` the three roles that do not serve it watch.
    let test = &report.grants[2];
    assert_eq!(
        test.wake_set.as_deref(),
        Some(
            &[
                woken("tester", WakeReason::State),
                woken("planner", WakeReason::Observer),
                woken("coder", WakeReason::Observer),
                woken("reviewer", WakeReason::Observer),
            ][..]
        )
    );
    assert_eq!(
        report.grants[0].wake_set.as_deref(),
        Some(&[woken("planner", WakeReason::State)][..])
    );
}

#[test]
fn run_a_replays_to_the_picks_it_made() {
    check_event_driven_run("run_a_event_driven.log", "slug-ed-1");
}

#[test]
fn run_b_replays_to_the_picks_it_made() {
    check_event_driven_run("run_b_event_driven.log", "slug-ed-2");
}

#[test]
fn a_turn_based_log_shows_the_shadow_comparison_beside_each_grant() {
    let report = why::explain(
        &TaskId("slug-1".into()),
        &events("run_turn_based.log"),
        &logged("run_turn_based.shadow.jsonl"),
    );
    assert_eq!(report.policy, SchedulingPolicy::TurnBased);
    assert_eq!(
        roles_of(&report),
        ["planner", "coder", "tester", "reviewer"]
    );
    for g in &report.grants {
        assert_eq!(g.how, How::DefaultHolder, "{g:?}");
        assert!(g.wake_set.is_none());
        let s = g.shadow.as_ref().expect("a shadow comparison");
        assert_eq!(s.comparison, "agree", "{g:?}");
        // The record `bbp shadow` kept for this very grant says the same.
        assert_eq!(s.logged_rules, Some(WAKE_RULES), "{g:?}");
        assert_eq!(s.logged_same, Some(true), "{g:?}");
        assert_eq!(s.would.first().map(|w| &w.role), Some(&g.role));
    }
    assert_eq!(report.grants[2].shadow.as_ref().unwrap().would.len(), 4);
}

#[test]
fn a_shadow_record_that_differs_from_the_replay_is_flagged() {
    let mut records = logged("run_turn_based.shadow.jsonl");
    if let ShadowKind::Auto { would, .. } = &mut records[0].kind {
        would.clear();
    }
    let report = why::explain(
        &TaskId("slug-1".into()),
        &events("run_turn_based.log"),
        &records,
    );
    let s = report.grants[0].shadow.as_ref().unwrap();
    assert_eq!(s.logged_same, Some(false));
    assert_eq!(
        report.grants[1].shadow.as_ref().unwrap().logged_same,
        Some(true)
    );
}

#[test]
fn a_grant_the_rule_does_not_reproduce_is_unexplained_not_a_match() {
    // The same log, but the first grant claims another role: no policy would
    // pick it at `planning`.
    let mut evs = events("run_a_event_driven.log");
    for e in &mut evs {
        if let Event::TurnGranted { role, turn, .. } = e {
            if turn.0 == 1 {
                *role = RoleId::from("reviewer");
            }
        }
    }
    let report = why::explain(&TaskId("slug-ed-1".into()), &evs, &[]);
    assert_eq!(
        report.grants[0].how,
        How::Unexplained {
            replayed: Some(role("planner"))
        }
    );
}

// ---------------------------------------------------------------- engine logs

fn log_of(fx: &Fx) -> Vec<Event> {
    fx.d.store.events(&fx.st().task, 0).expect("log")
}

fn hows(fx: &Fx) -> Vec<(String, How)> {
    why::explain(&fx.st().task, &log_of(fx), &[])
        .grants
        .into_iter()
        .map(|g| (g.role.0, g.how))
        .collect()
}

/// Yield, return, and a settled request: the three ways a grant bypasses the
/// policy. Each is labelled Explicit; the automatic grants around them are not.
#[test]
fn forced_grants_are_labelled_explicit() {
    let mut fx = Fx::new();
    fx.reach_build();
    // The coder yields to the tester, who answers and the turn returns.
    let q = posted(fx.post(Who::Coder, ask(Who::Tester, "Is AC2 testable?")));
    posted(fx.post(Who::Coder, yield_to(Who::Tester)));
    posted(fx.post(
        Who::Tester,
        Draft::new(MessageKind::Answer, "Yes.").reply_to(q),
    ));
    posted(fx.post(Who::Tester, pass()));
    assert_eq!(fx.turn_role(), Some(Who::Coder));
    // A request to the human, settled by an answer.
    let brief = fx.st().brief.expect("brief");
    let req = posted(
        fx.post(
            Who::Coder,
            Draft::new(MessageKind::RequestDecision, "Which retry policy?")
                .to(Recipient::Human)
                .refs(vec![Ref::art(brief)]),
        ),
    );
    posted(fx.human(HumanAction::Post(
        Draft::new(MessageKind::Answer, "Exponential.").reply_to(req),
    )));

    let report = why::explain(&fx.st().task, &log_of(&fx), &[]);
    let seen: Vec<(&str, &How)> = report
        .grants
        .iter()
        .map(|g| (g.role.0.as_str(), &g.how))
        .collect();
    assert!(
        seen.iter().any(|(r, h)| *r == "tester"
            && **h
                == How::Explicit(Cause::Consultation {
                    return_to: Some(role("coder"))
                })),
        "{seen:?}"
    );
    assert!(
        seen.iter()
            .any(|(r, h)| *r == "coder" && **h == How::Explicit(Cause::ConsultationReturn)),
        "{seen:?}"
    );
    assert!(
        seen.iter()
            .any(|(r, h)| *r == "coder" && **h == How::Explicit(Cause::Settlement)),
        "{seen:?}"
    );
    // The first four grants (planner, coder, tester, coder...) up to build are
    // automatic; an Explicit grant carries neither a set nor a comparison.
    for g in &report.grants {
        if matches!(g.how, How::Explicit(_)) {
            assert!(g.wake_set.is_none() && g.shadow.is_none(), "{g:?}");
        } else {
            assert_eq!(g.how, How::DefaultHolder, "{g:?}");
        }
    }
}

#[test]
fn a_human_ask_at_the_plan_gate_is_explicit() {
    let mut fx = Fx::new();
    fx.reach_plan_gate();
    let mut d = ask("planner", "why this approach?");
    d.to = vec![Recipient::Role(role("planner"))];
    posted(fx.human(HumanAction::Post(d)));
    let last = hows(&fx).pop().expect("a grant");
    assert_eq!(
        last,
        (
            "planner".to_owned(),
            How::Explicit(Cause::Consultation { return_to: None })
        )
    );
}

/// The reason an event-driven task picked a non-default role, and the quiet
/// board that does not.
#[test]
fn an_addressed_role_and_the_quiet_fallback_are_told_apart() {
    let mut fx = Fx::event_driven(four_roles());
    fx.reach_build();
    posted(fx.post(Who::Coder, ask(Who::Tester, "which 429 cases matter?")));
    posted(fx.post(Who::Coder, pass()));
    let report = why::explain(&fx.st().task, &log_of(&fx), &[]);
    let last = report.grants.last().expect("a grant");
    assert_eq!(last.role, role("tester"));
    assert_eq!(last.how, How::WakeSet(WakeReason::Addressed));
    assert_eq!(
        last.wake_set.as_deref(),
        Some(&[woken("tester", WakeReason::Addressed)][..])
    );

    let mut fx = Fx::event_driven(four_roles());
    fx.reach_build();
    posted(fx.post(Who::Coder, pass()));
    let report = why::explain(&fx.st().task, &log_of(&fx), &[]);
    let last = report.grants.last().expect("a grant");
    assert_eq!(last.role, role("coder"));
    assert_eq!(last.how, How::Fallback);
    assert_eq!(last.wake_set.as_deref(), Some(&[][..]));
}

/// Two selectable entries: the tester's approval names the coder, so at `review`
/// the set is the coder (addressed) then the reviewer (the state's server). The
/// first is picked, and the other is still listed with its reason.
#[test]
fn the_first_of_two_selectable_entries_is_the_pick() {
    let mut fx = Fx::event_driven(four_roles());
    fx.reach_test();
    stored(fx.report(RunStatus::Passed));
    posted(fx.verdict_to(
        fx.holder(State::Test),
        VerdictKind::Approve,
        vec![Recipient::Role(role("coder"))],
    ));
    assert_eq!(fx.st().state, State::Review);
    let report = why::explain(&fx.st().task, &log_of(&fx), &[]);
    let last = report.grants.last().expect("a grant");
    assert_eq!(last.role, role("coder"));
    assert_eq!(last.how, How::WakeSet(WakeReason::Addressed));
    assert_eq!(
        last.wake_set.as_deref(),
        Some(
            &[
                woken("coder", WakeReason::Addressed),
                woken("reviewer", WakeReason::State),
            ][..]
        )
    );
}

// ---------------------------------------------------------------- the CLI

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bbp_host_why_{}_{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// A store holding a fixture's log under `task` (read-only use: no task was opened here).
fn store_with(tag: &str, name: &str, task: &str) -> PathBuf {
    let dir = tempdir(tag);
    master::ensure(&dir).expect("master");
    let path = log_path_for(&dir, &TaskId(task.into()));
    std::fs::create_dir_all(path.parent().expect("tasks dir")).expect("mkdir");
    std::fs::copy(fixture(name), &path).expect("copy log");
    dir
}

fn bbp(args: &[&str]) -> (bool, String, String) {
    let out = Proc::new(env!("CARGO_BIN_EXE_bbp"))
        .args(args)
        .output()
        .expect("run bbp");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn the_command_prints_every_grant_or_only_the_turn_asked_for() {
    let dir = store_with("cmd", "run_a_event_driven.log", "slug-ed-1");
    let d = dir.to_str().unwrap();

    let (ok, all, err) = bbp(&["why", "--dir", d, "--task", "slug-ed-1"]);
    assert!(ok, "{err}");
    assert_eq!(all.matches("\"turn\":").count(), 4, "{all}");
    assert!(all.contains("\"policy\":\"EventDriven\""), "{all}");
    assert!(all.contains("\"WakeSet\":\"State\""), "{all}");

    // The task may be named positionally; --turn selects one grant.
    let (ok, one, err) = bbp(&["why", "slug-ed-1", "--dir", d, "--turn", "3"]);
    assert!(ok, "{err}");
    assert_eq!(one.matches("\"turn\":").count(), 1, "{one}");
    assert!(one.contains("\"role\":\"tester\""), "{one}");
    assert!(one.contains("\"Observer\""), "{one}");

    let (ok, _, err) = bbp(&["why", "--dir", d, "--task", "slug-ed-1", "--turn", "9"]);
    assert!(!ok && err.contains("no turn 9"), "{err}");
    let (ok, _, err) = bbp(&["why", "--dir", d, "--task", "slug-ed-1", "--turn", "x"]);
    assert!(!ok && err.contains("--turn must be a number"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_command_reads_a_turn_based_task_with_its_shadow_log() {
    let dir = store_with("tb", "run_turn_based.log", "slug-1");
    let task = TaskId("slug-1".into());
    std::fs::create_dir_all(shadow::log_path(&dir, &task).parent().unwrap()).unwrap();
    std::fs::copy(
        fixture("run_turn_based.shadow.jsonl"),
        shadow::log_path(&dir, &task),
    )
    .unwrap();
    let (ok, out, err) = bbp(&["why", "--dir", dir.to_str().unwrap(), "--task", "slug-1"]);
    assert!(ok, "{err}");
    assert!(out.contains("\"policy\":\"TurnBased\""), "{out}");
    assert!(out.contains("\"DefaultHolder\""), "{out}");
    assert_eq!(out.matches("\"comparison\":\"agree\"").count(), 4, "{out}");
    assert_eq!(out.matches("\"logged_same\":true").count(), 4, "{out}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_command_reads_and_never_writes() {
    let dir = store_with("ro", "run_a_event_driven.log", "slug-ed-1");
    let path = log_path_for(&dir, &TaskId("slug-ed-1".into()));
    let before = std::fs::read(&path).unwrap();
    let d = dir.to_str().unwrap();
    for args in [
        vec!["why", "--dir", d, "--task", "slug-ed-1"],
        vec!["card", "--dir", d, "--task", "slug-ed-1"],
    ] {
        let (ok, _, err) = bbp(&args);
        assert!(ok, "{err}");
    }
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "the log is untouched"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_card_carries_the_policy() {
    for (name, task, policy) in [
        ("run_a_event_driven.log", "slug-ed-1", "EventDriven"),
        ("run_turn_based.log", "slug-1", "TurnBased"),
    ] {
        let dir = store_with("card", name, task);
        let (ok, out, err) = bbp(&["card", "--dir", dir.to_str().unwrap(), "--task", task]);
        assert!(ok, "{err}");
        assert!(out.contains(&format!("\"policy\":\"{policy}\"")), "{out}");
        // The card's own fields are still there, flattened beside it.
        for key in [
            "\"task\":",
            "\"rev\":",
            "\"state\":",
            "\"turns\":",
            "\"spent\":",
        ] {
            assert!(out.contains(key), "{key} missing from {out}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
