#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The fence is checked transactionally, while successful retries replay.
mod common;
use common::*;
use rusty_bbp::*;

#[test]
fn plan_approval_requires_the_displayed_revision_even_with_the_same_spec() {
    let mut fx = Fx::new();
    let spec = fx.reach_plan_gate();
    let shown = fx.rev();
    fx.human(HumanAction::Post(
        Draft::new(MessageKind::Ask, "new information").to(Recipient::All),
    ));
    assert_eq!(
        rejected(fx.human(HumanAction::Fenced {
            expected_rev: shown,
            action: Box::new(HumanAction::Approve {
                gate: Gate::Plan,
                subject: spec,
                run: None
            })
        })),
        Code::StaleRev
    );
    assert_eq!(fx.d.state.state, State::PlanGate);
}

#[test]
fn a_successful_fenced_receipt_replays_after_the_task_closed() {
    let mut fx = Fx::new();
    let candidate = fx.reach_approved();
    let command = Command::Human {
        op: OpId("orch-retry".into()),
        action: HumanAction::Fenced {
            expected_rev: fx.rev(),
            action: Box::new(HumanAction::MergeReceipt {
                candidate,
                revision: "merged-sha".into(),
            }),
        },
    };
    assert_eq!(fx.d.dispatch(&command, fx.now).unwrap(), Response::Ok);
    assert_eq!(fx.d.state.state, State::Closed);
    let rev = fx.rev();
    assert_eq!(fx.d.dispatch(&command, fx.now).unwrap(), Response::Ok);
    assert_eq!(fx.rev(), rev);
    let events = fx.d.store.events(&fx.d.state.task, 0).unwrap();
    let view = TaskView::from_events(fx.d.state.task.clone(), &events);
    assert_eq!(view.receipt, Some((candidate, "merged-sha".into())));
    let json = rusty_serde::json::to_string(&view).unwrap();
    assert!(!json.contains("secret") && !json.contains("token") && !json.contains("master"));
}
