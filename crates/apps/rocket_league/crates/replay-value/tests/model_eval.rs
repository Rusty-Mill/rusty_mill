//! Model training mechanics on a synthetic separable set, and ΔV credit wiring
//! through the real `per_player_delta_v` path on a hand-built match.

use replay_analyzer::model::{
    CanonicalMatch, Event, GridCar, GridFrame, Kin, PlayerTrack, Resampled, Vec3,
};
use replay_value::config::{TrainConfig, ValueConfig};
use replay_value::dataset::{Dataset, Row};
use replay_value::features::N_FEATURES;
use replay_value::model::{Scaler, ValueModel};
use replay_value::per_player_delta_v;
use std::collections::BTreeMap;

#[test]
fn model_learns_a_separable_signal() {
    // y = 1 iff feature 0 > 0; features 1 and 2 are y-independent pseudo-noise.
    let mut rows = Vec::new();
    for i in 0..240i32 {
        let x0 = (i as f32 / 240.0) * 2.0 - 1.0; // -1.0 ..< 1.0, all distinct
        let mut x = [0.0f32; N_FEATURES];
        x[0] = x0;
        x[1] = ((i * 37) % 100) as f32 / 100.0 - 0.5;
        x[2] = ((i * 53) % 100) as f32 / 100.0 - 0.5;
        rows.push(Row {
            t: i as f32,
            team: 0,
            x,
            y: (x0 > 0.0) as u8 as f32,
        });
    }
    let ds = Dataset {
        rows,
        horizon_s: 10.0,
    };
    let model = ValueModel::train(&ds, &TrainConfig::default());

    let correct = ds
        .rows
        .iter()
        .filter(|r| (model.predict(&r.x) > 0.5) == (r.y > 0.5))
        .count();
    let acc = correct as f32 / ds.rows.len() as f32;
    assert!(acc > 0.95, "accuracy {acc}");
    assert!(model.log_loss(&ds) < 0.2, "loss {}", model.log_loss(&ds));

    // The separating feature must dominate: largest magnitude, positive sign.
    assert!(model.w[0] > 0.0, "w0 {}", model.w[0]);
    for j in 1..N_FEATURES {
        assert!(
            model.w[0].abs() > model.w[j].abs(),
            "w0 ({}) not dominant vs w{j} ({})",
            model.w[0],
            model.w[j]
        );
    }
}

#[test]
fn untrained_model_is_neutral() {
    let empty = Dataset {
        rows: vec![],
        horizon_s: 10.0,
    };
    let model = ValueModel::train(&empty, &TrainConfig::default());
    assert_eq!(model.n_train, 0);
    assert_eq!(model.predict(&[0.3; N_FEATURES]), 0.5);
}

fn v(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

fn gcar(pri: i32, team: i32, p: Vec3) -> GridCar {
    GridCar {
        pri,
        team: Some(team),
        p,
        v: v(0.0, 0.0, 0.0),
        boost: Some(128),
        rot: None,
    }
}

/// A model with an identity scaler and a single positive weight on `ball_y`.
fn ball_y_model() -> ValueModel {
    let mut w = [0.0f32; N_FEATURES];
    w[0] = 5.0;
    ValueModel {
        w,
        b: 0.0,
        scaler: Scaler {
            mean: [0.0; N_FEATURES],
            std: [1.0; N_FEATURES],
        },
        n_train: 1,
    }
}

#[test]
fn delta_v_credits_a_touch_that_advances_the_ball() {
    // Team 0 attacks +Y. A touch at t=0 with the ball deep in own half; 0.5 s
    // later the ball is deep in the opponent half -> the toucher gets +ΔV.
    let signs = BTreeMap::from([(0, 1)]);
    let frames = vec![
        GridFrame {
            t: 0.0,
            ball: Some(Kin {
                p: v(0.0, -3000.0, 100.0),
                v: v(0.0, 0.0, 0.0),
            }),
            cars: vec![gcar(1, 0, v(0.0, -2000.0, 17.0))],
        },
        GridFrame {
            t: 0.5,
            ball: Some(Kin {
                p: v(0.0, 3000.0, 100.0),
                v: v(0.0, 0.0, 0.0),
            }),
            cars: vec![gcar(1, 0, v(0.0, 2000.0, 17.0))],
        },
    ];
    let m = CanonicalMatch {
        replay_id: "t".into(),
        parser_version: "p".into(),
        analyzer_version: "a".into(),
        map: None,
        team_size: Some(1),
        record_fps: None,
        num_frames: 2,
        duration_s: 0.5,
        team_scores: BTreeMap::new(),
        players: vec![],
        tracks: vec![PlayerTrack {
            player: "P1".into(),
            pri: 1,
            team: Some(0),
            num_segments: 0,
            samples: vec![],
            gaps: vec![],
        }],
        frames: vec![],
        resampled: Resampled {
            hz: 30.0,
            team_attack_sign: signs,
            frames,
        },
        events: vec![Event::Touch {
            t: 0.0,
            pri: 1,
            player: Some("P1".into()),
            team: Some(0),
        }],
        features: vec![],
    };

    let pv = per_player_delta_v(&m, &ball_y_model(), &ValueConfig::default());
    assert_eq!(pv.len(), 1);
    assert_eq!(pv[0].pri, 1);
    assert_eq!(pv[0].touches, 1);
    assert_eq!(pv[0].player.as_deref(), Some("P1"));
    // ball_y −0.586 -> +0.586 with w0=5 ⇒ ~0.95 − 0.05.
    assert!(pv[0].sum_dv > 0.8, "sum_dv {}", pv[0].sum_dv);
}
