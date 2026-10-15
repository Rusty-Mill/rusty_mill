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
    (reorder(&new), new)
}

/// The new-order log as a pre-#657 engine would have written it: the first
/// `OpAccepted` after the crossing moves to directly after the crossing grant.
/// Everything after it is the same events in the same order.
fn reorder(new: &[Event]) -> Vec<Event> {
    let crossing = new
        .iter()
        .position(|e| {
            matches!(
                e,
                Event::Escalated {
                    reason: EscalateReason::Budget(BudgetField::Turns),
                    ..
                }
            )
        })
        .expect("the crossing");
    let grant = crossing - 1;
    assert!(matches!(new[grant], Event::TurnGranted { .. }));
    let op = crossing
        + new[crossing..]
            .iter()
            .position(|e| matches!(e, Event::OpAccepted { .. }))
            .expect("the accepted op");
    assert!(op > grant + 1, "the new order has the escalation between");
    let mut old = new.to_vec();
    let accepted = old.remove(op);
    old.insert(grant + 1, accepted);
    assert_eq!(
        names(&old[grant..grant + 5]),
        [
            "TurnGranted",
            "OpAccepted",
            "Escalated",
            "TurnEnded",
            "StateChanged"
        ],
        "the order a pre-#657 engine wrote (confirmed on 45824a55)"
    );
    old
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

// ------------------------------------------------------- after a resume

/// Human actions that lift the escalation: raise the turns limit, then resume
/// to planning. A turn is granted again by the resume.
fn resume(fx: &mut Fx) {
    ok(fx.human(HumanAction::BudgetExtended {
        field: BudgetField::Turns,
        limit: 60,
    }));
    let rev = fx.rev();
    ok(fx.human(HumanAction::Resume {
        target: State::Planning,
        expected_rev: rev,
    }));
    assert_eq!(fx.st().state, State::Planning);
}

fn ask_role(fx: &mut Fx, to: &str) {
    let mut d = ask(to, "why this approach?");
    d.to = vec![Recipient::Role(RoleId::from(to))];
    posted(fx.human(HumanAction::Post(d)));
}

/// Where two `wake_set`s differ: the roles in one and not the other, or with
/// another reason, and the state and rev stamps of the two folds.
fn describe(a: &TaskState, b: &TaskState) -> String {
    let (x, y) = (wake_set(a), wake_set(b));
    let only = |p: &[(RoleId, WakeReason)], q: &[(RoleId, WakeReason)]| {
        p.iter()
            .filter(|e| !q.contains(e))
            .cloned()
            .collect::<Vec<_>>()
    };
    format!(
        "state {:?}/{:?}; only in old {:?}, only in new {:?}; stamps old/new: last_turn_end {:?}/{:?}, ready_rev {:?}/{:?}, entered_rev {:?}/{:?}",
        a.state, b.state, only(&x, &y), only(&y, &x),
        a.last_turn_end, b.last_turn_end, a.ready_rev, b.ready_rev, a.entered_rev, b.entered_rev
    )
}

/// `wake_set` at every prefix of the old-order and new-order logs of `fx`,
/// under every schema. Returns the new-order sets that are not empty, by prefix.
fn same_wake_set_everywhere(fx: &Fx) -> Vec<(usize, Vec<(RoleId, WakeReason)>)> {
    let new = log(fx);
    let old = reorder(&new);
    let mut seen = Vec::new();
    for schema in SCHEMAS {
        let (old, new) = (with_schema(&old, schema), with_schema(&new, schema));
        for n in 1..=old.len() {
            let (a, b) = (fold(&old[..n]), fold(&new[..n]));
            assert_eq!(
                wake_set(&a),
                wake_set(&b),
                "schema {schema}, prefix {n} (after {:?} / {:?}): {}",
                names(&old[n - 1..n]),
                names(&new[n - 1..n]),
                describe(&a, &b)
            );
            if schema == LOG_SCHEMA && !wake_set(&b).is_empty() {
                seen.push((n, wake_set(&b)));
            }
        }
    }
    seen
}

/// The prefix right after the resume has put the task back in planning, before
/// the resume's own grant: no live turn, and the planner's `last_turn_end` is
/// still the revoke of the crossing turn, the stamp that differs by one.
fn after_resume_state_change(events: &[Event]) -> usize {
    let resumed = events
        .iter()
        .position(|e| matches!(e, Event::Resumed { .. }))
        .expect("a resume");
    resumed
        + events[resumed..]
            .iter()
            .position(|e| {
                matches!(
                    e,
                    Event::StateChanged {
                        to: State::Planning,
                        ..
                    }
                )
            })
            .expect("back to planning")
        + 1
}

#[test]
fn wake_set_is_the_same_through_a_resume() {
    let mut fx = one_turn_task(30);
    drop(fx.post("planner", pass()));
    resume(&mut fx);
    let seen = same_wake_set_everywhere(&fx);
    // The set that reads the crossing's stamp: the planner is woken by the
    // state (the resume's rev is later than the revoke's, in either order).
    let at = after_resume_state_change(&log(&fx));
    assert!(
        seen.contains(&(at, vec![(RoleId::from("planner"), WakeReason::State)])),
        "{seen:?}"
    );
}

/// The cooldown. A message to the planner from before the crossing is older
/// than the end of its last turn, so it does not wake the planner after the
/// resume, in either order. Dropping the planner's last turn end shows it was
/// the cooldown that held it back.
#[test]
fn the_cooldown_holds_a_message_older_than_the_planners_last_turn_in_either_order() {
    let mut fx = one_turn_task(30);
    ask_role(&mut fx, "planner"); // the planner's turn is live: no grant, the message waits
    drop(fx.post("planner", pass()));
    resume(&mut fx);
    let seen = same_wake_set_everywhere(&fx);
    let new = log(&fx);
    let at = after_resume_state_change(&new);
    let set = &seen.iter().find(|(n, _)| *n == at).expect("a set here").1;
    assert_eq!(set, &vec![(RoleId::from("planner"), WakeReason::State)]);
    // The control: the same board with no turn ever ended is Addressed.
    let mut no_cooldown = fold(&new[..at]);
    no_cooldown.last_turn_end.clear();
    assert_eq!(
        wake_set(&no_cooldown)[0],
        (RoleId::from("planner"), WakeReason::Addressed)
    );
}

/// A role Addressed after the resume. The coder has never had a turn, so its
/// `last_turn_end` is read as absent; the planner's turn after the resume ends
/// with a fresh stamp that replaces the crossing's.
#[test]
fn an_addressed_role_after_the_resume_is_woken_the_same_in_either_order() {
    let mut fx = one_turn_task(30);
    drop(fx.post("planner", pass()));
    resume(&mut fx);
    ask_role(&mut fx, "coder"); // the planner's turn is live: no grant
    drop(fx.post("planner", pass()));
    let seen = same_wake_set_everywhere(&fx);
    assert!(
        seen.iter()
            .any(|(_, set)| set.contains(&(RoleId::from("coder"), WakeReason::Addressed))),
        "{seen:?}"
    );
    // Once the planner's own turn has ended again, the crossing's stamps are
    // gone and the two orders fold to the very same state.
    let new = log(&fx);
    assert_eq!(fold(&reorder(&new)), fold(&new));
}
