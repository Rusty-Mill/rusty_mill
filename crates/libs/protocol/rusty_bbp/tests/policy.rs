#![allow(clippy::expect_used, clippy::unwrap_used)]
//! The scheduling seam: the policy recorded in `task_opened`, the old-log
//! default, and `pick_next` returning a set capped by the policy.

mod common;
use common::*;
use rusty_bbp::*;

fn opened(fx: &Fx) -> Event {
    fx.d.store
        .events(&fx.st().task, 0)
        .expect("events")
        .into_iter()
        .next()
        .expect("first event")
}

#[test]
fn the_policy_is_recorded_in_task_opened_and_folded() {
    let fx = Fx::new();
    assert!(matches!(
        opened(&fx),
        Event::TaskOpened {
            policy: SchedulingPolicy::TurnBased,
            ..
        }
    ));
    assert_eq!(fx.st().policy, SchedulingPolicy::TurnBased);
}

#[test]
fn task_opened_round_trips_with_its_policy() {
    let fx = Fx::new();
    let e = opened(&fx);
    let text = rusty_serde::json::to_string(&e).expect("encode");
    assert!(text.contains("\"policy\""), "{text}");
    let back: Event = rusty_serde::json::from_str(&text).expect("decode");
    assert_eq!(back, e);
}

/// A log written before the field existed has no `policy` key. It must still
/// decode, as `TurnBased`, with the schema unchanged.
#[test]
fn a_log_without_a_policy_replays_as_turn_based() {
    let fx = Fx::new();
    let e = opened(&fx);
    let text = rusty_serde::json::to_string(&e).expect("encode");
    let legacy = text.replacen(",\"policy\":\"TurnBased\"", "", 1);
    assert_ne!(legacy, text, "the key was not found to strip: {text}");
    let back: Event = rusty_serde::json::from_str(&legacy).expect("legacy decodes");
    assert_eq!(back, e);
    let evs = fx.d.store.events(&fx.st().task, 0).expect("events");
    let mut with_legacy = vec![back];
    with_legacy.extend_from_slice(&evs[1..]);
    let st = TaskState::fold(fx.st().task.clone(), &with_legacy);
    assert_eq!(st.policy, SchedulingPolicy::TurnBased);
    assert_eq!(&st, fx.st(), "replay is identical");
}

#[test]
fn a_malformed_policy_is_an_error_not_a_default() {
    let fx = Fx::new();
    let text = rusty_serde::json::to_string(&opened(&fx)).expect("encode");
    let bad = text.replacen("\"TurnBased\"", "\"Nope\"", 1);
    assert!(rusty_serde::json::from_str::<Event>(&bad).is_err());
}

#[test]
fn turn_based_picks_the_default_holder_and_never_more_than_one() {
    assert_eq!(SchedulingPolicy::TurnBased.max_live(), 1);
    let mut fx = Fx::new();
    fx.reach_review();
    let mut st = fx.st().clone();
    st.turn = None;
    for state in [State::Planning, State::Build, State::Test, State::Review] {
        st.state = state;
        let got = pick_next(&st);
        let want: Vec<RoleId> = st
            .roster
            .default_holder(state)
            .cloned()
            .into_iter()
            .collect();
        assert_eq!(got, want, "{state:?}");
        assert!(got.len() <= st.policy.max_live());
    }
}

#[test]
fn nobody_is_picked_where_no_role_serves_the_state() {
    let mut fx = Fx::new();
    fx.reach_merge_gate();
    let mut st = fx.st().clone();
    st.turn = None;
    for state in [
        State::PlanGate,
        State::MergeGate,
        State::Approved,
        State::Closed,
    ] {
        st.state = state;
        assert!(pick_next(&st).is_empty(), "{state:?}");
    }
}
