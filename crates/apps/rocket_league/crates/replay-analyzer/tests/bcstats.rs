//! Ballchasing-parity aggregate tests (`analyze::bcstats`).
//!
//! A synthetic single-player grid with hand-picked speeds, heights, boost values
//! and positions, so every bucket has a known count; plus the distribution
//! invariants (speed/air/quartile/third shares each sum to 100%).

use replay_analyzer::analyze::bcstats::ballchasing_stats;
use replay_analyzer::model::{
    CanonicalMatch, Event, GapReason, GridCar, GridFrame, Kin, PlayerTrack, Resampled, TrackSample,
    Vec3,
};
use std::collections::BTreeMap;

fn v(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

fn car(pri: i32, p: Vec3, vx: f32, boost: Option<u8>) -> GridCar {
    GridCar {
        pri,
        team: Some(0),
        p,
        v: v(vx, 0.0, 0.0),
        boost,
        rot: None,
    }
}

/// 10 Hz grid, player pri=1 (team 0, attacks +Y). A teammate pri=2 shadows it
/// 300 uu ahead every frame (constant mate distance). Frames chosen so each
/// speed / air / boost / third bucket gets a known count.
fn match_with(frames: Vec<GridFrame>, events: Vec<Event>, track: PlayerTrack) -> CanonicalMatch {
    CanonicalMatch {
        replay_id: "t".into(),
        parser_version: "test".into(),
        analyzer_version: "test".into(),
        map: None,
        team_size: Some(2),
        record_fps: None,
        num_frames: 0,
        duration_s: 0.0,
        team_scores: BTreeMap::new(),
        players: vec![],
        tracks: vec![track],
        frames: vec![],
        resampled: Resampled {
            hz: 10.0,
            team_attack_sign: BTreeMap::from([(0, 1)]),
            frames,
        },
        events,
        features: vec![],
    }
}

fn frame(t: f32, p: Vec3, vx: f32, boost: u8, ball: Option<Vec3>) -> GridFrame {
    GridFrame {
        t,
        ball: ball.map(|bp| Kin {
            p: bp,
            v: v(0.0, 0.0, 0.0),
        }),
        // teammate 300 uu ahead in +x, same team — constant mate distance.
        cars: vec![
            car(1, p, vx, Some(boost)),
            car(2, v(p.x + 300.0, p.y, p.z), 0.0, Some(50)),
        ],
    }
}

fn track() -> PlayerTrack {
    // Native-res boost mirror: 255→130→0→200. Used 255 bytes, collected 200.
    let s = |t: f32, b: u8| TrackSample {
        t,
        actor_id: 1,
        p: v(0.0, 0.0, 17.0),
        v: v(0.0, 0.0, 0.0),
        boost: Some(b),
        rot: None,
    };
    PlayerTrack {
        player: "P".into(),
        pri: 1,
        team: Some(0),
        num_segments: 1,
        samples: vec![s(0.0, 255), s(0.1, 130), s(0.2, 0), s(0.3, 200)],
        gaps: vec![],
    }
}

fn near(a: f32, b: f32, eps: f32) -> bool {
    (a - b).abs() < eps
}

fn scenario() -> CanonicalMatch {
    let ball = Some(v(0.0, 500.0, 93.0));
    let frames = vec![
        // slow / ground / full-boost(q3) / neutral-third / behind
        frame(0.0, v(0.0, 0.0, 17.0), 1000.0, 255, ball),
        // boost-speed / ground / q2 / defensive-third / behind
        frame(0.1, v(0.0, -2000.0, 17.0), 1500.0, 130, ball),
        // supersonic / high-air / zero(q0) / offensive-third / in-front
        frame(0.2, v(0.0, 3000.0, 700.0), 2300.0, 0, ball),
        // slow / low-air / q3 / neutral-third / (no ball)
        frame(0.3, v(0.0, 0.0, 300.0), 0.0, 200, None),
    ];
    let events = vec![
        Event::Demo {
            t: 1.0,
            attacker_pri: Some(1),
            attacker: Some("P".into()),
            victim_pri: Some(2),
            victim: Some("Q".into()),
        },
        Event::Demo {
            t: 2.0,
            attacker_pri: Some(2),
            attacker: Some("Q".into()),
            victim_pri: Some(1),
            victim: Some("P".into()),
        },
    ];
    match_with(frames, events, track())
}

#[test]
fn buckets_match_hand_counts() {
    let s = ballchasing_stats(&scenario());
    let p = s.iter().find(|p| p.pri == 1).expect("pri 1");

    // 4 present frames @ 0.1 s. Speeds 1000/1500/2300/0 → slow×2, boost×1, super×1.
    assert!(near(p.movement.avg_speed, 1200.0, 1e-3));
    assert!(near(p.movement.total_distance, 480.0, 1e-2)); // 4800 uu/s · 0.1 s
    assert!(near(p.movement.percent_slow, 50.0, 1e-3));
    assert!(near(p.movement.percent_boost_speed, 25.0, 1e-3));
    assert!(near(p.movement.percent_supersonic, 25.0, 1e-3));
    assert!(near(p.movement.time_supersonic_s, 0.1, 1e-4));

    // Heights 17/17/700/300 → ground×2, high-air×1, low-air×1.
    assert!(near(p.movement.percent_ground, 50.0, 1e-3));
    assert!(near(p.movement.percent_high_air, 25.0, 1e-3));
    assert!(near(p.movement.percent_low_air, 25.0, 1e-3));

    // Boost 255/130/0/200 → zero×1, full×1, quartiles q0×1,q2×1,q3×2.
    assert!(near(p.boost.percent_zero, 25.0, 1e-3));
    assert!(near(p.boost.percent_full, 25.0, 1e-3));
    assert!(near(p.boost.percent_0_25, 25.0, 1e-3));
    assert!(near(p.boost.percent_25_50, 0.0, 1e-3));
    assert!(near(p.boost.percent_50_75, 25.0, 1e-3));
    assert!(near(p.boost.percent_75_100, 50.0, 1e-3));
    // avg gauge = (100 + 50.98 + 0 + 78.43)/4 ≈ 57.35 %.
    assert!(near(p.boost.avg_amount, 57.35, 0.1));
    // collected 200 bytes / 2.55, used 255 / 2.55.
    assert!(near(p.boost.amount_collected, 78.43, 0.1));
    assert!(near(p.boost.amount_used, 100.0, 0.1));
    assert!(p.boost.bpm > 0.0 && p.boost.bcpm > 0.0);

    // Positioning (sign +1): y = 0/-2000/3000/0 → def×1, neutral×2, off×1.
    assert!(near(p.positioning.percent_defensive_third, 25.0, 1e-3));
    assert!(near(p.positioning.percent_neutral_third, 50.0, 1e-3));
    assert!(near(p.positioning.percent_offensive_third, 25.0, 1e-3));
    // behind ball: frames with a ball = 3; behind×2 (y<500), in-front×1.
    assert!(near(p.positioning.percent_behind_ball, 66.667, 0.05));
    assert!(near(p.positioning.percent_infront_ball, 33.333, 0.05));
    // teammate is a constant 300 uu away.
    assert!(near(p.positioning.avg_dist_to_mates, 300.0, 1e-2));

    // Demos from events.
    assert_eq!(p.demo.inflicted, 1);
    assert_eq!(p.demo.taken, 1);
}

#[test]
fn distributions_sum_to_full() {
    let s = ballchasing_stats(&scenario());
    let p = s.iter().find(|p| p.pri == 1).expect("pri 1");
    let m = &p.movement;
    assert!(near(
        m.percent_slow + m.percent_boost_speed + m.percent_supersonic,
        100.0,
        1e-2
    ));
    assert!(near(
        m.percent_ground + m.percent_low_air + m.percent_high_air,
        100.0,
        1e-2
    ));
    let b = &p.boost;
    assert!(near(
        b.percent_0_25 + b.percent_25_50 + b.percent_50_75 + b.percent_75_100,
        100.0,
        1e-2
    ));
    let pos = &p.positioning;
    assert!(near(
        pos.percent_defensive_third + pos.percent_neutral_third + pos.percent_offensive_third,
        100.0,
        1e-2
    ));
    assert!(near(
        pos.percent_defensive_half + pos.percent_offensive_half,
        100.0,
        1e-2
    ));
}

#[test]
fn boost_flow_skips_respawn_gap() {
    // A decrease bracketing a respawn gap must not count as "used".
    let mut t = track();
    t.samples = vec![
        TrackSample {
            t: 0.0,
            actor_id: 1,
            p: v(0.0, 0.0, 17.0),
            v: v(0.0, 0.0, 0.0),
            boost: Some(200),
            rot: None,
        },
        TrackSample {
            t: 1.0,
            actor_id: 2,
            p: v(0.0, 0.0, 17.0),
            v: v(0.0, 0.0, 0.0),
            boost: Some(20),
            rot: None,
        },
    ];
    t.gaps = vec![replay_analyzer::model::TrackGap {
        start: 0.0,
        end: 1.0,
        reason: GapReason::Respawn,
    }];
    let m = match_with(vec![], vec![], t);
    let s = ballchasing_stats(&m);
    assert!(near(s[0].boost.amount_used, 0.0, 1e-3));
}

#[test]
fn ordering_possession_and_last_defender() {
    // team 0: pri 1 always back (y=-2000), pri 2 always forward (y=+2000).
    // Team 0 holds the ball for the first half, team 1 the second; a team-1 goal
    // at t=0.25 is conceded by the back-most team-0 player (pri 1).
    let two = |t: f32| GridFrame {
        t,
        ball: Some(Kin {
            p: v(0.0, 0.0, 93.0),
            v: v(0.0, 0.0, 0.0),
        }),
        cars: vec![
            car(1, v(0.0, -2000.0, 17.0), 0.0, Some(50)),
            car(2, v(0.0, 2000.0, 17.0), 0.0, Some(50)),
        ],
    };
    let frames = vec![two(0.0), two(0.1), two(0.2), two(0.3)];
    let tk = |pri: i32| PlayerTrack {
        player: format!("P{pri}"),
        pri,
        team: Some(0),
        num_segments: 1,
        samples: vec![
            TrackSample {
                t: 0.0,
                actor_id: pri,
                p: v(0.0, 0.0, 17.0),
                v: v(0.0, 0.0, 0.0),
                boost: Some(50),
                rot: None,
            },
            TrackSample {
                t: 0.3,
                actor_id: pri,
                p: v(0.0, 0.0, 17.0),
                v: v(0.0, 0.0, 0.0),
                boost: Some(50),
                rot: None,
            },
        ],
        gaps: vec![],
    };
    let events = vec![
        Event::Possession {
            team: 0,
            start: 0.0,
            end: 0.15,
            touches: 1,
        },
        Event::Possession {
            team: 1,
            start: 0.2,
            end: 0.35,
            touches: 1,
        },
        Event::Goal {
            t: 0.25,
            scorer: None,
            team: Some(1),
        },
    ];
    let mut m = match_with(frames, events, tk(1));
    m.tracks.push(tk(2));

    let s = ballchasing_stats(&m);
    let p1 = s.iter().find(|p| p.pri == 1).unwrap();
    let p2 = s.iter().find(|p| p.pri == 2).unwrap();

    // pri 1 is the back-most every frame; pri 2 the forward-most.
    assert!(near(p1.positioning.percent_most_back, 100.0, 1e-3));
    assert!(near(p1.positioning.percent_most_forward, 0.0, 1e-3));
    assert!(near(p2.positioning.percent_most_forward, 100.0, 1e-3));
    assert!(near(p2.positioning.percent_most_back, 0.0, 1e-3));

    // Possession split: 2 frames team-0 possession, 2 frames team-1 (= no
    // possession for team 0); both distances are the same ~2001 uu here.
    assert!(near(
        p1.positioning.avg_dist_to_ball_possession,
        2001.4,
        1.0
    ));
    assert!(near(
        p1.positioning.avg_dist_to_ball_no_possession,
        2001.4,
        1.0
    ));

    // The conceded goal is charged to the last defender (back-most), pri 1.
    assert_eq!(p1.positioning.goals_against_while_last_defender, 1);
    assert_eq!(p2.positioning.goals_against_while_last_defender, 0);
}
