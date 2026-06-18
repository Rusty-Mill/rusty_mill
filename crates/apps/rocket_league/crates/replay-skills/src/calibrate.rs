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
//! place the floor as well as the top. We fit floors at the valley between weak
//! attempts and the real mechanic (`aerial_min_height`, `flick_min_up_dv`,
//! `power_shot_min_speed`, `redirect_min_angle_deg`) and, for aerials, the ramp top
//! too. The fit is self-guarding — no clear valley ⇒ the hand-set default holds. As
//! detectors expose candidates, extend [`refit_skill_config`].

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

/// Plausible band for each candidate-mode detection floor: the floor is fit to the
/// emptiest value inside this window — the valley between the "weak attempt" cluster
/// and the real-mechanic tail. The band is the **policy** knob: a percentile of all
/// candidates would track how *often* players do the mechanic (the very thing we
/// study), so we look for the structural gap instead, bounded to sane floors.
/// Brackets each detector's hand-set default (aerial 300uu, flick 550uu/s, power
/// 2000uu/s, redirect 55°).
const AERIAL_FLOOR_BAND: (f32, f32) = (100.0, 600.0);
const FLICK_FLOOR_BAND: (f32, f32) = (300.0, 800.0);
const POWER_SHOT_FLOOR_BAND: (f32, f32) = (1500.0, 2800.0);
const REDIRECT_FLOOR_BAND: (f32, f32) = (35.0, 80.0);

/// A valley must span at least this fraction of the band width to count — so a
/// continuous, unimodal distribution (many similar small gaps) is rejected and only
/// a genuine empty separation is accepted. The band is already a tuned policy
/// window, so a fraction of it is a sane "this gap is real" bar.
const MIN_VALLEY_FRAC: f32 = 0.15;

/// Fit a detection floor from candidate-mode metrics: the midpoint of the largest
/// gap between consecutive values whose midpoint lands inside `band` — i.e. the
/// valley separating weak attempts from the real mechanic.
///
/// Robust to the mechanic's *frequency* (a fixed percentile is not — see the band
/// note). Self-guarding: returns `None` unless the largest in-band gap spans at
/// least [`MIN_VALLEY_FRAC`] of the band (no real valley ⇒ a continuous/unimodal
/// distribution), so the caller keeps the hand-set default.
pub fn fit_floor_valley(candidates: &[f32], band: (f32, f32)) -> Option<f32> {
    let (lo, hi) = band;
    let min_gap = MIN_VALLEY_FRAC * (hi - lo);
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
    best.filter(|&(gap, _)| gap >= min_gap).map(|(_, mid)| mid)
}

/// Apply a self-guarding valley floor fit to one config field from a skill's
/// candidate distribution, when it has at least [`MIN_FIT_SAMPLES`] candidates.
fn fit_floor_into(
    field: &mut f32,
    candidates_by_skill: &BTreeMap<Skill, Vec<f32>>,
    skill: Skill,
    band: (f32, f32),
) {
    if let Some(c) = candidates_by_skill.get(&skill) {
        if c.len() >= MIN_FIT_SAMPLES {
            if let Some(floor) = fit_floor_valley(c, band) {
                *field = floor;
            }
        }
    }
}

/// Refit the calibratable confidence-ramp anchors from per-skill metric
/// distributions and stamp a `-fitted` version.
///
/// Candidate-mode metrics (`replay_skills::candidate_metrics`) let us fit detection
/// **floors** at the valley between weak attempts and the real mechanic
/// ([`fit_floor_valley`], self-guarding): `aerial_min_height`, `flick_min_up_dv`,
/// `power_shot_min_speed`, `redirect_min_angle_deg`. For aerials — the only skill
/// with a configurable ramp *top* — we also fit `high_aerial_height` to the
/// candidate [`AERIAL_TOP_Q`] percentile (the flick/power/redirect ramps use fixed
/// offsets, so they're floor-only); without aerial candidates we fall back to
/// fitting that top from the gated p90 of observed peak heights. Skills with fewer
/// than [`MIN_FIT_SAMPLES`] candidates, or no clear valley, keep their default — a
/// handful of replays shouldn't move a gate.
pub fn refit_skill_config(
    base: &SkillConfig,
    metrics_by_skill: &BTreeMap<Skill, Vec<f32>>,
    candidates_by_skill: &BTreeMap<Skill, Vec<f32>>,
) -> SkillConfig {
    let mut cfg = base.clone();

    // Aerial floor + top (top is candidate-driven, with a gated-p90 fallback).
    match candidates_by_skill.get(&Skill::Aerial) {
        Some(c) if c.len() >= MIN_FIT_SAMPLES => {
            if let Some(floor) = fit_floor_valley(c, AERIAL_FLOOR_BAND) {
                cfg.aerial_min_height = floor;
            }
            cfg.high_aerial_height = pctl(c, AERIAL_TOP_Q).max(cfg.aerial_min_height + 100.0);
        }
        _ => {
            if let Some(v) = metrics_by_skill.get(&Skill::Aerial) {
                if v.len() >= MIN_FIT_SAMPLES {
                    cfg.high_aerial_height = pctl(v, 0.90).max(cfg.aerial_min_height + 100.0);
                }
            }
        }
    }

    // Floor-only valley fits for the continuous-metric mechanics.
    fit_floor_into(
        &mut cfg.flick_min_up_dv,
        candidates_by_skill,
        Skill::Flick,
        FLICK_FLOOR_BAND,
    );
    fit_floor_into(
        &mut cfg.power_shot_min_speed,
        candidates_by_skill,
        Skill::PowerShot,
        POWER_SHOT_FLOOR_BAND,
    );
    fit_floor_into(
        &mut cfg.redirect_min_angle_deg,
        candidates_by_skill,
        Skill::Redirect,
        REDIRECT_FLOOR_BAND,
    );

    cfg.version = format!("{}-fitted", base.version);
    cfg
}
