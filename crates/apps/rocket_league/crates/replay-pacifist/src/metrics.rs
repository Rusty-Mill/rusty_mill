//! Scoring dimensions.
//!
//! Each Pacifist Score dimension is an independent [`MetricExtractor`]: it reads
//! the [`MatchContext`] and emits a 0..=100 [`Score`] for one player, with a
//! [`Confidence`] reflecting how much evidence it had. The aggregate score
//! (built in [`crate::scoring`]) confidence-weights the dimensions so weak
//! signals don't swing the headline number.
//!
//! **v1 (`pcfg-v1`): per-opportunity event rates.** The first corpus validation
//! (`docs/pacifist-score-validation.md`) showed the v0 per-frame-fraction
//! versions of these dimensions don't discriminate — their penalised conditions
//! are near-invariant by frame count across the whole ladder. The v1 extractors
//! instead count discrete **engagement episodes** (a player, as 1st man,
//! entering challenge range of the ball — the "commit" moment the guide's rules
//! are written about) and penalise episodes by their entry-time facts:
//!
//! - **over-extension** — criteria F17/T-1: an engagement begun against
//!   opposition possession with no teammate goalside behind (a last-man dive).
//! - **commitment discipline** — the double-commit (FM-1's counting model, the
//!   2nd-man side): the cover man joining the ball while their teammate's
//!   engagement is still live.
//! - **boost economy** — criteria F4/FM-2: an engagement begun with an
//!   effectively empty tank ("do not be aggressive with 0 boost").
//!
//! Each is oriented so a higher score means *more* Pacifist (more discipline,
//! less over-commit). Confidence saturates with the number of *opportunities*
//! (episodes), not frames.

use crate::context::{MatchContext, RelativePossession, Role};
use crate::PlayerId;

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
    /// *Implemented.* Punishes last-man dives: engaging against opposition
    /// possession with no cover behind (F17).
    OverExtension,
    /// *Implemented.* Punishes double-commits: the 2nd man joining the ball
    /// while the teammate's engagement is live (F9/FM-1).
    CommitmentDiscipline,
    /// *Implemented.* Punishes engaging with an empty tank (F4/FM-2).
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
// Engagement episodes — the shared per-opportunity primitive
// ---------------------------------------------------------------------------

/// Geometry for episode extraction. Enter/exit radii are hysteretic so one
/// challenge doesn't fragment into several episodes at the boundary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EngagementConfig {
    /// Distance (uu) at which a 1st man crossing toward the ball begins an
    /// engagement — challenge range.
    pub enter_radius_uu: f32,
    /// Distance (uu) beyond which a live engagement ends.
    pub exit_radius_uu: f32,
}

impl Default for EngagementConfig {
    fn default() -> Self {
        Self {
            enter_radius_uu: 900.0,
            exit_radius_uu: 1500.0,
        }
    }
}

/// One engagement episode: `player` was the 1st man and closed inside
/// `enter_radius_uu` of the ball, holding the ball area until the exit radius.
/// Entry-time facts are what the Pacifist rules judge — the decision was made
/// at the commit, not during the scramble that follows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Engagement {
    /// Frame index of the entry (into [`MatchContext::frame`]).
    pub enter_idx: usize,
    /// Frame index of the last frame of the episode.
    pub exit_idx: usize,
    /// Time of the entry frame (seconds).
    pub t_enter: f32,
    /// Boost carried at entry, `0..=100`.
    pub boost_at_entry: u8,
    /// True when some teammate was goalside of the ball at entry.
    pub covered_at_entry: bool,
    /// Possession at entry, from the engaging player's point of view.
    pub possession_at_entry: RelativePossession,
}

/// Extract `player`'s engagement episodes from the context.
///
/// State machine over frames: an episode opens on a frame where the player is
/// the 1st man within `enter_radius_uu` of the ball (having been outside — or
/// absent — before), and closes when they leave `exit_radius_uu`, lose their
/// facts (demolished / ball-less frame), or the timeline ends. Role flapping
/// *inside* a live episode does not close it; the commit already happened.
pub fn engagements(
    ctx: &MatchContext,
    player: PlayerId,
    cfg: &EngagementConfig,
) -> Vec<Engagement> {
    let mut out = Vec::new();
    let mut live: Option<Engagement> = None;

    for (idx, (snapshot, frame)) in ctx.frames().enumerate() {
        let me = frame.player(player);
        match (&mut live, me) {
            (Some(ep), Some(f)) if f.dist_to_ball <= cfg.exit_radius_uu => {
                ep.exit_idx = idx; // still engaged
            }
            (Some(_), _) => {
                // Left the ball area, was demolished, or the ball vanished.
                out.push(live.take().expect("live episode"));
            }
            (None, Some(f))
                if f.role == Role::FirstMan && f.dist_to_ball <= cfg.enter_radius_uu =>
            {
                let covered = frame.teammates_of(player).any(|t| t.goalside);
                live = Some(Engagement {
                    enter_idx: idx,
                    exit_idx: idx,
                    t_enter: snapshot.t,
                    boost_at_entry: f.boost,
                    covered_at_entry: covered,
                    possession_at_entry: frame.possession().relative_to(f.team),
                });
            }
            _ => {}
        }
    }
    if let Some(ep) = live {
        out.push(ep);
    }
    out
}

/// Roll penalised-vs-total episode counts into a [`DimensionScore`]:
/// `value = 100·(1 − penalised/total)`, confidence saturating at
/// `saturation_episodes`. No episodes → value 100 at confidence 0 (the
/// dimension didn't apply, so it drops out of the aggregate).
fn episode_score(
    dimension: DimensionId,
    total: usize,
    penalised: &[Evidence],
    saturation_episodes: f32,
    max_evidence: usize,
) -> DimensionScore {
    if total == 0 {
        return DimensionScore {
            dimension,
            value: Score::new(100.0),
            confidence: Confidence::new(0.0),
            evidence: Vec::new(),
        };
    }
    let fraction = penalised.len() as f32 / total as f32;
    let mut evidence = penalised.to_vec();
    evidence.truncate(max_evidence);
    DimensionScore {
        dimension,
        value: Score::new(100.0 * (1.0 - fraction)),
        confidence: Confidence::new(total as f32 / saturation_episodes),
        evidence,
    }
}

// ---------------------------------------------------------------------------
// Over-extension (1st man) — F17 / T-1
// ---------------------------------------------------------------------------

/// Penalises last-man dives: engagement episodes begun **against opposition or
/// contested possession** (challenging a ball your own team controls is just
/// playing it — T-1's polarity) with **no teammate goalside** at the commit
/// (F17: "do not dive in as last man").
#[derive(Debug, Clone, Copy)]
pub struct OverExtension {
    pub engagement: EngagementConfig,
    /// Episodes at which confidence reaches 1.0.
    pub saturation_episodes: f32,
    /// Cap on collected evidence entries.
    pub max_evidence: usize,
}

impl Default for OverExtension {
    fn default() -> Self {
        Self {
            engagement: EngagementConfig::default(),
            saturation_episodes: 12.0,
            max_evidence: 25,
        }
    }
}

impl MetricExtractor for OverExtension {
    fn id(&self) -> DimensionId {
        DimensionId::OverExtension
    }

    fn extract(&self, ctx: &MatchContext, player: PlayerId) -> DimensionScore {
        let episodes = engagements(ctx, player, &self.engagement);
        let opportunities: Vec<&Engagement> = episodes
            .iter()
            .filter(|e| e.possession_at_entry != RelativePossession::Ours)
            .collect();
        let penalised: Vec<Evidence> = opportunities
            .iter()
            .filter(|e| !e.covered_at_entry)
            .map(|e| Evidence {
                t: e.t_enter,
                detail: "committed as last man: no teammate goalside at the challenge".into(),
            })
            .collect();
        episode_score(
            DimensionId::OverExtension,
            opportunities.len(),
            &penalised,
            self.saturation_episodes,
            self.max_evidence,
        )
    }
}

// ---------------------------------------------------------------------------
// Commitment discipline (2nd man) — F9 / FM-1 double-commit counting
// ---------------------------------------------------------------------------

/// Penalises double-commits from the cover man's side: for each of the
/// **teammate's** engagement episodes, the player (the 2nd man while that
/// episode is live) is penalised if they also close inside `double_radius_uu`
/// of the ball before the episode ends — both cars on the ball, net open. The
/// opportunity count is the teammate's episodes, so a patient 2nd man scores
/// 100 across many chances rather than by default.
///
/// Semantics are 2v2-tuned: the 1st/2nd-man split assumes two players per side.
#[derive(Debug, Clone, Copy)]
pub struct CommitmentDiscipline {
    pub engagement: EngagementConfig,
    /// The 2nd man closing inside this ball radius (uu) during the teammate's
    /// live episode counts as a double-commit.
    pub double_radius_uu: f32,
    /// Teammate episodes at which confidence reaches 1.0.
    pub saturation_episodes: f32,
    /// Cap on collected evidence entries.
    pub max_evidence: usize,
}

impl Default for CommitmentDiscipline {
    fn default() -> Self {
        Self {
            engagement: EngagementConfig::default(),
            double_radius_uu: 1100.0,
            saturation_episodes: 12.0,
            max_evidence: 25,
        }
    }
}

impl MetricExtractor for CommitmentDiscipline {
    fn id(&self) -> DimensionId {
        DimensionId::CommitmentDiscipline
    }

    fn extract(&self, ctx: &MatchContext, player: PlayerId) -> DimensionScore {
        let mut opportunities = 0usize;
        let mut penalised = Vec::new();

        // The teammates whose engagements we cover (2v2 → one, but stay general).
        let mut teammates: Vec<PlayerId> = Vec::new();
        for (_, frame) in ctx.frames() {
            for f in frame.teammates_of(player) {
                if !teammates.contains(&f.player) {
                    teammates.push(f.player);
                }
            }
        }

        for mate in teammates {
            for ep in engagements(ctx, mate, &self.engagement) {
                // Only episodes where the player was actually the cover man at
                // the commit (alive, on the field, and not the engaging one).
                let Some((_, entry_frame)) = ctx.frame(ep.enter_idx) else {
                    continue;
                };
                let Some(me) = entry_frame.player(player) else {
                    continue;
                };
                if me.role != Role::SecondMan {
                    continue;
                }
                opportunities += 1;
                let joined = (ep.enter_idx..=ep.exit_idx).any(|idx| {
                    ctx.frame(idx)
                        .and_then(|(_, f)| f.player(player).map(|m| m.dist_to_ball))
                        .is_some_and(|d| d <= self.double_radius_uu)
                });
                if joined {
                    penalised.push(Evidence {
                        t: ep.t_enter,
                        detail: "double-commit: joined the ball during the teammate's challenge"
                            .into(),
                    });
                }
            }
        }

        episode_score(
            DimensionId::CommitmentDiscipline,
            opportunities,
            &penalised,
            self.saturation_episodes,
            self.max_evidence,
        )
    }
}

// ---------------------------------------------------------------------------
// Boost economy — F4 / FM-2
// ---------------------------------------------------------------------------

/// Penalises engaging with an effectively empty tank: of the player's
/// engagement episodes, those entered with boost at or below `empty_boost`
/// (F4: "do not be aggressive with 0 boost"; FM-2 names the 0-boost corner
/// flip as the guide's canonical Major fault).
#[derive(Debug, Clone, Copy)]
pub struct BoostEconomy {
    pub engagement: EngagementConfig,
    /// Boost level (`0..=100`) at or below which an engagement counts as
    /// entered empty. The guide says zero; a small allowance absorbs the
    /// byte→percent rounding and the last sliver of a burned tank.
    pub empty_boost: u8,
    /// Episodes at which confidence reaches 1.0.
    pub saturation_episodes: f32,
    /// Cap on collected evidence entries.
    pub max_evidence: usize,
}

impl Default for BoostEconomy {
    fn default() -> Self {
        Self {
            engagement: EngagementConfig::default(),
            empty_boost: 5,
            saturation_episodes: 12.0,
            max_evidence: 25,
        }
    }
}

impl MetricExtractor for BoostEconomy {
    fn id(&self) -> DimensionId {
        DimensionId::BoostEconomy
    }

    fn extract(&self, ctx: &MatchContext, player: PlayerId) -> DimensionScore {
        let episodes = engagements(ctx, player, &self.engagement);
        let penalised: Vec<Evidence> = episodes
            .iter()
            .filter(|e| e.boost_at_entry <= self.empty_boost)
            .map(|e| Evidence {
                t: e.t_enter,
                detail: "engaged the ball with an empty tank".into(),
            })
            .collect();
        episode_score(
            DimensionId::BoostEconomy,
            episodes.len(),
            &penalised,
            self.saturation_episodes,
            self.max_evidence,
        )
    }
}

// ---------------------------------------------------------------------------
// Tests — synthetic timelines built by hand.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{ContextConfig, MatchContext, PossessionSpan};
    use crate::{BallState, PlayerState, Pose, Quat, Team, Timeline, Vec3, WorldState};

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

    /// Ball fixed at y=3000; player 0 (blue) at `challenger_y`, teammate parked
    /// at `(2000, teammate_y)`.
    fn frame(t: f32, challenger_y: f32, teammate_y: f32, challenger_boost: u8) -> WorldState {
        WorldState {
            t,
            ball: Some(ball_at(0.0, 3000.0)),
            players: vec![
                player_with_boost(0, Team::Blue, pos(0.0, challenger_y), challenger_boost),
                player(1, Team::Blue, pos(2000.0, teammate_y)),
            ],
        }
    }

    /// An approach → challenge → retreat arc for player 0: far, closing, inside
    /// challenge range for a few frames, then away again. One episode.
    fn approach_timeline(teammate_y: f32, boost: u8) -> Timeline {
        let ys = [
            -1000.0, 500.0, 1800.0, 2400.0, 2600.0, 2700.0, 2600.0, 2400.0, 800.0, -1000.0,
        ];
        ys.iter()
            .enumerate()
            .map(|(i, y)| frame(i as f32 * 0.1, *y, teammate_y, boost))
            .collect()
    }

    fn ctx(timeline: &Timeline) -> MatchContext<'_> {
        MatchContext::derive(timeline, ContextConfig::default())
    }

    // -- engagement extraction ----------------------------------------------

    #[test]
    fn one_approach_is_one_episode_with_entry_facts() {
        let timeline = approach_timeline(-2000.0, 60);
        let eps = engagements(&ctx(&timeline), PlayerId(0), &EngagementConfig::default());

        assert_eq!(eps.len(), 1, "hysteresis holds one challenge together");
        let ep = &eps[0];
        // Entry happens at y=2400 (600uu from the ball), frame index 3.
        assert_eq!(ep.enter_idx, 3);
        assert!(ep.exit_idx > ep.enter_idx, "episode spans the close frames");
        assert_eq!(ep.boost_at_entry, 60);
        assert!(
            ep.covered_at_entry,
            "teammate at -2000 is goalside of the ball at 3000"
        );
    }

    #[test]
    fn far_player_has_no_episodes() {
        let timeline = approach_timeline(-2000.0, 60);
        let eps = engagements(&ctx(&timeline), PlayerId(1), &EngagementConfig::default());
        assert!(eps.is_empty(), "the parked cover man never engages");
    }

    #[test]
    fn two_separated_approaches_are_two_episodes() {
        let mut timeline = approach_timeline(-2000.0, 60);
        let second: Timeline = approach_timeline(-2000.0, 60)
            .into_iter()
            .map(|mut w| {
                w.t += 10.0;
                w
            })
            .collect();
        timeline.extend(second);
        let eps = engagements(&ctx(&timeline), PlayerId(0), &EngagementConfig::default());
        assert_eq!(eps.len(), 2);
    }

    // -- over-extension ------------------------------------------------------

    #[test]
    fn covered_challenge_scores_high() {
        let timeline = approach_timeline(-2000.0, 60); // teammate goalside
        let result = OverExtension::default().extract(&ctx(&timeline), PlayerId(0));
        assert_eq!(result.value, Score::new(100.0));
        assert!(result.confidence.get() > 0.0);
        assert!(result.evidence.is_empty());
    }

    #[test]
    fn last_man_dive_scores_low() {
        let timeline = approach_timeline(4000.0, 60); // teammate upfield of the ball
        let result = OverExtension::default().extract(&ctx(&timeline), PlayerId(0));
        assert_eq!(
            result.value,
            Score::new(0.0),
            "the guide's named Major shape"
        );
        assert_eq!(result.evidence.len(), 1);
    }

    #[test]
    fn challenges_on_own_possession_do_not_count() {
        // Same last-man dive geometry, but a possession span says blue controls
        // the ball throughout — playing your own ball is not an over-extension.
        let timeline = approach_timeline(4000.0, 60);
        let spans = [PossessionSpan {
            team: Team::Blue,
            start: 0.0,
            end: 10.0,
        }];
        let c = MatchContext::derive_with_possession(&timeline, ContextConfig::default(), &spans);
        let result = OverExtension::default().extract(&c, PlayerId(0));
        assert_eq!(
            result.confidence,
            Confidence::new(0.0),
            "no opportunities against opposition possession"
        );
    }

    // -- commitment discipline ------------------------------------------------

    /// Teammate (player 1) sits engaged on the ball; player 0 covers at
    /// `(cover_x, cover_y)`.
    fn teammate_engaged(t: f32, cover_x: f32, cover_y: f32) -> WorldState {
        WorldState {
            t,
            ball: Some(ball_at(0.0, 3000.0)),
            players: vec![
                player(0, Team::Blue, pos(cover_x, cover_y)),
                player(1, Team::Blue, pos(0.0, 2600.0)),
            ],
        }
    }

    #[test]
    fn patient_cover_man_scores_high() {
        let timeline: Timeline = (0..10)
            .map(|i| teammate_engaged(i as f32 * 0.1, 2000.0, -2000.0))
            .collect();
        let result = CommitmentDiscipline::default().extract(&ctx(&timeline), PlayerId(0));
        assert_eq!(
            result.value,
            Score::new(100.0),
            "held position all challenge"
        );
        assert!(
            result.confidence.get() > 0.0,
            "the teammate's episode is the opportunity"
        );
    }

    #[test]
    fn joining_the_challenge_is_a_double_commit() {
        // Cover man starts patient, then drives onto the ball mid-episode.
        let mut timeline: Timeline = (0..4)
            .map(|i| teammate_engaged(i as f32 * 0.1, 2000.0, -2000.0))
            .collect();
        timeline.extend((4..10).map(|i| teammate_engaged(i as f32 * 0.1, 300.0, 2900.0)));
        let result = CommitmentDiscipline::default().extract(&ctx(&timeline), PlayerId(0));
        assert_eq!(result.value, Score::new(0.0), "both cars on the ball");
        assert_eq!(result.evidence.len(), 1);
    }

    #[test]
    fn engaging_player_gets_no_commitment_opportunities() {
        let timeline: Timeline = (0..10)
            .map(|i| teammate_engaged(i as f32 * 0.1, 2000.0, -2000.0))
            .collect();
        // Player 1 is the engaged 1st man — the dimension judges the cover man.
        let result = CommitmentDiscipline::default().extract(&ctx(&timeline), PlayerId(1));
        assert_eq!(result.confidence, Confidence::new(0.0));
    }

    // -- boost economy ---------------------------------------------------------

    #[test]
    fn engaging_with_boost_scores_high() {
        let timeline = approach_timeline(-2000.0, 60);
        let result = BoostEconomy::default().extract(&ctx(&timeline), PlayerId(0));
        assert_eq!(result.value, Score::new(100.0));
        assert!(result.confidence.get() > 0.0);
    }

    #[test]
    fn engaging_empty_scores_low() {
        let timeline = approach_timeline(-2000.0, 0);
        let result = BoostEconomy::default().extract(&ctx(&timeline), PlayerId(0));
        assert_eq!(result.value, Score::new(0.0), "the guide's F4");
        assert_eq!(result.evidence.len(), 1);
    }

    #[test]
    fn never_engaging_is_zero_confidence() {
        let timeline = approach_timeline(-2000.0, 0);
        let result = BoostEconomy::default().extract(&ctx(&timeline), PlayerId(1));
        assert_eq!(result.confidence, Confidence::new(0.0));
    }
}
