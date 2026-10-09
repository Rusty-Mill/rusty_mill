#![allow(clippy::expect_used, clippy::unwrap_used)]
//! One test per row of review/traces.md.
mod common;
use common::*;
use rusty_bbp::*;

fn count<F: Fn(&Event) -> bool>(fx: &Fx, f: F) -> usize {
    fx.d.store
        .events(&fx.st().task, 0)
        .expect("log")
        .iter()
        .filter(|e| f(e))
        .count()
}

#[test]
fn tr_01_candidate_cannot_be_replaced_at_merge_gate() {
    let mut fx = Fx::new();
    let a = fx.reach_merge_gate();
    let diff = ArtId(2);
    let r = fx.put(
        Role::Coder,
        ArtifactPayload::Candidate(Candidate {
            base: "b".repeat(40),
            diffs: vec![diff],
        }),
        b"",
    );
    assert!(matches!(rejected(r), Code::StaleTurn | Code::TokenExpired));
    assert_eq!(fx.st().candidate, Some(a));
}

#[test]
fn tr_02_merge_approval_names_current_candidate() {
    let mut fx = Fx::new();
    let c = fx.reach_merge_gate();
    let (run, _) = fx.selected_run();
    assert_eq!(
        rejected(fx.human(HumanAction::Approve {
            gate: Gate::Merge,
            subject: ArtId(c.0 - 1),
            run: Some(run)
        })),
        Code::StaleSubject
    );
}

#[test]
fn tr_03_plan_approval_binds_to_requested_spec() {
    let mut fx = Fx::new();
    let spec = fx.reach_plan_gate();
    let brief = fx.st().brief.expect("brief");
    assert_eq!(
        rejected(fx.human(HumanAction::Approve {
            gate: Gate::Plan,
            subject: brief,
            run: None
        })),
        Code::StaleSubject
    );
    ok(fx.human(HumanAction::Approve {
        gate: Gate::Plan,
        subject: spec,
        run: None,
    }));
    assert_eq!(fx.st().approved_spec, Some(spec));
}

#[test]
fn tr_04_previous_turn_token_rejected_after_regrant() {
    let mut fx = Fx::new();
    fx.reach_build();
    let old = fx.tok(Role::Coder);
    posted(fx.post(Role::Coder, pass()));
    assert_eq!(fx.turn_role(), Some(Role::Coder));
    assert_ne!(fx.tok(Role::Coder), old);
    let op = fx.op();
    assert_eq!(
        rejected(fx.agent_with(
            Role::Coder,
            Some(old),
            op,
            AgentAction::Post(ask(Role::Tester, "?"))
        )),
        Code::StaleTurn
    );
}

#[test]
fn tr_05_tester_turn_waits_for_report() {
    let mut fx = Fx::new();
    fx.reach_test();
    assert_eq!(fx.turn_role(), None);
    assert!(fx.st().selected_run().is_some());
    stored(fx.report(RunStatus::Passed));
    assert_eq!(fx.turn_role(), Some(Role::Tester));
}

#[test]
fn tr_06_manifest_references_kind_checked() {
    let mut fx = Fx::new();
    let spec = fx.reach_build();
    assert_eq!(
        rejected(fx.put(
            Role::Coder,
            ArtifactPayload::Candidate(Candidate {
                base: "a".repeat(40),
                diffs: vec![spec]
            }),
            b"{}"
        )),
        Code::WrongKind
    );
}

#[test]
fn tr_07_zombie_run_report_rejected_after_rerun() {
    let mut fx = Fx::new();
    let c = fx.reach_review();
    let (old_run, old_secret) = fx.selected_run();
    let rev = fx.rev();
    ok(fx.human(HumanAction::Rerun {
        candidate: c,
        expected_rev: rev,
    }));
    assert_eq!(fx.st().state, State::Test);
    assert!(fx.st().verdicts.is_empty(), "verdicts staled");
    let (new_run, _) = fx.selected_run();
    assert_ne!(new_run, old_run);
    assert_eq!(
        rejected(fx.report_as(old_run, old_secret, RunStatus::Passed)),
        Code::StaleRun
    );
    stored(fx.report(RunStatus::Passed));
    assert_eq!(fx.turn_role(), Some(Role::Tester));
}

#[test]
fn tr_08_replanning_deselects_candidate() {
    let mut fx = Fx::new();
    let c = fx.reach_test();
    stored(fx.report(RunStatus::Passed));
    let rev = fx.rev();
    ok(fx.human(HumanAction::Reject {
        expected_rev: rev,
        target: State::Planning,
        reason: "scope changed".into(),
    }));
    assert_eq!(fx.st().state, State::Planning);
    assert_eq!(fx.st().approved_spec, None);
    assert_eq!(fx.st().candidate, None);
    assert_eq!(fx.turn_role(), Some(Role::Planner));
    // Old candidate cannot be judged or approved: the Tester holds no turn and there is no current candidate.
    let (run_before, _) = (RunId(1), ());
    let mut d = Draft::new(MessageKind::Verdict, "");
    d.body = Body::Verdict(Verdict {
        subject: c,
        run: run_before,
        verdict: VerdictKind::Approve,
        blocking: vec![],
        non_blocking: vec![],
    });
    d.refs = vec![Ref::art(c)];
    d.evidence = vec![Ref::art(c)];
    assert!(matches!(
        rejected(fx.post(Role::Tester, d)),
        Code::StaleTurn | Code::WrongState | Code::RoleForbidden
    ));
}

#[test]
fn tr_09_yield_and_return() {
    let mut fx = Fx::new();
    fx.reach_build();
    let before = fx.tok(Role::Coder);
    let q = posted(fx.post(
        Role::Coder,
        ask(Role::Tester, "Is AC2 testable as written?"),
    ));
    posted(fx.post(Role::Coder, yield_to(Role::Tester)));
    assert_eq!(fx.turn_role(), Some(Role::Tester));
    let brief = fx.st().brief.expect("brief");
    posted(fx.post(
        Role::Tester,
        Draft::new(MessageKind::Answer, "Yes.").reply_to(q),
    ));
    // Consultation turns may not propose.
    assert_eq!(
        rejected(fx.post(
            Role::Tester,
            Draft::new(MessageKind::Propose, "x").refs(vec![Ref::art(brief)])
        )),
        Code::RoleForbidden
    );
    posted(fx.post(Role::Tester, pass()));
    assert_eq!(fx.turn_role(), Some(Role::Coder));
    assert_ne!(fx.tok(Role::Coder), before, "fresh token on return");
}

#[test]
fn tr_10_replay_after_turn_ended_returns_original() {
    let mut fx = Fx::new();
    fx.reach_build();
    let diff = stored(fx.put(Role::Coder, ArtifactPayload::Diff, b"d"));
    let tok = fx.tok(Role::Coder);
    let op = fx.op();
    let payload = ArtifactPayload::Candidate(Candidate {
        base: "a".repeat(40),
        diffs: vec![diff],
    });
    let blob =
        fx.d.store
            .blob_put(&encode_artifact(&payload, b"").expect("encode"));
    let action = AgentAction::PutArtifact { blob, payload };
    let first = fx.agent_with(Role::Coder, Some(tok), op.clone(), action.clone());
    assert!(matches!(first, Response::Stored(_)));
    assert_eq!(fx.st().state, State::Test);
    let rev = fx.rev();
    let again = fx.agent_with(Role::Coder, Some(tok), op, action);
    assert_eq!(first, again);
    assert_eq!(fx.rev(), rev);
}

#[test]
fn tr_11_resume_to_review_requires_tester_approve() {
    let mut fx = Fx::new();
    fx.reach_test();
    stored(fx.report(RunStatus::Passed));
    // Escalate via a budget: lowering the turns limit under current spend escalates on that very command (E3).
    ok(fx.human(HumanAction::BudgetExtended {
        field: BudgetField::Turns,
        limit: 1,
    }));
    assert_eq!(fx.st().state, State::Escalated);
    ok(fx.human(HumanAction::BudgetExtended {
        field: BudgetField::Turns,
        limit: 60,
    }));
    let rev = fx.rev();
    assert_eq!(
        rejected(fx.human(HumanAction::Resume {
            target: State::Review,
            expected_rev: rev
        })),
        Code::NotReviewed
    );
    let rev = fx.rev();
    ok(fx.human(HumanAction::Resume {
        target: State::Test,
        expected_rev: rev,
    }));
    assert_eq!(fx.turn_role(), Some(Role::Tester));
}

#[test]
fn tr_12_read_budgets() {
    let mut fx = Fx::new();
    for _ in 0..30 {
        fx.agent(
            Role::Planner,
            AgentAction::Read {
                after: None,
                limit: 20,
            },
        );
    }
    assert_eq!(
        rejected(fx.agent(
            Role::Planner,
            AgentAction::Read {
                after: None,
                limit: 20
            }
        )),
        Code::TurnBudgetExhausted
    );
    assert_eq!(fx.st().spent.reads, 30);
    assert_eq!(fx.st().spent.reads_this_turn, 0);
}

#[test]
fn tr_13_error_run_escalates() {
    let mut fx = Fx::new();
    fx.reach_test();
    stored(fx.report(RunStatus::Error));
    assert_eq!(fx.st().state, State::Escalated);
    assert_eq!(fx.st().escalated_from, Some(State::Test));
    assert_eq!(fx.turn_role(), None);
    // Resume to test starts a fresh run.
    let rev = fx.rev();
    ok(fx.human(HumanAction::Resume {
        target: State::Test,
        expected_rev: rev,
    }));
    assert_eq!(fx.st().selected_run().and_then(|r| r.status), None);
    assert_eq!(fx.selected_run().0, RunId(2));
}

#[test]
fn tr_14_yield_to_non_consultable_role() {
    let mut fx = Fx::new();
    fx.reach_build();
    assert_eq!(
        rejected(fx.post(Role::Coder, yield_to(Role::Reviewer))),
        Code::WrongState
    );
}

#[test]
fn tr_15_stale_rev_versus_replay() {
    let mut fx = Fx::new();
    fx.reach_approved();
    let rev = fx.rev();
    let op = fx.op();
    let action = HumanAction::Reject {
        expected_rev: rev,
        target: State::Build,
        reason: "base drift".into(),
    };
    assert_eq!(
        fx.run(&Command::Human {
            op: op.clone(),
            action: action.clone()
        }),
        Response::Ok
    );
    assert_eq!(fx.st().state, State::Build);
    let after = fx.rev();
    // Same op: replay, no effect.
    assert_eq!(fx.run(&Command::Human { op, action }), Response::Ok);
    assert_eq!(fx.rev(), after);
    // New op with the old rev: stale.
    let op2 = fx.op();
    assert_eq!(
        rejected(fx.run(&Command::Human {
            op: op2,
            action: HumanAction::Reject {
                expected_rev: rev,
                target: State::Planning,
                reason: "again".into()
            }
        })),
        Code::StaleRev
    );
}

#[test]
fn tr_16_budget_escalates_once() {
    let mut fx = Fx::new();
    ok(fx.human(HumanAction::BudgetExtended {
        field: BudgetField::Messages,
        limit: 1,
    }));
    posted(fx.post(Role::Planner, ask(Role::Coder, "one")));
    assert_eq!(fx.st().state, State::Planning);
    posted(fx.post(Role::Planner, ask(Role::Coder, "two")));
    assert_eq!(
        fx.st().state,
        State::Escalated,
        "crossing write accepted, then escalated"
    );
    assert_eq!(count(&fx, |e| matches!(e, Event::Escalated { .. })), 1);
    fx.tick();
    assert_eq!(
        count(&fx, |e| matches!(e, Event::Escalated { .. })),
        1,
        "no re-escalation on tick"
    );
}

#[test]
fn tr_17_pending_request_blocks_default_turn_after_rebuild() {
    let mut fx = Fx::new();
    fx.reach_build();
    let brief = fx.st().brief.expect("brief");
    posted(
        fx.post(
            Role::Coder,
            Draft::new(MessageKind::RequestDecision, "Which retry policy?")
                .to(Recipient::Human)
                .refs(vec![Ref::art(brief)]),
        ),
    );
    assert_eq!(fx.turn_role(), None);
    fx.d.reload().expect("reload");
    fx.run(&Command::AbortTurn);
    fx.tick();
    assert_eq!(fx.turn_role(), None, "request still open");
    assert!(fx.st().pending_request.is_some());
}

#[test]
fn tr_18_human_answer_settles_request_and_regrants() {
    let mut fx = Fx::new();
    fx.reach_build();
    let brief = fx.st().brief.expect("brief");
    let req = posted(
        fx.post(
            Role::Coder,
            Draft::new(MessageKind::RequestDecision, "Which retry policy?")
                .to(Recipient::Human)
                .refs(vec![Ref::art(brief)]),
        ),
    );
    posted(fx.human(HumanAction::Post(
        Draft::new(MessageKind::Answer, "Exponential, 5 tries.").reply_to(req),
    )));
    assert!(fx.st().pending_request.is_none());
    assert_eq!(fx.turn_role(), Some(Role::Coder));
}

#[test]
fn tr_19_late_command_processed_as_if_tick_preceded() {
    let mut fx = Fx::new();
    let tok = fx.tok(Role::Planner);
    fx.advance(60_000);
    let op = fx.op();
    assert_eq!(
        rejected(fx.agent_with(
            Role::Planner,
            Some(tok),
            op,
            AgentAction::Post(ask(Role::Coder, "?"))
        )),
        Code::StaleTurn
    );
    assert_eq!(
        count(&fx, |e| matches!(
            e,
            Event::TurnEnded {
                cause: rusty_bbp::event::TurnEnd::Deadline,
                ..
            }
        )),
        1
    );
    assert_eq!(
        fx.turn_role(),
        Some(Role::Planner),
        "regranted by the deadline, before the command"
    );
}

#[test]
fn tr_20_agent_decision_forbidden() {
    let mut fx = Fx::new();
    let brief = fx.st().brief.expect("brief");
    let p = posted(fx.post(
        Role::Planner,
        Draft::new(MessageKind::Propose, "x").refs(vec![Ref::art(brief)]),
    ));
    let mut d = Draft::new(MessageKind::Decision, "");
    d.body = Body::Decision(Decision {
        subject: p,
        outcome: Outcome::Accept,
        note: String::new(),
    });
    d.refs = vec![Ref::Msg(p)];
    assert_eq!(rejected(fx.post(Role::Planner, d)), Code::DecisionForbidden);
}

#[test]
fn tr_21_reviewer_sees_only_human_messages_to_it() {
    let mut fx = Fx::new();
    fx.reach_review();
    posted(fx.human(HumanAction::Post(ask(
        Role::Reviewer,
        "Focus on the retry path.",
    ))));
    match fx.agent(
        Role::Reviewer,
        AgentAction::Read {
            after: None,
            limit: 20,
        },
    ) {
        Response::Read { messages, .. } => {
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].from, PrincipalId("human".into()));
        }
        other => panic!("{other:?}"),
    }
    assert!(
        fx.st().messages.len() > 1,
        "other messages exist but are hidden"
    );
}

#[test]
fn tr_22_coder_cannot_write_report() {
    let mut fx = Fx::new();
    fx.reach_build();
    let rep = Report {
        candidate: ArtId(1),
        run: RunId(1),
        profile_digest: DIGEST,
        status: RunStatus::Passed,
        profiles: vec![],
        tree: None,
        log: ArtId(1),
    };
    assert_eq!(
        rejected(fx.put(Role::Coder, ArtifactPayload::TestReport(rep), b"r")),
        Code::KindForbidden
    );
}

#[test]
fn tr_23_merge_receipt_names_approved_candidate() {
    let mut fx = Fx::new();
    let c = fx.reach_approved();
    assert_eq!(
        rejected(fx.human(HumanAction::MergeReceipt {
            candidate: ArtId(c.0 - 1),
            revision: "deadbeef".into()
        })),
        Code::StaleSubject
    );
    ok(fx.human(HumanAction::MergeReceipt {
        candidate: c,
        revision: "deadbeef".into(),
    }));
    assert_eq!(fx.st().state, State::Closed);
}

#[test]
fn tr_24_rerun_starts_exactly_one_run() {
    let mut fx = Fx::new();
    let c = fx.reach_test();
    let before = count(&fx, |e| matches!(e, Event::RunStarted { .. }));
    let rev = fx.rev();
    ok(fx.human(HumanAction::Rerun {
        candidate: c,
        expected_rev: rev,
    }));
    assert_eq!(
        count(&fx, |e| matches!(e, Event::RunStarted { .. })),
        before + 1
    );
}

#[test]
fn tr_25_fragment_cap() {
    let mut fx = Fx::new();
    let brief = fx.st().brief.expect("brief");
    let r = Ref::Art {
        id: brief,
        fragment: Some("x".repeat(200)),
    };
    assert_eq!(
        rejected(fx.post(
            Role::Planner,
            Draft::new(MessageKind::Propose, "p").refs(vec![r])
        )),
        Code::FragmentTooLong
    );
}

#[test]
fn tr_26_yield_does_not_nest() {
    let mut fx = Fx::new();
    fx.reach_build();
    posted(fx.post(Role::Coder, yield_to(Role::Tester)));
    assert_eq!(
        rejected(fx.post(Role::Tester, yield_to(Role::Planner))),
        Code::YieldNested
    );
}

#[test]
fn tr_27_spec_must_reference_brief() {
    let mut fx = Fx::new();
    assert_eq!(
        rejected(fx.put(
            Role::Planner,
            ArtifactPayload::Spec { brief: ArtId(42) },
            b"s"
        )),
        Code::RefUnresolved
    );
}

#[test]
fn tr_28_resume_to_approved_after_extension() {
    let mut fx = Fx::new();
    let c = fx.reach_approved();
    ok(fx.human(HumanAction::BudgetExtended {
        field: BudgetField::Messages,
        limit: 0,
    }));
    posted(fx.human(HumanAction::Post(ask(
        Role::Coder,
        "Anything else to note?",
    ))));
    posted(fx.post(
        Role::Coder,
        Draft::new(MessageKind::Answer, "No.").reply_to(MsgId(fx.st().messages.len() as u64)),
    ));
    assert_eq!(fx.st().state, State::Escalated);
    let rev = fx.rev();
    assert_eq!(
        rejected(fx.human(HumanAction::Resume {
            target: State::Approved,
            expected_rev: rev
        })),
        Code::BudgetExhausted
    );
    ok(fx.human(HumanAction::BudgetExtended {
        field: BudgetField::Messages,
        limit: 100,
    }));
    let rev = fx.rev();
    ok(fx.human(HumanAction::Resume {
        target: State::Approved,
        expected_rev: rev,
    }));
    ok(fx.human(HumanAction::MergeReceipt {
        candidate: c,
        revision: "abc".into(),
    }));
    assert_eq!(fx.st().state, State::Closed);
}

#[test]
fn tr_29_store_conflict_reloads_and_recomputes() {
    let mut fx = Fx::new();
    // Simulate another writer: append an event behind the driver's back.
    let task = fx.st().task.clone();
    let rev = fx.rev();
    fx.d.store
        .append(
            &task,
            rev,
            &[Event::BudgetExtended {
                field: BudgetField::Bytes,
                limit: 9_000_000,
            }],
        )
        .expect("append");
    let r = fx.post(Role::Planner, ask(Role::Coder, "?"));
    assert!(matches!(r, Response::Posted(_)));
    assert_eq!(
        fx.st().budget.bytes,
        9_000_000,
        "reloaded state includes the foreign event"
    );
}

#[test]
fn tr_30_new_candidate_stales_verdicts() {
    let mut fx = Fx::new();
    fx.reach_test();
    stored(fx.report(RunStatus::Passed));
    posted(fx.verdict(Role::Tester, VerdictKind::Revise));
    assert_eq!(fx.st().state, State::Build);
    assert_eq!(fx.st().verdicts.len(), 1);
    fx.candidate();
    assert!(fx.st().verdicts.is_empty());
    assert_eq!(fx.st().state, State::Test);
}

#[test]
fn tr_31_revise_then_replan_leaves_no_orphan_verdict() {
    // Found by the model test: Tester revise -> build; human rejection to planning
    // deselected the candidate but kept its verdict record.
    let mut fx = Fx::new();
    fx.reach_test();
    stored(fx.report(RunStatus::Passed));
    posted(fx.verdict(Role::Tester, VerdictKind::Revise));
    assert_eq!(fx.st().state, State::Build);
    let rev = fx.rev();
    ok(fx.human(HumanAction::Reject {
        expected_rev: rev,
        target: State::Planning,
        reason: "replan".into(),
    }));
    assert_eq!(fx.st().candidate, None);
    assert!(fx.st().verdicts.is_empty());
}
