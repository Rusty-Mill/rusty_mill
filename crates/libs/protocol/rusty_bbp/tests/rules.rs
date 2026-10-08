#![allow(clippy::expect_used, clippy::unwrap_used)]
//! One passing and at least one adversarial failing case per rule.
mod common;
use common::*;
use rusty_bbp::*;

#[test]
fn r1_body_shape_must_match_kind() {
    let mut fx = Fx::new();
    let mut d = Draft::new(MessageKind::Ask, "x");
    d.body = Body::Decision(Decision {
        subject: MsgId(1),
        outcome: Outcome::Accept,
        note: String::new(),
    });
    assert_eq!(rejected(fx.post(Role::Planner, d)), Code::UnknownKind);
    posted(fx.post(Role::Planner, ask(Role::Coder, "ok?")));
}

#[test]
fn r2_refs_required() {
    let mut fx = Fx::new();
    assert_eq!(
        rejected(fx.post(Role::Planner, Draft::new(MessageKind::Propose, "x"))),
        Code::RefsRequired
    );
    let brief = fx.st().brief.expect("brief");
    posted(fx.post(
        Role::Planner,
        Draft::new(MessageKind::Propose, "x").refs(vec![Ref::art(brief)]),
    ));
}

#[test]
fn r3_evidence_required_and_bound() {
    let mut fx = Fx::new();
    let brief = fx.st().brief.expect("brief");
    assert_eq!(
        rejected(fx.post(
            Role::Planner,
            Draft::new(MessageKind::Finding, "x").refs(vec![Ref::art(brief)])
        )),
        Code::EvidenceRequired
    );
    fx.reach_test();
    stored(fx.report(RunStatus::Passed));
    // Verdict whose evidence does not cite the run's report.
    let cand = fx.st().candidate.expect("c");
    let (run, _) = fx.selected_run();
    let mut d = Draft::new(MessageKind::Verdict, "");
    d.body = Body::Verdict(Verdict {
        subject: cand,
        run,
        verdict: VerdictKind::Approve,
        blocking: vec![],
        non_blocking: vec![],
    });
    d.refs = vec![Ref::art(cand)];
    d.evidence = vec![Ref::art(cand)];
    assert_eq!(rejected(fx.post(Role::Tester, d)), Code::EvidenceUnbound);
    posted(fx.verdict(Role::Tester, VerdictKind::Approve));
}

#[test]
fn r3_revise_requires_blocking_and_approve_requires_passed() {
    let mut fx = Fx::new();
    fx.reach_test();
    stored(fx.report(RunStatus::Failed));
    assert_eq!(
        rejected(fx.verdict(Role::Tester, VerdictKind::Approve)),
        Code::NotPassed
    );
    let cand = fx.st().candidate.expect("c");
    let (run, _) = fx.selected_run();
    let report = fx.report_id();
    let mut d = Draft::new(MessageKind::Verdict, "");
    d.body = Body::Verdict(Verdict {
        subject: cand,
        run,
        verdict: VerdictKind::Revise,
        blocking: vec![],
        non_blocking: vec![],
    });
    d.refs = vec![Ref::art(cand)];
    d.evidence = vec![Ref::art(report)];
    assert_eq!(rejected(fx.post(Role::Tester, d)), Code::BlockingRequired);
    posted(fx.verdict(Role::Tester, VerdictKind::Revise));
    assert_eq!(fx.st().state, State::Build);
    assert_eq!(fx.st().iteration, 1);
}

#[test]
fn r4_refs_resolve_and_fragment_cap() {
    let mut fx = Fx::new();
    assert_eq!(
        rejected(fx.post(
            Role::Planner,
            Draft::new(MessageKind::Propose, "x").refs(vec![Ref::art(ArtId(99))])
        )),
        Code::RefUnresolved
    );
    let brief = fx.st().brief.expect("brief");
    let long = Ref::Art {
        id: brief,
        fragment: Some("L".repeat(129)),
    };
    assert_eq!(
        rejected(fx.post(
            Role::Planner,
            Draft::new(MessageKind::Propose, "x").refs(vec![long])
        )),
        Code::FragmentTooLong
    );
    let short = Ref::Art {
        id: brief,
        fragment: Some("L1-L5".into()),
    };
    posted(fx.post(
        Role::Planner,
        Draft::new(MessageKind::Propose, "x").refs(vec![short]),
    ));
}

#[test]
fn r5_state_and_role_gates() {
    let mut fx = Fx::new();
    // Planner may not post a verdict; Tester may not propose.
    assert_eq!(
        rejected(fx.post(Role::Planner, Draft::new(MessageKind::Verdict, ""))),
        Code::RoleForbidden
    );
    fx.reach_build();
    assert_eq!(fx.turn_role(), Some(Role::Coder));
    let brief = fx.st().brief.expect("brief");
    // Tester holds no turn in build: token check fails before anything else.
    assert_eq!(
        rejected(fx.post(
            Role::Tester,
            Draft::new(MessageKind::Propose, "x").refs(vec![Ref::art(brief)])
        )),
        Code::StaleTurn
    );
    // No agent holds a turn at a gate.
    let mut fx2 = Fx::new();
    fx2.reach_merge_gate();
    assert_eq!(fx2.turn_role(), None);
}

#[test]
fn r6_verdict_role_and_subject() {
    let mut fx = Fx::new();
    fx.reach_test();
    stored(fx.report(RunStatus::Passed));
    // Reviewer in test: no turn, so stale turn.
    assert_eq!(
        rejected(fx.verdict(Role::Reviewer, VerdictKind::Approve)),
        Code::StaleTurn
    );
    // Subject missing from refs.
    let cand = fx.st().candidate.expect("c");
    let (run, _) = fx.selected_run();
    let report = fx.report_id();
    let mut d = Draft::new(MessageKind::Verdict, "");
    d.body = Body::Verdict(Verdict {
        subject: cand,
        run,
        verdict: VerdictKind::Approve,
        blocking: vec![],
        non_blocking: vec![],
    });
    d.refs = vec![Ref::art(report)];
    d.evidence = vec![Ref::art(report)];
    assert_eq!(rejected(fx.post(Role::Tester, d)), Code::SubjectMissing);
}

#[test]
fn r7_decisions_are_human_only() {
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
        note: "ok".into(),
    });
    d.refs = vec![Ref::Msg(p)];
    assert_eq!(
        rejected(fx.post(Role::Planner, d.clone())),
        Code::DecisionForbidden
    );
    posted(fx.human(HumanAction::Post(d)));
}

#[test]
fn r8_body_cap() {
    let mut fx = Fx::new();
    assert_eq!(
        rejected(fx.post(Role::Planner, ask(Role::Coder, &"x".repeat(1_201)))),
        Code::BodyTooLong
    );
    posted(fx.post(Role::Planner, ask(Role::Coder, &"x".repeat(1_200))));
}

#[test]
fn r9_recipients() {
    let mut fx = Fx::new();
    let brief = fx.st().brief.expect("brief");
    let d = Draft::new(MessageKind::RequestDecision, "?")
        .to(Recipient::Role(Role::Coder))
        .refs(vec![Ref::art(brief)]);
    assert_eq!(rejected(fx.post(Role::Planner, d)), Code::HumanRequired);
}

#[test]
fn r10_answer_replies_to_ask() {
    let mut fx = Fx::new();
    let brief = fx.st().brief.expect("brief");
    let p = posted(fx.post(
        Role::Planner,
        Draft::new(MessageKind::Propose, "x").refs(vec![Ref::art(brief)]),
    ));
    assert_eq!(
        rejected(fx.post(
            Role::Planner,
            Draft::new(MessageKind::Answer, "a").reply_to(p)
        )),
        Code::BadReplyTo
    );
    assert_eq!(
        rejected(fx.post(
            Role::Planner,
            Draft::new(MessageKind::Answer, "a").reply_to(MsgId(77))
        )),
        Code::BadReplyTo
    );
}

#[test]
fn r11_turn_read_budget_ends_turn() {
    let mut fx = Fx::new();
    for _ in 0..30 {
        assert!(matches!(
            fx.agent(
                Role::Planner,
                AgentAction::Read {
                    after: None,
                    limit: 20
                }
            ),
            Response::Read { .. }
        ));
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
    // Turn ended and regranted by the default schedule: a fresh token works.
    assert_eq!(fx.turn_role(), Some(Role::Planner));
    assert!(matches!(
        fx.agent(
            Role::Planner,
            AgentAction::Read {
                after: None,
                limit: 20
            }
        ),
        Response::Read { .. }
    ));
}

#[test]
fn r11_artifact_read_deduped_per_turn() {
    let mut fx = Fx::new();
    let brief = fx.st().brief.expect("brief");
    let before = fx.st().spent.reads;
    assert!(matches!(
        fx.agent(Role::Planner, AgentAction::GetArtifact { id: brief }),
        Response::Artifact(_)
    ));
    assert!(matches!(
        fx.agent(Role::Planner, AgentAction::GetArtifact { id: brief }),
        Response::Artifact(_)
    ));
    assert_eq!(fx.st().spent.reads, before + 1);
}

#[test]
fn r13_no_agent_path_creates_human_events() {
    // The type system does it: AgentAction has no approval variant. Assert the enum surface.
    let v = rusty_serde::json::to_string(&AgentAction::TaskCard).expect("json");
    assert!(!v.contains("Approve"));
}

#[test]
fn r14_token_required_and_turn_bound() {
    let mut fx = Fx::new();
    let op = fx.op();
    assert_eq!(
        rejected(fx.agent_with(
            Role::Planner,
            None,
            op,
            AgentAction::Post(ask(Role::Coder, "?"))
        )),
        Code::BadToken
    );
    let op = fx.op();
    assert_eq!(
        rejected(fx.agent_with(
            Role::Coder,
            Some(fx.tok(Role::Planner)),
            op,
            AgentAction::Post(ask(Role::Planner, "?"))
        )),
        Code::StaleTurn
    );
    // Card needs no token.
    let op = fx.op();
    assert!(matches!(
        fx.agent_with(Role::Coder, None, op, AgentAction::TaskCard),
        Response::Card(_)
    ));
}

#[test]
fn r15_op_replay_and_conflict() {
    let mut fx = Fx::new();
    let op = fx.op();
    let tok = fx.tok(Role::Planner);
    let r1 = fx.agent_with(
        Role::Planner,
        Some(tok),
        op.clone(),
        AgentAction::Post(ask(Role::Coder, "?")),
    );
    let rev = fx.rev();
    let r2 = fx.agent_with(
        Role::Planner,
        Some(tok),
        op.clone(),
        AgentAction::Post(ask(Role::Coder, "?")),
    );
    assert_eq!(r1, r2);
    assert_eq!(fx.rev(), rev, "replay appends nothing");
    assert_eq!(
        rejected(fx.agent_with(
            Role::Planner,
            Some(tok),
            op,
            AgentAction::Post(ask(Role::Coder, "different"))
        )),
        Code::OpConflict
    );
}

#[test]
fn r16_gating_table() {
    let mut fx = Fx::new();
    // Coder cannot write a spec; Planner cannot write a test report; spec only in planning.
    let brief = fx.st().brief.expect("brief");
    assert_eq!(
        rejected(fx.put(Role::Coder, ArtifactPayload::Spec { brief }, b"s")),
        Code::StaleTurn
    );
    assert_eq!(
        rejected(fx.put(Role::Planner, ArtifactPayload::Log, b"l")),
        Code::KindForbidden
    );
    fx.reach_build();
    assert_eq!(
        rejected(fx.put(Role::Coder, ArtifactPayload::Spec { brief }, b"s")),
        Code::KindForbidden
    );
    let too_big = BlobRef {
        sha: Sha256([1; 32]),
        len: 2_000_001,
    };
    assert_eq!(
        rejected(fx.agent(
            Role::Coder,
            AgentAction::PutArtifact {
                blob: too_big,
                payload: ArtifactPayload::Diff
            }
        )),
        Code::TooLarge
    );
}

#[test]
fn r16_runner_secret_and_consistency() {
    let mut fx = Fx::new();
    fx.reach_test();
    let (run, _) = fx.selected_run();
    assert_eq!(
        rejected(fx.report_as(run, RunSecret(Sha256([0; 32])), RunStatus::Passed)),
        Code::BadRunSecret
    );
    // Inconsistent: status passed with a failing profile.
    let (run, secret) = fx.selected_run();
    let cand = fx.st().candidate.expect("c");
    let blob = fx.d.store.blob_put(b"log");
    let op = fx.op();
    let log = stored(fx.run(&Command::Runner {
        op,
        run,
        secret,
        blob,
        payload: ArtifactPayload::Log,
    }));
    let rep = Report {
        candidate: cand,
        run,
        profile_digest: DIGEST,
        status: RunStatus::Passed,
        profiles: vec![ProfileResult {
            name: "t".into(),
            exit_code: 1,
            failed: 1,
        }],
        tree: None,
        log,
    };
    let op = fx.op();
    assert_eq!(
        rejected(fx.run(&Command::Runner {
            op,
            run,
            secret,
            blob,
            payload: ArtifactPayload::TestReport(rep)
        })),
        Code::InconsistentReport
    );
    let op = fx.op();
    assert_eq!(
        rejected(fx.run(&Command::Runner {
            op,
            run,
            secret,
            blob,
            payload: ArtifactPayload::Log
        })),
        Code::DuplicateReport
    );
}

#[test]
fn r17_candidate_needs_approved_spec_and_approval_binds() {
    let mut fx = Fx::new();
    fx.reach_plan_gate();
    // Not in build: wrong state first.
    assert_eq!(
        rejected(fx.put(
            Role::Coder,
            ArtifactPayload::Candidate(Candidate {
                base: "a".repeat(40),
                diffs: vec![]
            }),
            b"{}"
        )),
        Code::TokenExpired
    );
    let mut fx = Fx::new();
    let c = fx.reach_merge_gate();
    let (run, _) = fx.selected_run();
    assert_eq!(
        rejected(fx.human(HumanAction::Approve {
            gate: Gate::Merge,
            subject: ArtId(1),
            run: Some(run)
        })),
        Code::StaleSubject
    );
    assert_eq!(
        rejected(fx.human(HumanAction::Approve {
            gate: Gate::Merge,
            subject: c,
            run: Some(RunId(99))
        })),
        Code::StaleSubject
    );
    ok(fx.human(HumanAction::Approve {
        gate: Gate::Merge,
        subject: c,
        run: Some(run),
    }));
}

#[test]
fn r18_terminal_rejects_everything() {
    let mut fx = Fx::new();
    let rev = fx.rev();
    ok(fx.human(HumanAction::Cancel {
        expected_rev: rev,
        reason: "stop".into(),
    }));
    assert_eq!(fx.st().state, State::Cancelled);
    assert_eq!(
        rejected(fx.post(Role::Planner, ask(Role::Coder, "?"))),
        Code::Terminal
    );
    let rev = fx.rev();
    assert_eq!(
        rejected(fx.human(HumanAction::Resume {
            target: State::Planning,
            expected_rev: rev
        })),
        Code::Terminal
    );
}
