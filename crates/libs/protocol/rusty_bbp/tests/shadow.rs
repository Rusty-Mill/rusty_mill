#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Shadow mode: what an event-driven policy would have done beside each real
//! grant. The records are returned from `handle` and never logged. The
//! invariant that they match the real grants runs after every command of every
//! test (`Fx::run`); these are the named cases, and the ones that document
//! where the two rules differ.
//!
//! The comparison over the 31 traces is a report, not an assertion:
//! `BBP_SHADOW_TRACE=1 cargo test -p rusty_bbp --test traces -- --nocapture --test-threads=1`
//! prints one `SHADOW` line per record.

mod common;
use common::*;
use rusty_bbp::*;

/// An automatic record, unpacked.
#[derive(Debug)]
struct Auto {
    state: State,
    granted: Vec<RoleId>,
    would: Vec<(RoleId, WakeReason)>,
    cmp: Comparison,
}

fn autos(fx: &Fx) -> Vec<Auto> {
    fx.shadows
        .iter()
        .filter_map(|s| match &s.kind {
            ShadowKind::Auto { granted, would } => Some(Auto {
                state: s.state,
                granted: granted.clone(),
                would: would.clone(),
                cmp: s.comparison().expect("auto"),
            }),
            ShadowKind::Explicit { .. } => None,
        })
        .collect()
}

fn explicits(fx: &Fx) -> Vec<(State, RoleId, TurnKind)> {
    fx.shadows
        .iter()
        .filter_map(|s| match &s.kind {
            ShadowKind::Explicit { granted, kind } => Some((s.state, granted.clone(), *kind)),
            ShadowKind::Auto { .. } => None,
        })
        .collect()
}

#[test]
fn the_first_grant_agrees_with_the_state_trigger() {
    let fx = Fx::new();
    let a = autos(&fx);
    assert_eq!(a.len(), 1, "{a:?}");
    assert_eq!(a[0].state, State::Planning);
    assert_eq!(a[0].granted, [RoleId::from("planner")]);
    assert_eq!(a[0].would, [(RoleId::from("planner"), WakeReason::State)]);
    assert_eq!(a[0].cmp, Comparison::Agree);
}

/// A role with no turn yet has seen nothing, so every earlier artifact counts as
/// new for it; `Artifact` outranks `State` and ties go to the lowest
/// `turn_order`. The planner is woken at `test` and would be picked before the
/// tester. Recorded as a disagreement, with the tester still eligible: this is
/// the evidence that decides the baseline and tie-break (open question 3).
#[test]
fn at_test_the_planner_outranks_the_tester_by_the_artifact_trigger() {
    let mut fx = Fx::new();
    fx.reach_test();
    stored(fx.report(RunStatus::Passed));
    let a = autos(&fx).pop().expect("a grant at test");
    assert_eq!(a.state, State::Test);
    assert_eq!(a.granted, [RoleId::from("tester")]);
    assert_eq!(a.cmp, Comparison::Disagree);
    assert_eq!(a.would[0], (RoleId::from("planner"), WakeReason::Artifact));
    assert!(
        a.would.iter().any(|(r, _)| r.0 == "tester"),
        "{:?}",
        a.would
    );
}

/// A turn ended by its read budget is regranted to the same role. Its own turn
/// just ended and nothing new has happened, so the cooldown leaves `wake_set`
/// empty: the event-driven rule would fall back to the default holder, the
/// turn-based grant.
#[test]
fn a_regrant_after_the_turn_ended_with_nothing_new_falls_back() {
    let mut fx = Fx::new();
    for _ in 0..31 {
        fx.agent(
            Who::Planner,
            AgentAction::Read {
                after: None,
                limit: 20,
            },
        );
    }
    let last = autos(&fx).pop().expect("a regrant");
    assert_eq!(last.granted, [RoleId::from("planner")]);
    assert!(last.would.is_empty(), "{:?}", last.would);
    assert_eq!(last.cmp, Comparison::Fallback);
}

#[test]
fn a_yield_and_its_return_are_explicit_and_never_compared() {
    let mut fx = Fx::new();
    fx.reach_build();
    let n = fx.shadows.len();
    // The coder holds `build`; it yields to the tester, who answers and the turn returns.
    posted(fx.post(Who::Coder, yield_to(Who::Tester)));
    posted(fx.post(Who::Tester, pass()));
    let new = &fx.shadows[n..];
    assert!(new.iter().all(|s| s.comparison().is_none()), "{new:?}");
    let ex = explicits(&fx);
    assert!(
        ex.contains(&(State::Build, RoleId::from("tester"), TurnKind::Consultation)),
        "{ex:?}"
    );
    assert!(
        ex.contains(&(State::Build, RoleId::from("coder"), TurnKind::Default)),
        "{ex:?}"
    );
}

#[test]
fn a_human_ask_at_the_plan_gate_is_an_explicit_grant() {
    let mut fx = Fx::new();
    fx.reach_plan_gate();
    assert_eq!(fx.st().state, State::PlanGate);
    let n = fx.shadows.len();
    let mut d = ask("planner", "why this approach?");
    d.to = vec![Recipient::Role(RoleId::from("planner"))];
    posted(fx.human(HumanAction::Post(d)));
    let new = &fx.shadows[n..];
    assert_eq!(new.len(), 1, "{new:?}");
    assert!(matches!(
        &new[0].kind,
        ShadowKind::Explicit { granted, kind: TurnKind::Consultation } if granted.0 == "planner"
    ));
    assert_eq!(new[0].comparison(), None);
    assert!(autos(&fx).iter().all(|a| a.state != State::PlanGate));
}

#[test]
fn a_tick_with_nothing_to_grant_records_nothing() {
    let mut fx = Fx::new();
    let n = fx.shadows.len();
    fx.tick();
    fx.tick();
    assert_eq!(fx.shadows.len(), n, "a live turn: no grant, no record");
    assert!(fx.d.shadow.is_empty());
}

#[test]
fn no_shadow_is_recorded_above_one_live_turn() {
    let fx = Fx::concurrent(four_roles(), 2, default_budget());
    assert!(fx.shadows.is_empty(), "{:?}", fx.shadows);
    assert!(!fx.d.state.turns.is_empty());
}
