#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Schema compatibility. The golden is the event log of a fixed one-turn run,
//! written by the build before `max_live_turns` existed
//! and checked in unchanged: a one-turn task still writes it byte for byte.
mod common;
use common::*;
use rusty_bbp::*;

fn scenario() -> String {
    let mut fx = Fx::new();
    fx.reach_review();
    fx.agent(
        Who::Reviewer,
        AgentAction::Read {
            after: None,
            limit: 5,
        },
    );
    drop(fx.post(Who::Reviewer, ask(Who::Coder, "not allowed")));
    fx.advance(61_000);
    fx.tick();
    posted(fx.verdict(Who::Reviewer, VerdictKind::Approve));
    let c = fx.st().candidate.expect("candidate");
    let (run, _) = fx.selected_run();
    ok(fx.human(HumanAction::Approve {
        gate: Gate::Merge,
        subject: c,
        run: Some(run),
    }));
    ok(fx.human(HumanAction::MergeReceipt {
        candidate: c,
        revision: "abc".into(),
    }));
    let log = fx.d.store.events(&TaskId("T1".into()), 0).expect("log");
    log.iter()
        .map(|e| rusty_serde::json::to_string(e).expect("json") + "\n")
        .collect()
}

#[test]
fn a_one_turn_log_is_byte_identical_to_schema_2_before_live_turns() {
    let got = scenario();
    let want = include_str!("golden/n1_happy.log");
    assert_eq!(got.lines().count(), want.lines().count());
    for (i, (g, w)) in got.lines().zip(want.lines()).enumerate() {
        assert_eq!(g, w, "event {i} differs");
    }
    assert!(!got.contains("max_live_turns") && !got.contains("\"slice\""));
}

fn log_of(fx: &Fx) -> Vec<Event> {
    fx.d.store.events(&TaskId("T1".into()), 0).expect("log")
}

#[test]
fn schema_is_2_at_one_live_turn_and_3_above() {
    for (live, schema) in [(1, LOG_SCHEMA), (2, LOG_SCHEMA_CONCURRENT)] {
        let fx = Fx::concurrent(four_roles(), live, default_budget());
        assert_eq!(log_schema(&log_of(&fx)), Some(schema), "cap {live}");
    }
    let fx = Fx::concurrent(four_roles(), 2, default_budget());
    let text = rusty_serde::json::to_string(&log_of(&fx)[0]).expect("json");
    assert!(text.contains("\"max_live_turns\":2"));
}

#[test]
fn a_schema_2_log_cannot_raise_the_cap() {
    // A log that claims schema 2 but carries a cap folds as serialized.
    let fx = Fx::concurrent(four_roles(), 2, default_budget());
    let mut log = log_of(&fx);
    if let Event::TaskOpened { schema, .. } = &mut log[0] {
        *schema = LOG_SCHEMA;
    }
    assert_eq!(TaskState::fold(TaskId("T1".into()), &log).max_live_turns, 1);
}
