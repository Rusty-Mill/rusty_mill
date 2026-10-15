#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Live event-driven scheduling: the opt-in policy, through the real engine.
//! Each case runs the same script under `TurnBased` and `EventDriven`, so the
//! difference is the policy and nothing else. No shadow record is made for an
//! event-driven task (`Fx::run` checks that after every command).

mod common;
use common::*;
use rusty_bbp::*;

fn holder(fx: &Fx) -> RoleId {
    fx.turn_holder().expect("a live turn")
}

fn kind(fx: &Fx) -> TurnKind {
    fx.st().turn().expect("a live turn").kind
}

#[test]
fn the_default_is_turn_based_and_event_driven_is_recorded() {
    assert_eq!(Fx::new().st().policy, SchedulingPolicy::TurnBased);
    let fx = Fx::event_driven(four_roles());
    assert_eq!(fx.st().policy, SchedulingPolicy::EventDriven);
    let first = fx.d.store.events(&fx.st().task, 0).expect("events");
    let text = rusty_serde::json::to_string(&first[0]).expect("encode");
    assert!(text.contains("\"policy\":\"EventDriven\""), "{text}");
}

#[test]
fn event_driven_needs_exactly_one_live_turn() {
    let roles = four_roles();
    let mut store = MemStore::new();
    let open = |live, brief| {
        Command::Open(OpenTask {
            task: TaskId("T1".into()),
            roster: roles.clone(),
            repo: "r".into(),
            profile_digest: DIGEST,
            budget: default_budget(),
            turn_ms: 60_000,
            request_ms: 60_000,
            max_live_turns: live,
            brief,
            human: PrincipalId("human".into()),
            policy: SchedulingPolicy::EventDriven,
        })
    };
    let brief = store.blob_put(b"brief");
    let mut d = Driver::new(store, TaskId("T1".into()));
    match d.dispatch(&open(2, brief), Time(1)).expect("dispatch") {
        Response::Rejected(r) => {
            assert_eq!(r.code, Code::BadLiveTurns);
            assert!(r.detail.contains("event-driven"), "{}", r.detail);
        }
        other => panic!("expected a rejection, got {other:?}"),
    }
    assert!(!d.state.opened, "a refused open opens nothing");
}

/// The coder asks the tester a question and ends its turn. Turn-based regrants
/// the coder and the question waits for `test`; event-driven gives the tester
/// the next turn at once.
#[test]
fn a_named_question_gets_the_addressed_role_the_next_turn() {
    let script = |fx: &mut Fx| {
        fx.reach_build();
        posted(fx.post(Who::Coder, ask(Who::Tester, "which 429 cases matter?")));
        posted(fx.post(Who::Coder, pass()));
    };
    let mut turn_based = Fx::new();
    script(&mut turn_based);
    assert_eq!(holder(&turn_based), RoleId::from("coder"));

    let mut live = Fx::event_driven(four_roles());
    script(&mut live);
    assert_eq!(holder(&live), RoleId::from("tester"));
    assert_eq!(live.st().state, State::Build, "the state did not move");
    assert_eq!(kind(&live), TurnKind::Default);
}

/// Nothing new on the board: the quiet fallback is the default holder, exactly
/// as turn-based, so the task keeps moving.
#[test]
fn a_quiet_board_falls_back_to_the_default_holder() {
    let script = |fx: &mut Fx| {
        fx.reach_build();
        posted(fx.post(Who::Coder, pass()));
    };
    let mut turn_based = Fx::new();
    script(&mut turn_based);
    let mut live = Fx::event_driven(four_roles());
    script(&mut live);
    assert_eq!(holder(&turn_based), RoleId::from("coder"));
    assert_eq!(holder(&live), holder(&turn_based));
}

/// A role that only watches the artifact (an observer) is not selectable at one
/// live turn: the coder stores a diff and passes, and the coder goes again.
#[test]
fn an_observer_is_never_picked() {
    let mut fx = Fx::event_driven(four_roles());
    fx.reach_build();
    stored(fx.put(Who::Coder, ArtifactPayload::Diff, b"--- a\n+++ b\n"));
    posted(fx.post(Who::Coder, pass()));
    assert_eq!(holder(&fx), RoleId::from("coder"));
}

/// A broadcast verdict addresses no one, so the state's own server (the
/// reviewer) judges next, as under turn-based.
#[test]
fn a_broadcast_verdict_hands_the_turn_to_the_state_server() {
    let mut fx = Fx::event_driven(four_roles());
    fx.reach_test();
    stored(fx.report(RunStatus::Passed));
    posted(fx.verdict(fx.holder(State::Test), VerdictKind::Approve));
    assert_eq!(fx.st().state, State::Review);
    assert_eq!(holder(&fx), fx.holder(State::Review));
}

/// A whole task, start to merge gate, on both policies: same states, same
/// final state, and the human can approve the merge on the event-driven one.
#[test]
fn an_event_driven_task_reaches_approved() {
    let mut fx = Fx::event_driven(four_roles());
    fx.reach_approved();
    assert_eq!(fx.st().state, State::Approved);
}

fn two_finals() -> Vec<Role> {
    let mut roles = four_roles();
    roles.push(
        Role::new("second", 9)
            .post(&[
                MessageKind::Ask,
                MessageKind::Answer,
                MessageKind::Finding,
                MessageKind::Verdict,
                MessageKind::RequestDecision,
                MessageKind::Pass,
            ])
            .read_artifacts(&[
                ArtifactKind::Brief,
                ArtifactKind::Spec,
                ArtifactKind::Diff,
                ArtifactKind::Candidate,
                ArtifactKind::TestReport,
                ArtifactKind::Log,
            ])
            .verdict_at(&[Stage::Review])
            .final_decision(),
    );
    roles
}

/// The tester approves at `test` and names the second final role. Both
/// finalizers serve `review`.
fn tester_names_the_second_final(fx: &mut Fx) -> ArtId {
    let cand = fx.reach_test();
    stored(fx.report(RunStatus::Passed));
    let tester = fx.holder(State::Test);
    posted(fx.verdict_to(
        tester,
        VerdictKind::Approve,
        vec![Recipient::Role("second".into())],
    ));
    assert_eq!(fx.st().state, State::Review);
    cand
}

/// R17, through the real engine and no injected event. Turn-based never gives
/// the second finalizer a turn at `review`. Event-driven gives it the turn
/// because it was named; its `revise` goes to the log as an ordinary verdict,
/// blocks the gate for that candidate and run, and sends the task back to
/// `build`. The human cannot approve the merge.
#[test]
fn a_revise_from_a_second_final_role_blocks_the_merge_gate() {
    let mut turn_based = Fx::with_roles(two_finals());
    tester_names_the_second_final(&mut turn_based);
    assert_eq!(holder(&turn_based), RoleId::from("reviewer"));

    let mut fx = Fx::event_driven(two_finals());
    let cand = tester_names_the_second_final(&mut fx);
    let (run, _) = fx.selected_run();
    assert_eq!(holder(&fx), RoleId::from("second"), "named, so it judges");
    posted(fx.verdict("second", VerdictKind::Revise));

    assert!(
        fx.st().final_blocked(),
        "R17: a final role asked for revision"
    );
    assert!(!fx.st().final_approved());
    assert_eq!(fx.st().state, State::Build, "revise returns to build");
    match fx.human(HumanAction::Approve {
        gate: Gate::Merge,
        subject: cand,
        run: Some(run),
    }) {
        Response::Rejected(_) => {}
        other => panic!("the merge must be refused, got {other:?}"),
    }
}

/// A judge that may post a review verdict but is not a finalizer.
fn with_non_final_judge() -> Vec<Role> {
    let mut roles = two_finals();
    let i = roles.len() - 1;
    roles[i].id = RoleId::from("judge2");
    roles[i].final_decision = false;
    roles
}

fn approve_then_finalize(fx: &mut Fx) {
    assert_eq!(fx.st().state, State::Review);
    assert_eq!(holder(fx), RoleId::from("judge2"), "named, so it judges");
    posted(fx.verdict("judge2", VerdictKind::Approve));
    // Its approve is recorded and moves nothing: the finalizer still reviews.
    assert_eq!(fx.st().state, State::Review);
    assert!(!fx.st().final_approved());
    assert_eq!(
        holder(fx),
        fx.holder(State::Review),
        "the finalizer is next"
    );
    posted(fx.verdict(fx.holder(State::Review), VerdictKind::Approve));
    assert_eq!(fx.st().state, State::MergeGate);
    assert!(fx.st().final_approved() && !fx.st().final_blocked());
}

/// An addressed non-final judge approves at `review`. Under T11 that must not
/// open the merge gate: the task would sit there with nobody to schedule and
/// the human's approval refused. The finalizer reviews, then the human merges.
#[test]
fn a_non_final_judge_approving_at_review_does_not_strand_the_gate() {
    let mut fx = Fx::event_driven(with_non_final_judge());
    let cand = fx.reach_test();
    stored(fx.report(RunStatus::Passed));
    let tester = fx.holder(State::Test);
    posted(fx.verdict_to(
        tester,
        VerdictKind::Approve,
        vec![Recipient::Role("judge2".into())],
    ));
    approve_then_finalize(&mut fx);
    let (run, _) = fx.selected_run();
    ok(fx.human(HumanAction::Approve {
        gate: Gate::Merge,
        subject: cand,
        run: Some(run),
    }));
    assert_eq!(fx.st().state, State::Approved);
}

/// A finalizer's own revise still blocks (R17); the guard only withholds the
/// gate from a non-final approve.
#[test]
fn a_final_roles_approve_still_opens_the_gate() {
    let mut fx = Fx::event_driven(two_finals());
    fx.reach_merge_gate();
    assert_eq!(fx.st().state, State::MergeGate);
}
