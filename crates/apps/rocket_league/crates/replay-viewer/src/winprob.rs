//! Optional momentum overlay: a coarse P(team 0 scores the next goal) curve.
//!
//! A bridge to `replay_value`: it trains the (deliberately simple, logistic)
//! value model on the match's own outcomes, then samples its prediction evenly
//! across the grid into a small curve the viewer draws under the timeline. Kept
//! separate from [`crate::scene`] so the core projection stays decoupled.
//!
//! The model is basic (held-out AUC ≈ 0.71), so read the strip as rough
//! momentum, not a calibrated win probability.

use replay_analyzer::model::CanonicalMatch;
use replay_value::{evaluate, features, ValueConfig};

use crate::scene::Scene;

/// Number of evenly-spaced samples in the curve — enough for a smooth strip,
/// small in the payload.
const SAMPLES: usize = 300;

/// Fill [`Scene::win_prob`] with P(team 0 scores next), sampled across the match.
pub fn attach_winprob(scene: &mut Scene, m: &CanonicalMatch, cfg: &ValueConfig) {
    let frames = &m.resampled.frames;
    if frames.is_empty() {
        return;
    }
    let ev = evaluate(m, cfg);
    let signs = &m.resampled.team_attack_sign;
    let n = SAMPLES.min(frames.len());
    let last = frames.len() - 1;

    let mut out = Vec::with_capacity(n);
    for k in 0..n {
        let idx = if n > 1 { k * last / (n - 1) } else { 0 };
        // Guard against a non-finite prediction (e.g. a zero-variance scaler on a
        // degenerate match) so the curve is always valid `[0, 1]`.
        let p = features::features_at(frames, idx, 0, signs, &m.events)
            .map(|sf| ev.model.predict(&sf.x))
            .filter(|p| p.is_finite())
            .unwrap_or(0.5)
            .clamp(0.0, 1.0);
        out.push((p * 1000.0).round() / 1000.0);
    }
    scene.win_prob = out;
}
