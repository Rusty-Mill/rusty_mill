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
use crate::{FieldGeometry, PlayerId, Team};

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
    /// *Implemented.* Punishes the back man caught upfield at the opponent's
    /// commit — the wrong area for the situation × role.
    PositioningFit,
    /// *Implemented.* Punishes failing to recover goalside after an
    /// engagement ends (ball-chasing instead of rotating out).
    RotationSoundness,
    /// *Implemented.* Punishes challenges the opponent clearly wins to —
    /// committing when beaten to the ball.
    ChallengeTiming,
    /// *Implemented.* Punishes the cover man not holding a goalside line
    /// while the teammate's engagement is live.
    ShadowQuality,
    /// *Implemented.* Punishes low-percentage hero shots (long range or wide
    /// angle), from the replay's scoreboard shot events.
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
// Positioning fit (back man, defensive) — the right area for situation × role
// ---------------------------------------------------------------------------

/// The team `player` appears on, from the first frame carrying their facts.
fn team_of(ctx: &MatchContext, player: PlayerId) -> Option<Team> {
    ctx.frames()
        .find_map(|(_, frame)| frame.player(player).map(|f| f.team))
}

/// Penalises the back man caught upfield when the **opponent** commits: for
/// each opposing player's engagement entry at which `player` was the 2nd man,
/// the player is penalised if they were not goalside of the ball at that
/// moment. The situation (an opponent attacking the ball) × the role (you are
/// the cover) demands the goalside area; the 1st man's job at that moment is
/// to pressure, so only 2nd-man entries are opportunities.
#[derive(Debug, Clone, Copy)]
pub struct PositioningFit {
    pub engagement: EngagementConfig,
    /// Opponent episodes at which confidence reaches 1.0.
    pub saturation_episodes: f32,
    /// Cap on collected evidence entries.
    pub max_evidence: usize,
}

impl Default for PositioningFit {
    fn default() -> Self {
        Self {
            engagement: EngagementConfig::default(),
            saturation_episodes: 12.0,
            max_evidence: 25,
        }
    }
}

impl MetricExtractor for PositioningFit {
    fn id(&self) -> DimensionId {
        DimensionId::PositioningFit
    }

    fn extract(&self, ctx: &MatchContext, player: PlayerId) -> DimensionScore {
        let Some(my_team) = team_of(ctx, player) else {
            return episode_score(DimensionId::PositioningFit, 0, &[], 1.0, 0);
        };

        let mut opponents: Vec<PlayerId> = Vec::new();
        for (_, frame) in ctx.frames() {
            for f in frame.facts() {
                if f.team != my_team && !opponents.contains(&f.player) {
                    opponents.push(f.player);
                }
            }
        }

        let mut opportunities = 0usize;
        let mut penalised = Vec::new();
        for opp in opponents {
            for ep in engagements(ctx, opp, &self.engagement) {
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
                if !me.goalside {
                    penalised.push(Evidence {
                        t: ep.t_enter,
                        detail: "caught upfield as the back man at the opponent's commit".into(),
                    });
                }
            }
        }

        episode_score(
            DimensionId::PositioningFit,
            opportunities,
            &penalised,
            self.saturation_episodes,
            self.max_evidence,
        )
    }
}

// ---------------------------------------------------------------------------
// Rotation soundness — recovery to cover after a challenge
// ---------------------------------------------------------------------------

/// Penalises failing to rotate out: after each of the player's own engagement
/// episodes ends, they should recover **goalside of the ball** within
/// `recovery_window_s` (the guide's back-post/out-of-the-way recovery, reduced
/// to its measurable core). Episodes whose window runs past the end of the
/// timeline are not judged — there is no full window to fail in.
#[derive(Debug, Clone, Copy)]
pub struct RotationSoundness {
    pub engagement: EngagementConfig,
    /// Seconds after the episode ends within which the player must register
    /// goalside of the ball.
    pub recovery_window_s: f32,
    /// Episodes at which confidence reaches 1.0.
    pub saturation_episodes: f32,
    /// Cap on collected evidence entries.
    pub max_evidence: usize,
}

impl Default for RotationSoundness {
    fn default() -> Self {
        Self {
            engagement: EngagementConfig::default(),
            recovery_window_s: 4.0,
            saturation_episodes: 12.0,
            max_evidence: 25,
        }
    }
}

impl MetricExtractor for RotationSoundness {
    fn id(&self) -> DimensionId {
        DimensionId::RotationSoundness
    }

    fn extract(&self, ctx: &MatchContext, player: PlayerId) -> DimensionScore {
        let Some(last_t) = ctx
            .len()
            .checked_sub(1)
            .and_then(|i| ctx.frame(i))
            .map(|(s, _)| s.t)
        else {
            return episode_score(DimensionId::RotationSoundness, 0, &[], 1.0, 0);
        };

        let mut opportunities = 0usize;
        let mut penalised = Vec::new();
        for ep in engagements(ctx, player, &self.engagement) {
            let Some((exit_snapshot, _)) = ctx.frame(ep.exit_idx) else {
                continue;
            };
            let deadline = exit_snapshot.t + self.recovery_window_s;
            if last_t < deadline {
                continue; // truncated window — nothing to judge
            }
            opportunities += 1;
            let mut recovered = false;
            let mut idx = ep.exit_idx + 1;
            while let Some((snapshot, frame)) = ctx.frame(idx) {
                if snapshot.t > deadline {
                    break;
                }
                if frame.player(player).is_some_and(|f| f.goalside) {
                    recovered = true;
                    break;
                }
                idx += 1;
            }
            if !recovered {
                penalised.push(Evidence {
                    t: ep.t_enter,
                    detail: "never recovered goalside after the challenge".into(),
                });
            }
        }

        episode_score(
            DimensionId::RotationSoundness,
            opportunities,
            &penalised,
            self.saturation_episodes,
            self.max_evidence,
        )
    }
}

// ---------------------------------------------------------------------------
// Challenge timing — engaging on the right beat
// ---------------------------------------------------------------------------

/// Penalises second-strike commits, judged by the **race outcome** in the
/// touch stream ([`crate::context::MatchContext::touches`]) rather than
/// entry-time kinematics.
///
/// The first cut of this dimension compared time-to-ball estimates at the
/// commit and came out strongly *inverted* on the corpus (ρ = −0.317):
/// deliberately slowing an approach to contain, fake, or hold a challenge is
/// elite technique, and a kinematic "beaten to the ball" test penalises
/// exactly that. So v2.1 re-grounds it in what actually happened:
///
/// - An **opportunity** is a realized challenge — an engagement against a
///   ball the team does not own in which the player *touches* the ball.
///   Shadowing and containment (closing in without contact) are not
///   challenges and are never judged here.
/// - A challenge is **penalised** when an opponent got the first strike:
///   they touched after the player's commit and more than `even_margin_s`
///   before the player's own first touch. A near-simultaneous 50/50 is not a
///   timing fault.
///
/// With no touch data attached the dimension never applies.
#[derive(Debug, Clone, Copy)]
pub struct ChallengeTiming {
    pub engagement: EngagementConfig,
    /// An opponent's first strike within this many seconds of the player's
    /// own first touch counts as an even challenge, not a lost race.
    pub even_margin_s: f32,
    /// Realized challenges at which confidence reaches 1.0.
    pub saturation_episodes: f32,
    /// Cap on collected evidence entries.
    pub max_evidence: usize,
}

impl Default for ChallengeTiming {
    fn default() -> Self {
        Self {
            engagement: EngagementConfig::default(),
            even_margin_s: 0.25,
            saturation_episodes: 12.0,
            max_evidence: 25,
        }
    }
}

impl MetricExtractor for ChallengeTiming {
    fn id(&self) -> DimensionId {
        DimensionId::ChallengeTiming
    }

    fn extract(&self, ctx: &MatchContext, player: PlayerId) -> DimensionScore {
        let Some(my_team) = team_of(ctx, player) else {
            return episode_score(DimensionId::ChallengeTiming, 0, &[], 1.0, 0);
        };
        let opponent_of = |id: PlayerId| {
            ctx.frames()
                .find_map(|(_, f)| f.player(id).map(|facts| facts.team != my_team))
                .unwrap_or(false)
        };

        let mut opportunities = 0usize;
        let mut penalised = Vec::new();

        for ep in engagements(ctx, player, &self.engagement) {
            if ep.possession_at_entry == RelativePossession::Ours {
                continue; // playing your own ball is not a challenge
            }
            let Some((exit_snapshot, _)) = ctx.frame(ep.exit_idx) else {
                continue;
            };
            let window = ep.t_enter..=exit_snapshot.t;

            // My first touch inside the episode — no touch, no challenge.
            let Some(my_touch_t) = ctx
                .touches()
                .iter()
                .find(|touch| touch.player == player && window.contains(&touch.t))
                .map(|touch| touch.t)
            else {
                continue;
            };
            opportunities += 1;

            let first_strike_lost = ctx.touches().iter().any(|touch| {
                window.contains(&touch.t)
                    && touch.t < my_touch_t - self.even_margin_s
                    && opponent_of(touch.player)
            });
            if first_strike_lost {
                penalised.push(Evidence {
                    t: ep.t_enter,
                    detail: "second-strike commit: the opponent won the first touch".into(),
                });
            }
        }

        episode_score(
            DimensionId::ChallengeTiming,
            opportunities,
            &penalised,
            self.saturation_episodes,
            self.max_evidence,
        )
    }
}

// ---------------------------------------------------------------------------
// Shadow quality — the cover man holding a goalside line
// ---------------------------------------------------------------------------

/// Penalises drifting cover: for each of the teammate's engagement episodes at
/// which the player was the 2nd man, the player is penalised if either
///
/// - they were goalside of the ball for less than `min_goalside_share` of the
///   episode (the wrong side of the ball to help), or
/// - their mean distance to the engaging teammate exceeded `max_trail_uu` —
///   the guide's "string theory" spacing (O2-1/O2-2: trail the 1st man by a
///   couple of pad-lengths, "close enough that the imaginary string stays
///   tight… not so far it snaps") broke, leaving no immediate step-in if the
///   challenge is lost.
///
/// `max_trail_uu` is a documented approximation of the guide's pad-length
/// language, not a literal unit conversion — like the engagement radii, it is
/// an uncalibrated starting point. Distinct from [`CommitmentDiscipline`]:
/// that punishes *joining* the challenge (too close), this punishes covering
/// it from the wrong side or too loose a line — a patient but upfield or
/// over-detached 2nd man passes the first and fails this one.
#[derive(Debug, Clone, Copy)]
pub struct ShadowQuality {
    pub engagement: EngagementConfig,
    /// Minimum fraction of the teammate's episode the player must spend
    /// goalside of the ball.
    pub min_goalside_share: f32,
    /// Mean distance (uu) to the engaging teammate beyond which the cover
    /// man's string-theory spacing has snapped.
    pub max_trail_uu: f32,
    /// Teammate episodes at which confidence reaches 1.0.
    pub saturation_episodes: f32,
    /// Cap on collected evidence entries.
    pub max_evidence: usize,
}

impl Default for ShadowQuality {
    fn default() -> Self {
        Self {
            engagement: EngagementConfig::default(),
            min_goalside_share: 0.5,
            max_trail_uu: 4000.0,
            saturation_episodes: 12.0,
            max_evidence: 25,
        }
    }
}

impl MetricExtractor for ShadowQuality {
    fn id(&self) -> DimensionId {
        DimensionId::ShadowQuality
    }

    fn extract(&self, ctx: &MatchContext, player: PlayerId) -> DimensionScore {
        let mut opportunities = 0usize;
        let mut penalised = Vec::new();

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
                let Some((_, entry_frame)) = ctx.frame(ep.enter_idx) else {
                    continue;
                };
                let Some(me) = entry_frame.player(player) else {
                    continue;
                };
                if me.role != Role::SecondMan {
                    continue;
                }
                let (mut present, mut goalside, mut trail_sum) = (0usize, 0usize, 0.0_f32);
                for idx in ep.enter_idx..=ep.exit_idx {
                    let Some((snapshot, frame)) = ctx.frame(idx) else {
                        continue;
                    };
                    let Some(f) = frame.player(player) else {
                        continue;
                    };
                    present += 1;
                    goalside += usize::from(f.goalside);
                    let subject = snapshot.players.iter().find(|p| p.player == player);
                    let teammate = snapshot.players.iter().find(|p| p.player == mate);
                    if let (Some(subject), Some(teammate)) = (subject, teammate) {
                        trail_sum += subject.pose.position.distance(teammate.pose.position);
                    }
                }
                if present == 0 {
                    continue; // demolished for the whole episode — no line to hold
                }
                opportunities += 1;
                let goalside_share = goalside as f32 / present as f32;
                let mean_trail = trail_sum / present as f32;
                if goalside_share < self.min_goalside_share {
                    penalised.push(Evidence {
                        t: ep.t_enter,
                        detail: "covered the challenge from upfield of the ball".into(),
                    });
                } else if mean_trail > self.max_trail_uu {
                    penalised.push(Evidence {
                        t: ep.t_enter,
                        detail: "string theory spacing snapped: too far back to step in".into(),
                    });
                }
            }
        }

        episode_score(
            DimensionId::ShadowQuality,
            opportunities,
            &penalised,
            self.saturation_episodes,
            self.max_evidence,
        )
    }
}

// ---------------------------------------------------------------------------
// Shot selection — high-percentage over hero gambles
// ---------------------------------------------------------------------------

/// Penalises low-percentage hero shots: each of the player's scoreboard shot
/// events ([`crate::context::MatchContext::shots`]) is judged by the ball's
/// position at the shot — farther than `max_range_uu` from the goal mouth, or
/// wider than `max_off_angle_deg` off the goal axis, is a gamble. Both
/// thresholds are uncalibrated starting points; the design doc rates this
/// dimension Low confidence, and the default weight reflects that. With no
/// shot events attached the dimension never applies.
#[derive(Debug, Clone, Copy)]
pub struct ShotSelection {
    pub field: FieldGeometry,
    /// Shots from farther than this (uu, ball to goal-mouth center in the
    /// ground plane) count as low-percentage.
    pub max_range_uu: f32,
    /// Shots from wider than this off the straight-at-goal axis (degrees)
    /// count as low-percentage.
    pub max_off_angle_deg: f32,
    /// Shots at which confidence reaches 1.0 — lower than the episode
    /// dimensions because shots are rare.
    pub saturation_shots: f32,
    /// Cap on collected evidence entries.
    pub max_evidence: usize,
}

impl Default for ShotSelection {
    fn default() -> Self {
        Self {
            field: FieldGeometry::default(),
            max_range_uu: 4000.0,
            max_off_angle_deg: 60.0,
            saturation_shots: 6.0,
            max_evidence: 25,
        }
    }
}

impl MetricExtractor for ShotSelection {
    fn id(&self) -> DimensionId {
        DimensionId::ShotSelection
    }

    fn extract(&self, ctx: &MatchContext, player: PlayerId) -> DimensionScore {
        let Some(my_team) = team_of(ctx, player) else {
            return episode_score(DimensionId::ShotSelection, 0, &[], 1.0, 0);
        };
        let opponent_goal_y = -self.field.own_goal_y(my_team);

        let mut opportunities = 0usize;
        let mut penalised = Vec::new();
        // Shots and frames are both time-sorted; walk them with one cursor.
        let mut idx = 0usize;
        for shot in ctx.shots().iter().filter(|s| s.player == player) {
            while idx + 1 < ctx.len() && ctx.frame(idx).is_some_and(|(s, _)| s.t < shot.t) {
                idx += 1;
            }
            let Some(ball) = ctx.frame(idx).and_then(|(s, _)| s.ball.as_ref()) else {
                continue;
            };
            opportunities += 1;
            let dx = ball.pose.position.x;
            let dy = opponent_goal_y - ball.pose.position.y;
            let range = (dx * dx + dy * dy).sqrt();
            let off_angle = dx.abs().atan2(dy.abs()).to_degrees();
            if range > self.max_range_uu || off_angle > self.max_off_angle_deg {
                penalised.push(Evidence {
                    t: shot.t,
                    detail: format!(
                        "low-percentage shot: {range:.0}uu out, {off_angle:.0}° off the goal axis"
                    ),
                });
            }
        }

        episode_score(
            DimensionId::ShotSelection,
            opportunities,
            &penalised,
            self.saturation_shots,
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

    // -- positioning fit -------------------------------------------------------

    /// Orange 2 (their 1st man) at `(0, opp_y)` attacking the ball at (0,3000);
    /// blue 1 parked at (0,2000) is blue's 1st man; blue 0 covers at
    /// `(2000, cover_y)`; orange 3 parked deep.
    fn opponent_attacks(t: f32, opp_y: f32, cover_y: f32) -> WorldState {
        WorldState {
            t,
            ball: Some(ball_at(0.0, 3000.0)),
            players: vec![
                player(0, Team::Blue, pos(2000.0, cover_y)),
                player(1, Team::Blue, pos(0.0, 2000.0)),
                player(2, Team::Orange, pos(0.0, opp_y)),
                player(3, Team::Orange, pos(3000.0, 4800.0)),
            ],
        }
    }

    fn opponent_attack_timeline(cover_y: f32) -> Timeline {
        let ys = [
            5500.0, 4600.0, 4000.0, 3600.0, 3400.0, 3600.0, 4000.0, 4600.0,
        ];
        ys.iter()
            .enumerate()
            .map(|(i, y)| opponent_attacks(i as f32 * 0.1, *y, cover_y))
            .collect()
    }

    #[test]
    fn goalside_back_man_at_opponent_commit_scores_high() {
        let timeline = opponent_attack_timeline(-2000.0);
        let result = PositioningFit::default().extract(&ctx(&timeline), PlayerId(0));
        assert_eq!(result.value, Score::new(100.0));
        assert!(
            result.confidence.get() > 0.0,
            "the opponent's episode counts"
        );
    }

    #[test]
    fn upfield_back_man_at_opponent_commit_is_penalised() {
        let timeline = opponent_attack_timeline(4500.0); // upfield of the ball
        let result = PositioningFit::default().extract(&ctx(&timeline), PlayerId(0));
        assert_eq!(result.value, Score::new(0.0));
        assert_eq!(result.evidence.len(), 1);
    }

    #[test]
    fn first_man_gets_no_positioning_opportunities() {
        // Blue 1 is blue's 1st man during the opponent's attack — their job is
        // pressure, so the dimension never applies to them here.
        let timeline = opponent_attack_timeline(-2000.0);
        let result = PositioningFit::default().extract(&ctx(&timeline), PlayerId(1));
        assert_eq!(result.confidence, Confidence::new(0.0));
    }

    // -- rotation soundness ----------------------------------------------------

    /// Player 0 challenges a ball in their own defensive zone (0,-3000), then
    /// ends up at the given post-exit ys; teammate parked far upfield.
    fn defense_timeline(post_exit_ys: [f32; 3]) -> Timeline {
        let mut ys = vec![1000.0, -500.0, -1800.0, -2400.0, -2600.0, -2600.0, -2400.0];
        ys.extend(post_exit_ys);
        ys.iter()
            .enumerate()
            .map(|(i, y)| WorldState {
                t: i as f32 * 0.1,
                ball: Some(ball_at(0.0, -3000.0)),
                players: vec![
                    player(0, Team::Blue, pos(0.0, *y)),
                    player(1, Team::Blue, pos(2000.0, 4000.0)),
                ],
            })
            .collect()
    }

    fn quick_recovery() -> RotationSoundness {
        RotationSoundness {
            recovery_window_s: 0.2,
            ..Default::default()
        }
    }

    #[test]
    fn recovering_goalside_after_the_challenge_scores_high() {
        // Exits the challenge toward their own goal (goalside of the ball).
        let timeline = defense_timeline([-4600.0, -4800.0, -5000.0]);
        let result = quick_recovery().extract(&ctx(&timeline), PlayerId(0));
        assert_eq!(result.value, Score::new(100.0));
        assert!(result.confidence.get() > 0.0);
    }

    #[test]
    fn drifting_upfield_after_the_challenge_is_penalised() {
        // Exits the challenge upfield of the ball and stays there.
        let timeline = defense_timeline([-1200.0, -1000.0, -800.0]);
        let result = quick_recovery().extract(&ctx(&timeline), PlayerId(0));
        assert_eq!(result.value, Score::new(0.0));
        assert_eq!(result.evidence.len(), 1);
    }

    #[test]
    fn truncated_recovery_window_is_not_judged() {
        // Default 4s window runs past this ~1s timeline: no opportunity.
        let timeline = defense_timeline([-1200.0, -1000.0, -800.0]);
        let result = RotationSoundness::default().extract(&ctx(&timeline), PlayerId(0));
        assert_eq!(result.confidence, Confidence::new(0.0));
    }

    // -- challenge timing --------------------------------------------------------

    use crate::context::TouchEvent;

    /// Player 0's approach arc (entry at t=0.3, episode through t=0.7) with an
    /// opponent on the field, judged under the given touch stream.
    fn challenge_result(touches: Vec<TouchEvent>) -> DimensionScore {
        let ys = [
            -1000.0, 500.0, 1800.0, 2400.0, 2600.0, 2700.0, 2600.0, 2400.0, 800.0, -1000.0,
        ];
        let timeline: Timeline = ys
            .iter()
            .enumerate()
            .map(|(i, y)| WorldState {
                t: i as f32 * 0.1,
                ball: Some(ball_at(0.0, 3000.0)),
                players: vec![
                    player(0, Team::Blue, pos(0.0, *y)),
                    player(1, Team::Blue, pos(2000.0, -2000.0)),
                    player(2, Team::Orange, pos(3000.0, 4800.0)),
                ],
            })
            .collect();
        let c = MatchContext::derive(&timeline, ContextConfig::default()).with_touches(touches);
        ChallengeTiming::default().extract(&c, PlayerId(0))
    }

    fn touch(t: f32, id: u32) -> TouchEvent {
        TouchEvent {
            t,
            player: PlayerId(id),
        }
    }

    #[test]
    fn winning_the_first_touch_scores_high() {
        let result = challenge_result(vec![touch(0.5, 0)]);
        assert_eq!(result.value, Score::new(100.0));
        assert!(result.confidence.get() > 0.0, "a realized challenge counts");
    }

    #[test]
    fn losing_the_first_strike_is_penalised() {
        // The opponent touches at 0.4, well before my 0.7 contact.
        let result = challenge_result(vec![touch(0.4, 2), touch(0.7, 0)]);
        assert_eq!(result.value, Score::new(0.0));
        assert_eq!(result.evidence.len(), 1);
    }

    #[test]
    fn an_even_fifty_fifty_is_not_a_timing_fault() {
        // The opponent's touch lands inside the even margin of mine.
        let result = challenge_result(vec![touch(0.55, 2), touch(0.7, 0)]);
        assert_eq!(result.value, Score::new(100.0));
    }

    #[test]
    fn containment_without_contact_is_not_a_challenge() {
        // I close in but never touch: shadowing, not a challenge — and with no
        // touch of mine, the opponent's touches don't create opportunities.
        let result = challenge_result(vec![touch(0.4, 2)]);
        assert_eq!(result.confidence, Confidence::new(0.0));
    }

    #[test]
    fn no_touch_data_means_the_dimension_never_applies() {
        let result = challenge_result(Vec::new());
        assert_eq!(result.confidence, Confidence::new(0.0));
    }

    // -- shadow quality ----------------------------------------------------------

    #[test]
    fn goalside_cover_holds_the_line() {
        // Goalside of the ball and within the string-theory trail band
        // (~3256uu from the engaging teammate at (0,2600)).
        let timeline: Timeline = (0..10)
            .map(|i| teammate_engaged(i as f32 * 0.1, 600.0, -600.0))
            .collect();
        let result = ShadowQuality::default().extract(&ctx(&timeline), PlayerId(0));
        assert_eq!(result.value, Score::new(100.0));
        assert!(result.confidence.get() > 0.0);
    }

    #[test]
    fn upfield_cover_fails_the_line_but_not_commitment() {
        // Patient (never near the ball) but covering from upfield of it: passes
        // commitment discipline, fails shadow quality — the dimensions are
        // measuring different faults.
        let timeline: Timeline = (0..10)
            .map(|i| teammate_engaged(i as f32 * 0.1, 2000.0, 4500.0))
            .collect();
        let shadow = ShadowQuality::default().extract(&ctx(&timeline), PlayerId(0));
        assert_eq!(shadow.value, Score::new(0.0));
        let commitment = CommitmentDiscipline::default().extract(&ctx(&timeline), PlayerId(0));
        assert_eq!(commitment.value, Score::new(100.0));
    }

    #[test]
    fn hanging_back_too_far_breaks_the_string() {
        // Goalside (so the first condition passes clean) but ~8621uu from the
        // engaging teammate — the O2-1/O2-2 trail band snapped.
        let timeline: Timeline = (0..10)
            .map(|i| teammate_engaged(i as f32 * 0.1, 600.0, -6000.0))
            .collect();
        let result = ShadowQuality::default().extract(&ctx(&timeline), PlayerId(0));
        assert_eq!(result.value, Score::new(0.0));
        assert_eq!(result.evidence.len(), 1);
        assert!(result.evidence[0].detail.contains("string theory"));
    }

    // -- shot selection ------------------------------------------------------------

    use crate::context::ShotEvent;

    fn shot_result(ball_x: f32, ball_y: f32) -> DimensionScore {
        let timeline: Timeline = (0..5)
            .map(|i| WorldState {
                t: i as f32,
                ball: Some(ball_at(ball_x, ball_y)),
                players: vec![player(0, Team::Blue, pos(0.0, 0.0))],
            })
            .collect();
        let c =
            MatchContext::derive(&timeline, ContextConfig::default()).with_shots(vec![ShotEvent {
                t: 2.0,
                player: PlayerId(0),
            }]);
        ShotSelection::default().extract(&c, PlayerId(0))
    }

    #[test]
    fn close_central_shot_scores_high() {
        // Blue shoots at the orange goal (+y): ball at (0,4000) is 1120uu out,
        // dead central.
        let result = shot_result(0.0, 4000.0);
        assert_eq!(result.value, Score::new(100.0));
        assert!(result.confidence.get() > 0.0);
    }

    #[test]
    fn long_range_hero_shot_is_penalised() {
        // From the wrong half of the pitch: 7120uu out.
        let result = shot_result(0.0, -2000.0);
        assert_eq!(result.value, Score::new(0.0));
        assert_eq!(result.evidence.len(), 1);
    }

    #[test]
    fn wide_angle_shot_is_penalised() {
        // Near the corner: inside range but ~85° off the goal axis.
        let result = shot_result(3800.0, 4800.0);
        assert_eq!(result.value, Score::new(0.0));
    }

    #[test]
    fn no_shots_means_the_dimension_never_applies() {
        let timeline = approach_timeline(-2000.0, 60);
        let result = ShotSelection::default().extract(&ctx(&timeline), PlayerId(0));
        assert_eq!(result.confidence, Confidence::new(0.0));
    }
}
