//! Unit tests for T4 event detectors: touch detection (velocity discontinuity +
//! nearest car), possession chaining, kickoff detection, and demo dedup.

use replay_analyzer::analyze::events::{demos, kickoffs, possessions, touches};
use replay_analyzer::analyze::reconstruct::DemoSample;
use replay_analyzer::model::{
    Event, GridCar, GridFrame, Kin, PlayerTrack, Resampled, TrackSample, Vec3,
};
use std::collections::BTreeMap;

fn v(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

fn car(pri: i32, team: i32, p: Vec3) -> GridCar {
    GridCar {
        pri,
        team: Some(team),
        p,
        v: v(0.0, 0.0, 0.0),
        boost: Some(50),
        rot: None,
    }
}

fn track(pri: i32, team: i32) -> PlayerTrack {
    PlayerTrack {
        player: format!("p{pri}"),
        pri,
        team: Some(team),
        num_segments: 1,
        samples: vec![TrackSample {
            t: 0.0,
            actor_id: pri,
            p: v(0.0, 0.0, 0.0),
            v: v(0.0, 0.0, 0.0),
            boost: None,
            rot: None,
        }],
        gaps: vec![],
    }
}

fn grid(frames: Vec<GridFrame>) -> Resampled {
    Resampled {
        hz: 30.0,
        team_attack_sign: BTreeMap::new(),
        frames,
    }
}

#[test]
fn touch_detected_on_velocity_flip_near_car_and_attributed_to_nearest() {
    // Ball reverses (dv huge). pri 1 is on the ball; pri 2 is far away.
    let frames = vec![
        GridFrame {
            t: 0.0,
            ball: Some(Kin {
                p: v(0.0, 0.0, 100.0),
                v: v(0.0, -1000.0, 0.0),
            }),
            cars: vec![
                car(1, 0, v(0.0, 0.0, 100.0)),
                car(2, 1, v(3000.0, 0.0, 100.0)),
            ],
        },
        GridFrame {
            t: 0.033,
            ball: Some(Kin {
                p: v(0.0, 0.0, 100.0),
                v: v(0.0, 1000.0, 0.0),
            }),
            cars: vec![
                car(1, 0, v(0.0, 0.0, 100.0)),
                car(2, 1, v(3000.0, 0.0, 100.0)),
            ],
        },
    ];
    let evs = touches(&grid(frames), &[track(1, 0), track(2, 1)]);
    assert_eq!(evs.len(), 1, "exactly one touch");
    match &evs[0] {
        Event::Touch {
            pri, player, team, ..
        } => {
            assert_eq!(*pri, 1, "attributed to the nearest car");
            assert_eq!(player.as_deref(), Some("p1"));
            assert_eq!(*team, Some(0));
        }
        _ => panic!("expected a touch"),
    }
}

#[test]
fn no_touch_when_velocity_jumps_but_no_car_is_near() {
    // A wall/ground bounce: big dv but every car is far from the ball.
    let frames = vec![
        GridFrame {
            t: 0.0,
            ball: Some(Kin {
                p: v(0.0, 0.0, 100.0),
                v: v(0.0, -1000.0, 0.0),
            }),
            cars: vec![car(1, 0, v(3000.0, 3000.0, 100.0))],
        },
        GridFrame {
            t: 0.033,
            ball: Some(Kin {
                p: v(0.0, 0.0, 100.0),
                v: v(0.0, 1000.0, 0.0),
            }),
            cars: vec![car(1, 0, v(3000.0, 3000.0, 100.0))],
        },
    ];
    assert!(touches(&grid(frames), &[track(1, 0)]).is_empty());
}

#[test]
fn possessions_chain_consecutive_same_team_touches() {
    let t = |t: f32, team: i32| Event::Touch {
        t,
        pri: team,
        player: None,
        team: Some(team),
    };
    // team runs: [0,0] then [1,1,1] then [0].
    let touches = vec![
        t(0.0, 0),
        t(1.0, 0),
        t(2.0, 1),
        t(3.0, 1),
        t(4.0, 1),
        t(5.0, 0),
    ];
    let poss = possessions(&touches);
    assert_eq!(poss.len(), 3);
    let teams: Vec<(i32, usize)> = poss
        .iter()
        .map(|p| match p {
            Event::Possession { team, touches, .. } => (*team, *touches),
            _ => panic!(),
        })
        .collect();
    assert_eq!(teams, vec![(0, 2), (1, 3), (0, 1)]);
}

#[test]
fn kickoffs_collapse_centered_stationary_runs() {
    let centered = |t: f32| GridFrame {
        t,
        ball: Some(Kin {
            p: v(0.0, 0.0, 93.0),
            v: v(0.0, 0.0, 0.0),
        }),
        cars: vec![
            car(1, 0, v(0.0, -4608.0, 17.0)),
            car(2, 1, v(0.0, 4608.0, 17.0)),
        ],
    };
    let moving = |t: f32| GridFrame {
        t,
        ball: Some(Kin {
            p: v(0.0, 0.0, 93.0),
            v: v(0.0, 2000.0, 0.0),
        }),
        cars: vec![car(1, 0, v(0.0, -1000.0, 17.0))],
    };
    // A kickoff run, the ball is hit, then a second kickoff run later.
    let frames = vec![
        centered(23.0),
        centered(23.03),
        centered(23.06),
        moving(24.0),
        centered(120.0),
        centered(120.03),
    ];
    let evs = kickoffs(&grid(frames));
    let times: Vec<f32> = evs
        .iter()
        .map(|e| match e {
            Event::Kickoff { t } => *t,
            _ => panic!(),
        })
        .collect();
    assert_eq!(
        times,
        vec![23.0, 120.0],
        "one event per kickoff run, at its start"
    );
}

#[test]
fn demos_dedup_per_victim_refractory_but_keep_distinct() {
    let d = |t: f32, attacker: i32, victim: i32| DemoSample {
        t,
        attacker_pri: Some(attacker),
        victim_pri: Some(victim),
    };
    let samples = vec![
        d(10.0, 1, 2), // victim 2 demoed
        d(10.5, 1, 2), // re-replication of same victim within refractory -> dropped
        d(11.0, 2, 1), // distinct victim (1) -> kept
        d(20.0, 1, 2), // victim 2 again, well after respawn -> kept
    ];
    let evs = demos(&samples, &[track(1, 0), track(2, 1)]);
    assert_eq!(evs.len(), 3);
    match &evs[0] {
        Event::Demo {
            attacker, victim, ..
        } => {
            assert_eq!(attacker.as_deref(), Some("p1"));
            assert_eq!(victim.as_deref(), Some("p2"));
        }
        _ => panic!(),
    }
}

/// The replicated demolish attribute can arrive again up to ~3.5 s after the demolition
/// (measured on a real 3v3 match: duplicates 2.6, 3.0 and 3.5 s apart). Those are the same
/// demolition; a second demo of the same victim well after the respawn is a new one.
#[test]
fn demo_re_replications_up_to_3_5_s_later_are_one_demolition() {
    let d = |t: f32| DemoSample {
        t,
        attacker_pri: Some(1),
        victim_pri: Some(2),
    };
    let times = |samples: Vec<DemoSample>| -> Vec<f32> {
        demos(&samples, &[track(1, 0), track(2, 1)])
            .iter()
            .map(Event::time)
            .collect()
    };
    assert_eq!(
        times(vec![d(35.0), d(37.6)]),
        [35.0],
        "2.6 s apart: duplicate"
    );
    assert_eq!(
        times(vec![d(100.0), d(103.0)]),
        [100.0],
        "3.0 s apart: duplicate"
    );
    assert_eq!(
        times(vec![d(393.0), d(396.5)]),
        [393.0],
        "3.5 s apart: duplicate"
    );
    assert_eq!(
        times(vec![d(10.0), d(14.5)]),
        [10.0, 14.5],
        "4.5 s apart: a new demolition"
    );
}
