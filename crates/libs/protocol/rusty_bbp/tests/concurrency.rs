#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Phase 2a: several live turns, at most one per role, with slices reserved at grant.

mod common;
use common::*;
use rusty_bbp::*;

/// The four test roles plus `coder2` and `coder3`, which also build.
fn builders() -> Vec<Role> {
    let mut roles = four_roles();
    for (name, order) in [("coder2", 4), ("coder3", 5)] {
        let base = roles
            .iter()
            .find(|r| r.id.0 == "coder")
            .expect("coder")
            .clone();
        let mut r = base;
        r.id = RoleId(name.into());
        r.turn_order = order;
        roles.push(r);
    }
    roles
}

fn budget(messages: u64, reads_per_turn: u64) -> Budget {
    Budget {
        messages,
        reads_per_turn,
        ..default_budget()
    }
}

fn live_roles(fx: &Fx) -> Vec<String> {
    fx.st().turns.iter().map(|t| t.role.0.clone()).collect()
}

fn build_phase(live: u32, b: Budget) -> Fx {
    let mut fx = Fx::concurrent(builders(), live, b);
    fx.reach_build();
    fx
}

#[test]
fn open_rejects_a_live_turn_cap_outside_one_to_roster_size() {
    for live in [0, 7] {
        let mut fx = Fx::concurrent(builders(), live, default_budget());
        assert!(!fx.st().opened, "cap {live} must not open the task");
        assert!(fx
            .d
            .store
            .events(&TaskId("T1".into()), 0)
            .unwrap()
            .is_empty());
        fx.tick();
    }
    assert!(Fx::concurrent(builders(), 6, default_budget()).st().opened);
}

#[test]
fn serving_roles_hold_turns_up_to_the_cap() {
    let fx = build_phase(2, default_budget());
    assert_eq!(
        live_roles(&fx),
        ["coder", "coder2"],
        "turn_order, capped at 2"
    );
    let (a, b) = (&fx.st().turns[0], &fx.st().turns[1]);
    assert_ne!(
        fx.st().token_for(&pid(a.role.clone()), a.id),
        fx.st().token_for(&pid(b.role.clone()), b.id)
    );
    assert_eq!(
        fx.st().spent.turns,
        2 + 1,
        "planning turn + two build turns"
    );
}

#[test]
fn cap_of_one_keeps_a_single_turn_and_no_slice() {
    let fx = build_phase(1, default_budget());
    assert_eq!(live_roles(&fx), ["coder"]);
    assert_eq!(fx.st().turns[0].slice, None);
}

#[test]
fn concurrent_writers_store_distinct_diffs() {
    let mut fx = build_phase(2, default_budget());
    let a = stored(fx.put("coder", ArtifactPayload::Diff, b"--- a\n+++ b\n"));
    let b = stored(fx.put("coder2", ArtifactPayload::Diff, b"--- c\n+++ d\n"));
    assert_ne!(a, b);
    assert_eq!(
        live_roles(&fx),
        ["coder", "coder2"],
        "storing a diff ends no turn"
    );
}

#[test]
fn a_role_without_a_live_turn_is_stale_and_a_wrong_token_too() {
    let mut fx = build_phase(2, default_budget());
    assert_eq!(
        rejected(fx.post("coder3", ask("coder", "?"))),
        Code::StaleTurn,
        "coder3 is serving but over the cap"
    );
    let op = fx.op();
    let wrong = fx.tok("coder");
    assert_eq!(
        rejected(fx.agent_with(
            "coder2",
            Some(wrong),
            op,
            AgentAction::Post(ask("coder", "?"))
        )),
        Code::StaleTurn,
        "coder's token is not coder2's"
    );
}

#[test]
fn a_candidate_revokes_every_other_live_turn() {
    let mut fx = build_phase(2, default_budget());
    let diff = stored(fx.put("coder", ArtifactPayload::Diff, b"--- a\n+++ b\n"));
    stored(fx.put(
        "coder",
        ArtifactPayload::Candidate(Candidate {
            base: "a".repeat(40),
            diffs: vec![diff],
        }),
        b"{}",
    ));
    assert_eq!(fx.st().state, State::Test);
    assert!(
        !live_roles(&fx).contains(&"coder2".to_string()),
        "coder2's build turn was revoked"
    );
    let log = fx.d.store.events(&TaskId("T1".into()), 0).unwrap();
    assert!(log.iter().any(|e| matches!(
        e,
        Event::TurnEnded {
            cause: TurnEnd::Revoked,
            ..
        }
    )));
    // The late write is refused (test waits for the runner, so nobody is live),
    // and the log still folds to the state.
    assert_eq!(
        rejected(fx.put("coder2", ArtifactPayload::Diff, b"late")),
        Code::TokenExpired
    );
    assert_eq!(
        *fx.st(),
        TaskState::fold(
            fx.st().task.clone(),
            &fx.d.store.events(&TaskId("T1".into()), 0).unwrap()
        )
    );
}

#[test]
fn a_pass_ends_one_turn_and_leaves_the_others() {
    let mut fx = build_phase(2, default_budget());
    let coder2 = fx.st().turn_of(&RoleId("coder2".into())).expect("turn").id;
    posted(fx.post("coder", pass()));
    let after = fx
        .st()
        .turn_of(&RoleId("coder2".into()))
        .expect("still live");
    assert_eq!(after.id, coder2, "coder2's turn is untouched");
    assert_eq!(
        live_roles(&fx).len(),
        2,
        "coder was regranted into its free slot"
    );
}

#[test]
fn deadlines_expire_each_turn_on_its_own() {
    let mut fx = build_phase(2, default_budget());
    let coder = fx
        .st()
        .turn_of(&RoleId("coder".into()))
        .expect("turn")
        .clone();
    // Move one deadline forward by ending coder2's turn and regranting it later.
    fx.advance(30_000);
    posted(fx.post("coder2", pass()));
    let coder2 = fx
        .st()
        .turn_of(&RoleId("coder2".into()))
        .expect("regranted")
        .clone();
    assert!(coder2.deadline > coder.deadline);
    fx.now = coder.deadline;
    fx.tick();
    assert_eq!(
        fx.st().turn_of(&RoleId("coder2".into())).map(|t| t.id),
        Some(coder2.id),
        "coder2 kept its turn"
    );
    let log = fx.d.store.events(&TaskId("T1".into()), 0).unwrap();
    assert!(log.iter().any(|e| matches!(
        e,
        Event::TurnEnded { turn, cause: TurnEnd::Deadline } if *turn == coder.id
    )));
    assert!(
        !log.iter().any(|e| matches!(
            e,
            Event::TurnEnded { turn, cause: TurnEnd::Deadline } if *turn == coder2.id
        )),
        "coder2's later deadline has not passed"
    );
}

#[test]
fn reads_per_turn_is_counted_per_turn() {
    let mut fx = build_phase(2, budget(40, 2));
    let read = AgentAction::Read {
        after: None,
        limit: 5,
    };
    for _ in 0..2 {
        assert!(matches!(
            fx.agent("coder", read.clone()),
            Response::Read { .. }
        ));
    }
    assert_eq!(
        rejected(fx.agent("coder", read.clone())),
        Code::TurnBudgetExhausted
    );
    // coder2 has read nothing, so its own count is unaffected.
    assert!(matches!(
        fx.agent("coder2", read.clone()),
        Response::Read { .. }
    ));
    assert_eq!(
        fx.st()
            .turn_of(&RoleId("coder2".into()))
            .expect("live")
            .reads,
        1
    );
}

#[test]
fn slices_never_sum_past_the_headroom() {
    let fx = build_phase(2, budget(20, 30));
    let slices: Vec<Slice> = fx
        .st()
        .turns
        .iter()
        .map(|t| t.slice.expect("slice"))
        .collect();
    let spent = fx.st().spent;
    let sum_m: u64 = slices.iter().map(|s| s.messages).sum();
    let sum_b: u64 = slices.iter().map(|s| s.bytes).sum();
    assert!(sum_m + spent.messages <= 20);
    assert!(sum_b + spent.bytes <= fx.st().budget.bytes);
    assert_eq!(slices[0].messages, (20 - spent.messages) / 2);
}

#[test]
fn a_write_past_the_slice_ends_only_that_turn_and_total_never_overshoots() {
    let mut fx = build_phase(2, budget(12, 30));
    let slice = fx
        .st()
        .turn_of(&RoleId("coder".into()))
        .expect("turn")
        .slice
        .expect("slice");
    let coder2_slice = fx
        .st()
        .turn_of(&RoleId("coder2".into()))
        .expect("turn")
        .slice
        .expect("slice");
    for _ in 0..slice.messages {
        posted(fx.post("coder", ask("coder2", "x")));
    }
    assert_eq!(
        rejected(fx.post("coder", ask("coder2", "one too many"))),
        Code::TurnBudgetExhausted
    );
    for _ in 0..coder2_slice.messages {
        posted(fx.post("coder2", ask("coder", "y")));
    }
    assert_eq!(
        rejected(fx.post("coder2", ask("coder", "one too many"))),
        Code::TurnBudgetExhausted
    );
    assert!(
        fx.st().spent.messages <= fx.st().budget.messages,
        "concurrent turns jointly stayed inside the total ({} of {})",
        fx.st().spent.messages,
        fx.st().budget.messages
    );
    assert!(fx.st().exhausted.is_empty(), "no budget escalation fired");
}

#[test]
fn unspent_slice_returns_when_a_turn_ends() {
    let mut fx = build_phase(2, budget(20, 30));
    posted(fx.post("coder", pass()));
    // The regranted coder slice is the unreserved headroom, not the old slice.
    let coder = fx.st().turn_of(&RoleId("coder".into())).expect("regranted");
    let other = fx.st().turn_of(&RoleId("coder2".into())).expect("live");
    let m = fx.st().budget.messages - fx.st().spent.messages - other.slice.expect("slice").messages;
    assert_eq!(coder.slice.expect("slice").messages, m);
}

#[test]
fn a_log_written_before_live_turns_decodes_and_folds_as_one_turn() {
    let old = include_str!("golden/n1_happy.log");
    let events: Vec<Event> = old
        .lines()
        .map(|l| rusty_serde::json::from_str(l).expect("old line decodes"))
        .collect();
    let st = TaskState::fold(TaskId("T1".into()), &events);
    assert_eq!(st.max_live_turns, 1);
    assert_eq!(st.state, State::Closed);
}

/// Replay equals live across an expired turn: the fold takes no clock, so an
/// expiry is state only once a logged `turn_ended(deadline)` says so, and a
/// replica that reads the log reaches the same state and the same decisions.
fn replay_equals_live(live: u32) {
    let mut fx = Fx::concurrent(builders(), live, default_budget());
    fx.reach_build();
    let task = TaskId("T1".into());
    let late = fx.st().turns[0].deadline;
    fx.now = late; // the deadline has passed; no command has seen it yet
    let log = fx.d.store.events(&task, 0).unwrap();
    // Before any command: the expired turn is still live in the fold, as in the live state.
    assert_eq!(*fx.st(), TaskState::fold(task.clone(), &log));
    assert!(fx.st().turn_of(&RoleId("coder".into())).is_some());
    // The same command at the same instant gives the same events from either state.
    let cmd = Command::Agent {
        principal: pid("coder"),
        token: Some(fx.tok("coder")),
        op: OpId("late".into()),
        action: AgentAction::Post(ask("planner", "late")),
    };
    let replica = TaskState::fold(task.clone(), &log);
    assert_eq!(
        handle(fx.st(), &cmd, fx.now),
        handle(&replica, &cmd, fx.now)
    );
    // Dispatch: the deadline is logged, the role is regranted a fresh turn, and
    // the expired token is refused as stale.
    assert_eq!(rejected(fx.run(&cmd)), Code::StaleTurn);
    let log = fx.d.store.events(&task, 0).unwrap();
    assert!(log.iter().any(|e| matches!(
        e,
        Event::TurnEnded {
            cause: TurnEnd::Deadline,
            ..
        }
    )));
    // A fresh driver over the same store reconstructs the live state.
    let mut other = Driver::new(std::mem::take(&mut fx.d.store), task.clone());
    other.reload().expect("reload");
    assert_eq!(other.state, TaskState::fold(task, &log));
    assert!(
        other
            .state
            .turn_of(&RoleId("coder".into()))
            .map(|t| t.deadline)
            != Some(late)
    );
}

#[test]
fn replay_equals_live_with_an_expired_turn_at_one_live_turn() {
    replay_equals_live(1);
}

#[test]
fn replay_equals_live_with_an_expired_turn_above_one_live_turn() {
    replay_equals_live(2);
}

/// The four roles plus a second reviewer, so `review` has two final-decision judges.
fn two_reviewers() -> Vec<Role> {
    let mut roles = four_roles();
    let mut second = roles
        .iter()
        .find(|r| r.id.0 == "reviewer")
        .expect("reviewer")
        .clone();
    second.id = RoleId("reviewer2".into());
    second.turn_order = 6;
    roles.push(second);
    roles
}

/// KNOWN 2a GAP, owner: the repository owner; closes with 2b's snapshot-hash
/// binding (design note, section d) or earlier with a `read_rev` on verdicts.
///
/// A verdict binds to the candidate, the selected run and that run's report. It
/// does not bind to the log revision the reviewer read, so a `finding` appended
/// by a concurrent turn after the read does not void it. Only a transition
/// revokes concurrent turns; a finding is not one. This test pins today's
/// behavior so the gap is visible; when the gap is closed it must flip.
#[test]
fn known_gap_a_verdict_is_not_bound_to_the_revision_the_reviewer_read() {
    let mut fx = Fx::concurrent(two_reviewers(), 2, default_budget());
    fx.reach_review();
    assert_eq!(live_roles(&fx), ["reviewer", "reviewer2"]);
    // reviewer reads at revision R ...
    assert!(matches!(
        fx.agent(
            "reviewer",
            AgentAction::Read {
                after: None,
                limit: 5
            }
        ),
        Response::Read { .. }
    ));
    let read_at = fx.rev();
    // ... reviewer2 then appends a finding against the same candidate and report ...
    let cand = fx.st().candidate.expect("candidate");
    let report = fx.report_id();
    let finding = Draft::new(MessageKind::Finding, "the retry loop never backs off")
        .refs(vec![Ref::art(cand)])
        .evidence(vec![Ref::art(report)]);
    posted(fx.post("reviewer2", finding));
    assert!(fx.rev() > read_at, "an append landed after the read");
    // ... and the first reviewer's approve is still accepted.
    posted(fx.verdict("reviewer", VerdictKind::Approve));
    assert_eq!(fx.st().state, State::MergeGate);
}

#[test]
fn abort_turn_ends_the_named_turn_only() {
    let mut fx = build_phase(2, default_budget());
    let coder = fx.st().turn_of(&RoleId("coder".into())).expect("turn").id;
    let coder2 = fx.st().turn_of(&RoleId("coder2".into())).expect("turn").id;
    fx.run(&Command::AbortTurn { turn: Some(coder) });
    assert_eq!(
        fx.st().turn_of(&RoleId("coder2".into())).map(|t| t.id),
        Some(coder2),
        "the other turn is untouched"
    );
    let regranted = fx
        .st()
        .turn_of(&RoleId("coder".into()))
        .expect("regranted")
        .id;
    assert_ne!(regranted, coder, "the aborted turn was replaced");
    let log = fx.d.store.events(&TaskId("T1".into()), 0).unwrap();
    let aborted: Vec<_> = log
        .iter()
        .filter(|e| {
            matches!(
                e,
                Event::TurnEnded {
                    cause: TurnEnd::Aborted,
                    ..
                }
            )
        })
        .collect();
    assert_eq!(aborted.len(), 1);
}

#[test]
fn abort_turn_of_a_turn_that_is_not_live_does_nothing() {
    let mut fx = build_phase(2, default_budget());
    let before = fx.st().clone();
    fx.run(&Command::AbortTurn {
        turn: Some(TurnId(999)),
    });
    assert_eq!(fx.st().turns, before.turns);
}

#[test]
fn abort_turn_without_an_id_ends_every_live_turn() {
    let mut fx = build_phase(2, default_budget());
    let old: Vec<TurnId> = fx.st().turns.iter().map(|t| t.id).collect();
    fx.run(&Command::AbortTurn { turn: None });
    assert!(fx.st().turns.iter().all(|t| !old.contains(&t.id)));
    assert_eq!(fx.st().turns.len(), 2, "both slots were refilled");
}

mod model {
    use super::*;
    use proptest::prelude::*;

    const BUILDERS: [&str; 3] = ["coder", "coder2", "coder3"];

    #[derive(Clone, Debug)]
    enum Step {
        Ask(usize),
        Pass(usize),
        Read(usize),
        Diff(usize),
        Candidate(usize),
        Tick(u32),
        Abort,
    }

    fn step() -> impl Strategy<Value = Step> {
        prop_oneof![
            4 => (0..3usize).prop_map(Step::Ask),
            2 => (0..3usize).prop_map(Step::Pass),
            2 => (0..3usize).prop_map(Step::Read),
            2 => (0..3usize).prop_map(Step::Diff),
            1 => (0..3usize).prop_map(Step::Candidate),
            1 => (0u32..120_000).prop_map(Step::Tick),
            1 => Just(Step::Abort),
        ]
    }

    fn run(fx: &mut Fx, s: &Step) {
        match s {
            Step::Ask(i) => drop(fx.post(BUILDERS[*i], ask("planner", "q"))),
            Step::Pass(i) => drop(fx.post(BUILDERS[*i], pass())),
            Step::Read(i) => drop(fx.agent(
                BUILDERS[*i],
                AgentAction::Read {
                    after: None,
                    limit: 5,
                },
            )),
            Step::Diff(i) => drop(fx.put(BUILDERS[*i], ArtifactPayload::Diff, b"--- a\n+++ b\n")),
            Step::Candidate(i) => {
                let diff = fx
                    .st()
                    .artifacts
                    .values()
                    .find(|a| a.kind == ArtifactKind::Diff)
                    .map(|a| a.id);
                if let Some(diff) = diff {
                    let c = ArtifactPayload::Candidate(Candidate {
                        base: "a".repeat(40),
                        diffs: vec![diff],
                    });
                    drop(fx.put(BUILDERS[*i], c, b"{}"));
                }
            }
            Step::Tick(ms) => {
                fx.advance(u64::from(*ms));
                fx.tick();
            }
            Step::Abort => drop(fx.run(&Command::AbortTurn { turn: None })),
        }
    }

    fn check(fx: &Fx) {
        let st = fx.st();
        assert!(st.turns.len() <= st.max_live_turns as usize, "cap holds");
        for (i, t) in st.turns.iter().enumerate() {
            assert!(
                st.turns[..i].iter().all(|o| o.role != t.role),
                "one per role"
            );
        }
        // Slices are reserved, so concurrent writers cannot jointly overshoot.
        assert!(
            st.spent.messages <= st.budget.messages,
            "messages overshoot"
        );
        assert!(st.spent.bytes <= st.budget.bytes, "bytes overshoot");
        let r = st.reserved();
        if st.max_live_turns > 1 && st.state == State::Build {
            assert!(st.spent.messages + r.messages <= st.budget.messages);
            assert!(st.spent.bytes + r.bytes <= st.budget.bytes);
        }
        let log = fx.d.store.events(&st.task, 0).expect("log");
        assert_eq!(
            *st,
            TaskState::fold(st.task.clone(), &log),
            "state is the fold"
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 200, .. ProptestConfig::default() })]
        #[test]
        fn concurrent_builders_hold_invariants(
            live in 2u32..=3,
            steps in proptest::collection::vec(step(), 1..50),
        ) {
            let mut fx = Fx::concurrent(builders(), live, Budget { messages: 12, ..default_budget() });
            fx.reach_build();
            for s in &steps {
                run(&mut fx, s);
                check(&fx);
            }
        }
    }
}
