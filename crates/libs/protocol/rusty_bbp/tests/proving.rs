#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Stage-1 proving run: scripted Coder and Tester, stub Planner, Reviewer, human
//! and runner, all through the normal APIs. No fixture bypasses an admission rule.
mod common;
use common::*;
use rusty_bbp::event::TurnEnd;
use rusty_bbp::*;

#[test]
fn stage_one_end_to_end() {
    let mut fx = Fx::new();

    // Planning: Planner writes the spec and opens the gate; human approves the exact spec.
    let spec = fx.reach_build();
    assert_eq!(fx.st().approved_spec, Some(spec));

    // Build: Coder asks the Tester a question through a yield, gets an answer, then submits.
    let q = posted(fx.post(
        Role::Coder,
        ask(Role::Tester, "Should 429 count as retryable?"),
    ));
    posted(fx.post(Role::Coder, yield_to(Role::Tester)));
    posted(fx.post(
        Role::Tester,
        Draft::new(MessageKind::Answer, "Yes, with backoff.").reply_to(q),
    ));
    posted(fx.post(Role::Tester, pass()));
    assert_eq!(fx.turn_role(), Some(Role::Coder));
    let c1 = fx.candidate();
    assert_eq!(fx.st().state, State::Test);

    // Test: first run fails; Tester revises; Coder submits c2; run passes.
    stored(fx.report(RunStatus::Failed));
    posted(fx.verdict(Role::Tester, VerdictKind::Revise));
    assert_eq!(fx.st().state, State::Build);
    assert_eq!(fx.st().iteration, 1);
    let c2 = fx.candidate();
    assert_ne!(c1, c2);
    assert!(fx.st().verdicts.is_empty());

    // A lost response: replay the report with the same op returns the original.
    let (run, secret) = fx.selected_run();
    let cand = fx.st().candidate.expect("c");
    let log_blob = fx.d.store.blob_put(b"log2");
    let op_log = fx.op();
    let log = stored(fx.run(&Command::Runner {
        op: op_log,
        run,
        secret,
        blob: log_blob,
        payload: ArtifactPayload::Log,
    }));
    let rep = Report {
        candidate: cand,
        run,
        profile_digest: DIGEST,
        status: RunStatus::Passed,
        profiles: vec![ProfileResult {
            name: "test".into(),
            exit_code: 0,
            failed: 0,
        }],
        tree: Some(Sha256([2; 32])),
        sandbox: "test".into(),
        log,
    };
    let payload = ArtifactPayload::TestReport(rep);
    let rep_blob =
        fx.d.store
            .blob_put(&encode_artifact(&payload, b"").expect("encode"));
    let op_rep = fx.op();
    let cmd = Command::Runner {
        op: op_rep,
        run,
        secret,
        blob: rep_blob,
        payload,
    };
    let first = fx.run(&cmd);
    let rev = fx.rev();
    assert_eq!(fx.run(&cmd), first);
    assert_eq!(fx.rev(), rev);
    assert_eq!(fx.turn_role(), Some(Role::Tester));

    // A reader crash mid-slice: re-reading from the same offset returns the same page.
    let page1 = fx.agent(
        Role::Tester,
        AgentAction::Read {
            after: None,
            limit: 5,
        },
    );
    let page2 = fx.agent(
        Role::Tester,
        AgentAction::Read {
            after: None,
            limit: 5,
        },
    );
    match (&page1, &page2) {
        (Response::Read { messages: a, .. }, Response::Read { messages: b, .. }) => {
            assert_eq!(a, b)
        }
        other => panic!("{other:?}"),
    }

    // Tester approves; Reviewer reads only the human channel, inspects artifacts, approves.
    posted(fx.verdict(Role::Tester, VerdictKind::Approve));
    assert_eq!(fx.st().state, State::Review);
    assert!(matches!(
        fx.agent(Role::Reviewer, AgentAction::GetArtifact { id: c2 }),
        Response::Artifact(_)
    ));
    assert_eq!(
        rejected(fx.agent(Role::Reviewer, AgentAction::GetArtifact { id: log })),
        Code::RoleForbidden
    );
    posted(fx.verdict(Role::Reviewer, VerdictKind::Approve));
    assert_eq!(fx.st().state, State::MergeGate);

    // Stale approval: a zombie supervisor from run 1 cannot touch anything; a rerun stales the approvals.
    let rev = fx.rev();
    ok(fx.human(HumanAction::Rerun {
        candidate: c2,
        expected_rev: rev,
    }));
    assert_eq!(fx.st().state, State::Test);
    let (run2, _) = fx.selected_run();
    assert_ne!(run2, run);
    assert_eq!(
        rejected(fx.report_as(run, secret, RunStatus::Passed)),
        Code::StaleRun
    );
    assert_eq!(
        rejected(fx.human(HumanAction::Approve {
            gate: Gate::Merge,
            subject: c2,
            run: Some(run)
        })),
        Code::WrongState
    );

    // Back through test and review on the new run.
    stored(fx.report(RunStatus::Passed));
    posted(fx.verdict(Role::Tester, VerdictKind::Approve));
    posted(fx.verdict(Role::Reviewer, VerdictKind::Approve));
    let (run2, _) = fx.selected_run();
    assert_eq!(
        rejected(fx.human(HumanAction::Approve {
            gate: Gate::Merge,
            subject: c2,
            run: Some(run)
        })),
        Code::StaleSubject,
        "approval against the superseded run"
    );
    ok(fx.human(HumanAction::Approve {
        gate: Gate::Merge,
        subject: c2,
        run: Some(run2),
    }));
    assert_eq!(fx.st().state, State::Approved);

    // Late write with a dead token.
    let dead = fx.tok(Role::Coder);
    let op = fx.op();
    assert!(matches!(
        rejected(fx.agent_with(
            Role::Coder,
            Some(dead),
            op,
            AgentAction::Post(ask(Role::Tester, "late"))
        )),
        Code::TokenExpired | Code::StaleTurn
    ));

    // Merge receipt closes; everything after is terminal.
    ok(fx.human(HumanAction::MergeReceipt {
        candidate: c2,
        revision: "0123abcd".into(),
    }));
    assert_eq!(fx.st().state, State::Closed);
    assert_eq!(
        rejected(fx.human(HumanAction::Post(ask(Role::Coder, "?")))),
        Code::Terminal
    );

    // The log replays to the same state, and the card never carried a token.
    let log_events = fx.d.store.events(&fx.st().task, 0).expect("log");
    assert_eq!(TaskState::fold(fx.st().task.clone(), &log_events), *fx.st());
    let card = rusty_serde::json::to_string(&fx.st().card()).expect("json");
    assert!(!card.contains("token"));
    assert!(log_events.iter().any(|e| matches!(
        e,
        Event::TurnEnded {
            cause: TurnEnd::Yield,
            ..
        }
    )));
}
