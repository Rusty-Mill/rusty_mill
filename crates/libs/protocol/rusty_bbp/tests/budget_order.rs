#![allow(clippy::expect_used, clippy::unwrap_used)]
//! The order of events around the grant that crosses the turns budget.
//!
//! `grant` evaluates the crossing itself, right after it emits `TurnGranted`
//! (#657), so on every path that mints a turn the log reads
//! `TurnGranted, Escalated, ..` and the command's own `OpAccepted` or
//! `Rejected` comes last. Before #657 only the end-of-command sweep escalated,
//! so an accepted command wrote `TurnGranted, OpAccepted, Escalated`, and a
//! refused command or a deadline regrant did not escalate until a later
//! command. The new order is pinned exactly, on the three paths that reach it.

mod common;
use common::*;
use rusty_bbp::*;

/// A task that allows one live turn and one turn in all, so the first regrant
/// crosses the budget.
fn one_turn_task(reads_per_turn: u64) -> Fx {
    let b = Budget {
        turns: 1,
        reads_per_turn,
        ..default_budget()
    };
    Fx::concurrent(four_roles(), 1, b)
}

fn log(fx: &Fx) -> Vec<Event> {
    fx.d.store.events(&TaskId("T1".into()), 0).expect("log")
}

/// The variant name of each event: `TurnEnded { .. }` is `TurnEnded`.
fn names(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .map(|e| {
            let s = format!("{e:?}");
            s.chars().take_while(|c| c.is_alphanumeric()).collect()
        })
        .collect()
}

/// Replay equals live, and the budget fired exactly once.
fn check_replay_and_single_crossing(fx: &Fx) {
    let events = log(fx);
    assert_eq!(
        *fx.st(),
        TaskState::fold(TaskId("T1".into()), &events),
        "replay equals live"
    );
    let crossings = events
        .iter()
        .filter(|e| {
            matches!(
                e,
                Event::Escalated {
                    reason: EscalateReason::Budget(BudgetField::Turns),
                    ..
                }
            )
        })
        .count();
    assert_eq!(crossings, 1, "one crossing, one escalation");
    assert_eq!(fx.st().state, State::Escalated);
    assert_eq!(fx.st().spent.turns, 2, "one past the limit, no more");
}

/// What one command wrote.
fn written(fx: &mut Fx, f: impl FnOnce(&mut Fx) -> Response) -> (Response, Vec<String>) {
    let before = log(fx).len();
    let resp = f(fx);
    (resp, names(&log(fx)[before..]))
}

#[test]
fn an_accepted_command_writes_the_escalation_before_its_own_op_accepted() {
    let mut fx = one_turn_task(30);
    let (resp, seen) = written(&mut fx, |fx| fx.post("planner", pass()));
    assert!(matches!(resp, Response::Posted(_)), "{resp:?}");
    assert_eq!(
        seen,
        [
            "MessageAppended",
            "Charged",
            "TurnEnded", // the planner's pass
            "TurnGranted",
            "Escalated",
            "TurnEnded", // the escalation revokes the new turn
            "StateChanged",
            "OpAccepted",
        ]
    );
    check_replay_and_single_crossing(&fx);
}

#[test]
fn a_refused_command_writes_the_escalation_before_its_rejection() {
    let mut fx = one_turn_task(1);
    let read = AgentAction::Read {
        after: None,
        limit: 5,
    };
    assert!(matches!(
        fx.agent("planner", read.clone()),
        Response::Read { .. }
    ));
    let (resp, seen) = written(&mut fx, |fx| fx.agent("planner", read));
    assert_eq!(rejected(resp), Code::TurnBudgetExhausted);
    assert_eq!(
        seen,
        [
            "TurnEnded", // the refused read ends the turn
            "TurnGranted",
            "Escalated",
            "TurnEnded",
            "StateChanged",
            "Rejected",
        ]
    );
    check_replay_and_single_crossing(&fx);
}

#[test]
fn a_deadline_regrant_writes_the_escalation_before_the_commands_rejection() {
    let mut fx = one_turn_task(30);
    fx.now = fx.st().turns[0].deadline;
    let (resp, seen) = written(&mut fx, |fx| fx.post("planner", pass()));
    assert_eq!(rejected(resp), Code::TokenExpired);
    assert_eq!(
        seen,
        [
            "TurnEnded", // the deadline
            "TurnGranted",
            "Escalated",
            "TurnEnded",
            "StateChanged",
            "Rejected",
        ]
    );
    check_replay_and_single_crossing(&fx);
}

// ---------------------------------------------------------------- the old order

/// The log of the accepted-command crossing, and the same events in the order a
/// pre-#657 engine wrote them. #657 made `grant` escalate right after
/// `TurnGranted`, so `Escalated` (with the revoke and `StateChanged` it causes)
/// now comes before the command's `OpAccepted`; before, the end-of-command sweep
/// escalated, after `OpAccepted`. The old order is the new one with `OpAccepted`
/// moved to directly after the crossing grant. No other event moves.
fn both_orders() -> (Vec<Event>, Vec<Event>) {
    let mut fx = one_turn_task(30);
    drop(fx.post("planner", pass()));
    let new = log(&fx);
    let grant = new
        .iter()
        .rposition(|e| matches!(e, Event::TurnGranted { .. }))
        .expect("the crossing grant");
    let op = new
        .iter()
        .rposition(|e| matches!(e, Event::OpAccepted { .. }))
        .expect("the accepted op");
    assert!(op > grant + 1, "the new order has the escalation between");
    let mut old = new.clone();
    let accepted = old.remove(op);
    old.insert(grant + 1, accepted);
    assert_eq!(
        names(&old[old.len() - 5..]),
        [
            "TurnGranted",
            "OpAccepted",
            "Escalated",
            "TurnEnded",
            "StateChanged"
        ],
        "the order a pre-#657 engine wrote (confirmed on 45824a55)"
    );
    (old, new)
}

fn with_schema(events: &[Event], schema: u32) -> Vec<Event> {
    let mut out = events.to_vec();
    match out.first_mut() {
        Some(Event::TaskOpened { schema: s, .. }) => *s = schema,
        other => panic!("a log starts with task_opened, not {other:?}"),
    }
    out
}

const SCHEMAS: [u32; 3] = [LOG_SCHEMA, LOG_SCHEMA_CONCURRENT, LOG_SCHEMA_EVENT_DRIVEN];

fn fold(events: &[Event]) -> TaskState {
    TaskState::fold(TaskId("T1".into()), events)
}

/// A log in the old order folds, under every schema that accepts it, to the
/// state the new order folds to, except for three rev stamps. They record the
/// rev of the event that set them, and `OpAccepted` moving up shifts the
/// events after it down by one: so each is exactly one more in the old order.
#[test]
fn a_log_in_the_old_order_folds_to_the_same_state_but_for_three_rev_stamps() {
    let (old, new) = both_orders();
    let planner = RoleId::from("planner");
    for schema in SCHEMAS {
        let (from_old, from_new) = (
            fold(&with_schema(&old, schema)),
            fold(&with_schema(&new, schema)),
        );
        let at = |st: &TaskState| {
            (
                st.last_turn_end[&planner].0,
                st.ready_rev.0,
                st.entered_rev.0,
            )
        };
        let (o, n) = (at(&from_old), at(&from_new));
        assert_eq!(
            (o.0, o.1, o.2),
            (n.0 + 1, n.1 + 1, n.2 + 1),
            "schema {schema}: each stamp is one later in the old order"
        );
        // Everything else is equal: put the stamps back and compare whole states.
        let mut normalised = from_old.clone();
        normalised.last_turn_end = from_new.last_turn_end.clone();
        normalised.ready_rev = from_new.ready_rev;
        normalised.entered_rev = from_new.entered_rev;
        assert_eq!(normalised, from_new, "schema {schema}");
        assert_ne!(from_old, from_new, "schema {schema}: the stamps do differ");
        assert_eq!(from_old.rev, from_new.rev, "schema {schema}");
        assert_eq!(from_old.state, State::Escalated, "schema {schema}");
        // And no other role's stamp exists to differ.
        assert_eq!(from_old.last_turn_end.len(), 1, "schema {schema}");
    }
}

/// `wake_set` reads those three stamps, so check it directly: the same set at
/// every prefix of the two logs, under every schema. The prefixes are compared
/// at equal length, which is also the point where the two logs differ
/// (between the crossing grant and the end of the escalation).
#[test]
fn wake_set_is_the_same_at_every_prefix_of_either_order() {
    let (old, new) = both_orders();
    assert_eq!(old.len(), new.len());
    let mut differing_prefixes = 0;
    for schema in SCHEMAS {
        let (old, new) = (with_schema(&old, schema), with_schema(&new, schema));
        for n in 1..=old.len() {
            let (a, b) = (fold(&old[..n]), fold(&new[..n]));
            assert_eq!(
                wake_set(&a),
                wake_set(&b),
                "schema {schema}, prefix {n} of {}: after {:?} / {:?}",
                old.len(),
                names(&old[n - 1..n]),
                names(&new[n - 1..n])
            );
            differing_prefixes += usize::from(a != b);
        }
    }
    assert!(
        differing_prefixes > 0,
        "the prefixes do differ as states, or this compares nothing"
    );
}
