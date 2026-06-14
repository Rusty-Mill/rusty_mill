//! Role-assignment hysteresis: a brief time_to_ball blip must NOT swap the 1st
//! man, but a sustained lead (≥ role_min_persist_s) must.

use replay_analyzer::model::Vec3;
use replay_scoring::config::ScoreConfig;
use replay_scoring::features::{CarView, FrameView, Third};
use replay_scoring::roles::assign;

fn zero() -> Vec3 {
    Vec3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    }
}

/// A car whose only relevant field is `time_to_ball`.
fn cv(pri: i32, ttb: f32) -> CarView {
    CarView {
        pri,
        team: 0,
        p: zero(),
        v: zero(),
        pa: zero(),
        boost: None,
        valid_pos: true,
        dist_to_ball: ttb * 100.0,
        closing_speed: 100.0,
        time_to_ball: ttb,
        goalside: false,
        ball_third: Third::Mid,
    }
}

/// pri1 leads except a short blip and then a sustained takeover by pri2.
fn frames() -> Vec<FrameView> {
    let mut out = Vec::new();
    let mut t = 0.0;
    let push = |t: f32, ttb1: f32, ttb2: f32| FrameView {
        t,
        ball: None,
        cars: vec![cv(1, ttb1), cv(2, ttb2)],
    };
    // 0.0..0.9: pri1 clearly leads.
    for _ in 0..10 {
        out.push(push(t, 0.5, 1.5));
        t += 0.1;
    }
    // 1.0..1.1: pri2 briefly faster (0.2 s blip < 0.4 s persist).
    out.push(push(t, 1.5, 0.5));
    t += 0.1;
    out.push(push(t, 1.5, 0.5));
    t += 0.1;
    // back to pri1.
    for _ in 0..5 {
        out.push(push(t, 0.5, 1.5));
        t += 0.1;
    }
    // 1.7..: pri2 sustained faster (>= 0.4 s).
    for _ in 0..10 {
        out.push(push(t, 1.5, 0.5));
        t += 0.1;
    }
    out
}

#[test]
fn brief_blip_does_not_swap_first_man_but_sustained_lead_does() {
    let fs = frames();
    let roles = assign(&fs, &[0], &ScoreConfig::default());

    // Early: pri1 is 1st.
    assert_eq!(roles.first(0, 0), Some(1));
    // During the 2-frame blip (index ~11): still pri1 (hysteresis holds).
    let blip_idx = fs.iter().position(|f| (f.t - 1.1).abs() < 1e-3).unwrap();
    assert_eq!(
        roles.first(blip_idx, 0),
        Some(1),
        "brief blip must not swap"
    );
    // After a sustained takeover (last frame): pri2 is 1st.
    assert_eq!(
        roles.first(fs.len() - 1, 0),
        Some(2),
        "sustained lead must swap"
    );
}
