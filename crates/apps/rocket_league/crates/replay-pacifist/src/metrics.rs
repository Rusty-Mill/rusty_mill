//! Scoring dimensions.
//!
//! Each Pacifist Score dimension is an independent [`MetricExtractor`]: it reads
//! the [`MatchContext`] and emits a 0..=100 [`Score`] for one player, with a
//! [`Confidence`] reflecting how much evidence it had. The aggregate score
//! (built in [`crate::scoring`]) confidence-weights the dimensions so weak
//! signals don't swing the headline number.
//!
//! Extractors read the *enriched* context — per-frame [`Role`], distances, and
//! goalside facts derived once in [`crate::context`] — rather than re-deriving
//! geometry from raw positions. That keeps each extractor a thin, role-aware
//! reader. Three high-confidence dimensions are implemented:
//!
//! - **over-extension** (1st man) — diving in as the clear 1st man with no cover
//!   behind. Blames the player who committed.
//! - **commitment discipline** (2nd man) — the cover man leaving the net, pushing
//!   up-field of the ball instead of holding goalside. The role-attributed
//!   counterpart to over-extension: it blames the man who should have stayed.
//! - **boost economy** — arriving at the decisive moment (the engaging man, near
//!   the ball) empty.
//!
//! Each is oriented so a higher score means *more* Pacifist (more discipline,
//! less over-commit).

use crate::context::{FrameContext, MatchContext, Role};
use crate::{PlayerId, WorldState};

// ---------------------------------------------------------------------------
// Shared scoring types
// ---------------------------------------------------------------------------

/// Identifies a scoring dimension.
///
/// The full rubric (design doc §2.2) names eight dimensions; all are declared
/// here so [`crate::scoring::ScoringConfig`]'s key space and the roadmap are
/// explicit. Only the variants marked *implemented* have an extractor and a
/// default weight today — the rest are produced by nothing and carry weight 0
/// until their extractor lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DimensionId {
    /// *Implemented.* Punishes diving in as the clear 1st man with no cover behind.
    OverExtension,
    /// *Implemented.* Punishes the 2nd man leaving the net (pushing up-field).
    CommitmentDiscipline,
    /// *Implemented.* Rewards arriving at the decisive moment with boost on hand.
    BoostEconomy,
    /// The right area for the situation × role. Not yet implemented.
    PositioningFit,
    /// Back-post / out-of-the-way recovery to cover. Not yet implemented.
    RotationSoundness,
    /// 1st man engaging on the right beat. Not yet implemented.
    ChallengeTiming,
    /// 2nd man holding a covering line/distance. Not yet implemented.
    ShadowQuality,
    /// High-percentage shot selection over hero gambles. Not yet implemented.
    ShotSelection,
}

impl DimensionId {
    /// A short, stable, human-readable label for output. Exhaustive on purpose:
    /// a new dimension won't compile until it is given a label.
    pub fn label(self) -> &'static str {
        match self {
            DimensionId::OverExtension => "over-extension",
            DimensionId::CommitmentDiscipline => "commitment-discipline",
            DimensionId::BoostEconomy => "boost-economy",
            DimensionId::PositioningFit => "positioning-fit",
            DimensionId::RotationSoundness => "rotation-soundness",
            DimensionId::ChallengeTiming => "challenge-timing",
            DimensionId::ShadowQuality => "shadow-quality",
            DimensionId::ShotSelection => "shot-selection",
        }
    }
}

/// A dimension result in `0.0..=100.0`. Construction clamps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Score(f32);

impl Score {
    pub fn new(value: f32) -> Self {
        Self(value.clamp(0.0, 100.0))
    }

    pub fn get(self) -> f32 {
        self.0
    }
}

/// How much evidence backed a score, in `0.0..=1.0`. Construction clamps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Confidence(f32);

impl Confidence {
    pub fn new(value: f32) -> Self {
        Self(value.clamp(0.0, 1.0))
    }

    pub fn get(self) -> f32 {
        self.0
    }
}

/// A single moment that contributed to a score — kept so the eventual UI can
/// jump to the exact frame. Collection is capped per extractor.
#[derive(Debug, Clone, PartialEq)]
pub struct Evidence {
    pub t: f32,
    pub detail: String,
}

/// The result of scoring one player on one dimension.
#[derive(Debug, Clone, PartialEq)]
pub struct DimensionScore {
    pub dimension: DimensionId,
    pub value: Score,
    pub confidence: Confidence,
    pub evidence: Vec<Evidence>,
}

/// A scoring dimension: read the enriched context, score one player.
pub trait MetricExtractor {
    fn id(&self) -> DimensionId;
    fn extract(&self, ctx: &MatchContext, player: PlayerId) -> DimensionScore;
}

// ---------------------------------------------------------------------------
// Shared tally
// ---------------------------------------------------------------------------

/// Roll a per-frame binary classifier into a [`DimensionScore`].
///
/// For each frame `classify` returns `Some(penalised)` when the frame is
/// *relevant* to the dimension (`penalised = true` for a frame that counts
/// against the player) or `None` when the dimension doesn't apply that frame.
/// The score rewards a low penalised fraction:
///
/// ```text
/// value = 100 * (1 - penalised_frames / relevant_frames)
/// ```
///
/// Confidence rises with the number of relevant frames, saturating at
/// `saturation_frames`. A player with no relevant frames gets value 100 and
/// confidence 0 — the dimension simply doesn't apply to them, so it drops out of
/// the aggregate entirely. Evidence is collected on penalised frames, capped at
/// `max_evidence`.
fn tally<F>(
    ctx: &MatchContext,
    dimension: DimensionId,
    saturation_frames: f32,
    max_evidence: usize,
    evidence_detail: &str,
    mut classify: F,
) -> DimensionScore
where
    F: FnMut(&WorldState, &FrameContext) -> Option<bool>,
{
    let mut relevant: u32 = 0;
    let mut penalised: u32 = 0;
    let mut evidence = Vec::new();

    for (snapshot, frame) in ctx.frames() {
        let Some(is_penalised) = classify(snapshot, frame) else {
            continue;
        };
        relevant += 1;
        if is_penalised {
            penalised += 1;
            if evidence.len() < max_evidence {
                evidence.push(Evidence {
                    t: snapshot.t,
                    detail: evidence_detail.to_string(),
                });
            }
        }
    }

    if relevant == 0 {
        return DimensionScore {
            dimension,
            value: Score::new(100.0),
            confidence: Confidence::new(0.0),
            evidence,
        };
    }

    let penalised_fraction = penalised as f32 / relevant as f32;
    DimensionScore {
        dimension,
        value: Score::new(100.0 * (1.0 - penalised_fraction)),
        confidence: Confidence::new(relevant as f32 / saturation_frames),
        evidence,
    }
}

// ---------------------------------------------------------------------------
// Over-extension (1st man)
// ---------------------------------------------------------------------------

/// Penalises committing as 1st man with no cover behind.
///
/// For each frame in which `player` is the *clear* 1st man — the role-derived 1st
/// man, and closer to the ball than the next teammate by at least
/// `clear_margin_uu` — it checks whether any teammate is goalside of the ball.
/// Frames where no teammate covers are "exposed" (see [`tally`]).
///
/// High-confidence (positions only) and a direct expression of the
/// patience-and-control ethos. A player who is never the clear 1st man gets
/// confidence 0.
#[derive(Debug, Clone, Copy)]
pub struct OverExtension {
    /// How much closer to the ball the 1st man must be than the next teammate to
    /// count as the *clear* 1st man. `0.0` means "strictly closest".
    pub clear_margin_uu: f32,
    /// Number of 1st-man frames at which confidence reaches 1.0.
    pub confidence_saturation_frames: f32,
    /// Cap on collected evidence entries.
    pub max_evidence: usize,
}

impl Default for OverExtension {
    fn default() -> Self {
        Self {
            clear_margin_uu: 0.0,
            confidence_saturation_frames: 300.0,
            max_evidence: 25,
        }
    }
}

impl MetricExtractor for OverExtension {
    fn id(&self) -> DimensionId {
        DimensionId::OverExtension
    }

    fn extract(&self, ctx: &MatchContext, player: PlayerId) -> DimensionScore {
        tally(
            ctx,
            DimensionId::OverExtension,
            self.confidence_saturation_frames,
            self.max_evidence,
            "1st man committed with no teammate goalside of the ball",
            |_snapshot, frame| {
                let me = frame.player(player)?;
                if me.role != Role::FirstMan {
                    return None;
                }
                // Clear 1st man: closer than the next teammate by the margin.
                let nearest_mate = frame
                    .teammates_of(player)
                    .map(|t| t.dist_to_ball)
                    .fold(f32::INFINITY, f32::min);
                if me.dist_to_ball + self.clear_margin_uu >= nearest_mate {
                    return None;
                }
                let covered = frame.teammates_of(player).any(|t| t.goalside);
                Some(!covered)
            },
        )
    }
}

// ---------------------------------------------------------------------------
// Commitment discipline (2nd man)
// ---------------------------------------------------------------------------

/// Penalises the 2nd man for leaving the net — pushing up-field of the ball
/// instead of holding a covering position goalside of it.
///
/// For each frame in which `player` is the 2nd man, it is penalised when they are
/// *not* goalside of the ball (see [`tally`]). This is the role-attributed
/// counterpart to [`OverExtension`]: over-extension blames the 1st man who dove,
/// commitment discipline blames the cover man who failed to stay back — together
/// they account for a double-commit from both sides.
///
/// Semantics are 2v2-tuned: the 1st/2nd-man split assumes two players per side,
/// so this maps cleanly onto 2s and not onto three-deep rotation.
#[derive(Debug, Clone, Copy)]
pub struct CommitmentDiscipline {
    /// Number of 2nd-man frames at which confidence reaches 1.0.
    pub confidence_saturation_frames: f32,
    /// Cap on collected evidence entries.
    pub max_evidence: usize,
}

impl Default for CommitmentDiscipline {
    fn default() -> Self {
        Self {
            confidence_saturation_frames: 300.0,
            max_evidence: 25,
        }
    }
}

impl MetricExtractor for CommitmentDiscipline {
    fn id(&self) -> DimensionId {
        DimensionId::CommitmentDiscipline
    }

    fn extract(&self, ctx: &MatchContext, player: PlayerId) -> DimensionScore {
        tally(
            ctx,
            DimensionId::CommitmentDiscipline,
            self.confidence_saturation_frames,
            self.max_evidence,
            "2nd man pushed up-field of the ball, leaving the net open",
            |_snapshot, frame| {
                let me = frame.player(player)?;
                if me.role != Role::SecondMan {
                    return None;
                }
                Some(!me.goalside)
            },
        )
    }
}

// ---------------------------------------------------------------------------
// Boost economy
// ---------------------------------------------------------------------------

/// Rewards arriving at the decisive moment with boost on hand.
///
/// A "decisive moment" is the frame where `player` is the 1st man (the engaging
/// man) and within `engage_radius_uu` of the ball. In such a frame the player is
/// "empty" if their boost is below `empty_boost`. The score rewards a low empty
/// fraction (see [`tally`]); higher means better boost economy.
///
/// Needs only positions and boost, so it is high-confidence. A player who is
/// never the engaging man gets confidence 0.
#[derive(Debug, Clone, Copy)]
pub struct BoostEconomy {
    /// How close to the ball (replay units) the 1st man must be for a frame to
    /// count as a decisive moment.
    pub engage_radius_uu: f32,
    /// Boost level below which the player counts as "arriving empty", `0..=100`.
    pub empty_boost: u8,
    /// Number of engaged frames at which confidence reaches 1.0.
    pub confidence_saturation_frames: f32,
    /// Cap on collected evidence entries.
    pub max_evidence: usize,
}

impl Default for BoostEconomy {
    fn default() -> Self {
        Self {
            engage_radius_uu: 1200.0,
            empty_boost: 12,
            confidence_saturation_frames: 200.0,
            max_evidence: 25,
        }
    }
}

impl MetricExtractor for BoostEconomy {
    fn id(&self) -> DimensionId {
        DimensionId::BoostEconomy
    }

    fn extract(&self, ctx: &MatchContext, player: PlayerId) -> DimensionScore {
        tally(
            ctx,
            DimensionId::BoostEconomy,
            self.confidence_saturation_frames,
            self.max_evidence,
            "engaged the ball with little to no boost",
            |_snapshot, frame| {
                let me = frame.player(player)?;
                if me.role != Role::FirstMan || me.dist_to_ball > self.engage_radius_uu {
                    return None;
                }
                Some(me.boost < self.empty_boost)
            },
        )
    }
}

// ---------------------------------------------------------------------------
// Tests — synthetic timelines built by hand.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{ContextConfig, MatchContext};
    use crate::{BallState, PlayerState, Pose, Quat, Team, Timeline, Vec3};

    fn pos(x: f32, y: f32) -> Pose {
        Pose {
            position: Vec3::new(x, y, 17.0),
            rotation: Quat::IDENTITY,
        }
    }

    fn ball_at(x: f32, y: f32) -> BallState {
        BallState {
            pose: Pose {
                position: Vec3::new(x, y, 93.0),
                rotation: Quat::IDENTITY,
            },
            velocity: Vec3::ZERO,
        }
    }

    fn player(id: u32, team: Team, p: Pose) -> PlayerState {
        player_with_boost(id, team, p, 33)
    }

    fn player_with_boost(id: u32, team: Team, p: Pose, boost: u8) -> PlayerState {
        PlayerState {
            player: PlayerId(id),
            team,
            pose: p,
            velocity: Vec3::ZERO,
            boost,
            demolished: false,
            name: None,
        }
    }

    /// player 0 near the ball (1st man), player 1 parked far (2nd man), both blue.
    fn frame(t: f32, ball_y: f32, challenger_y: f32, teammate_y: f32) -> WorldState {
        WorldState {
            t,
            ball: Some(ball_at(0.0, ball_y)),
            players: vec![
                player(0, Team::Blue, pos(0.0, challenger_y)),
                player(1, Team::Blue, pos(2000.0, teammate_y)),
            ],
        }
    }

    fn ctx(timeline: &Timeline) -> MatchContext<'_> {
        MatchContext::derive(timeline, ContextConfig::default())
    }

    // -- over-extension (1st man) -------------------------------------------

    #[test]
    fn committed_with_cover_scores_high() {
        let timeline: Timeline = (0..10)
            .map(|i| frame(i as f32, 3000.0, 2900.0, -2000.0))
            .collect();
        let result = OverExtension::default().extract(&ctx(&timeline), PlayerId(0));

        assert_eq!(result.value, Score::new(100.0), "always covered");
        assert!(result.confidence.get() > 0.0);
        assert!(result.evidence.is_empty());
    }

    #[test]
    fn committed_without_cover_scores_low() {
        let timeline: Timeline = (0..10)
            .map(|i| frame(i as f32, 3000.0, 2900.0, 3500.0))
            .collect();
        let result = OverExtension::default().extract(&ctx(&timeline), PlayerId(0));

        assert_eq!(result.value, Score::new(0.0), "never covered");
        assert_eq!(result.evidence.len(), 10);
    }

    #[test]
    fn half_covered_scores_in_between() {
        let mut timeline: Timeline = (0..4)
            .map(|i| frame(i as f32, 3000.0, 2900.0, -2000.0))
            .collect();
        timeline.extend((4..8).map(|i| frame(i as f32, 3000.0, 2900.0, 3500.0)));
        let result = OverExtension::default().extract(&ctx(&timeline), PlayerId(0));

        assert_eq!(result.value, Score::new(50.0));
        assert_eq!(result.evidence.len(), 4);
    }

    #[test]
    fn never_first_man_is_zero_confidence() {
        // Player 1 is always the far man → never the 1st man.
        let timeline: Timeline = (0..10)
            .map(|i| frame(i as f32, 3000.0, 2900.0, -2000.0))
            .collect();
        let result = OverExtension::default().extract(&ctx(&timeline), PlayerId(1));

        assert_eq!(
            result.confidence,
            Confidence::new(0.0),
            "metric doesn't apply"
        );
    }

    #[test]
    fn ball_less_and_demolished_frames_are_skipped() {
        let mut timeline: Timeline = Vec::new();
        timeline.push(WorldState {
            t: 0.0,
            ball: None,
            players: vec![player(0, Team::Blue, pos(0.0, 2900.0))],
        });
        let mut demo = frame(1.0, 3000.0, 2900.0, 3500.0);
        demo.players[0].demolished = true;
        timeline.push(demo);
        timeline.push(frame(2.0, 3000.0, 2900.0, 3500.0));

        let result = OverExtension::default().extract(&ctx(&timeline), PlayerId(0));
        assert_eq!(result.value, Score::new(0.0));
        assert_eq!(result.evidence.len(), 1);
        assert_eq!(result.evidence[0].t, 2.0);
    }

    #[test]
    fn orange_orientation_is_mirrored() {
        let make = |t: f32, teammate_y: f32| WorldState {
            t,
            ball: Some(ball_at(0.0, -3000.0)),
            players: vec![
                player(2, Team::Orange, pos(0.0, -2900.0)),
                player(3, Team::Orange, pos(2000.0, teammate_y)),
            ],
        };
        let covered: Timeline = (0..5).map(|i| make(i as f32, 2000.0)).collect();
        let exposed: Timeline = (0..5).map(|i| make(i as f32, -3500.0)).collect();

        assert_eq!(
            OverExtension::default()
                .extract(&ctx(&covered), PlayerId(2))
                .value,
            Score::new(100.0)
        );
        assert_eq!(
            OverExtension::default()
                .extract(&ctx(&exposed), PlayerId(2))
                .value,
            Score::new(0.0)
        );
    }

    #[test]
    fn evidence_is_capped() {
        let cfg = OverExtension {
            max_evidence: 3,
            ..OverExtension::default()
        };
        let timeline: Timeline = (0..20)
            .map(|i| frame(i as f32, 3000.0, 2900.0, 3500.0))
            .collect();
        let result = cfg.extract(&ctx(&timeline), PlayerId(0));

        assert_eq!(result.value, Score::new(0.0));
        assert_eq!(
            result.evidence.len(),
            3,
            "evidence capped, score unaffected"
        );
    }

    // -- commitment discipline (2nd man) ------------------------------------

    #[test]
    fn cover_man_goalside_scores_high() {
        // Player 1 is the 2nd man, sitting goalside (-2000, behind the ball).
        let timeline: Timeline = (0..10)
            .map(|i| frame(i as f32, 3000.0, 2900.0, -2000.0))
            .collect();
        let result = CommitmentDiscipline::default().extract(&ctx(&timeline), PlayerId(1));

        assert_eq!(result.value, Score::new(100.0), "cover man always holds");
        assert!(result.confidence.get() > 0.0);
        assert!(result.evidence.is_empty());
    }

    #[test]
    fn cover_man_up_field_scores_low() {
        // Player 1 (2nd man) pushed up to +3500, ahead of the ball at +3000.
        let timeline: Timeline = (0..10)
            .map(|i| frame(i as f32, 3000.0, 2900.0, 3500.0))
            .collect();
        let result = CommitmentDiscipline::default().extract(&ctx(&timeline), PlayerId(1));

        assert_eq!(result.value, Score::new(0.0), "cover man never holds");
        assert_eq!(result.evidence.len(), 10);
    }

    #[test]
    fn cover_man_half_holding_scores_in_between() {
        let mut timeline: Timeline = (0..4)
            .map(|i| frame(i as f32, 3000.0, 2900.0, -2000.0))
            .collect();
        timeline.extend((4..8).map(|i| frame(i as f32, 3000.0, 2900.0, 3500.0)));
        let result = CommitmentDiscipline::default().extract(&ctx(&timeline), PlayerId(1));

        assert_eq!(result.value, Score::new(50.0));
        assert_eq!(result.evidence.len(), 4);
    }

    #[test]
    fn first_man_does_not_get_a_commitment_score() {
        // Player 0 is the 1st man, so this 2nd-man metric doesn't apply.
        let timeline: Timeline = (0..10)
            .map(|i| frame(i as f32, 3000.0, 2900.0, 3500.0))
            .collect();
        let result = CommitmentDiscipline::default().extract(&ctx(&timeline), PlayerId(0));

        assert_eq!(
            result.confidence,
            Confidence::new(0.0),
            "1st man isn't the cover man"
        );
    }

    #[test]
    fn commitment_orange_orientation_is_mirrored() {
        // Orange attacks -y; player 2 on the ball (1st man), player 3 the cover man.
        let make = |t: f32, teammate_y: f32| WorldState {
            t,
            ball: Some(ball_at(0.0, -3000.0)),
            players: vec![
                player(2, Team::Orange, pos(0.0, -2900.0)),
                player(3, Team::Orange, pos(2000.0, teammate_y)),
            ],
        };
        // Cover man goalside (+2000, behind the ball for orange) → holds.
        let held: Timeline = (0..5).map(|i| make(i as f32, 2000.0)).collect();
        // Cover man up-field (-3500, ahead of the ball) → left the net.
        let left: Timeline = (0..5).map(|i| make(i as f32, -3500.0)).collect();

        assert_eq!(
            CommitmentDiscipline::default()
                .extract(&ctx(&held), PlayerId(3))
                .value,
            Score::new(100.0)
        );
        assert_eq!(
            CommitmentDiscipline::default()
                .extract(&ctx(&left), PlayerId(3))
                .value,
            Score::new(0.0)
        );
    }

    #[test]
    fn commitment_skips_ball_less_and_demolished_frames() {
        let mut timeline: Timeline = Vec::new();
        // No ball → skipped.
        timeline.push(WorldState {
            t: 0.0,
            ball: None,
            players: vec![
                player(0, Team::Blue, pos(0.0, 2900.0)),
                player(1, Team::Blue, pos(2000.0, 3500.0)),
            ],
        });
        // Player 1 demolished → skipped.
        let mut demo = frame(1.0, 3000.0, 2900.0, 3500.0);
        demo.players[1].demolished = true;
        timeline.push(demo);
        // One real up-field frame for the cover man.
        timeline.push(frame(2.0, 3000.0, 2900.0, 3500.0));

        let result = CommitmentDiscipline::default().extract(&ctx(&timeline), PlayerId(1));
        assert_eq!(result.value, Score::new(0.0));
        assert_eq!(result.evidence.len(), 1);
        assert_eq!(result.evidence[0].t, 2.0);
    }

    // -- boost economy ------------------------------------------------------

    /// Player 0 sits on the ball (1st man), teammate parked far away.
    fn boost_frame(t: f32, boost: u8) -> WorldState {
        WorldState {
            t,
            ball: Some(ball_at(0.0, 0.0)),
            players: vec![
                player_with_boost(0, Team::Blue, pos(0.0, 100.0), boost),
                player(1, Team::Blue, pos(0.0, 3000.0)),
            ],
        }
    }

    #[test]
    fn boost_engaged_with_boost_scores_high() {
        let timeline: Timeline = (0..10).map(|i| boost_frame(i as f32, 60)).collect();
        let result = BoostEconomy::default().extract(&ctx(&timeline), PlayerId(0));

        assert_eq!(result.value, Score::new(100.0), "never empty");
        assert!(result.confidence.get() > 0.0);
        assert!(result.evidence.is_empty());
    }

    #[test]
    fn boost_engaged_empty_scores_low() {
        let timeline: Timeline = (0..10).map(|i| boost_frame(i as f32, 0)).collect();
        let result = BoostEconomy::default().extract(&ctx(&timeline), PlayerId(0));

        assert_eq!(result.value, Score::new(0.0), "always empty");
        assert_eq!(result.evidence.len(), 10);
    }

    #[test]
    fn boost_half_empty_scores_in_between() {
        let mut timeline: Timeline = (0..4).map(|i| boost_frame(i as f32, 80)).collect();
        timeline.extend((4..8).map(|i| boost_frame(i as f32, 0)));
        let result = BoostEconomy::default().extract(&ctx(&timeline), PlayerId(0));

        assert_eq!(result.value, Score::new(50.0));
        assert_eq!(result.evidence.len(), 4);
    }

    #[test]
    fn boost_never_engaged_is_zero_confidence() {
        // Player 1 is always the far man → never the engaging 1st man.
        let timeline: Timeline = (0..10).map(|i| boost_frame(i as f32, 0)).collect();
        let result = BoostEconomy::default().extract(&ctx(&timeline), PlayerId(1));

        assert_eq!(result.confidence, Confidence::new(0.0), "never engaged");
    }

    #[test]
    fn boost_out_of_engage_radius_does_not_apply() {
        // 1st man, but parked well beyond engage radius.
        let timeline: Timeline = (0..10)
            .map(|i| WorldState {
                t: i as f32,
                ball: Some(ball_at(0.0, 0.0)),
                players: vec![
                    player_with_boost(0, Team::Blue, pos(0.0, 3000.0), 0),
                    player(1, Team::Blue, pos(0.0, 5000.0)),
                ],
            })
            .collect();
        let result = BoostEconomy::default().extract(&ctx(&timeline), PlayerId(0));

        assert_eq!(
            result.confidence,
            Confidence::new(0.0),
            "too far to be a decisive moment"
        );
    }

    #[test]
    fn boost_skips_ball_less_and_demolished_frames() {
        let mut timeline: Timeline = Vec::new();
        timeline.push(WorldState {
            t: 0.0,
            ball: None,
            players: vec![player_with_boost(0, Team::Blue, pos(0.0, 100.0), 0)],
        });
        let mut demo = boost_frame(1.0, 0);
        demo.players[0].demolished = true;
        timeline.push(demo);
        timeline.push(boost_frame(2.0, 0));

        let result = BoostEconomy::default().extract(&ctx(&timeline), PlayerId(0));
        assert_eq!(result.value, Score::new(0.0));
        assert_eq!(result.evidence.len(), 1);
        assert_eq!(result.evidence[0].t, 2.0);
    }
}
