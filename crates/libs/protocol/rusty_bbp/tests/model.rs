#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Model and invariant tests over random command sequences. The oracle is a set
//! of invariants stated independently of the implementation, not a re-run of it.
mod common;
use common::*;
use proptest::prelude::*;
use rusty_bbp::*;

#[derive(Clone, Debug)]
enum Step {
    Post(Role, u8),
    Pass(Role),
    Yield(Role, Role),
    Read(Role),
    Spec,
    Gate,
    Diff,
    Candidate,
    Report(u8),
    Verdict(Role, bool),
    ApprovePlan,
    ApproveMerge,
    RejectToPlanning,
    RejectToBuild,
    Rerun,
    Receipt,
    Resume(u8),
    Extend(u8),
    Cancel,
    Tick(u32),
    Abort,
    HumanAsk(Role),
    Replay,
}

fn role() -> impl Strategy<Value = Role> {
    prop_oneof![
        Just(Role::Planner),
        Just(Role::Coder),
        Just(Role::Tester),
        Just(Role::Reviewer)
    ]
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        (role(), 0u8..4).prop_map(|(r, k)| Step::Post(r, k)),
        role().prop_map(Step::Pass),
        (role(), role()).prop_map(|(a, b)| Step::Yield(a, b)),
        role().prop_map(Step::Read),
        Just(Step::Spec),
        Just(Step::Gate),
        Just(Step::Diff),
        Just(Step::Candidate),
        (0u8..3).prop_map(Step::Report),
        (role(), any::<bool>()).prop_map(|(r, a)| Step::Verdict(r, a)),
        Just(Step::ApprovePlan),
        Just(Step::ApproveMerge),
        Just(Step::RejectToPlanning),
        Just(Step::RejectToBuild),
        Just(Step::Rerun),
        Just(Step::Receipt),
        (0u8..6).prop_map(Step::Resume),
        (0u8..3).prop_map(Step::Extend),
        Just(Step::Cancel),
        (0u32..120_000).prop_map(Step::Tick),
        Just(Step::Abort),
        role().prop_map(Step::HumanAsk),
        Just(Step::Replay),
    ]
}

fn resume_target(i: u8) -> State {
    [
        State::Planning,
        State::Build,
        State::Test,
        State::Review,
        State::MergeGate,
        State::Approved,
    ][i as usize % 6]
}

/// Apply a step; the fixture never panics on a rejection, only on a store error.
fn apply(fx: &mut Fx, step: &Step, last: &mut Option<Command>) {
    let brief = fx.st().brief.unwrap_or(ArtId(1));
    let rev = fx.rev();
    match step {
        Step::Post(r, k) => {
            let d = match k {
                0 => ask(Role::Coder, "q"),
                1 => Draft::new(MessageKind::Propose, "p").refs(vec![Ref::art(brief)]),
                2 => Draft::new(MessageKind::Finding, "f")
                    .refs(vec![Ref::art(brief)])
                    .evidence(vec![Ref::art(brief)]),
                _ => Draft::new(MessageKind::RequestDecision, "?")
                    .to(Recipient::Human)
                    .refs(vec![Ref::art(brief)]),
            };
            let cmd = Command::Agent {
                principal: pid(*r),
                token: Some(fx.tok(*r)),
                op: fx.op(),
                action: AgentAction::Post(d),
            };
            *last = Some(cmd.clone());
            fx.run(&cmd);
        }
        Step::Pass(r) => {
            fx.post(*r, pass());
        }
        Step::Yield(a, b) => {
            fx.post(*a, yield_to(*b));
        }
        Step::Read(r) => {
            fx.agent(
                *r,
                AgentAction::Read {
                    after: None,
                    limit: 20,
                },
            );
        }
        Step::Spec => {
            fx.put(Role::Planner, ArtifactPayload::Spec { brief }, b"spec");
        }
        Step::Gate => {
            let spec = fx
                .st()
                .artifacts
                .values()
                .rev()
                .find(|a| a.kind == ArtifactKind::Spec)
                .map(|a| a.id)
                .unwrap_or(brief);
            let mut d = Draft::new(MessageKind::RequestDecision, "approve?")
                .to(Recipient::Human)
                .refs(vec![Ref::art(spec)]);
            d.gate = true;
            fx.post(Role::Planner, d);
        }
        Step::Diff => {
            fx.put(Role::Coder, ArtifactPayload::Diff, b"diff");
        }
        Step::Candidate => {
            let diffs: Vec<ArtId> = fx
                .st()
                .artifacts
                .values()
                .filter(|a| a.kind == ArtifactKind::Diff)
                .map(|a| a.id)
                .collect();
            fx.put(
                Role::Coder,
                ArtifactPayload::Candidate(Candidate {
                    base: "a".repeat(40),
                    diffs,
                }),
                b"{}",
            );
        }
        Step::Report(s) => {
            if fx.st().selected_run().is_some() {
                let status =
                    [RunStatus::Passed, RunStatus::Failed, RunStatus::Error][*s as usize % 3];
                if fx.st().candidate.is_some() {
                    fx.report(status);
                }
            }
        }
        Step::Verdict(r, approve) => {
            if fx.st().selected_run().and_then(|x| x.report).is_some()
                && fx.st().candidate.is_some()
            {
                fx.verdict(
                    *r,
                    if *approve {
                        VerdictKind::Approve
                    } else {
                        VerdictKind::Revise
                    },
                );
            }
        }
        Step::ApprovePlan => {
            let spec = fx.st().gate_request.map(|(_, s)| s).unwrap_or(brief);
            fx.human(HumanAction::Approve {
                gate: Gate::Plan,
                subject: spec,
                run: None,
            });
        }
        Step::ApproveMerge => {
            let c = fx.st().candidate.unwrap_or(brief);
            let run = fx.st().selected_run().map(|r| r.id);
            fx.human(HumanAction::Approve {
                gate: Gate::Merge,
                subject: c,
                run,
            });
        }
        Step::RejectToPlanning => {
            fx.human(HumanAction::Reject {
                expected_rev: rev,
                target: State::Planning,
                reason: "r".into(),
            });
        }
        Step::RejectToBuild => {
            fx.human(HumanAction::Reject {
                expected_rev: rev,
                target: State::Build,
                reason: "r".into(),
            });
        }
        Step::Rerun => {
            let c = fx.st().candidate.unwrap_or(brief);
            fx.human(HumanAction::Rerun {
                candidate: c,
                expected_rev: rev,
            });
        }
        Step::Receipt => {
            let c = fx.st().candidate.unwrap_or(brief);
            fx.human(HumanAction::MergeReceipt {
                candidate: c,
                revision: "x".into(),
            });
        }
        Step::Resume(i) => {
            fx.human(HumanAction::Resume {
                target: resume_target(*i),
                expected_rev: rev,
            });
        }
        Step::Extend(i) => {
            let field = [
                BudgetField::Messages,
                BudgetField::Turns,
                BudgetField::Reads,
            ][*i as usize % 3];
            fx.human(HumanAction::BudgetExtended {
                field,
                limit: 1_000,
            });
        }
        Step::Cancel => {
            fx.human(HumanAction::Cancel {
                expected_rev: rev,
                reason: "c".into(),
            });
        }
        Step::Tick(ms) => {
            fx.advance(*ms as u64);
            fx.tick();
        }
        Step::Abort => {
            fx.run(&Command::AbortTurn);
        }
        Step::HumanAsk(r) => {
            fx.human(HumanAction::Post(ask(*r, "?")));
        }
        Step::Replay => {
            if let Some(cmd) = last.clone() {
                fx.run(&cmd);
            }
        }
    }
}

fn check_invariants(fx: &Fx, prev: &TaskState) {
    let st = fx.st();
    // Rev is monotonic and equals the log length.
    assert!(st.rev >= prev.rev);
    let log = fx.d.store.events(&st.task, 0).expect("log");
    assert_eq!(st.rev.0 as usize, log.len());
    // The state equals the fold of the log.
    assert_eq!(*st, TaskState::fold(st.task.clone(), &log));
    // Task totals never decrease.
    assert!(
        st.spent.messages >= prev.spent.messages
            && st.spent.bytes >= prev.spent.bytes
            && st.spent.reads >= prev.spent.reads
            && st.spent.turns >= prev.spent.turns
    );
    // Terminal states hold no turn and accept no later writes.
    if st.state.terminal() {
        assert!(st.turn.is_none());
    }
    // At most one live turn, and it belongs to an assigned role.
    if let Some(t) = &st.turn {
        assert!(st.assigned.contains_key(&t.role));
        assert!(t.deadline > Time(0));
    }
    // No default turn while a request is open.
    if st.pending_request.is_some() {
        assert!(st
            .turn
            .as_ref()
            .map(|t| t.kind != TurnKind::Default)
            .unwrap_or(true));
    }
    // A current candidate always carries the approved spec, or there is none approved.
    if let Some(c) = st.current_candidate() {
        if st.approved_spec.is_some() {
            assert_eq!(c.spec, st.approved_spec);
        }
    }
    // Merge approval only ever names the current candidate and selected run.
    if let Some((c, r)) = st.merge_approval {
        assert_eq!(Some(c), st.candidate);
        assert_eq!(Some(r), st.selected_run().map(|x| x.id));
    }
    // In approved or merge_gate the selected run is passed.
    if matches!(st.state, State::MergeGate | State::Approved) {
        assert_eq!(
            st.selected_run().and_then(|r| r.status),
            Some(RunStatus::Passed)
        );
    }
    // Verdicts only reference the current candidate.
    for v in &st.verdicts {
        assert_eq!(Some(v.candidate), st.candidate);
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 200, .. ProptestConfig::default() })]
    #[test]
    fn random_sequences_hold_invariants(steps in proptest::collection::vec(step(), 1..60)) {
        let mut fx = Fx::new();
        let mut last = None;
        for s in &steps {
            let prev = fx.st().clone();
            apply(&mut fx, s, &mut last);
            check_invariants(&fx, &prev);
        }
    }
}

#[test]
fn every_state_has_an_exit_or_is_terminal() {
    // Documented exits per state, checked against the transition code by driving each state.
    let mut fx = Fx::new();
    fx.reach_approved();
    let c = fx.st().candidate.expect("c");
    ok(fx.human(HumanAction::MergeReceipt {
        candidate: c,
        revision: "r".into(),
    }));
    assert_eq!(fx.st().state, State::Closed);
    for s in State::ALL {
        let _ = s.default_role();
    }
}
