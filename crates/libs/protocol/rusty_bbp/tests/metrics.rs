#![allow(clippy::expect_used, clippy::unwrap_used)]
//! The metrics fold, on scenarios whose values are known by construction.

mod common;
use common::*;
use rusty_bbp::metrics::{self, GateKind, Overlap, Work};
use rusty_bbp::*;

const WON: &[u8] = b"--- a/flag.txt\n+++ b/flag.txt\n@@ -1 +1 @@\n-no\n+ok\n";

fn log_of(fx: &Fx) -> Vec<Event> {
    fx.d.store.events(&TaskId("T1".into()), 0).expect("log")
}

fn blobs(fx: &Fx) -> impl Fn(&BlobRef) -> Option<Vec<u8>> + '_ {
    |b| fx.d.store.blob_get(&b.sha).ok()
}

fn submit(fx: &mut Fx, role: &str, diffs: Vec<ArtId>) {
    stored(fx.put(
        role,
        ArtifactPayload::Candidate(Candidate {
            base: "a".repeat(40),
            diffs,
        }),
        b"{}",
    ));
}

#[test]
fn a_losing_builders_work_is_counted_as_discarded() {
    let mut fx = Fx::concurrent(builders(), 2, default_budget());
    fx.reach_build();
    let loser = b"--- a/flag.txt\n+++ b/flag.txt\n@@ -1,2 +1,2 @@\n-no\n+yes\n";
    stored(fx.put("coder2", ArtifactPayload::Diff, loser));
    posted(fx.post("coder2", ask("planner", "is yes fine?")));
    let won = stored(fx.put("coder", ArtifactPayload::Diff, WON));
    submit(&mut fx, "coder", vec![won]);
    let m = metrics::of(&log_of(&fx));
    assert_eq!(m.peak_live, 2);
    assert_eq!(
        m.revocations, 1,
        "coder2's turn was revoked by the candidate"
    );
    assert_eq!(
        m.discarded,
        Work {
            messages: 1,
            reads: 0,
            bytes: loser.len() as u64 + "is yes fine?".len() as u64
        }
    );
    assert!(m.discarded_share > 0.0 && m.discarded_share < 1.0);
    assert_eq!(
        m.charged.messages,
        1 + 1,
        "the gate request and coder2's ask"
    );
}

#[test]
fn overlap_separates_other_files_other_regions_and_the_same_lines() {
    let mut fx = Fx::concurrent(builders(), 2, default_budget());
    fx.reach_build();
    let won = stored(fx.put("coder", ArtifactPayload::Diff, WON));
    // Same file, same lines; same file, far away; another file.
    stored(fx.put(
        "coder2",
        ArtifactPayload::Diff,
        b"+++ b/flag.txt\n@@ -1,3 +1,3 @@\n",
    ));
    stored(fx.put(
        "coder2",
        ArtifactPayload::Diff,
        b"+++ b/flag.txt\n@@ -40,2 +40,2 @@\n",
    ));
    stored(fx.put(
        "coder2",
        ArtifactPayload::Diff,
        b"+++ b/other.txt\n@@ -1 +1 @@\n",
    ));
    submit(&mut fx, "coder", vec![won]);
    let o = metrics::overlap(&log_of(&fx), &blobs(&fx));
    assert_eq!(
        o,
        Overlap {
            unused_diffs: 3,
            file_overlap_diffs: 2,
            hunk_overlap_diffs: 1
        }
    );
}

#[test]
fn no_candidate_means_no_overlap_to_report() {
    let mut fx = Fx::concurrent(builders(), 2, default_budget());
    fx.reach_build();
    stored(fx.put("coder", ArtifactPayload::Diff, WON));
    assert_eq!(
        metrics::overlap(&log_of(&fx), &blobs(&fx)),
        Overlap::default()
    );
}

#[test]
fn refusals_and_stale_writes_are_counted_once() {
    let mut fx = Fx::concurrent(builders(), 2, budget(12, 30));
    fx.reach_build();
    let slice = fx
        .st()
        .turn_of(&RoleId("coder".into()))
        .unwrap()
        .slice
        .unwrap()
        .messages;
    for _ in 0..slice {
        posted(fx.post("coder", ask("coder2", "x")));
    }
    assert_eq!(
        rejected(fx.post("coder", ask("coder2", "y"))),
        Code::TurnBudgetExhausted
    );
    // coder3 holds no turn: a stale write.
    assert_eq!(
        rejected(fx.post("coder3", ask("coder", "z"))),
        Code::StaleTurn
    );
    let m = metrics::of(&log_of(&fx));
    assert_eq!(m.budget_refusals, 1);
    assert_eq!(m.stale_rejections, 1);
}

/// Two testers are live together (finalizers are serialized, testers are not).
/// `tester2` appends a finding during `tester`'s turn; `then` is what `tester`
/// does between that finding and its approve. The engine now refuses an
/// approve over unread content, so the live turn's `unseen` set is cleared
/// first: this emulates a log written before the refusal existed, which is what
/// the counter still has to read.
fn verdict_after_finding(then: impl FnOnce(&mut Fx)) -> (u64, u64) {
    let mut fx = Fx::concurrent(two_testers(), 2, default_budget());
    fx.reach_test();
    stored(fx.report(RunStatus::Passed));
    let read = |after, limit| AgentAction::Read { after, limit };
    assert!(matches!(
        fx.agent("tester", read(None, 50)),
        Response::Read { .. }
    ));
    let cand = fx.st().candidate.unwrap();
    let report = fx.report_id();
    let finding = Draft::new(MessageKind::Finding, "no backoff")
        .refs(vec![Ref::art(cand)])
        .evidence(vec![Ref::art(report)]);
    posted(fx.post("tester2", finding));
    then(&mut fx);
    fx.d.state.turns.iter_mut().for_each(|t| t.unseen.clear());
    posted(fx.verdict("tester", VerdictKind::Approve));
    let m = metrics::of(&log_of(&fx));
    (m.verdicts, m.stale_verdicts)
}

/// The refusal and the counter agree: the engine refuses the stale verdict, so
/// the log of a current build holds none.
#[test]
fn the_engine_refuses_what_the_counter_would_count() {
    let mut fx = Fx::concurrent(two_testers(), 2, default_budget());
    fx.reach_test();
    stored(fx.report(RunStatus::Passed));
    let read = AgentAction::Read {
        after: None,
        limit: 50,
    };
    assert!(matches!(fx.agent("tester", read), Response::Read { .. }));
    let (cand, report) = (fx.st().candidate.unwrap(), fx.report_id());
    let finding = Draft::new(MessageKind::Finding, "no backoff")
        .refs(vec![Ref::art(cand)])
        .evidence(vec![Ref::art(report)]);
    posted(fx.post("tester2", finding));
    assert_eq!(
        rejected(fx.verdict("tester", VerdictKind::Approve)),
        Code::StaleSubject
    );
    let m = metrics::of(&log_of(&fx));
    assert_eq!((m.verdicts, m.stale_verdicts), (0, 0));
}

#[test]
fn a_verdict_after_a_concurrent_finding_is_a_stale_verdict() {
    assert_eq!(verdict_after_finding(|_| {}), (1, 1));
}

#[test]
fn fetching_an_artifact_does_not_count_as_having_read_the_messages() {
    // The tester reads the messages, misses the finding, then fetches the
    // immutable test report: a read charge, but not a view of the messages.
    let got = verdict_after_finding(|fx| {
        let report = fx.report_id();
        assert!(matches!(
            fx.agent("tester", AgentAction::GetArtifact { id: report }),
            Response::Artifact(_)
        ));
    });
    assert_eq!(got, (1, 1));
}

#[test]
fn reading_the_messages_after_the_finding_clears_it() {
    let got = verdict_after_finding(|fx| {
        assert!(matches!(
            fx.agent(
                "tester",
                AgentAction::Read {
                    after: None,
                    limit: 50
                }
            ),
            Response::Read { .. }
        ));
    });
    assert_eq!(got, (1, 0));
}

#[test]
fn a_page_that_stops_short_does_not_count_as_having_seen_the_finding() {
    let got = verdict_after_finding(|fx| {
        let r = fx.agent(
            "tester",
            AgentAction::Read {
                after: None,
                limit: 1,
            },
        );
        assert!(matches!(r, Response::Read { more: true, .. }));
    });
    assert_eq!(got, (1, 1));
}

#[test]
fn a_serialized_run_has_nothing_to_report() {
    let mut fx = Fx::new();
    fx.reach_approved();
    let m = metrics::of(&log_of(&fx));
    assert_eq!(m.peak_live, 1);
    assert_eq!(
        (m.revocations, m.budget_refusals, m.stale_verdicts),
        (0, 0, 0)
    );
    assert_eq!(m.discarded_share, 0.0);
    assert_eq!(m.discarded.bytes, 0);
}

#[test]
fn each_human_gate_answer_is_listed_with_the_op_that_carried_it() {
    let mut fx = Fx::new();
    fx.reach_approved();
    let log = log_of(&fx);
    let got = metrics::gate_answers(&log);
    assert_eq!(
        got.iter().map(|g| g.gate).collect::<Vec<_>>(),
        [GateKind::PlanApproval, GateKind::MergeApproval]
    );
    assert_ne!(got[0].op, got[1].op);
    // The op is the one the human command carried.
    let human_ops: Vec<&OpId> = log
        .iter()
        .filter_map(|e| match e {
            Event::OpAccepted { principal, op, .. } if *principal == fx.st().human => Some(op),
            _ => None,
        })
        .collect();
    assert!(got.iter().all(|g| human_ops.contains(&&g.op)));
}

#[test]
fn a_rejection_is_a_gate_answer_and_agent_ops_are_not() {
    let mut fx = Fx::new();
    fx.reach_plan_gate();
    let rev = fx.rev();
    ok(fx.human(HumanAction::Reject {
        expected_rev: rev,
        target: State::Planning,
        reason: "again".into(),
    }));
    let got = metrics::gate_answers(&log_of(&fx));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].gate, GateKind::Rejection);
}
