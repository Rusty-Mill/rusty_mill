//! Possession chains: the segmentation rules on synthetic touch sequences, and the
//! invariant that every real touch belongs to exactly one chain.

use replay_analyzer::model::Event;
use replay_analyzer::{analyze::build_canonical, decode::boxcars_adapter::BoxcarsParser};
use replay_analyzer::{decode::ReplayParser, CanonicalMatch};
use replay_scoring::chains::CHAIN_GAP_S;
use replay_scoring::{chains, ChainEnd, Episode};

fn canonical(name: &str) -> CanonicalMatch {
    let path = format!(
        "{}/../assets/replays/{name}.replay",
        env!("CARGO_MANIFEST_DIR")
    );
    let data = std::fs::read(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    build_canonical(&BoxcarsParser::new().parse(&data).expect("decode"), name)
}

fn touch(t: f32, team: i32) -> Event {
    Event::Touch {
        t,
        pri: 1,
        player: None,
        team: Some(team),
    }
}
fn goal(t: f32, team: i32) -> Event {
    Event::Goal {
        t,
        scorer: None,
        team: Some(team),
    }
}

/// `(team, touches, end)` of the chains over `events` (a real match's frames, synthetic events).
fn ends(events: Vec<Event>) -> Vec<(i32, u8, ChainEnd)> {
    let mut m = canonical("42f2");
    m.events = events;
    chains(&m, &[], &[])
        .iter()
        .map(|e| match e {
            Episode::Chain {
                team, touches, end, ..
            } => (*team, *touches, *end),
            other => panic!("{other:?}"),
        })
        .collect()
}

#[test]
fn a_chain_is_one_teams_touches_and_ends_when_the_other_team_touches() {
    let e = ends(vec![
        touch(1.0, 0),
        touch(2.0, 0),
        touch(3.0, 1),
        touch(4.0, 1),
    ]);
    assert_eq!(e, [(0, 2, ChainEnd::Lost), (1, 2, ChainEnd::Dead)]);
}

#[test]
fn a_long_loose_ball_or_a_goal_splits_a_chain() {
    let gap = ends(vec![touch(1.0, 0), touch(1.5 + CHAIN_GAP_S, 0)]);
    assert_eq!(gap, [(0, 1, ChainEnd::Dead), (0, 1, ChainEnd::Dead)]);
    let split = ends(vec![touch(1.0, 0), goal(2.0, 1), touch(2.5, 0)]);
    assert_eq!(split.len(), 2, "a goal splits the run: {split:?}");
}

#[test]
fn a_goal_soon_after_the_last_touch_is_a_goal_chain() {
    assert_eq!(
        ends(vec![touch(1.0, 0), goal(2.0, 0)]),
        [(0, 1, ChainEnd::Goal)]
    );
}

#[test]
fn every_touch_is_in_exactly_one_chain_and_chains_are_ordered() {
    for name in ["42f2", "419a"] {
        let m = canonical(name);
        let known = m
            .events
            .iter()
            .filter(|e| matches!(e, Event::Touch { team: Some(_), .. }))
            .count();
        let cs = chains(&m, &[], &[]);
        let total: usize = cs
            .iter()
            .map(|c| match c {
                Episode::Chain { touches, .. } => *touches as usize,
                _ => 0,
            })
            .sum();
        assert_eq!(total, known, "{name}");
        assert!(
            cs.windows(2).all(|w| w[0].t() <= w[1].t()),
            "{name}: time order"
        );
    }
}
