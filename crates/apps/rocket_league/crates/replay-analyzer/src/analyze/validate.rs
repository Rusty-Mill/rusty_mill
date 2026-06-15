//! External validation of the reconstruction against an independent parser.
//!
//! The analyzer's per-player aggregates ([`crate::model::PlayerFeatures`]) are
//! inferred from a from-scratch network-stream reconstruction; a number we
//! cannot validate is a bug we have not found yet (design spec §0). This module
//! is the cross-check: it pairs our aggregates against ground truth distilled
//! from **ballchasing.com** (which parses the same replays with an independent
//! pipeline) and measures agreement.
//!
//! Three reconstruction channels are checked, one per independently-derived
//! quantity, so drift localizes to a channel:
//! - **velocity** → `time_supersonic_s`   vs ballchasing `time_supersonic_speed`
//! - **position** → `mean_dist_to_ball`    vs ballchasing `avg_distance_to_ball`
//! - **boost**    → `boost_used`           vs ballchasing `bcpm × duration_min`
//!
//! Touches and possession time are deliberately absent: the ballchasing API
//! does not expose them, so they cannot be validated against this source.
//!
//! Everything here is pure (no I/O, no network). The companion binary
//! (`src/bin/validate.rs`) loads the committed fixture and corpus replays and
//! drives these functions; the `external_validation` test gates on them.

use std::collections::HashMap;

use serde::Deserialize;

use crate::model::PlayerFeatures;

/// One player's ground-truth stats for a single replay (a row of the committed
/// `assets/corpus/ballchasing_stats.json` fixture).
#[derive(Debug, Clone, Deserialize)]
pub struct GtPlayer {
    pub name: String,
    pub team: i32,
    /// Seconds at supersonic speed (ballchasing `movement.time_supersonic_speed`).
    pub time_supersonic_s: Option<f32>,
    /// Mean car-to-ball distance, uu (ballchasing `positioning.avg_distance_to_ball`).
    pub avg_dist_to_ball: Option<f32>,
    /// Boost consumed per minute (ballchasing `boost.bcpm`).
    pub bcpm: Option<f32>,
    /// Boost used per minute (ballchasing `boost.bpm`).
    pub bpm: Option<f32>,
}

/// Ground truth for one replay.
#[derive(Debug, Clone, Deserialize)]
pub struct GtReplay {
    pub duration_s: Option<f32>,
    pub players: Vec<GtPlayer>,
}

/// The committed ground-truth fixture: replay id → ground truth.
#[derive(Debug, Clone, Deserialize)]
pub struct GroundTruth {
    pub source: String,
    pub replays: HashMap<String, GtReplay>,
}

/// One matched (analyzer, ground-truth) observation for a single metric.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pair {
    pub ours: f32,
    pub theirs: f32,
}

/// Accumulated matched pairs across the corpus, one channel per field. Boost is
/// carried twice (against ballchasing's `bcpm` and `bpm` derivations) so the
/// validator can pick the better-correlating mapping empirically.
#[derive(Debug, Default, Clone)]
pub struct Samples {
    pub supersonic: Vec<Pair>,
    pub dist_to_ball: Vec<Pair>,
    pub boost_bcpm: Vec<Pair>,
    pub boost_bpm: Vec<Pair>,
}

/// Result of pairing one replay: players matched by name and total ground-truth
/// players seen (for coverage).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchCount {
    pub matched: usize,
    pub total: usize,
}

/// Match this replay's analyzer features to its ground-truth players, then append
/// the matched per-channel observations into `acc`. `duration_s` converts
/// ballchasing's per-minute boost rates into a comparable match total.
///
/// Matching is **team-anchored**, not string-equality: ballchasing mangles
/// player names inconsistently (ASCII-folds `Thómer-Kun`→`Thomer-Kun`, strips
/// `357ß`→`357`, and occasionally shows a different platform string entirely),
/// while the analyzer preserves the true in-replay name. So we match exactly by
/// name first, then pair the unique remaining player per team — unambiguous in
/// 2v2 and immune to ballchasing's name formatting.
pub fn pair_replay(
    features: &[PlayerFeatures],
    gt: &GtReplay,
    duration_s: f32,
    acc: &mut Samples,
) -> MatchCount {
    let minutes = duration_s / 60.0;
    let mut feat_used = vec![false; features.len()];
    let mut gt_used = vec![false; gt.players.len()];
    let mut pairs: Vec<(usize, usize)> = Vec::new();

    // Pass 1: exact name match.
    for (gi, g) in gt.players.iter().enumerate() {
        if let Some((fi, _)) = features
            .iter()
            .enumerate()
            .find(|(fi, f)| !feat_used[*fi] && f.player == g.name)
        {
            feat_used[fi] = true;
            gt_used[gi] = true;
            pairs.push((fi, gi));
        }
    }

    // Pass 2: per team, pair the *unique* remaining named track with the unique
    // remaining ground-truth player (forced 1-1 → unambiguous). Skip `<unknown>`
    // / team-less spurious tracks; skip teams with >1 residual (can't disambiguate
    // two mangled names without leaning on the very stats we are validating).
    for team in 0..2 {
        let ours: Vec<usize> = (0..features.len())
            .filter(|&i| !feat_used[i])
            .filter(|&i| features[i].team == Some(team) && features[i].player != "<unknown>")
            .collect();
        let theirs: Vec<usize> = (0..gt.players.len())
            .filter(|&i| !gt_used[i] && gt.players[i].team == team)
            .collect();
        if ours.len() == 1 && theirs.len() == 1 {
            feat_used[ours[0]] = true;
            gt_used[theirs[0]] = true;
            pairs.push((ours[0], theirs[0]));
        }
    }

    for (fi, gi) in &pairs {
        let (ours, g) = (&features[*fi], &gt.players[*gi]);
        if let Some(t) = g.time_supersonic_s {
            acc.supersonic.push(Pair {
                ours: ours.time_supersonic_s,
                theirs: t,
            });
        }
        if let Some(t) = g.avg_dist_to_ball {
            acc.dist_to_ball.push(Pair {
                ours: ours.mean_dist_to_ball,
                theirs: t,
            });
        }
        if let Some(bcpm) = g.bcpm {
            acc.boost_bcpm.push(Pair {
                ours: ours.boost_used,
                theirs: bcpm * minutes,
            });
        }
        if let Some(bpm) = g.bpm {
            acc.boost_bpm.push(Pair {
                ours: ours.boost_used,
                theirs: bpm * minutes,
            });
        }
    }
    MatchCount {
        matched: pairs.len(),
        total: gt.players.len(),
    }
}

/// Agreement statistics for one channel's matched pairs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Agreement {
    pub n: usize,
    /// Spearman rank correlation (scale/offset-robust "same quantity?" check).
    pub spearman: Option<f32>,
    /// Pearson correlation.
    pub pearson: Option<f32>,
    /// Median relative error |ours − theirs| / max(|theirs|, `floor`).
    pub median_rel_err: f32,
}

/// Compute agreement for a channel. `floor` keeps near-zero ground-truth values
/// from blowing up the relative-error stat (it stays a calibration sanity bound,
/// not a divide-by-noise).
pub fn agreement(pairs: &[Pair], floor: f32) -> Agreement {
    let n = pairs.len();
    let ours: Vec<f32> = pairs.iter().map(|p| p.ours).collect();
    let theirs: Vec<f32> = pairs.iter().map(|p| p.theirs).collect();

    let mut rel: Vec<f32> = pairs
        .iter()
        .map(|p| (p.ours - p.theirs).abs() / p.theirs.abs().max(floor))
        .collect();
    rel.sort_by(f32::total_cmp);
    let median_rel_err = match rel.len() {
        0 => 0.0,
        len if len % 2 == 1 => rel[len / 2],
        len => 0.5 * (rel[len / 2 - 1] + rel[len / 2]),
    };

    Agreement {
        n,
        spearman: spearman(&ours, &theirs),
        pearson: pearson(&ours, &theirs),
        median_rel_err,
    }
}

/// Average-rank transform (ties share the mean of their rank span), 1-based.
fn rankdata(v: &[f32]) -> Vec<f32> {
    let mut idx: Vec<usize> = (0..v.len()).collect();
    idx.sort_by(|&a, &b| v[a].total_cmp(&v[b]));
    let mut ranks = vec![0.0f32; v.len()];
    let mut i = 0;
    while i < idx.len() {
        let mut j = i + 1;
        while j < idx.len() && v[idx[j]] == v[idx[i]] {
            j += 1;
        }
        // Mean of the 1-based ranks i+1..=j shared by this tie group.
        let avg = ((i + 1 + j) as f32) / 2.0;
        for &k in &idx[i..j] {
            ranks[k] = avg;
        }
        i = j;
    }
    ranks
}

/// Pearson correlation, or `None` if either input has zero variance / too few.
fn pearson(a: &[f32], b: &[f32]) -> Option<f32> {
    let n = a.len();
    if n < 3 || n != b.len() {
        return None;
    }
    let nf = n as f32;
    let (ma, mb) = (a.iter().sum::<f32>() / nf, b.iter().sum::<f32>() / nf);
    let (mut sab, mut saa, mut sbb) = (0.0f32, 0.0f32, 0.0f32);
    for (x, y) in a.iter().zip(b) {
        let (dx, dy) = (x - ma, y - mb);
        sab += dx * dy;
        saa += dx * dx;
        sbb += dy * dy;
    }
    if saa <= 0.0 || sbb <= 0.0 {
        return None;
    }
    Some(sab / (saa.sqrt() * sbb.sqrt()))
}

/// Spearman rank correlation, `None` if too few or degenerate.
pub fn spearman(xs: &[f32], ys: &[f32]) -> Option<f32> {
    if xs.len() < 3 || xs.len() != ys.len() {
        return None;
    }
    pearson(&rankdata(xs), &rankdata(ys))
}

// --- locked tolerances (set from observed corpus agreement, with margin) ---

/// Relative-error floors per channel (passed to [`agreement`]).
pub const FLOOR_SUPERSONIC_S: f32 = 2.0;
/// Distance relative-error floor (uu).
pub const FLOOR_DIST_UU: f32 = 100.0;
/// Boost relative-error floor (boost units).
pub const FLOOR_BOOST: f32 = 50.0;

/// Minimum fraction of ground-truth players matched to a reconstructed track.
pub const MIN_COVERAGE: f32 = 0.97;
/// Minimum Spearman of `time_supersonic_s` vs ballchasing (velocity channel).
pub const MIN_SPEARMAN_SUPERSONIC: f32 = 0.92;
/// Minimum Spearman of `mean_dist_to_ball` vs ballchasing (position channel).
pub const MIN_SPEARMAN_DIST: f32 = 0.92;
/// Minimum Spearman of `boost_used` vs ballchasing (boost channel).
pub const MIN_SPEARMAN_BOOST: f32 = 0.85;
/// Max median relative error per channel.
pub const MAX_MED_REL_SUPERSONIC: f32 = 0.20;
pub const MAX_MED_REL_DIST: f32 = 0.10;
pub const MAX_MED_REL_BOOST: f32 = 0.20;

/// Whole-corpus agreement report: name-match coverage plus per-channel stats.
#[derive(Debug, Clone)]
pub struct Report {
    pub matched: usize,
    pub total: usize,
    pub coverage: f32,
    pub supersonic: Agreement,
    pub dist_to_ball: Agreement,
    pub boost_bcpm: Agreement,
    pub boost_bpm: Agreement,
}

impl Report {
    /// The boost mapping with the stronger Spearman — the one the gate scores
    /// against, since ballchasing's `bcpm`/`bpm` semantics are picked empirically.
    pub fn boost(&self) -> (&'static str, &Agreement) {
        let sp = |a: &Agreement| a.spearman.unwrap_or(f32::NEG_INFINITY);
        if sp(&self.boost_bcpm) >= sp(&self.boost_bpm) {
            ("bcpm", &self.boost_bcpm)
        } else {
            ("bpm", &self.boost_bpm)
        }
    }

    /// Tolerance violations; empty ⇒ the reconstruction is validated.
    pub fn failures(&self) -> Vec<String> {
        let sp = |a: &Agreement| a.spearman.unwrap_or(f32::NEG_INFINITY);
        let mut f = Vec::new();
        if self.coverage < MIN_COVERAGE {
            f.push(format!("coverage {:.3} < {MIN_COVERAGE}", self.coverage));
        }
        let mut chan = |name: &str, a: &Agreement, min_rho: f32, max_rel: f32| {
            if sp(a) < min_rho {
                f.push(format!("{name} spearman {:.3} < {min_rho}", sp(a)));
            }
            if a.median_rel_err > max_rel {
                f.push(format!(
                    "{name} med_rel_err {:.3} > {max_rel}",
                    a.median_rel_err
                ));
            }
        };
        chan(
            "supersonic",
            &self.supersonic,
            MIN_SPEARMAN_SUPERSONIC,
            MAX_MED_REL_SUPERSONIC,
        );
        chan(
            "dist_to_ball",
            &self.dist_to_ball,
            MIN_SPEARMAN_DIST,
            MAX_MED_REL_DIST,
        );
        let (bn, ba) = self.boost();
        chan(
            &format!("boost({bn})"),
            ba,
            MIN_SPEARMAN_BOOST,
            MAX_MED_REL_BOOST,
        );
        f
    }
}

/// Reduce accumulated [`Samples`] (and name-match counts) to a [`Report`].
pub fn evaluate(acc: &Samples, matched: usize, total: usize) -> Report {
    Report {
        matched,
        total,
        coverage: if total > 0 {
            matched as f32 / total as f32
        } else {
            0.0
        },
        supersonic: agreement(&acc.supersonic, FLOOR_SUPERSONIC_S),
        dist_to_ball: agreement(&acc.dist_to_ball, FLOOR_DIST_UU),
        boost_bcpm: agreement(&acc.boost_bcpm, FLOOR_BOOST),
        boost_bpm: agreement(&acc.boost_bpm, FLOOR_BOOST),
    }
}
