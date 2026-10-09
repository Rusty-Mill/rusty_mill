//! Optional **impact overlay**: per-player and per-touch ΔV (scoring-prob swing).
//!
//! A bridge to `replay_value`, sibling to [`crate::winprob`]: it trains the value
//! model on the match's own outcomes, credits each touch the change it caused in
//! its team's scoring probability (ΔV), and writes those swings back onto the
//! scene — a per-player **impact** total in the roster, and a per-touch `dv` the
//! viewer tints (+ helped / − hurt). Kept separate from [`crate::scene`] so the
//! core projection stays decoupled from the value model.
//!
//! The model is basic (see [`crate::winprob`]); read ΔV as a rough impact signal,
//! not a calibrated credit.

use std::collections::BTreeMap;

use replay_analyzer::model::CanonicalMatch;
use replay_value::{evaluate, per_touch_delta_v, ValueConfig};

use crate::scene::Scene;

/// Round to 3 decimals — ΔV is a small probability swing; keeps the payload tight.
fn round3(x: f32) -> f32 {
    (x * 1000.0).round() / 1000.0
}

/// Quantize a time to the scene's 2-decimal grid, for exact `(pri, t)` keying.
fn time_key(t: f32) -> i64 {
    (t * 100.0).round() as i64
}

/// Credit per-player total ΔV ([`crate::scene::ScenePlayer::impact`]) and tag each
/// `touch` event with its swing ([`crate::scene::SceneEvent::dv`]).
pub fn attach_impact(scene: &mut Scene, m: &CanonicalMatch, cfg: &ValueConfig) {
    if m.resampled.frames.is_empty() {
        return;
    }
    let model = evaluate(m, cfg).model;
    let touches = per_touch_delta_v(m, &model, cfg);

    // Per-player impact: total ΔV over the player's credited touches.
    let mut sum: BTreeMap<i32, f32> = BTreeMap::new();
    for tv in &touches {
        *sum.entry(tv.pri).or_default() += tv.dv;
    }
    for p in &mut scene.players {
        if let Some(s) = sum.get(&p.pri) {
            p.impact = Some(round3(*s));
        }
    }

    // Tag each touch event with the ΔV of the touch at the same `(pri, time)`. The
    // scene rounds event times to the same 2-decimal grid the ΔV times quantize
    // to, so the key matches exactly — no tolerance window (which would risk
    // crediting a neighbouring touch).
    let by_key: BTreeMap<(i32, i64), f32> = touches
        .iter()
        .map(|tv| ((tv.pri, time_key(tv.t)), tv.dv))
        .collect();
    for ev in &mut scene.events {
        if ev.kind != "touch" {
            continue;
        }
        if let Some(pri) = ev.pri {
            if let Some(dv) = by_key.get(&(pri, time_key(ev.t))) {
                ev.dv = Some(round3(*dv));
            }
        }
    }
}
