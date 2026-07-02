//! Confidence-weighted aggregation of dimension scores into a [`PacifistScore`].
//!
//! Each [`MetricExtractor`] scores a player on one dimension; this layer blends
//! those into a single headline number using per-dimension weights from a
//! [`ScoringConfig`]. The blend is confidence-weighted so a clean, high-confidence
//! signal moves the headline more than a sparse one, and a dimension that didn't
//! apply at all (confidence 0) drops out of the value entirely.
//!
//! ```text
//! value      = Σ(w·c·v) / Σ(w·c)      (None when Σ(w·c) == 0 — nothing backed it)
//! confidence = Σ(w·c)   / Σ(w)        (weighted coverage of the rubric)
//! influence  = (w·c)    / Σ(w·c)      (per-dimension share of the headline)
//! ```
//!
//! The [`Analyzer`] is the small registry that owns the extractors and the config
//! and runs the whole thing for one player.

use std::cmp::Ordering;
use std::collections::HashMap;

use crate::context::{ContextConfig, MatchContext};
use crate::metrics::{
    BoostEconomy, ChallengeTiming, CommitmentDiscipline, Confidence, DimensionId, DimensionScore,
    Evidence, MetricExtractor, OverExtension, PositioningFit, RotationSoundness, Score,
    ShadowQuality, ShotSelection,
};
use crate::severity::{self, FaultSummary, SeverityConfig};
use crate::{PlayerId, Timeline};

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// Per-dimension weights for the aggregate. Constructed at the edge and threaded
/// in — never a global. A dimension with no entry weighs `0.0`, so it is present
/// in the breakdown but contributes nothing to the headline.
///
/// The default weights are deliberately uncalibrated starting points. Calibrate
/// them by scoring a handful of replays you'd label "very Pacifist" vs "very not"
/// and tuning until the ranking matches your judgement.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoringConfig {
    weights: HashMap<DimensionId, f32>,
}

impl ScoringConfig {
    /// Build a config from explicit weights. Negative weights are clamped to `0.0`
    /// so a misconfigured weight can never invert the aggregate.
    pub fn new(weights: HashMap<DimensionId, f32>) -> Self {
        let weights = weights.into_iter().map(|(d, w)| (d, w.max(0.0))).collect();
        Self { weights }
    }

    /// The weight for `dimension`, or `0.0` if none was configured.
    pub fn weight(&self, dimension: DimensionId) -> f32 {
        self.weights.get(&dimension).copied().unwrap_or(0.0)
    }
}

impl Default for ScoringConfig {
    fn default() -> Self {
        // The full eight-dimension rubric, weighted by the design doc's own
        // confidence tiers: the High-confidence core ethos leads (over-extension
        // and commitment discipline at top billing, boost economy supporting),
        // the Med-confidence positional dimensions follow, and the proxy-heavy
        // Low tier (challenge timing, shot selection) trails so its noise can't
        // swing the headline.
        let weights = HashMap::from([
            (DimensionId::OverExtension, 1.0),
            (DimensionId::CommitmentDiscipline, 1.0),
            (DimensionId::BoostEconomy, 0.6),
            (DimensionId::PositioningFit, 0.8),
            (DimensionId::RotationSoundness, 0.8),
            (DimensionId::ShadowQuality, 0.8),
            (DimensionId::ChallengeTiming, 0.4),
            (DimensionId::ShotSelection, 0.3),
        ]);
        Self::new(weights)
    }
}

// ---------------------------------------------------------------------------
// Result
// ---------------------------------------------------------------------------

/// One dimension's contribution to a [`PacifistScore`], kept for coaching
/// transparency. `influence` is this dimension's share of the headline value —
/// `(weight·confidence) / Σ(weight·confidence)`, in `0.0..=1.0`.
#[derive(Debug, Clone, PartialEq)]
pub struct DimensionBreakdown {
    pub dimension: DimensionId,
    pub value: Score,
    pub confidence: Confidence,
    pub weight: f32,
    pub influence: f32,
    pub evidence: Vec<Evidence>,
}

/// A player's headline Pacifist Score plus the per-dimension breakdown.
///
/// `value` is `None` when no weighted dimension had any backing evidence
/// (`Σ(w·c) == 0`) — there is no honest number to report, so we don't invent one.
/// `confidence` is the weighted coverage of the rubric. `breakdown` is sorted by
/// `influence`, highest first. `faults` is the FM-1 severity ledger — counts,
/// the driving-test verdict, and the timestamped fault list; when a Major is
/// present the headline `value` is capped (see [`aggregate_with_faults`]).
#[derive(Debug, Clone, PartialEq)]
pub struct PacifistScore {
    pub value: Option<Score>,
    pub confidence: Confidence,
    pub breakdown: Vec<DimensionBreakdown>,
    pub faults: FaultSummary,
}

/// Blend per-dimension scores into a [`PacifistScore`] using `config`'s weights,
/// with an empty fault ledger (verdict `Pass`, no cap). Callers that ran a
/// severity pass use [`aggregate_with_faults`] instead.
pub fn aggregate(scores: Vec<DimensionScore>, config: &ScoringConfig) -> PacifistScore {
    aggregate_with_faults(scores, config, FaultSummary::empty(), f32::INFINITY)
}

/// Blend per-dimension scores and attach the FM-1 fault ledger.
///
/// The severity model changes the *shape* of the aggregate, per the guide:
/// **Majors cap** the headline value at `major_cap` (an instant failure should
/// never read as a good match, however clean the averages); **Minors do not**
/// touch the number — the dimension averages already price each penalised
/// episode in, so subtracting again would double-count. Minors show up in the
/// counts and the verdict instead.
pub fn aggregate_with_faults(
    scores: Vec<DimensionScore>,
    config: &ScoringConfig,
    faults: FaultSummary,
    major_cap: f32,
) -> PacifistScore {
    let mut sum_w = 0.0_f32;
    let mut sum_wc = 0.0_f32;
    let mut sum_wcv = 0.0_f32;

    // Carry each dimension's weighted coverage forward so influence is computed
    // once the totals are known.
    struct Row {
        score: DimensionScore,
        weight: f32,
        wc: f32,
    }

    let rows: Vec<Row> = scores
        .into_iter()
        .map(|score| {
            let weight = config.weight(score.dimension);
            let wc = weight * score.confidence.get();
            sum_w += weight;
            sum_wc += wc;
            sum_wcv += wc * score.value.get();
            Row { score, weight, wc }
        })
        .collect();

    let value = (sum_wc > 0.0).then(|| {
        let mean = sum_wcv / sum_wc;
        Score::new(if faults.majors > 0 {
            mean.min(major_cap)
        } else {
            mean
        })
    });
    let confidence = if sum_w > 0.0 {
        Confidence::new(sum_wc / sum_w)
    } else {
        Confidence::new(0.0)
    };

    let mut breakdown: Vec<DimensionBreakdown> = rows
        .into_iter()
        .map(|row| DimensionBreakdown {
            dimension: row.score.dimension,
            value: row.score.value,
            confidence: row.score.confidence,
            weight: row.weight,
            influence: if sum_wc > 0.0 { row.wc / sum_wc } else { 0.0 },
            evidence: row.score.evidence,
        })
        .collect();

    // Highest influence first, so the coaching read leads with what drove the score.
    breakdown.sort_by(|a, b| {
        b.influence
            .partial_cmp(&a.influence)
            .unwrap_or(Ordering::Equal)
    });

    PacifistScore {
        value,
        confidence,
        breakdown,
        faults,
    }
}

// ---------------------------------------------------------------------------
// Analyzer
// ---------------------------------------------------------------------------

/// Owns the extractor registry, the scoring config, and the context-derivation
/// config, and scores one player end-to-end. Composition over inheritance:
/// adding a dimension is adding an extractor to the `Vec` and a weight to the
/// config — nothing else changes.
pub struct Analyzer {
    extractors: Vec<Box<dyn MetricExtractor>>,
    config: ScoringConfig,
    context_config: ContextConfig,
    severity: SeverityConfig,
}

impl Analyzer {
    /// Build an analyzer from an explicit set of extractors and configs, with
    /// the default severity thresholds (override via [`Analyzer::with_severity`]).
    pub fn new(
        extractors: Vec<Box<dyn MetricExtractor>>,
        config: ScoringConfig,
        context_config: ContextConfig,
    ) -> Self {
        Self {
            extractors,
            config,
            context_config,
            severity: SeverityConfig::default(),
        }
    }

    /// Replace the FM-1 severity thresholds.
    pub fn with_severity(mut self, severity: SeverityConfig) -> Self {
        self.severity = severity;
        self
    }

    /// The context-derivation config, so a caller can derive a matching
    /// [`MatchContext`] once and score many players with [`Analyzer::score_player_in`].
    pub fn context_config(&self) -> ContextConfig {
        self.context_config
    }

    /// Derive the context for `timeline` and score one player. Convenience over
    /// [`Analyzer::score_player_in`] when scoring a single player.
    pub fn score_player(&self, timeline: &Timeline, player: PlayerId) -> PacifistScore {
        let ctx = MatchContext::derive(timeline, self.context_config);
        self.score_player_in(&ctx, player)
    }

    /// Run every extractor against a pre-derived context for `player`, run the
    /// FM-1 severity pass, and aggregate. Use this to score several players
    /// without re-deriving the context each time.
    pub fn score_player_in(&self, ctx: &MatchContext, player: PlayerId) -> PacifistScore {
        let scores = self
            .extractors
            .iter()
            .map(|extractor| extractor.extract(ctx, player))
            .collect();
        let ledger = FaultSummary::from_faults(
            severity::faults(ctx, player, &self.severity),
            self.severity.minor_allowance,
        );
        aggregate_with_faults(scores, &self.config, ledger, self.severity.major_cap)
    }
}

impl Default for Analyzer {
    /// The standard analyzer: the full eight-dimension rubric at its default
    /// settings, with the default scoring weights and context derivation.
    fn default() -> Self {
        Self::new(
            vec![
                Box::new(OverExtension::default()),
                Box::new(CommitmentDiscipline::default()),
                Box::new(BoostEconomy::default()),
                Box::new(PositioningFit::default()),
                Box::new(RotationSoundness::default()),
                Box::new(ShadowQuality::default()),
                Box::new(ChallengeTiming::default()),
                Box::new(ShotSelection::default()),
            ],
            ScoringConfig::default(),
            ContextConfig::default(),
        )
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn score(dimension: DimensionId, value: f32, confidence: f32) -> DimensionScore {
        DimensionScore {
            dimension,
            value: Score::new(value),
            confidence: Confidence::new(confidence),
            evidence: Vec::new(),
        }
    }

    fn config(weights: &[(DimensionId, f32)]) -> ScoringConfig {
        ScoringConfig::new(weights.iter().copied().collect())
    }

    #[test]
    fn config_clamps_negative_and_defaults_missing_to_zero() {
        let cfg = config(&[(DimensionId::OverExtension, -5.0)]);
        assert_eq!(
            cfg.weight(DimensionId::OverExtension),
            0.0,
            "negative clamped"
        );
        assert_eq!(
            cfg.weight(DimensionId::BoostEconomy),
            0.0,
            "missing defaults to 0"
        );
    }

    #[test]
    fn single_dimension_passes_through() {
        let cfg = config(&[(DimensionId::OverExtension, 1.0)]);
        let result = aggregate(vec![score(DimensionId::OverExtension, 80.0, 0.5)], &cfg);

        assert_eq!(result.value, Some(Score::new(80.0)));
        // confidence = w·c / w = c.
        assert_eq!(result.confidence, Confidence::new(0.5));
        assert_eq!(result.breakdown.len(), 1);
        assert!((result.breakdown[0].influence - 1.0).abs() < 1e-6);
    }

    #[test]
    fn weighted_blend_matches_the_formula() {
        // value = (1·1·100 + 1·1·0) / (1·1 + 1·1) = 50.
        let cfg = config(&[
            (DimensionId::OverExtension, 1.0),
            (DimensionId::CommitmentDiscipline, 1.0),
        ]);
        let result = aggregate(
            vec![
                score(DimensionId::OverExtension, 100.0, 1.0),
                score(DimensionId::CommitmentDiscipline, 0.0, 1.0),
            ],
            &cfg,
        );

        assert_eq!(result.value, Some(Score::new(50.0)));
        assert_eq!(result.confidence, Confidence::new(1.0));
    }

    #[test]
    fn confidence_weighting_discounts_sparse_signals() {
        // A high but low-confidence dimension barely moves a low high-confidence one.
        // value = (1·1·20 + 1·0.1·100) / (1 + 0.1) = (20 + 10) / 1.1 ≈ 27.27.
        let cfg = config(&[
            (DimensionId::OverExtension, 1.0),
            (DimensionId::BoostEconomy, 1.0),
        ]);
        let result = aggregate(
            vec![
                score(DimensionId::OverExtension, 20.0, 1.0),
                score(DimensionId::BoostEconomy, 100.0, 0.1),
            ],
            &cfg,
        );

        let v = result.value.expect("has a value").get();
        assert!((v - 30.0 / 1.1).abs() < 1e-3, "got {v}");
    }

    #[test]
    fn zero_confidence_dimension_drops_out_of_value_but_stays_in_breakdown() {
        let cfg = config(&[
            (DimensionId::OverExtension, 1.0),
            (DimensionId::BoostEconomy, 1.0),
        ]);
        let result = aggregate(
            vec![
                score(DimensionId::OverExtension, 70.0, 1.0),
                score(DimensionId::BoostEconomy, 0.0, 0.0), // never applied
            ],
            &cfg,
        );

        // BoostEconomy contributes nothing: value is pure OverExtension.
        assert_eq!(result.value, Some(Score::new(70.0)));
        assert_eq!(result.breakdown.len(), 2, "still reported for transparency");
        let boost = result
            .breakdown
            .iter()
            .find(|d| d.dimension == DimensionId::BoostEconomy)
            .unwrap();
        assert_eq!(boost.influence, 0.0);
    }

    #[test]
    fn no_backed_evidence_yields_no_value() {
        // Weighted dimensions, but every confidence is 0 → nothing to report.
        let cfg = config(&[(DimensionId::OverExtension, 1.0)]);
        let result = aggregate(vec![score(DimensionId::OverExtension, 100.0, 0.0)], &cfg);

        assert_eq!(result.value, None);
        assert_eq!(result.confidence, Confidence::new(0.0));
    }

    #[test]
    fn unweighted_dimensions_never_drive_the_value() {
        // Dimension present with confidence but zero weight → no influence, no value.
        let cfg = config(&[]); // nothing weighted
        let result = aggregate(vec![score(DimensionId::OverExtension, 100.0, 1.0)], &cfg);

        assert_eq!(result.value, None, "Σ(w·c) == 0");
        assert_eq!(result.breakdown[0].influence, 0.0);
    }

    #[test]
    fn breakdown_is_sorted_by_influence_descending() {
        let cfg = config(&[
            (DimensionId::OverExtension, 0.2),
            (DimensionId::CommitmentDiscipline, 1.0),
            (DimensionId::BoostEconomy, 0.5),
        ]);
        let result = aggregate(
            vec![
                score(DimensionId::OverExtension, 50.0, 1.0),
                score(DimensionId::CommitmentDiscipline, 50.0, 1.0),
                score(DimensionId::BoostEconomy, 50.0, 1.0),
            ],
            &cfg,
        );

        let order: Vec<DimensionId> = result.breakdown.iter().map(|d| d.dimension).collect();
        assert_eq!(
            order,
            vec![
                DimensionId::CommitmentDiscipline,
                DimensionId::BoostEconomy,
                DimensionId::OverExtension,
            ]
        );
    }

    /// A stub extractor that returns a fixed score, to test analyzer wiring
    /// without depending on any real dimension's semantics.
    struct Stub(DimensionScore);
    impl MetricExtractor for Stub {
        fn id(&self) -> DimensionId {
            self.0.dimension
        }
        fn extract(&self, _ctx: &MatchContext, _player: PlayerId) -> DimensionScore {
            self.0.clone()
        }
    }

    #[test]
    fn analyzer_runs_every_extractor_and_aggregates() {
        let analyzer = Analyzer::new(
            vec![
                Box::new(Stub(score(DimensionId::OverExtension, 100.0, 1.0))),
                Box::new(Stub(score(DimensionId::CommitmentDiscipline, 0.0, 1.0))),
            ],
            config(&[
                (DimensionId::OverExtension, 1.0),
                (DimensionId::CommitmentDiscipline, 1.0),
            ]),
            ContextConfig::default(),
        );

        // Timeline is irrelevant to the stubs; an empty one exercises the wiring.
        let result = analyzer.score_player(&Vec::new(), PlayerId(0));
        assert_eq!(result.value, Some(Score::new(50.0)));
        assert_eq!(result.breakdown.len(), 2);
    }

    #[test]
    fn default_analyzer_registers_the_full_rubric() {
        let analyzer = Analyzer::default();
        let result = analyzer.score_player(&Vec::new(), PlayerId(0));
        // Empty timeline: nothing applies, so no value, but all eight are reported.
        assert_eq!(result.value, None);
        assert_eq!(result.breakdown.len(), 8);
        assert_eq!(result.faults, FaultSummary::empty(), "no play, no faults");
    }

    // -- FM-1 severity shape ---------------------------------------------------

    use crate::severity::{Fault, Severity, Verdict};

    fn ledger(minors: u32, majors: u32) -> FaultSummary {
        let faults = (0..minors)
            .map(|i| Fault {
                t: i as f32,
                severity: Severity::Minor,
                criterion: "F17",
                detail: String::new(),
            })
            .chain((0..majors).map(|i| Fault {
                t: 100.0 + i as f32,
                severity: Severity::Major,
                criterion: "FM-2",
                detail: String::new(),
            }))
            .collect();
        FaultSummary::from_faults(faults, 15)
    }

    #[test]
    fn a_major_caps_the_headline_value() {
        let cfg = config(&[(DimensionId::OverExtension, 1.0)]);
        let result = aggregate_with_faults(
            vec![score(DimensionId::OverExtension, 90.0, 1.0)],
            &cfg,
            ledger(0, 1),
            40.0,
        );
        assert_eq!(result.value, Some(Score::new(40.0)), "capped, not averaged");
        assert_eq!(result.faults.verdict, Verdict::Fail);
    }

    #[test]
    fn a_major_below_the_cap_leaves_the_value_alone() {
        let cfg = config(&[(DimensionId::OverExtension, 1.0)]);
        let result = aggregate_with_faults(
            vec![score(DimensionId::OverExtension, 25.0, 1.0)],
            &cfg,
            ledger(0, 1),
            40.0,
        );
        assert_eq!(result.value, Some(Score::new(25.0)), "min, not floor");
    }

    #[test]
    fn minors_never_touch_the_number_only_the_verdict() {
        let cfg = config(&[(DimensionId::OverExtension, 1.0)]);
        let result = aggregate_with_faults(
            vec![score(DimensionId::OverExtension, 90.0, 1.0)],
            &cfg,
            ledger(16, 0),
            40.0,
        );
        assert_eq!(
            result.value,
            Some(Score::new(90.0)),
            "already priced into the dimension averages"
        );
        assert_eq!(result.faults.verdict, Verdict::Fail, "16 > the allowance");
    }

    #[test]
    fn plain_aggregate_carries_an_empty_passing_ledger() {
        let cfg = config(&[(DimensionId::OverExtension, 1.0)]);
        let result = aggregate(vec![score(DimensionId::OverExtension, 90.0, 1.0)], &cfg);
        assert_eq!(result.value, Some(Score::new(90.0)));
        assert_eq!(result.faults.verdict, Verdict::Pass);
        assert!(result.faults.faults.is_empty());
    }
}
