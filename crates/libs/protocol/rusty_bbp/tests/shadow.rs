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

/// At `test` the tester serves the state and is first; the others only watch
/// the report, so they are recorded as observers and never outrank it.
#[test]
fn at_test_the_server_is_first_and_the_others_are_observers() {
    let mut fx = Fx::new();
    fx.reach_test();
    stored(fx.report(RunStatus::Passed));
    let a = autos(&fx).pop().expect("a grant at test");
    assert_eq!(a.state, State::Test);
    assert_eq!(a.granted, [RoleId::from("tester")]);
    assert_eq!(a.cmp, Comparison::Agree);
    assert_eq!(a.would[0], (RoleId::from("tester"), WakeReason::State));
    assert!(
        a.would[1..]
            .iter()
            .all(|(_, why)| *why == WakeReason::Observer),
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

/// A grant that crosses the turn budget escalates in the same command. The
/// record keeps the state the grant was made in and the rev of its own
/// `turn_granted`, not the state and rev it escalated to.
fn one_turn_budget() -> Fx {
    let budget = Budget {
        turns: 1,
        ..default_budget()
    };
    Fx::concurrent(four_roles(), 1, budget)
}

#[test]
fn an_automatic_grant_that_escalates_keeps_its_pre_grant_state() {
    let mut fx = one_turn_budget();
    posted(fx.post(Who::Planner, pass()));
    assert_eq!(fx.st().state, State::Escalated, "the second grant crossed");
    let a = autos(&fx).pop().expect("a grant");
    assert_eq!(a.state, State::Planning);
    let rev = fx
        .shadows
        .iter()
        .rev()
        .find(|s| s.comparison().is_some())
        .expect("auto")
        .rev;
    let log = fx.d.store.events(&fx.st().task, 0).expect("events");
    assert!(
        matches!(&log[rev.0 as usize - 1], Event::TurnGranted { .. }),
        "the record points at its turn_granted, not the escalation after it"
    );
}

#[test]
fn an_explicit_grant_that_escalates_keeps_its_pre_grant_state() {
    let mut fx = one_turn_budget();
    posted(fx.post(Who::Planner, yield_to(Who::Tester)));
    assert_eq!(fx.st().state, State::Escalated, "the consultation crossed");
    let ex = explicits(&fx);
    assert_eq!(ex.len(), 1, "{ex:?}");
    assert_eq!(ex[0].0, State::Planning);
    assert_eq!(ex[0].1, RoleId::from("tester"));
}

/// The sink sees exactly the records of each successful dispatch that made any,
/// and a dispatch that grants nothing never calls it.
#[test]
fn a_driver_hands_its_shadow_records_to_the_sink() {
    use std::sync::{Arc, Mutex};
    let mut fx = Fx::new();
    let seen: Arc<Mutex<Vec<Shadow>>> = Arc::default();
    let sink = Arc::clone(&seen);
    let d = std::mem::replace(&mut fx.d, Driver::new(MemStore::new(), TaskId("x".into())));
    fx.d = d.with_shadow_sink(Box::new(move |s| {
        sink.lock().expect("lock").extend_from_slice(s)
    }));
    let n = fx.shadows.len();
    fx.reach_build();
    fx.tick();
    let seen = seen.lock().expect("lock").clone();
    assert!(!seen.is_empty());
    assert_eq!(seen, fx.shadows[n..], "the sink saw every record, once");
}

// ---------------------------------------------------------------- where turn-based wastes a turn
//
// Each case drives the real engine and reads the shadow record of the regrant
// after a turn ended. The real grant is always turn-based; the record says what
// the event-driven rule would have done.

/// Idle default holder. The coder passes with nothing new anywhere; turn-based
/// regrants the coder, who just said it had nothing to do. The event-driven set
/// is empty too (cooldown, no trigger), so it falls back to the same role: no
/// divergence, and no better choice exists. This is the turn `max_chain`
/// (deferred) would have to bound.
#[test]
fn case_1_an_idle_default_holder_is_regranted_and_the_set_is_empty() {
    let mut fx = Fx::new();
    fx.reach_build();
    posted(fx.post(Who::Coder, pass()));
    let a = autos(&fx).pop().expect("a regrant");
    assert_eq!(
        (a.state, a.granted),
        (State::Build, vec![RoleId::from("coder")])
    );
    assert!(a.would.is_empty(), "{:?}", a.would);
    assert_eq!(a.cmp, Comparison::Fallback);
}

/// Addressed non-default role. The coder asks the tester a question and ends
/// its turn. Turn-based regrants the coder, and the tester, who cannot serve
/// `build`, sits on the question until `test`. Event-driven selects the tester
/// at once (`Addressed`); the coder is in cooldown and absent.
#[test]
fn case_2_a_question_to_a_non_default_role_is_picked_up_at_once() {
    let mut fx = Fx::new();
    fx.reach_build();
    posted(fx.post(Who::Coder, ask(Who::Tester, "which 429 cases matter?")));
    posted(fx.post(Who::Coder, pass()));
    let a = autos(&fx).pop().expect("a regrant");
    assert_eq!(
        a.granted,
        [RoleId::from("coder")],
        "turn-based: the coder again"
    );
    assert_eq!(a.would, [(RoleId::from("tester"), WakeReason::Addressed)]);
    assert!(a.would[0].1.selectable(1));
    assert_eq!(a.cmp, Comparison::Disagree);
}

/// Observer-only artifact wake. The coder stores a diff and ends its turn. The
/// planner, tester and reviewer can read it but none of them serves `build`, so
/// each is recorded as an observer and none is selectable: the set holds
/// observers only and the rule falls back to the coder, as turn-based did.
#[test]
fn case_3_an_artifact_alone_leaves_only_observers_and_falls_back() {
    let mut fx = Fx::new();
    fx.reach_build();
    stored(fx.put(Who::Coder, ArtifactPayload::Diff, b"--- a\n+++ b\n"));
    posted(fx.post(Who::Coder, pass()));
    let a = autos(&fx).pop().expect("a regrant");
    assert_eq!(a.granted, [RoleId::from("coder")]);
    let mut seen: Vec<_> = a.would.iter().map(|(r, w)| (r.0.as_str(), *w)).collect();
    seen.sort();
    assert_eq!(
        seen,
        [
            ("planner", WakeReason::Observer),
            ("reviewer", WakeReason::Observer),
            ("tester", WakeReason::Observer),
        ]
    );
    assert_eq!(a.cmp, Comparison::Fallback);
}

/// Regression guard for open question 4. A `verdict` defaults to `to: [all]`,
/// and `all` names no one, so it addresses no one for scheduling. The tester's
/// approval used to address the planner and coder, who outranked the reviewer,
/// who serves `review` but is `scoped` and cannot even read it. Now the set is
/// the reviewer alone and agrees with turn-based. (The old behaviour caused all
/// 11 remaining disagreements over the 31 traces.)
#[test]
fn case_4_a_broadcast_verdict_addresses_no_one_and_agrees() {
    let mut fx = Fx::new();
    fx.reach_review();
    let a = autos(&fx).pop().expect("the grant at review");
    assert_eq!(
        (a.state, a.granted),
        (State::Review, vec![RoleId::from("reviewer")])
    );
    assert_eq!(a.would, [(RoleId::from("reviewer"), WakeReason::State)]);
    assert_eq!(a.cmp, Comparison::Agree);
}

/// The same approval with a named recipient still addresses that role. The
/// tester tells the coder directly; the coder (`Addressed`) outranks the
/// reviewer (`State`), so event-driven would hand the coder the turn. Scheduling
/// only: visibility and read scope are unchanged.
#[test]
fn case_5_a_verdict_naming_a_recipient_still_addresses_it() {
    let mut fx = Fx::new();
    fx.reach_test();
    stored(fx.report(RunStatus::Passed));
    let tester = fx.holder(State::Test);
    posted(fx.verdict_to(
        tester,
        VerdictKind::Approve,
        vec![Recipient::Role("coder".into())],
    ));
    assert_eq!(fx.st().state, State::Review);
    let a = autos(&fx).pop().expect("the grant at review");
    assert_eq!(
        a.granted,
        [RoleId::from("reviewer")],
        "turn-based: the reviewer"
    );
    assert_eq!(
        a.would,
        [
            (RoleId::from("coder"), WakeReason::Addressed),
            (RoleId::from("reviewer"), WakeReason::State),
        ]
    );
    assert_eq!(a.cmp, Comparison::Disagree);
}

/// Every record says which wake rules computed its `would`, so records written
/// under different rules are never summed. One written before the field existed
/// has no `rules` and reads as the first rules.
#[test]
fn a_record_carries_its_wake_rules_and_an_old_one_reads_as_the_first() {
    let fx = Fx::new();
    let rec = fx.shadows[0].clone();
    assert_eq!(rec.rules, WAKE_RULES);
    let text = rusty_serde::json::to_string(&rec).expect("encode");
    let key = format!("\"rules\":{WAKE_RULES},");
    assert!(text.contains(&key), "{text}");
    let old: Shadow = rusty_serde::json::from_str(&text.replacen(&key, "", 1)).expect("old");
    assert_eq!(old.rules, 1);
    assert_eq!(
        Shadow {
            rules: WAKE_RULES,
            ..old
        },
        rec
    );
}
