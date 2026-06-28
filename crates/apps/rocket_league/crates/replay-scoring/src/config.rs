//! Versioned, data-driven scoring configuration.
//!
//! Per the spec, weights / target bands / calibration curves / tier table /
//! archetype centroids / leak→chapter map live in config, never hard-coded into
//! the engine. The built-in [`ScoreConfig::default`] is a *pre-calibration*
//! starting point — the bands are deliberately guesses; real numbers require a
//! labeled corpus. Changing any default MUST bump [`SCORE_CONFIG_VERSION`].

use serde::{Deserialize, Serialize};

/// Version stamped onto every report; bump when the defaults below change.
pub const SCORE_CONFIG_VERSION: &str = "scfg-v6";

/// Which sub-score a metric feeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    First,
    Second,
    General,
}

/// The metric registry. Each variant is computed by [`crate::metrics`] and
/// normalized by a [`Curve`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    OvercommitRate,
    GoalsideDiscipline1st,
    SupportSpacing,
    CentralSupportFraction,
    DoubleCommitRate,
    BoostManagement,
    PossessionRetention,
    BallChaseIndex,
    GoalsideDisciplineTeam,
    ChallengeTiming,
    FirstTouchValue,
    TransitionReadiness,
    RecoverySpeed,
    AerialPresence,
    // --- bcstats-derived metrics (graduated on corpus rank evidence) ---
    FacingBallShare,
    ReverseDriving,
}

impl Metric {
    /// Stable string key (for reports / leak selection).
    pub fn key(&self) -> &'static str {
        match self {
            Metric::OvercommitRate => "overcommit_rate",
            Metric::GoalsideDiscipline1st => "goalside_discipline_1st",
            Metric::SupportSpacing => "support_spacing",
            Metric::CentralSupportFraction => "central_support_fraction",
            Metric::DoubleCommitRate => "double_commit_rate",
            Metric::BoostManagement => "boost_management",
            Metric::PossessionRetention => "possession_retention",
            Metric::BallChaseIndex => "ball_chase_index",
            Metric::GoalsideDisciplineTeam => "goalside_discipline_team",
            Metric::ChallengeTiming => "challenge_timing",
            Metric::FirstTouchValue => "first_touch_value",
            Metric::TransitionReadiness => "transition_readiness",
            Metric::RecoverySpeed => "recovery_speed",
            Metric::AerialPresence => "aerial_presence",
            Metric::FacingBallShare => "facing_ball_share",
            Metric::ReverseDriving => "reverse_driving",
        }
    }
}

/// A monotonic-or-banded calibration curve mapping a raw metric to `0..=100`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Curve {
    /// Higher raw is better: `<= zero` → 0, `>= full` → 100 (linear between).
    Higher { zero: f32, full: f32 },
    /// Lower raw is better: `>= zero` → 0, `<= full` → 100 (linear between).
    Lower { zero: f32, full: f32 },
    /// Inside `[lo, hi]` → 100; linearly to 0 by `falloff` beyond each edge.
    Band { lo: f32, hi: f32, falloff: f32 },
}

impl Curve {
    /// Map a raw metric value to a `0..=100` score.
    pub fn normalize(&self, raw: f32) -> f32 {
        let s = match *self {
            Curve::Higher { zero, full } => lerp01(raw, zero, full),
            Curve::Lower { zero, full } => lerp01(raw, zero, full),
            Curve::Band { lo, hi, falloff } => {
                if raw >= lo && raw <= hi {
                    1.0
                } else if raw < lo {
                    lerp01(raw, lo - falloff, lo)
                } else {
                    lerp01(raw, hi + falloff, hi)
                }
            }
        };
        (s * 100.0).clamp(0.0, 100.0)
    }
}

/// Linear interpolation of `x` from `a`→0 to `b`→1, clamped to `[0,1]`. Works in
/// either direction (`b` may be less than `a`).
fn lerp01(x: f32, a: f32, b: f32) -> f32 {
    if (b - a).abs() < f32::EPSILON {
        return if x >= b { 1.0 } else { 0.0 };
    }
    ((x - a) / (b - a)).clamp(0.0, 1.0)
}

/// One metric's full specification: which sub-score, calibration, weight within
/// its sub-score, and the book chapter its deficit maps to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricSpec {
    pub metric: Metric,
    pub role: Role,
    pub curve: Curve,
    pub weight: f32,
    pub chapter: String,
    /// A **candidate** metric: computed and cross-checked against ΔV/rank by
    /// [`crate::reconcile`], but kept *out* of the composite, sub-scores, and leak
    /// selection until it earns its place. Promote it (clear the flag, set a
    /// weight) with [`crate::reconcile::promote_candidates`] once ΔV shows it
    /// carries signal. Defaults to `false` so existing configs deserialize
    /// unchanged.
    #[serde(default)]
    pub experimental: bool,
}

/// A player-type archetype centroid in normalized behavioral-feature space.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Centroid {
    pub name: String,
    /// Expected normalized (`0..=1`) values keyed by the behavioral features
    /// `[overcommit, central_support, ball_chase, double_commit, boost_hoard]`.
    pub vector: [f32; 5],
}

/// A licence tier and the minimum composite to earn it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tier {
    pub min_composite: f32,
    pub name: String,
}

/// Full scoring configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreConfig {
    pub version: String,
    pub metrics: Vec<MetricSpec>,
    /// Top-level sub-score weights `[first, second, general]` (renormalized).
    pub top_weights: [f32; 3],
    /// Tiers, ascending by `min_composite`.
    pub tiers: Vec<Tier>,
    pub centroids: Vec<Centroid>,

    // --- thresholds used by feature/role/metric computation ---
    /// Role hysteresis: a 1st/2nd swap must persist this long (s) to register.
    pub role_min_persist_s: f32,
    /// time_to_ball below this (s) counts as "pressuring" (double-commit).
    pub press_ttb_s: f32,
    /// |x| below this (uu) is "central" support.
    pub central_x: f32,
    /// Field third boundary on attacking-frame y (uu).
    pub third_y: f32,
    /// Seconds after a kickoff to exclude from positional metrics.
    pub kickoff_exclude_s: f32,
    /// Seconds after a demo to exclude the victim from positional metrics.
    pub respawn_exclude_s: f32,
    /// Minimum valid positional frames for a confident report.
    pub min_sample_frames: usize,
    /// Both 1st men within this radius (uu) of the ball ⇒ a 50/50 contest.
    pub challenge_radius_uu: f32,
    /// Minimum boost (%) for a challenge arrival to count as "with boost".
    pub challenge_boost_min: f32,
    /// Arrival is "not late" if our time_to_ball ≤ opp's × this margin.
    pub challenge_late_margin: f32,
    /// Min cosine alignment of heading→ball/play to count as "facing".
    pub facing_cos_min: f32,
    /// Car centre above this height (uu) is airborne (aerial / off-ground).
    pub airborne_z_uu: f32,
    /// |pitch| and |roll| (rad) below this is "wheels-down" (upright).
    pub upright_max_rad: f32,
    /// Cap (s) on a single recovery (and the value charged for non-recoveries).
    pub recovery_cap_s: f32,
    /// Post-touch ball speed (uu/s) above which a touch counts as a boom.
    pub boom_speed_uu: f32,
    /// Window (s) after a possession flip over which a ready 2nd man closes on
    /// the ball (its time-to-ball shrinks) as it steps up to 1st.
    pub transition_window_s: f32,
    /// Boost-collection rate (units/s) that maps to a full collection score in
    /// the boost-management composite.
    pub boost_collect_scale: f32,
}

impl Default for ScoreConfig {
    fn default() -> Self {
        let m = |metric, role, curve, weight, chapter: &str| MetricSpec {
            metric,
            role,
            curve,
            weight,
            chapter: chapter.to_string(),
            experimental: false,
        };
        ScoreConfig {
            version: SCORE_CONFIG_VERSION.to_string(),
            metrics: vec![
                // --- 1st man (pressure quality) ---
                m(
                    Metric::OvercommitRate,
                    Role::First,
                    Curve::Lower {
                        zero: 0.50,
                        full: 0.10,
                    },
                    0.25,
                    "Controlled counterattacks",
                ),
                m(
                    Metric::GoalsideDiscipline1st,
                    Role::First,
                    Curve::Higher {
                        zero: 0.50,
                        full: 0.90,
                    },
                    0.25,
                    "Core game states / defence",
                ),
                m(
                    Metric::ChallengeTiming,
                    Role::First,
                    Curve::Higher {
                        zero: 0.30,
                        full: 0.80,
                    },
                    0.30,
                    "Trigger discipline",
                ),
                m(
                    Metric::FirstTouchValue,
                    Role::First,
                    Curve::Higher {
                        zero: -0.20,
                        full: 0.60,
                    },
                    0.20,
                    "Ground control / stop booming",
                ),
                // --- 2nd man (support quality) ---
                m(
                    Metric::SupportSpacing,
                    Role::Second,
                    Curve::Band {
                        lo: 1200.0,
                        hi: 2600.0,
                        falloff: 1400.0,
                    },
                    0.25,
                    "Central support",
                ),
                m(
                    Metric::CentralSupportFraction,
                    Role::Second,
                    Curve::Higher {
                        zero: 0.30,
                        full: 0.75,
                    },
                    0.25,
                    "Central support",
                ),
                m(
                    Metric::DoubleCommitRate,
                    Role::Second,
                    Curve::Lower {
                        zero: 0.40,
                        full: 0.08,
                    },
                    0.25,
                    "Central support",
                ),
                m(
                    Metric::TransitionReadiness,
                    Role::Second,
                    Curve::Higher {
                        zero: 0.30,
                        full: 0.80,
                    },
                    0.25,
                    "Core game states / transitions",
                ),
                // --- general (cross-role habits) ---
                m(
                    Metric::BoostManagement,
                    Role::General,
                    Curve::Higher {
                        zero: 0.12,
                        full: 0.45,
                    },
                    0.20,
                    "Fundamentals / boost",
                ),
                m(
                    Metric::PossessionRetention,
                    Role::General,
                    Curve::Higher {
                        zero: 0.30,
                        full: 0.60,
                    },
                    0.20,
                    "Ground control / stop booming",
                ),
                m(
                    Metric::BallChaseIndex,
                    Role::General,
                    Curve::Lower {
                        zero: 0.60,
                        full: 0.20,
                    },
                    0.20,
                    "Positioning",
                ),
                m(
                    Metric::GoalsideDisciplineTeam,
                    Role::General,
                    Curve::Higher {
                        zero: 0.70,
                        full: 0.97,
                    },
                    0.20,
                    "Defence structure",
                ),
                m(
                    Metric::RecoverySpeed,
                    Role::General,
                    Curve::Lower {
                        zero: 2.0,
                        full: 0.8,
                    },
                    0.20,
                    "Air system / recovery",
                ),
                m(
                    Metric::AerialPresence,
                    Role::General,
                    Curve::Higher {
                        zero: 0.01,
                        full: 0.10,
                    },
                    0.20,
                    "Air system / aerial threat",
                ),
                // --- bcstats-derived metrics, graduated on corpus rank evidence ---
                // Ball-watching is a *low-rank* habit: facing the ball more
                // correlates with lower rank (ρ_rank ≈ −0.47 on the 723-player
                // corpus), so the curve is "lower is better" — the opposite of the
                // first experimental guess. Promoted on rank (its ΔV signal is ~0).
                m(
                    Metric::FacingBallShare,
                    Role::General,
                    Curve::Lower {
                        zero: 0.55,
                        full: 0.30,
                    },
                    0.20,
                    "Positioning / awareness",
                ),
                // Time spent reversing on the ground is a strong negative rank
                // signal (ρ_rank ≈ −0.61, one of the strongest in the rubric).
                m(
                    Metric::ReverseDriving,
                    Role::General,
                    Curve::Lower {
                        zero: 0.20,
                        full: 0.02,
                    },
                    0.20,
                    "Fundamentals / car control",
                ),
                // No experimental candidates ship today. To add one, append a spec
                // with `experimental: true` (e.g. `MetricSpec { experimental: true,
                // ..m(metric, role, curve, weight, chapter) }`): it is computed and
                // reconciled against ΔV/rank but stays out of the composite until
                // `reconcile::promote_candidates` graduates it on evidence.
            ],
            top_weights: [0.35, 0.30, 0.35],
            tiers: vec![
                Tier {
                    min_composite: 0.0,
                    name: "Unranked".into(),
                },
                Tier {
                    min_composite: 45.0,
                    name: "Silver Licence".into(),
                },
                Tier {
                    min_composite: 58.0,
                    name: "Gold Licence".into(),
                },
                Tier {
                    min_composite: 68.0,
                    name: "Platinum Licence".into(),
                },
                Tier {
                    min_composite: 76.0,
                    name: "Diamond Licence".into(),
                },
                Tier {
                    min_composite: 84.0,
                    name: "Elite Licence".into(),
                },
                Tier {
                    min_composite: 92.0,
                    name: "Pacifist Master".into(),
                },
            ],
            // Behavioral vector order: [overcommit, central_support, ball_chase, double_commit, boost_hoard]
            centroids: vec![
                Centroid {
                    name: "Calm Controller".into(),
                    vector: [0.15, 0.75, 0.20, 0.10, 0.55],
                },
                Centroid {
                    name: "Diver".into(),
                    vector: [0.70, 0.30, 0.75, 0.40, 0.45],
                },
                Centroid {
                    name: "Stacker".into(),
                    vector: [0.50, 0.25, 0.80, 0.70, 0.40],
                },
            ],
            role_min_persist_s: 0.4,
            press_ttb_s: 1.2,
            central_x: 1800.0,
            third_y: 1707.0,
            kickoff_exclude_s: 3.0,
            respawn_exclude_s: 3.0,
            min_sample_frames: 1500,
            challenge_radius_uu: 900.0,
            challenge_boost_min: 12.0,
            challenge_late_margin: 1.25,
            facing_cos_min: 0.30,
            airborne_z_uu: 300.0,
            upright_max_rad: 0.45,
            recovery_cap_s: 2.5,
            boom_speed_uu: 4000.0,
            transition_window_s: 1.5,
            boost_collect_scale: 30.0,
        }
    }
}
