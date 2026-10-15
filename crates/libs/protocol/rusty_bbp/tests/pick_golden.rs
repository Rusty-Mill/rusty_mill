#![allow(clippy::expect_used, clippy::unwrap_used)]
//! The event-driven pick, pinned by logs. Each golden is the event log of a
//! fixed event-driven run, written by the build before the pick rule became a
//! public function (`wake::pick_from`) and checked in unchanged: the pick, and
//! so every grant and every logged byte, is what it was. `BBP_BLESS=1` rewrites
//! them; do that only for a deliberate change of the rule.
mod common;
use common::*;
use rusty_bbp::*;
use std::path::PathBuf;

fn golden(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.log"))
}

fn text(fx: &Fx) -> String {
    let log = fx.d.store.events(&fx.st().task, 0).expect("log");
    log.iter()
        .map(|e| rusty_serde::json::to_string(e).expect("json") + "\n")
        .collect()
}

/// A whole task, planning to approved, picked by `State` at every step.
fn happy() -> Fx {
    let mut fx = Fx::event_driven(four_roles());
    fx.reach_approved();
    fx
}

/// The coder asks the tester a question and passes: the tester is picked
/// (`Addressed`), passes in turn, and the coder is picked again.
fn question() -> Fx {
    let mut fx = Fx::event_driven(four_roles());
    fx.reach_build();
    posted(fx.post(Who::Coder, ask(Who::Tester, "which 429 cases matter?")));
    posted(fx.post(Who::Coder, pass()));
    posted(fx.post(Who::Tester, pass()));
    fx
}

/// Two selectable entries at `review`: the tester's approval names the coder
/// (`Addressed`) and the reviewer is the state's server (`State`). The coder is
/// picked, then the reviewer.
fn named_verdict() -> Fx {
    let mut fx = Fx::event_driven(four_roles());
    fx.reach_test();
    stored(fx.report(RunStatus::Passed));
    posted(fx.verdict_to(
        fx.holder(State::Test),
        VerdictKind::Approve,
        vec![Recipient::Role("coder".into())],
    ));
    posted(fx.post(Who::Coder, pass()));
    posted(fx.verdict(fx.holder(State::Review), VerdictKind::Approve));
    fx
}

/// A quiet board: nothing is selectable, so the default holder again.
fn quiet() -> Fx {
    let mut fx = Fx::event_driven(four_roles());
    fx.reach_build();
    posted(fx.post(Who::Coder, pass()));
    posted(fx.post(Who::Coder, pass()));
    fx
}

fn check(name: &str, fx: Fx) {
    let got = text(&fx);
    let path = golden(name);
    if std::env::var_os("BBP_BLESS").is_some() {
        std::fs::write(&path, &got).expect("write golden");
        return;
    }
    let want = std::fs::read_to_string(&path).expect("golden present");
    assert_eq!(got.lines().count(), want.lines().count(), "{name}");
    for (i, (g, w)) in got.lines().zip(want.lines()).enumerate() {
        assert_eq!(g, w, "{name}: event {i} differs");
    }
}

#[test]
fn an_event_driven_task_picks_as_it_did_before_the_rule_was_public() {
    check("ed_happy", happy());
    check("ed_question", question());
    check("ed_named_verdict", named_verdict());
    check("ed_quiet", quiet());
}
