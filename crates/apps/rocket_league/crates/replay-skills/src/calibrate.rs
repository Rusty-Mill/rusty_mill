//! Corpus calibration for the skill detectors: turn a labeled replay corpus into
//! evidence about whether detected-skill magnitudes track player rank, and into
//! fitted [`SkillConfig`] anchors.
//!
//! Pure and testable. The companion binary (`src/bin/calibrate_skills.rs`) does the
//! I/O — parse the corpus, `detect_all`, join each player to their rank — and leans
//! on these functions for the statistics and fitting.
//!
//! **Headline: validation.** Per skill, the binary reports the observed metric
//! distribution (p10/p50/p90 of e.g. aerial peak height, power-shot ball speed)
//! and the Spearman correlation between a player's *mean* metric and their rank
//! tier — i.e. "do better-ranked players actually pull bigger aerials / faster
//! shots?". That's the detector-precision check the backlog asks for.
//!
//! **Fitting and candidate-mode.** A *gated* metric is observed only for events
//! that already passed the detector's floor, so a hard minimum can't be lowered
//! from it (selection bias). **Candidate-mode** (`replay_skills::candidate_metrics`)
//! removes that blind spot: each detector also emits the pre-gate metric for
//! *every* candidate event, so the calibrator sees the full distribution and can
//! place the floor as well as the top. For aerials we fit both `aerial_min_height`
//! and `high_aerial_height` from the candidate touch-height distribution; without
//! candidates we fall back to fitting only the top from gated peak heights (and
//! leave floors at their defaults). As detectors expose candidates, extend
//! [`refit_skill_config`].

use std::collections::BTreeMap;

use crate::config::SkillConfig;
use crate::skill::Skill;

/// Linear-interpolated percentile `p∈[0,1]` of an ascending-sorted slice.
pub fn percentile(sorted: &[f32], p: f32) -> f32 {
    match sorted {
        [] => 0.0,
        [only] => *only,
        _ => {
            let rank = p.clamp(0.0, 1.0) * (sorted.len() - 1) as f32;
            let lo = rank.floor() as usize;
            let hi = rank.ceil() as usize;
            sorted[lo] + (sorted[hi] - sorted[lo]) * (rank - lo as f32)
        }
    }
}

/// Percentile of an *unsorted* sample (finite values only); `0.0` if empty.
pub fn pctl(values: &[f32], p: f32) -> f32 {
    let mut s: Vec<f32> = values.iter().copied().filter(|v| v.is_finite()).collect();
    s.sort_by(f32::total_cmp);
    percentile(&s, p)
}

/// Average-rank transform (ties share the mean of their rank span), 1-based.
fn rankdata(v: &[f32]) -> Vec<f32> {
    let n = v.len();
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&i, &j| v[i].total_cmp(&v[j]));
    let mut ranks = vec![0.0f32; n];
    let mut i = 0;
    while i < n {
        let mut j = i;
        while j + 1 < n && v[idx[j + 1]] == v[idx[i]] {
            j += 1;
        }
        let avg = (i + j) as f32 / 2.0 + 1.0;
        for &k in &idx[i..=j] {
            ranks[k] = avg;
        }
        i = j + 1;
    }
    ranks
}

/// Pearson correlation, or `None` if either input has zero variance.
fn pearson(a: &[f32], b: &[f32]) -> Option<f32> {
    let n = a.len() as f32;
    let (ma, mb) = (a.iter().sum::<f32>() / n, b.iter().sum::<f32>() / n);
    let (mut cov, mut va, mut vb) = (0.0f32, 0.0f32, 0.0f32);
    for (x, y) in a.iter().zip(b) {
        let (dx, dy) = (x - ma, y - mb);
        cov += dx * dy;
        va += dx * dx;
        vb += dy * dy;
    }
    (va > 0.0 && vb > 0.0).then(|| cov / (va.sqrt() * vb.sqrt()))
}

/// Spearman rank correlation of paired samples; `None` if too few or degenerate.
pub fn spearman(xs: &[f32], ys: &[f32]) -> Option<f32> {
    if xs.len() != ys.len() || xs.len() < 2 {
        return None;
    }
    pearson(&rankdata(xs), &rankdata(ys))
}

/// Fit a `[lo, hi]` band to the `lo_q`/`hi_q` percentiles of an observed sample.
pub fn fit_band(observed: &[f32], lo_q: f32, hi_q: f32) -> (f32, f32) {
    (pctl(observed, lo_q), pctl(observed, hi_q))
}

/// Minimum sample size before we trust a fitted anchor over the default.
const MIN_FIT_SAMPLES: usize = 8;

/// Percentile of the aerial *candidate* (every-touch car height) distribution used
/// for the full-confidence ramp top. High enough to sit among genuine aerials.
const AERIAL_TOP_Q: f32 = 0.995;

/// Plausible band (uu) for the aerial detection floor. The floor is fit to the
/// emptiest height inside this window — the valley between the dense ground-touch
/// cluster (~17uu) and the aerial tail. The band is the **policy** knob: a percentile
/// of *all* touches would track how often players aerial (the very thing we study),
/// so we look for the structural gap instead, bounded to physically sane floors.
const AERIAL_FLOOR_BAND: (f32, f32) = (100.0, 600.0);

/// Fit the aerial detection floor from candidate-mode touch heights: the midpoint
/// of the largest gap between consecutive heights whose midpoint lands inside
/// [`AERIAL_FLOOR_BAND`] — i.e. the valley separating ground touches from aerials.
///
/// Robust to the aerial *fraction* (a fixed percentile is not — see the band note).
/// `None` when there's no clear gap in the band, so the caller keeps the default.
pub fn fit_aerial_floor(candidates: &[f32]) -> Option<f32> {
    let (lo, hi) = AERIAL_FLOOR_BAND;
    let mut s: Vec<f32> = candidates
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .collect();
    s.sort_by(f32::total_cmp);
    let mut best: Option<(f32, f32)> = None; // (gap width, midpoint)
    for w in s.windows(2) {
        let mid = 0.5 * (w[0] + w[1]);
        let gap = w[1] - w[0];
        if (lo..=hi).contains(&mid) && best.is_none_or(|(g, _)| gap > g) {
            best = Some((gap, mid));
        }
    }
    best.map(|(_, mid)| mid)
}

/// Refit the calibratable confidence-ramp anchors from per-skill metric
/// distributions and stamp a `-fitted` version.
///
/// For aerials, candidate-mode metrics (car height at *every* touch, from
/// `replay_skills::candidate_metrics`) let us fit both the floor and the top:
/// `aerial_min_height` ← the ground/aerial valley ([`fit_aerial_floor`]), and
/// `high_aerial_height` ← the candidate [`AERIAL_TOP_Q`] percentile (kept clear of
/// the floor). Without candidates we fall back to fitting only the top from the
/// gated p90 of observed aerial peak heights, leaving the floor at its default.
/// Skills with fewer than [`MIN_FIT_SAMPLES`] candidates (or observations), or with
/// no clear valley, keep the default — a handful of replays shouldn't move a gate.
pub fn refit_skill_config(
    base: &SkillConfig,
    metrics_by_skill: &BTreeMap<Skill, Vec<f32>>,
    candidates_by_skill: &BTreeMap<Skill, Vec<f32>>,
) -> SkillConfig {
    let mut cfg = base.clone();

    match candidates_by_skill.get(&Skill::Aerial) {
        // Candidate-mode: fit the floor at the valley and the top from the tail.
        Some(c) if c.len() >= MIN_FIT_SAMPLES => {
            if let Some(floor) = fit_aerial_floor(c) {
                cfg.aerial_min_height = floor;
            }
            cfg.high_aerial_height = pctl(c, AERIAL_TOP_Q).max(cfg.aerial_min_height + 100.0);
        }
        // Fallback (no candidates): top only, from gated peak heights; floor kept.
        _ => {
            if let Some(v) = metrics_by_skill.get(&Skill::Aerial) {
                if v.len() >= MIN_FIT_SAMPLES {
                    cfg.high_aerial_height = pctl(v, 0.90).max(cfg.aerial_min_height + 100.0);
                }
            }
        }
    }

    cfg.version = format!("{}-fitted", base.version);
    cfg
}
