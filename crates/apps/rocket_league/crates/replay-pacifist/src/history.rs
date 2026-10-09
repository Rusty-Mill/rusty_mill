//! Multi-match aggregation: combine a player's [`PacifistScore`]s across many
//! matches into one stable per-player history.
//!
//! [`crate::scoring::aggregate`] already confidence-weights *dimensions*
//! within one match into a headline value; this module runs the identical
//! confidence-weighted-mean architecture one level up, across *matches* —
//! weighting each match's contribution to a dimension by how many
//! opportunities it actually had ([`crate::metrics::DimensionScore::opportunities`]),
//! then feeding the resulting per-dimension rollup back through
//! [`crate::scoring::aggregate`] for the headline, so the two levels share
//! one formula rather than two ad hoc ones.
//!
//! The motivation is the corpus validation's own finding
//! (`docs/pacifist-score-validation.md`, the v1 section): a single match
//! yields only 5–15 episodes per dimension, so within-bracket resolution is
//! near a hard ceiling regardless of how good the extractors are — "multi-
//! match aggregation… is the realistic path to stable per-player placement."
//! This module is that aggregation; a service layer with per-account replay
//! history would be the caller. **Player identity is deliberately out of
//! scope here** — grouping which matches belong to which real person (by
//! platform ID, display name, or otherwise) is a caller concern; this module
//! only combines a slice of already-identified same-player records.
//!
//! One deliberate asymmetry with single-match scoring: [`PlayerHistory::value`]
//! is never capped by Majors the way [`PacifistScore::value`] is within one
//! match — see that field's docs for why.

use std::collections::HashMap;

use crate::metrics::{Confidence, DimensionId, DimensionScore, Score};
use crate::scoring::{aggregate, PacifistScore, ScoringConfig};
use crate::severity::{Fault, Severity, Verdict};

/// One match's already-computed [`PacifistScore`], labeled for the history —
/// a replay id, filename, or match date, whatever the caller uses to
/// identify a match. Only [`FaultHistory::recent_majors`] surfaces the label.
///
/// `records` passed to [`aggregate_history`] are assumed **chronological**
/// (oldest first): [`PlayerHistory::trend`] and "recent" Major faults both
/// read that ordering directly rather than re-deriving it, since this crate
/// has no notion of match dates.
pub struct MatchRecord {
    pub label: String,
    pub score: PacifistScore,
}

/// Knobs for combining matches. Constructed at the edge and threaded in, like
/// every other config in this crate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HistoryConfig {
    /// Total opportunities (summed across every match) at which a
    /// dimension's history confidence reaches 1.0. An uncalibrated
    /// approximation, like the per-match `saturation_episodes` it extends —
    /// set higher because a stable player-level read should demand more
    /// repetition than one match's headline does.
    pub saturation_opportunities: f32,
    /// Cap on the number of recent Major faults retained for coaching
    /// review.
    pub max_recent_majors: usize,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            // ~5 matches' worth of the per-match default (12 opportunities).
            saturation_opportunities: 60.0,
            max_recent_majors: 25,
        }
    }
}

/// One dimension's opportunity-weighted read across every match handed in.
/// Mirrors [`crate::scoring::DimensionBreakdown`]'s fields, plus
/// `matches_with_data` — a per-match breakdown always reports all eight
/// dimensions for transparency, so a dimension can appear here with
/// `opportunities == 0` (never applied in any match) exactly as it would in
/// one match's `breakdown`.
#[derive(Debug, Clone, PartialEq)]
pub struct DimensionHistory {
    pub dimension: DimensionId,
    pub value: Option<Score>,
    pub confidence: Confidence,
    pub weight: f32,
    pub influence: f32,
    pub opportunities: usize,
    pub matches_with_data: usize,
}

/// The FM-1 fault ledger across every match — totals and a pass **rate**,
/// not a single pass/fail: the guide's driving-test framing (15 minors, one
/// Major) is stated per match, and extending it to a lifetime allowance
/// isn't something the source material specifies, so this reports the
/// honest underlying numbers instead of inventing one.
#[derive(Debug, Clone, PartialEq)]
pub struct FaultHistory {
    pub minors_total: u32,
    pub majors_total: u32,
    pub matches: usize,
    pub matches_passed: usize,
    /// `matches_passed / matches`; `0.0` when there are no matches.
    pub pass_rate: f32,
    /// The most recent Major faults, each labeled with the match they came
    /// from, capped at [`HistoryConfig::max_recent_majors`]. Every Major is
    /// worth surfacing individually — one is an instant verdict failure.
    pub recent_majors: Vec<(String, Fault)>,
}

/// A player's aggregated Pacifist profile across every match handed in.
///
/// `value` is the opportunity-weighted dimension average — **not** capped by
/// Majors the way one match's [`PacifistScore::value`] is. Capping a single
/// match on an instant-failure fault is the driving-test framing working as
/// intended; applying that same cap to an N-match aggregate would make the
/// "stable across matches" headline just as volatile as its worst single
/// match, which defeats the reason to aggregate at all. Severity instead
/// surfaces through `faults` (totals and a pass **rate**) and `trend` (which
/// does carry each match's own capped value, so a bad match is still visible
/// there) — read the headline as "average technique," and `faults` as "how
/// often does play collapse."
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerHistory {
    pub matches: usize,
    pub value: Option<Score>,
    pub confidence: Confidence,
    /// Sorted by influence, highest first — same convention as
    /// [`crate::scoring::PacifistScore::breakdown`].
    pub dimensions: Vec<DimensionHistory>,
    pub faults: FaultHistory,
    /// Each match's own **capped** headline value in input order, for a
    /// trend view. `None` entries are matches with no scoreable evidence.
    pub trend: Vec<Option<Score>>,
}

/// Opportunity-weighted accumulator for one dimension across matches.
#[derive(Default)]
struct Acc {
    sum_opportunity_value: f32,
    sum_opportunities: usize,
    matches_with_data: usize,
}

/// Combine `records` into a [`PlayerHistory`]. `scoring` supplies the
/// dimension weights for the recombined headline — pass the same
/// [`ScoringConfig`] the records were originally scored with. `config` sets
/// the multi-match thresholds.
pub fn aggregate_history(
    records: &[MatchRecord],
    scoring: &ScoringConfig,
    config: &HistoryConfig,
) -> PlayerHistory {
    if records.is_empty() {
        return PlayerHistory {
            matches: 0,
            value: None,
            confidence: Confidence::new(0.0),
            dimensions: Vec::new(),
            faults: fault_history(records, config),
            trend: Vec::new(),
        };
    }

    // Every dimension that appeared in any match's breakdown gets an entry —
    // matching the per-match convention that all eight dimensions are always
    // reported, at confidence 0 if a dimension never had an opportunity.
    let mut per_dim: HashMap<DimensionId, Acc> = HashMap::new();
    for r in records {
        for d in &r.score.breakdown {
            let acc = per_dim.entry(d.dimension).or_default();
            if d.opportunities > 0 {
                acc.sum_opportunity_value += d.opportunities as f32 * d.value.get();
                acc.sum_opportunities += d.opportunities;
                acc.matches_with_data += 1;
            }
        }
    }

    // Recombine through the identical within-match formula, one level up:
    // each dimension's history becomes a single synthetic DimensionScore.
    let synthetic: Vec<DimensionScore> = per_dim
        .iter()
        .map(|(&dimension, acc)| {
            if acc.sum_opportunities > 0 {
                DimensionScore {
                    dimension,
                    value: Score::new(acc.sum_opportunity_value / acc.sum_opportunities as f32),
                    confidence: Confidence::new(
                        acc.sum_opportunities as f32 / config.saturation_opportunities,
                    ),
                    evidence: Vec::new(),
                    opportunities: acc.sum_opportunities,
                }
            } else {
                DimensionScore {
                    dimension,
                    value: Score::new(100.0),
                    confidence: Confidence::new(0.0),
                    evidence: Vec::new(),
                    opportunities: 0,
                }
            }
        })
        .collect();
    let headline = aggregate(synthetic, scoring);

    let dimensions: Vec<DimensionHistory> = headline
        .breakdown
        .iter()
        .map(|b| {
            let acc_matches = per_dim.get(&b.dimension).map_or(0, |a| a.matches_with_data);
            DimensionHistory {
                dimension: b.dimension,
                value: (b.confidence.get() > 0.0).then_some(b.value),
                confidence: b.confidence,
                weight: b.weight,
                influence: b.influence,
                opportunities: b.opportunities,
                matches_with_data: acc_matches,
            }
        })
        .collect();

    PlayerHistory {
        matches: records.len(),
        value: headline.value,
        confidence: headline.confidence,
        dimensions,
        faults: fault_history(records, config),
        trend: records.iter().map(|r| r.score.value).collect(),
    }
}

fn fault_history(records: &[MatchRecord], config: &HistoryConfig) -> FaultHistory {
    let mut minors_total = 0u32;
    let mut majors_total = 0u32;
    let mut matches_passed = 0usize;
    let mut recent_majors: Vec<(String, Fault)> = Vec::new();

    for r in records {
        minors_total += r.score.faults.minors;
        majors_total += r.score.faults.majors;
        if r.score.faults.verdict == Verdict::Pass {
            matches_passed += 1;
        }
        for f in &r.score.faults.faults {
            if f.severity == Severity::Major {
                recent_majors.push((r.label.clone(), f.clone()));
            }
        }
    }

    // `records` is chronological, so the tail is the most recent.
    if recent_majors.len() > config.max_recent_majors {
        recent_majors = recent_majors.split_off(recent_majors.len() - config.max_recent_majors);
    }

    let matches = records.len();
    FaultHistory {
        minors_total,
        majors_total,
        matches,
        matches_passed,
        pass_rate: if matches > 0 {
            matches_passed as f32 / matches as f32
        } else {
            0.0
        },
        recent_majors,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scoring::DimensionBreakdown;
    use crate::severity::FaultSummary;

    fn breakdown(dimension: DimensionId, value: f32, opportunities: usize) -> DimensionBreakdown {
        DimensionBreakdown {
            dimension,
            value: Score::new(value),
            confidence: Confidence::new(if opportunities > 0 { 1.0 } else { 0.0 }),
            weight: 1.0,
            influence: 0.0,
            evidence: Vec::new(),
            opportunities,
        }
    }

    fn record(
        label: &str,
        value: Option<f32>,
        breakdown: Vec<DimensionBreakdown>,
        faults: FaultSummary,
    ) -> MatchRecord {
        MatchRecord {
            label: label.to_string(),
            score: PacifistScore {
                value: value.map(Score::new),
                confidence: Confidence::new(0.0),
                breakdown,
                faults,
            },
        }
    }

    fn major(t: f32) -> Fault {
        Fault {
            t,
            severity: Severity::Major,
            criterion: "FM-2",
            detail: String::new(),
        }
    }

    #[test]
    fn opportunity_weighted_mean_not_naive_average() {
        // 2 opportunities at 100 vs 8 at 0: weighted mean is 20, not 50.
        let r1 = record(
            "m1",
            Some(100.0),
            vec![breakdown(DimensionId::BoostEconomy, 100.0, 2)],
            FaultSummary::empty(),
        );
        let r2 = record(
            "m2",
            Some(0.0),
            vec![breakdown(DimensionId::BoostEconomy, 0.0, 8)],
            FaultSummary::empty(),
        );
        let hist = aggregate_history(
            &[r1, r2],
            &ScoringConfig::default(),
            &HistoryConfig::default(),
        );
        let dim = hist
            .dimensions
            .iter()
            .find(|d| d.dimension == DimensionId::BoostEconomy)
            .expect("reported");
        let v = dim.value.expect("has data").get();
        assert!((v - 20.0).abs() < 1e-3, "got {v}");
        assert_eq!(dim.opportunities, 10);
        assert_eq!(dim.matches_with_data, 2);
    }

    #[test]
    fn dimension_absent_everywhere_reports_zero_confidence() {
        let r1 = record(
            "m1",
            Some(100.0),
            vec![breakdown(DimensionId::OverExtension, 100.0, 0)],
            FaultSummary::empty(),
        );
        let hist = aggregate_history(&[r1], &ScoringConfig::default(), &HistoryConfig::default());
        let dim = hist
            .dimensions
            .iter()
            .find(|d| d.dimension == DimensionId::OverExtension)
            .expect("still reported for transparency");
        assert_eq!(dim.value, None);
        assert_eq!(dim.confidence, Confidence::new(0.0));
        assert_eq!(dim.matches_with_data, 0);
    }

    #[test]
    fn confidence_saturates_on_total_opportunities_across_matches() {
        let cfg = HistoryConfig {
            saturation_opportunities: 20.0,
            ..HistoryConfig::default()
        };
        let r1 = record(
            "m1",
            Some(50.0),
            vec![breakdown(DimensionId::BoostEconomy, 50.0, 10)],
            FaultSummary::empty(),
        );
        let r2 = record(
            "m2",
            Some(50.0),
            vec![breakdown(DimensionId::BoostEconomy, 50.0, 10)],
            FaultSummary::empty(),
        );
        let hist = aggregate_history(&[r1, r2], &ScoringConfig::default(), &cfg);
        let dim = hist
            .dimensions
            .iter()
            .find(|d| d.dimension == DimensionId::BoostEconomy)
            .unwrap();
        assert_eq!(dim.opportunities, 20);
        assert_eq!(dim.matches_with_data, 2);
        assert_eq!(dim.confidence, Confidence::new(1.0));
    }

    #[test]
    fn fault_totals_and_pass_rate_across_matches() {
        let r1 = record(
            "m1",
            Some(80.0),
            vec![],
            FaultSummary::from_faults(vec![], 15),
        );
        let r2 = record(
            "m2",
            Some(20.0),
            vec![],
            FaultSummary::from_faults(vec![major(1.0)], 15),
        );
        let r3 = record(
            "m3",
            Some(90.0),
            vec![],
            FaultSummary::from_faults(vec![], 15),
        );
        let hist = aggregate_history(
            &[r1, r2, r3],
            &ScoringConfig::default(),
            &HistoryConfig::default(),
        );
        assert_eq!(hist.faults.matches, 3);
        assert_eq!(hist.faults.matches_passed, 2);
        assert!((hist.faults.pass_rate - 2.0 / 3.0).abs() < 1e-6);
        assert_eq!(hist.faults.majors_total, 1);
        assert_eq!(hist.faults.recent_majors.len(), 1);
        assert_eq!(hist.faults.recent_majors[0].0, "m2");
    }

    #[test]
    fn recent_majors_capped_to_the_most_recent() {
        let records: Vec<MatchRecord> = (0..5)
            .map(|i| {
                record(
                    &format!("m{i}"),
                    Some(50.0),
                    vec![],
                    FaultSummary::from_faults(vec![major(i as f32)], 15),
                )
            })
            .collect();
        let cfg = HistoryConfig {
            max_recent_majors: 2,
            ..HistoryConfig::default()
        };
        let hist = aggregate_history(&records, &ScoringConfig::default(), &cfg);
        assert_eq!(hist.faults.majors_total, 5);
        assert_eq!(hist.faults.recent_majors.len(), 2);
        assert_eq!(hist.faults.recent_majors[0].0, "m3");
        assert_eq!(hist.faults.recent_majors[1].0, "m4");
    }

    #[test]
    fn trend_preserves_match_order_including_none() {
        let r1 = record("m1", Some(30.0), vec![], FaultSummary::empty());
        let r2 = record("m2", None, vec![], FaultSummary::empty());
        let r3 = record("m3", Some(70.0), vec![], FaultSummary::empty());
        let hist = aggregate_history(
            &[r1, r2, r3],
            &ScoringConfig::default(),
            &HistoryConfig::default(),
        );
        assert_eq!(
            hist.trend,
            vec![Some(Score::new(30.0)), None, Some(Score::new(70.0))]
        );
    }

    #[test]
    fn empty_records_yield_an_honest_empty_history() {
        let hist = aggregate_history(&[], &ScoringConfig::default(), &HistoryConfig::default());
        assert_eq!(hist.matches, 0);
        assert_eq!(hist.value, None);
        assert_eq!(hist.confidence, Confidence::new(0.0));
        assert!(hist.dimensions.is_empty());
        assert_eq!(hist.faults.pass_rate, 0.0);
    }

    #[test]
    fn headline_is_not_major_capped_unlike_a_single_match() {
        // This match's own headline (40.0) was capped by its Major; the
        // aggregate should report the uncapped dimension average instead —
        // capping an N-match history on one incident would make it as
        // volatile as its worst match, defeating the point of aggregating.
        let r1 = record(
            "m1",
            Some(40.0),
            vec![breakdown(DimensionId::OverExtension, 90.0, 10)],
            FaultSummary::from_faults(vec![major(1.0)], 15),
        );
        let cfg = ScoringConfig::new(HashMap::from([(DimensionId::OverExtension, 1.0)]));
        let hist = aggregate_history(&[r1], &cfg, &HistoryConfig::default());
        assert_eq!(
            hist.value,
            Some(Score::new(90.0)),
            "uncapped dimension average"
        );
        assert_eq!(hist.faults.majors_total, 1, "the Major still shows up here");
        assert_eq!(
            hist.trend,
            vec![Some(Score::new(40.0))],
            "trend keeps the capped per-match value"
        );
    }

    #[test]
    fn headline_reuses_the_dimension_aggregate_formula() {
        let cfg = ScoringConfig::new(HashMap::from([(DimensionId::OverExtension, 1.0)]));
        let r1 = record(
            "m1",
            Some(0.0),
            vec![breakdown(DimensionId::OverExtension, 80.0, 5)],
            FaultSummary::empty(),
        );
        let hist = aggregate_history(&[r1], &cfg, &HistoryConfig::default());
        assert_eq!(hist.value, Some(Score::new(80.0)));
    }
}
