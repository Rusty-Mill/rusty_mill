#![allow(clippy::expect_used, clippy::unwrap_used)]
//! `bbp why` over a turn-based task, one live turn and two, through the real
//! engine. The same script runs at both caps, so the live-turn count is the only
//! difference.
//!
//! What the count decides, and where. Shadow mode and `wake_set` are defined at
//! one live turn only (`engine::shadowing`, and `why` skips both above it), so
//! at two there is no comparison, no shadow record and no wake set to print. An
//! observer is selectable only above one live turn (`WakeReason::selectable`),
//! but no real task reaches that: `pick_next` ignores `wake_set` when turn-based
//! and `open` refuses `EventDriven` above one live turn. So the selectable
//! observer is a unit-level fact (`tests/wake.rs`); here it is pinned from the
//! other side: at two live turns an observer-only board is still just the
//! default holder, and nothing is recorded about it.

#[path = "../../../libs/protocol/rusty_bbp/tests/common/mod.rs"]
mod core_fx;

use core_fx::*;
use rusty_bbp::*;
use rusty_bbp_host::shadow;
use rusty_bbp_host::why::{self, Cause, How, Report};
use std::path::PathBuf;

fn role(r: &str) -> RoleId {
    RoleId::from(r)
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bbp_host_live_{}_{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn task(live: u32) -> Fx {
    Fx::concurrent(four_roles(), live, default_budget())
}

/// Build-state script, the same at any cap:
/// 1. the coder stores a diff and passes: the tester and reviewer only watch it
///    (observers), so the board has nothing selectable at one live turn;
/// 2. the coder asks the tester a question and passes: the tester is addressed,
///    but the default holder still goes (turn-based).
fn script(fx: &mut Fx) {
    fx.reach_build();
    stored(fx.put(Who::Coder, ArtifactPayload::Diff, b"--- a\n+++ b\n"));
    posted(fx.post(Who::Coder, pass()));
    posted(fx.post(Who::Coder, ask(Who::Tester, "which 429 cases matter?")));
    posted(fx.post(Who::Coder, pass()));
}

fn replay(fx: &Fx, logged: &[Shadow]) -> Report {
    let events = fx.d.store.events(&fx.st().task, 0).expect("log");
    why::explain(&fx.st().task, &events, logged)
}

/// The records the host keeps for `fx`'s task, written and read back the way
/// `bbp mod` and `bbp shadow` do.
fn kept(fx: &Fx, tag: &str) -> Vec<Shadow> {
    let dir = tempdir(tag);
    shadow::append(&dir, &fx.st().task, &fx.shadows).expect("append");
    let read = shadow::read(&dir, &fx.st().task).expect("read");
    assert_eq!(read.unreadable, 0);
    assert_eq!(
        read.records, fx.shadows,
        "what was kept is what was captured"
    );
    read.records
}

#[test]
fn at_one_live_turn_each_grant_carries_its_shadow_comparison_and_the_kept_record_agrees() {
    let mut fx = task(1);
    script(&mut fx);
    let logged = kept(&fx, "one");
    let report = replay(&fx, &logged);

    let comparisons: Vec<(&str, &str)> = report
        .grants
        .iter()
        .map(|g| {
            let s = g.shadow.as_ref().expect("a comparison at one live turn");
            assert_eq!(g.how, How::DefaultHolder, "{g:?}");
            // The kept record is the replay's: same rules, same wake set.
            assert_eq!(s.logged_rules, Some(WAKE_RULES), "{g:?}");
            assert_eq!(s.logged_same, Some(true), "{g:?}");
            (g.role.0.as_str(), s.comparison)
        })
        .collect();
    // planner (plan), coder (build), then the two passes: the observer-only
    // board is a fallback (the observers are not selectable at one live turn),
    // and the addressed tester is a disagreement (the default holder went).
    assert_eq!(
        comparisons,
        [
            ("planner", "agree"),
            ("coder", "agree"),
            ("coder", "fallback"),
            ("coder", "disagree"),
        ]
    );

    // The observers are in the set, and not the pick.
    let fallback = report.grants[2].shadow.as_ref().unwrap();
    assert_eq!(
        fallback
            .would
            .iter()
            .map(|w| (w.role.0.as_str(), w.reason))
            .collect::<Vec<_>>(),
        [
            ("planner", WakeReason::Observer),
            ("tester", WakeReason::Observer),
            ("reviewer", WakeReason::Observer)
        ]
    );
    let disagree = report.grants[3].shadow.as_ref().unwrap();
    assert_eq!(disagree.would[0].role, role("tester"));
    assert_eq!(disagree.would[0].reason, WakeReason::Addressed);

    // The comparison the replay prints is the one the record itself gives.
    for (g, rec) in report.grants.iter().zip(&logged) {
        let want = match rec.comparison() {
            Some(Comparison::Agree) => "agree",
            Some(Comparison::Fallback) => "fallback",
            Some(Comparison::Disagree) => "disagree",
            None => panic!("an automatic grant: {rec:?}"),
        };
        assert_eq!(g.shadow.as_ref().unwrap().comparison, want, "{g:?}");
    }
}

#[test]
fn at_two_live_turns_there_is_no_comparison_no_wake_set_and_no_record() {
    let mut fx = task(2);
    script(&mut fx);
    assert!(
        fx.shadows.is_empty(),
        "shadow mode is off above one live turn"
    );
    let logged = kept(&fx, "two");
    assert!(logged.is_empty());
    let report = replay(&fx, &logged);

    let roles: Vec<&str> = report.grants.iter().map(|g| g.role.0.as_str()).collect();
    assert_eq!(roles, ["planner", "coder", "coder", "coder"]);
    for g in &report.grants {
        // The same observer-only and addressed boards as above, and neither is
        // consulted: the serving list rules, so every grant is the default holder.
        assert_eq!(g.how, How::DefaultHolder, "{g:?}");
        assert!(g.shadow.is_none() && g.wake_set.is_none(), "{g:?}");
    }
}

/// A forced grant while another turn is live: with two testers serving `test`
/// (both live), one yields to the coder. The coder's consultation is granted
/// with the other tester's turn still open. Only a cap above one allows it.
#[test]
fn an_explicit_grant_while_another_turn_is_live() {
    let mut fx = Fx::concurrent(two_testers(), 2, default_budget());
    fx.reach_test();
    stored(fx.report(RunStatus::Passed));
    assert_eq!(fx.st().turns.len(), 2, "both testers are live");
    posted(fx.post("tester", ask(Who::Coder, "is the retry loop intended?")));
    posted(fx.post("tester", yield_to(Who::Coder)));
    let live: Vec<&str> = fx.st().turns.iter().map(|t| t.role.0.as_str()).collect();
    assert_eq!(live, ["tester2", "coder"], "the other tester is still live");

    let report = replay(&fx, &[]);
    let last = report.grants.last().expect("a grant");
    assert_eq!(last.role, role("coder"));
    assert_eq!(last.kind, TurnKind::Consultation);
    assert!(
        matches!(
            &last.how,
            How::Explicit(Cause::Consultation { return_to: Some(r) }) if *r == role("tester")
        ),
        "{last:?}"
    );
    // An Explicit grant carries neither a set nor a comparison, and the cap is
    // above one, so there is no shadow record of it either.
    assert!(last.wake_set.is_none() && last.shadow.is_none());
    assert!(fx.shadows.is_empty());
    // The automatic grants around it are the serving list, not a pick.
    for g in report
        .grants
        .iter()
        .filter(|g| !matches!(g.how, How::Explicit(_)))
    {
        assert_eq!(g.how, How::DefaultHolder, "{g:?}");
    }
}

/// At one live turn the same ask is a forced grant too, once the yield hands the
/// turn over; the shadow log keeps it as explicit and never compares it.
#[test]
fn a_yield_at_one_live_turn_is_explicit_and_kept_as_explicit() {
    let mut fx = task(1);
    fx.reach_build();
    posted(fx.post(Who::Coder, ask(Who::Tester, "which 429 cases matter?")));
    posted(fx.post(Who::Coder, yield_to(Who::Tester)));
    let logged = kept(&fx, "yield");
    let report = replay(&fx, &logged);
    let last = report.grants.last().expect("a grant");
    assert_eq!(last.role, role("tester"));
    assert!(matches!(
        last.how,
        How::Explicit(Cause::Consultation { .. })
    ));
    assert!(last.shadow.is_none());
    let explicit = logged
        .iter()
        .filter(|r| matches!(r.kind, ShadowKind::Explicit { .. }))
        .count();
    assert_eq!(explicit, 1, "{logged:?}");
}
