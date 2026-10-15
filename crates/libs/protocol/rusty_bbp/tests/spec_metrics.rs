#![allow(clippy::expect_used, clippy::unwrap_used)]
//! `spec_conflicts` on logs the engine wrote: two planners author the spec at
//! once at `max_live_turns = 2`.

mod common;
use common::*;
use rusty_bbp::spec_metrics::{spec_conflicts, PairOutcome};
use rusty_bbp::*;

/// The four roles plus a second planner, so `planning` has two spec authors.
fn two_planners() -> Vec<Role> {
    let mut roles = four_roles();
    let mut second = roles
        .iter()
        .find(|r| r.id.0 == "planner")
        .expect("planner")
        .clone();
    second.id = RoleId("planner2".into());
    second.turn_order = 8;
    roles.push(second);
    roles
}

fn put_spec(fx: &mut Fx, role: &str, markdown: &str) -> ArtId {
    let brief = fx.st().brief.expect("brief");
    stored(fx.put(role, ArtifactPayload::Spec { brief }, markdown.as_bytes()))
}

fn body(changes: &[(usize, &str)]) -> String {
    (1..=10)
        .map(|i| match changes.iter().find(|(l, _)| *l == i) {
            Some((_, t)) => format!("{t}\n"),
            None => format!("line {i}\n"),
        })
        .collect()
}

/// planner stores a base; planner2 and then planner each edit it; planner
/// asks for the gate with its own version and the human approves it.
fn authored(planner2_edit: &[(usize, &str)], planner_edit: &[(usize, &str)]) -> (Fx, [ArtId; 3]) {
    let mut fx = Fx::concurrent(two_planners(), 2, default_budget());
    assert_eq!(fx.st().turns.len(), 2, "both planners are live");
    let base = put_spec(&mut fx, "planner", &body(&[]));
    let other = put_spec(&mut fx, "planner2", &body(planner2_edit));
    let mine = put_spec(&mut fx, "planner", &body(planner_edit));
    let mut d = Draft::new(MessageKind::RequestDecision, "Approve the plan?")
        .to(Recipient::Human)
        .refs(vec![Ref::art(mine)]);
    d.gate = true;
    posted(fx.post("planner", d));
    ok(fx.human(HumanAction::Approve {
        gate: Gate::Plan,
        subject: mine,
        run: None,
    }));
    (fx, [base, other, mine])
}

fn run(fx: &Fx) -> rusty_bbp::spec_metrics::SpecConflicts {
    let log = fx.d.store.events(&TaskId("T1".into()), 0).expect("log");
    spec_conflicts(&log, &|b| fx.d.store.blob_get(&b.sha).ok())
}

#[test]
fn two_planners_editing_one_line_conflict() {
    let (fx, [base, other, mine]) = authored(&[(4, "from planner2")], &[(4, "from planner")]);
    let r = run(&fx);
    assert_eq!(r.approved, Some(mine));
    let p = r.pairs.iter().find(|p| p.version == other).expect("pair");
    assert_eq!((p.base, p.outcome), (Some(base), PairOutcome::Conflict));
    let b = r.pairs.iter().find(|p| p.version == base).expect("pair");
    assert_eq!(
        b.outcome,
        PairOutcome::ParallelRewrite,
        "nothing came before the base"
    );
}

#[test]
fn two_planners_editing_different_lines_do_not() {
    let (fx, [base, other, _]) = authored(&[(2, "from planner2")], &[(9, "from planner")]);
    let r = run(&fx);
    let p = r.pairs.iter().find(|p| p.version == other).expect("pair");
    assert_eq!((p.base, p.outcome), (Some(base), PairOutcome::Clean));
    assert_eq!(r.conflicts(), 0);
}
