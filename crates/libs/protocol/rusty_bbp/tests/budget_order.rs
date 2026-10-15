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
