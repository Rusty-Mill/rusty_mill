//! Unit tests for T2 fixed-rate resampling: uniform grid, linear interpolation,
//! and gap-aware liveness (interpolation must never bridge a respawn gap).

use replay_analyzer::analyze::reconstruct::Reconstruction;
use replay_analyzer::analyze::resample::resample;
use replay_analyzer::model::{GapReason, GridFrame, PlayerTrack, TrackGap, TrackSample, Vec3};

fn s(t: f32, x: f32) -> TrackSample {
    TrackSample {
        t,
        actor_id: 1,
        p: Vec3 { x, y: 0.0, z: 17.0 },
        v: Vec3 {
            x: 10.0,
            y: 0.0,
            z: 0.0,
        },
        boost: Some((t * 10.0).round() as u8),
        rot: None,
    }
}

/// One player live on [0,1] and [2,3] with a dead gap on (1,2); a ball moving
/// in +Y over [0,1].
fn recon() -> Reconstruction {
    let track = PlayerTrack {
        player: "P".into(),
        pri: 1,
        team: Some(0),
        num_segments: 2,
        samples: vec![s(0.0, 0.0), s(1.0, 300.0), s(2.0, 300.0), s(3.0, 600.0)],
        gaps: vec![TrackGap {
            start: 1.0,
            end: 2.0,
            reason: GapReason::Respawn,
        }],
    };
    let ball_samples = vec![
        TrackSample {
            t: 0.0,
            actor_id: 99,
            p: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 93.0,
            },
            v: Vec3 {
                x: 0.0,
                y: 1000.0,
                z: 0.0,
            },
            boost: None,
            rot: None,
        },
        TrackSample {
            t: 1.0,
            actor_id: 99,
            p: Vec3 {
                x: 0.0,
                y: 1000.0,
                z: 93.0,
            },
            v: Vec3 {
                x: 0.0,
                y: 1000.0,
                z: 0.0,
            },
            boost: None,
            rot: None,
        },
    ];
    Reconstruction {
        frames: Vec::new(),
        tracks: vec![track],
        ball_samples,
        demos: Vec::new(),
        pickups: Vec::new(),
        powerslides: Vec::new(),
        pri_scores: Default::default(),
        stat_events: Vec::new(),
        pri_body: Default::default(),
        pri_camera: Default::default(),
        pri_steer: Default::default(),
    }
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.5
}

/// The grid frame whose time is closest to `t` (grid spacing is coarse here).
fn at_time(grid: &[GridFrame], t: f32) -> &GridFrame {
    grid.iter()
        .min_by(|a, b| (a.t - t).abs().total_cmp(&(b.t - t).abs()))
        .expect("non-empty grid")
}

#[test]
fn grid_is_uniform_at_requested_rate() {
    let grid = resample(&recon(), 10.0);
    assert!(!grid.is_empty());
    // Span [0,3] at 10 Hz, inclusive => 31 ticks.
    assert_eq!(grid.len(), 31);
    for w in grid.windows(2) {
        assert!(
            near(w[1].t - w[0].t, 0.1),
            "non-uniform dt: {} -> {}",
            w[0].t,
            w[1].t
        );
    }
}

#[test]
fn positions_are_linearly_interpolated() {
    let grid = resample(&recon(), 10.0);
    // t=0.5 -> halfway between x=0 and x=300.
    let f = at_time(&grid, 0.5);
    assert!(near(f.t, 0.5), "located t={}", f.t);
    let car = f.cars.iter().find(|c| c.pri == 1).expect("car live at 0.5");
    assert!(near(car.p.x, 150.0), "interp x = {}", car.p.x);
    // Boost is carried from the sample at/before t (step), not interpolated:
    // at t=0.5 that is the t=0 sample (boost 0), not 5.
    assert_eq!(car.boost, Some(0), "boost carried from t=0 sample");

    // Ball moving +Y: at t=0.5, y halfway to 1000.
    let ball = f.ball.expect("ball at 0.5");
    assert!(near(ball.p.y, 500.0), "ball y = {}", ball.p.y);
    assert!(near(ball.v.y, 1000.0), "ball vy = {}", ball.v.y);
}

#[test]
fn dead_actor_is_absent_during_gap_but_present_around_it() {
    let grid = resample(&recon(), 10.0);

    let present = |t: f32| at_time(&grid, t).cars.iter().any(|c| c.pri == 1);

    // Live before the gap, absent inside it, live after it.
    assert!(present(0.5), "should be live at 0.5");
    assert!(!present(1.5), "must be absent inside gap (1,2)");
    assert!(present(2.5), "should be live again at 2.5");

    // Crucially, interpolation never bridged the gap with a phantom position.
    let inside_gap = grid
        .iter()
        .filter(|f| f.t > 1.0 && f.t < 2.0)
        .all(|f| f.cars.is_empty());
    assert!(inside_gap, "no car samples anywhere inside the gap");
}

#[test]
fn empty_reconstruction_yields_empty_grid() {
    let empty = Reconstruction {
        frames: Vec::new(),
        tracks: Vec::new(),
        ball_samples: Vec::new(),
        demos: Vec::new(),
        pickups: Vec::new(),
        powerslides: Vec::new(),
        pri_scores: Default::default(),
        stat_events: Vec::new(),
        pri_body: Default::default(),
        pri_camera: Default::default(),
        pri_steer: Default::default(),
    };
    assert!(resample(&empty, 30.0).is_empty());
}
