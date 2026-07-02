//! `CanonicalMatch` → [`Timeline`] bridge — the one format-facing seam.
//!
//! The original PacifistScore repo carried its own boxcars adapter
//! (`pacifist-replay`) to produce a `Timeline`. Consolidated into this
//! workspace, that layer is superseded by `replay-analyzer`'s canonical model
//! (identity-coalesced tracks, gap-aware fixed-rate resampling, validated
//! against ballchasing and an independent reconstructor), so the bridge is a
//! thin, lossy-on-purpose projection of the resampled grid:
//!
//! - One [`WorldState`] per grid frame, in world coordinates (the domain's
//!   goalside/geometry math is world-frame like the grid, blue defending `-Y`).
//! - A car absent from a grid frame (demolished / not yet spawned — the grid is
//!   gap-aware and never carries a dead car forward) is simply absent, which is
//!   exactly the domain's contract: `demolished` stays `false` and absence
//!   means "no facts this frame".
//! - Boost is rescaled from the raw replicated byte (0..=255) to the domain's
//!   0..=100.

use replay_analyzer::field;
use replay_analyzer::model::CanonicalMatch;
use std::collections::{BTreeMap, BTreeSet};

use crate::{BallState, PlayerId, PlayerState, Pose, Quat, Team, Timeline, Vec3, WorldState};

/// Boost carried before the first gauge observation. A freshly spawned car in
/// Rocket League holds 33 boost, and the grid's boost is `None` only until the
/// first replicated observation, so 33 is the honest fill-in.
const SPAWN_BOOST: u8 = 33;

/// Project the canonical resampled grid into a [`Timeline`].
///
/// Cars without a resolved team (or with a non-0/1 team id) are omitted rather
/// than guessed at, matching the domain's "unresolved cars are omitted"
/// contract. Player display names are attached on each player's first frame
/// only; [`crate::roster`] picks names up from any frame that carries one.
pub fn timeline_from_canonical(m: &CanonicalMatch) -> Timeline {
    let names: BTreeMap<i32, &str> = m
        .tracks
        .iter()
        .map(|tr| (tr.pri, tr.player.as_str()))
        .collect();
    let mut named: BTreeSet<i32> = BTreeSet::new();

    m.resampled
        .frames
        .iter()
        .map(|f| WorldState {
            t: f.t,
            ball: f.ball.as_ref().map(|kin| BallState {
                pose: Pose {
                    position: vec3(kin.p),
                    // No dimension reads ball orientation; the grid doesn't
                    // carry it either.
                    rotation: Quat::IDENTITY,
                },
                velocity: vec3(kin.v),
            }),
            players: f
                .cars
                .iter()
                .filter_map(|car| {
                    let team = Team::from_side(u8::try_from(car.team?).ok()?)?;
                    let player = PlayerId(u32::try_from(car.pri).ok()?);
                    let name = if named.insert(car.pri) {
                        names.get(&car.pri).map(|n| n.to_string())
                    } else {
                        None
                    };
                    Some(PlayerState {
                        player,
                        team,
                        pose: Pose {
                            position: vec3(car.p),
                            // The grid carries Euler orientation, but no current
                            // dimension reads it; convert when one does.
                            rotation: Quat::IDENTITY,
                        },
                        velocity: vec3(car.v),
                        boost: car
                            .boost
                            .map(|b| field::boost_percent(b).round() as u8)
                            .unwrap_or(SPAWN_BOOST),
                        // The gap-aware grid omits dead cars instead of carrying
                        // them, so absence already encodes demolition.
                        demolished: false,
                        name,
                    })
                })
                .collect(),
        })
        .collect()
}

fn vec3(v: replay_analyzer::model::Vec3) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_analyzer::model::{GridCar, GridFrame, Kin, Resampled};

    fn kin(x: f32, y: f32, z: f32) -> Kin {
        Kin {
            p: replay_analyzer::model::Vec3 { x, y, z },
            v: replay_analyzer::model::Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
        }
    }

    fn car(pri: i32, team: Option<i32>, boost: Option<u8>) -> GridCar {
        GridCar {
            pri,
            team,
            p: replay_analyzer::model::Vec3 {
                x: 1.0,
                y: 2.0,
                z: 17.0,
            },
            v: replay_analyzer::model::Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            boost,
            rot: None,
        }
    }

    /// A minimal canonical match wrapping the given grid frames; everything the
    /// bridge doesn't read is left empty.
    fn canonical(frames: Vec<GridFrame>) -> CanonicalMatch {
        CanonicalMatch {
            replay_id: "test".into(),
            parser_version: "test".into(),
            analyzer_version: "test".into(),
            map: None,
            team_size: Some(2),
            record_fps: Some(30.0),
            num_frames: frames.len(),
            duration_s: frames.len() as f32 / 30.0,
            team_scores: Default::default(),
            players: Vec::new(),
            tracks: Vec::new(),
            frames: Vec::new(),
            resampled: Resampled {
                hz: 30.0,
                team_attack_sign: Default::default(),
                frames,
            },
            events: Vec::new(),
            features: Vec::new(),
            pickups: Vec::new(),
            powerslides: Vec::new(),
        }
    }

    #[test]
    fn maps_grid_frames_to_world_states() {
        let m = canonical(vec![GridFrame {
            t: 1.5,
            ball: Some(kin(10.0, 20.0, 93.0)),
            cars: vec![car(3, Some(0), Some(255)), car(7, Some(1), Some(0))],
        }]);
        let tl = timeline_from_canonical(&m);

        assert_eq!(tl.len(), 1);
        assert_eq!(tl[0].t, 1.5);
        let ball = tl[0].ball.as_ref().expect("ball present");
        assert_eq!(ball.pose.position, Vec3::new(10.0, 20.0, 93.0));
        assert_eq!(tl[0].players.len(), 2);
        assert_eq!(tl[0].players[0].player, PlayerId(3));
        assert_eq!(tl[0].players[0].team, Team::Blue);
        assert_eq!(tl[0].players[0].boost, 100, "raw 255 -> 100");
        assert_eq!(tl[0].players[1].team, Team::Orange);
        assert_eq!(tl[0].players[1].boost, 0);
    }

    #[test]
    fn unresolved_team_and_unobserved_boost() {
        let m = canonical(vec![GridFrame {
            t: 0.0,
            ball: None,
            cars: vec![car(1, None, None), car(2, Some(0), None)],
        }]);
        let tl = timeline_from_canonical(&m);

        assert!(tl[0].ball.is_none(), "ball-less frame stays ball-less");
        assert_eq!(tl[0].players.len(), 1, "team-less car omitted, not guessed");
        assert_eq!(
            tl[0].players[0].boost, SPAWN_BOOST,
            "unobserved -> spawn value"
        );
    }

    #[test]
    fn name_attaches_on_first_appearance_and_reaches_roster() {
        let mut m = canonical(vec![
            GridFrame {
                t: 0.0,
                ball: None,
                cars: vec![car(3, Some(0), Some(85))],
            },
            GridFrame {
                t: 0.1,
                ball: None,
                cars: vec![car(3, Some(0), Some(85))],
            },
        ]);
        m.tracks.push(replay_analyzer::model::PlayerTrack {
            player: "tester".into(),
            pri: 3,
            team: Some(0),
            num_segments: 1,
            samples: Vec::new(),
            gaps: Vec::new(),
        });
        let tl = timeline_from_canonical(&m);

        assert_eq!(tl[0].players[0].name.as_deref(), Some("tester"));
        assert_eq!(tl[1].players[0].name, None, "first appearance only");
        let roster = crate::roster(&tl);
        assert_eq!(roster.len(), 1);
        assert_eq!(roster[0].name.as_deref(), Some("tester"));
    }
}
