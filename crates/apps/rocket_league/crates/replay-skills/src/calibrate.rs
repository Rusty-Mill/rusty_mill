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
//! **Fitting is deliberately conservative.** A detector's metric is only observed
//! for events that already *passed* its gate, so a hard minimum gate can't be
//! lowered from observed data (selection bias). We therefore refit only the
//! explicit confidence-ramp *upper* anchors (today: [`SkillConfig::high_aerial_height`])
//! to a population percentile, leaving hard floors at their defaults. As detectors
//! gain more upper anchors, extend [`refit_skill_config`].

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

/// Refit the calibratable confidence-ramp anchors from per-skill observed metric
/// distributions, leaving hard floors untouched, and stamp a `-fitted` version.
///
/// Today this fits [`SkillConfig::high_aerial_height`] — the explicit "full
/// confidence" anchor for aerials — to the corpus p90 of observed aerial peak
/// heights (kept clear of the floor). Extend the match as detectors gain more
/// upper anchors. Skills with fewer than [`MIN_FIT_SAMPLES`] observations keep
/// the default (a handful of replays shouldn't move a threshold).
pub fn refit_skill_config(
    base: &SkillConfig,
    metrics_by_skill: &BTreeMap<Skill, Vec<f32>>,
) -> SkillConfig {
    let mut cfg = base.clone();

    if let Some(v) = metrics_by_skill.get(&Skill::Aerial) {
        if v.len() >= MIN_FIT_SAMPLES {
            // p90 of peak heights, but never at/below the detection floor.
            cfg.high_aerial_height = pctl(v, 0.90).max(cfg.aerial_min_height + 100.0);
        }
    }

    cfg.version = format!("{}-fitted", base.version);
    cfg
}
