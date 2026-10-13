#![allow(clippy::expect_used, clippy::unwrap_used)]
//! The scheduling seam: the policy recorded in `task_opened`, the old-log
//! default. `pick_next` is crate-private and unit-tested beside it.

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

fn open_task() -> OpenTask {
    let mut store = MemStore::new();
    OpenTask {
        task: TaskId("T1".into()),
        roster: four_roles(),
        repo: "r".into(),
        profile_digest: DIGEST,
        budget: Budget {
            messages: 1,
            bytes: 1,
            reads: 1,
            reads_per_turn: 1,
            turns: 1,
            iterations: 1,
        },
        turn_ms: 1,
        request_ms: 1,
        brief: store.blob_put(b"brief"),
        human: PrincipalId("human".into()),
        policy: SchedulingPolicy::TurnBased,
    }
}

const KEY: &str = ",\"policy\":\"TurnBased\"";

/// A serialized `OpenTask` or `Command::Open` from before the field existed
/// opens as `TurnBased`.
#[test]
fn a_legacy_open_task_and_command_decode_as_turn_based() {
    let o = open_task();
    let text = rusty_serde::json::to_string(&o).expect("encode");
    assert!(text.contains(KEY), "{text}");
    let back: OpenTask = rusty_serde::json::from_str(&text.replacen(KEY, "", 1)).expect("legacy");
    assert_eq!(back, o);

    let text = rusty_serde::json::to_string(&Command::Open(o.clone())).expect("encode");
    assert!(text.contains(KEY), "{text}");
    let cmd: Command = rusty_serde::json::from_str(&text.replacen(KEY, "", 1)).expect("legacy");
    assert_eq!(cmd, Command::Open(o));
}

#[test]
fn a_malformed_policy_in_an_open_task_is_refused() {
    let text = rusty_serde::json::to_string(&open_task()).expect("encode");
    let bad = text.replacen("\"TurnBased\"", "\"Nope\"", 1);
    assert!(rusty_serde::json::from_str::<OpenTask>(&bad).is_err());
}
