//! The enriched per-frame context: facts derived once and shared by every
//! extractor.
//!
//! Extractors used to re-derive "who is the 1st man", "is this player goalside",
//! and so on inline from positions. That logic now lives here, computed once per
//! frame into a [`MatchContext`], so the extractors become thin readers over
//! pre-derived [`PlayerFacts`]. The design intent was always for extractors to
//! read this context rather than the raw [`Timeline`]; this is that seam.
//!
//! Two derived facts have settled definitions; one is explicitly provisional:
//!
//! - **Role** (1st/2nd man) — per team, per frame, by a configurable [`RoleRule`].
//! - **Goalside / distance to ball** — pure geometry against [`FieldGeometry`].
//! - **Possession** — a *positional proxy* (who is in clear control near the
//!   ball). It is deliberately weak: a faithful possession signal wants
//!   last-touch detection, which needs replay-format work that can't be verified
//!   here. Treat [`Possession`] as a v1 placeholder, tunable via [`ContextConfig`]
//!   and replaceable once touch decode lands.

use crate::{BallState, FieldGeometry, PlayerId, PlayerState, Team, Timeline, WorldState};

// ---------------------------------------------------------------------------
// Derived facts
// ---------------------------------------------------------------------------

/// A player's role at a single instant. Derived, per-moment, never fixed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Engaging / applying pressure / nearest the play.
    FirstMan,
    /// Cover / shadow / net.
    SecondMan,
}

/// How 1st/2nd man is decided each frame, within a team.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleRule {
    /// 1st man = teammate closest to the ball by straight-line distance.
    NearestToBall,
    /// 1st man = teammate with the lowest estimated time-to-ball: distance
    /// divided by closing speed (the component of velocity toward the ball).
    /// A player not closing on the ball is deprioritised toward 2nd man.
    TimeToBall,
}

/// Which team, if any, is in clear control of the ball (a positional proxy — see
/// the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Possession {
    /// One team is in clear control.
    Team(Team),
    /// No team has clear control: a loose ball, an even challenge, or no ball.
    Contested,
}

/// Possession from one team's point of view (the system's three-valued model).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelativePossession {
    Ours,
    Theirs,
    Neutral,
}

impl Possession {
    /// This possession state as seen by `team`.
    pub fn relative_to(self, team: Team) -> RelativePossession {
        match self {
            Possession::Team(t) if t == team => RelativePossession::Ours,
            Possession::Team(_) => RelativePossession::Theirs,
            Possession::Contested => RelativePossession::Neutral,
        }
    }
}

/// Per-player derived facts for one frame. Only alive (present, non-demolished)
/// players get facts; a demolished or absent player simply has none that frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerFacts {
    pub player: PlayerId,
    pub team: Team,
    pub role: Role,
    /// Straight-line distance from this player to the ball, in replay units.
    pub dist_to_ball: f32,
    /// True when the player is strictly goalside of the ball (between the ball
    /// and the team's own goal).
    pub goalside: bool,
    /// Boost amount carried into this frame, `0..=100`.
    pub boost: u8,
}

/// Derived facts for one frame.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameContext {
    facts: Vec<PlayerFacts>,
    possession: Possession,
}

impl FrameContext {
    /// All player facts this frame (alive players only).
    pub fn facts(&self) -> &[PlayerFacts] {
        &self.facts
    }

    /// Facts for one player this frame, if they were alive and on the field.
    pub fn player(&self, id: PlayerId) -> Option<&PlayerFacts> {
        self.facts.iter().find(|f| f.player == id)
    }

    /// Facts for the teammates of `id` this frame (same team, excluding `id`).
    /// Empty if `id` itself wasn't on the field.
    pub fn teammates_of(&self, id: PlayerId) -> impl Iterator<Item = &PlayerFacts> {
        let team = self.player(id).map(|f| f.team);
        self.facts
            .iter()
            .filter(move |f| Some(f.team) == team && f.player != id)
    }

    /// Who held the ball this frame.
    pub fn possession(&self) -> Possession {
        self.possession
    }
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// Knobs for context derivation, constructed at the edge and threaded in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContextConfig {
    pub role_rule: RoleRule,
    pub field: FieldGeometry,
    /// Max distance (uu) the controlling player may be from the ball to count as
    /// in control.
    pub control_radius_uu: f32,
    /// How much closer than the nearest opponent the controller must be (uu) for
    /// possession to be theirs rather than contested.
    pub control_margin_uu: f32,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            role_rule: RoleRule::NearestToBall,
            field: FieldGeometry::default(),
            // Uncalibrated positional-possession defaults: "in control" means
            // touching range and clearly closer than any opponent.
            control_radius_uu: 350.0,
            control_margin_uu: 250.0,
        }
    }
}

// ---------------------------------------------------------------------------
// MatchContext
// ---------------------------------------------------------------------------

/// A [`Timeline`] plus its per-frame derived facts, computed once and shared.
///
/// Borrows the timeline; the derived facts run parallel to it. Extractors read
/// frames via [`MatchContext::frames`].
pub struct MatchContext<'t> {
    timeline: &'t Timeline,
    frames: Vec<FrameContext>,
    shots: Vec<ShotEvent>,
    touches: Vec<TouchEvent>,
}

impl<'t> MatchContext<'t> {
    /// Derive the context for `timeline` under `config`, with the positional
    /// possession proxy (see the module docs — a v1 placeholder).
    pub fn derive(timeline: &'t Timeline, config: ContextConfig) -> Self {
        Self::derive_inner(timeline, config, None)
    }

    /// Derive the context with **authoritative possession spans** (touch-decoded
    /// runs, e.g. the canonical model's possession events via
    /// [`crate::bridge::possession_spans`]) instead of the positional proxy.
    ///
    /// Spans are authoritative: a frame inside a span belongs to that span's
    /// team, and a frame outside every span is [`Possession::Contested`] — the
    /// touch model already treats between-runs as loose, so the proxy is not
    /// consulted as a fallback.
    pub fn derive_with_possession(
        timeline: &'t Timeline,
        config: ContextConfig,
        spans: &[PossessionSpan],
    ) -> Self {
        Self::derive_inner(timeline, config, Some(spans))
    }

    fn derive_inner(
        timeline: &'t Timeline,
        config: ContextConfig,
        spans: Option<&[PossessionSpan]>,
    ) -> Self {
        let frames = timeline
            .iter()
            .map(|snapshot| {
                let mut frame = derive_frame(snapshot, &config);
                if let Some(spans) = spans {
                    frame.possession = spans
                        .iter()
                        .find(|s| s.start <= snapshot.t && snapshot.t <= s.end)
                        .map_or(Possession::Contested, |s| Possession::Team(s.team));
                }
                frame
            })
            .collect();
        Self {
            timeline,
            frames,
            shots: Vec::new(),
            touches: Vec::new(),
        }
    }

    /// Attach shot events (sorted by time) — see [`ShotEvent`].
    pub fn with_shots(mut self, mut shots: Vec<ShotEvent>) -> Self {
        shots.sort_by(|a, b| a.t.total_cmp(&b.t));
        self.shots = shots;
        self
    }

    /// The attached shot events, sorted by time. Empty unless the caller
    /// attached them via [`MatchContext::with_shots`].
    pub fn shots(&self) -> &[ShotEvent] {
        &self.shots
    }

    /// Attach ball-touch events (sorted by time) — see [`TouchEvent`].
    pub fn with_touches(mut self, mut touches: Vec<TouchEvent>) -> Self {
        touches.sort_by(|a, b| a.t.total_cmp(&b.t));
        self.touches = touches;
        self
    }

    /// The attached touch events, sorted by time. Empty unless the caller
    /// attached them via [`MatchContext::with_touches`].
    pub fn touches(&self) -> &[TouchEvent] {
        &self.touches
    }

    /// Iterate `(raw snapshot, derived facts)` pairs in frame order.
    pub fn frames(&self) -> impl Iterator<Item = (&WorldState, &FrameContext)> {
        self.timeline.iter().zip(self.frames.iter())
    }

    /// The `(raw snapshot, derived facts)` pair at frame `idx`, if in range.
    pub fn frame(&self, idx: usize) -> Option<(&WorldState, &FrameContext)> {
        Some((self.timeline.get(idx)?, self.frames.get(idx)?))
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Count the frames each team was in control (and the contested remainder).
    pub fn possession_counts(&self) -> PossessionCounts {
        let mut counts = PossessionCounts::default();
        for frame in &self.frames {
            match frame.possession {
                Possession::Team(Team::Blue) => counts.blue += 1,
                Possession::Team(Team::Orange) => counts.orange += 1,
                Possession::Contested => counts.contested += 1,
            }
        }
        counts
    }
}

/// Frame counts of ball control across a match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PossessionCounts {
    pub blue: usize,
    pub orange: usize,
    pub contested: usize,
}

/// One touch-decoded possession run: `team` controlled the ball from `start`
/// to `end` (seconds, inclusive). The authoritative replacement for the
/// positional proxy — see [`MatchContext::derive_with_possession`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PossessionSpan {
    pub team: Team,
    pub start: f32,
    pub end: f32,
}

/// One shot on goal by `player` at time `t`, from the replay's scoreboard
/// counters (see [`crate::bridge::shots`]). Attached to the context via
/// [`MatchContext::with_shots`]; empty for synthetic/domain-only callers, in
/// which case shot-conditional dimensions simply never apply.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShotEvent {
    pub t: f32,
    pub player: PlayerId,
}

/// One ball touch by `player` at time `t`, from the canonical touch decode
/// (see [`crate::bridge::touches`]). Attached via
/// [`MatchContext::with_touches`]; empty for synthetic/domain-only callers,
/// in which case touch-conditional dimensions simply never apply.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchEvent {
    pub t: f32,
    pub player: PlayerId,
}

// ---------------------------------------------------------------------------
// Derivation
// ---------------------------------------------------------------------------

fn derive_frame(snapshot: &WorldState, config: &ContextConfig) -> FrameContext {
    let Some(ball) = snapshot.ball.as_ref() else {
        return FrameContext {
            facts: Vec::new(),
            possession: Possession::Contested,
        };
    };

    let alive: Vec<&PlayerState> = snapshot.players.iter().filter(|p| !p.demolished).collect();

    let first_blue = first_man(&alive, Team::Blue, ball, config.role_rule);
    let first_orange = first_man(&alive, Team::Orange, ball, config.role_rule);

    let facts = alive
        .iter()
        .map(|p| {
            let first_man = match p.team {
                Team::Blue => first_blue,
                Team::Orange => first_orange,
            };
            let role = if Some(p.player) == first_man {
                Role::FirstMan
            } else {
                Role::SecondMan
            };
            PlayerFacts {
                player: p.player,
                team: p.team,
                role,
                dist_to_ball: p.pose.position.distance(ball.pose.position),
                goalside: is_goalside(
                    p.pose.position.y,
                    ball.pose.position.y,
                    config.field.own_goal_y(p.team),
                ),
                boost: p.boost,
            }
        })
        .collect();

    FrameContext {
        facts,
        possession: derive_possession(&alive, ball, config),
    }
}

/// The 1st man for `team`: the alive player minimising the role metric, ties
/// broken by the lower [`PlayerId`] for determinism. `None` if the team has no
/// alive player on the field.
fn first_man(
    alive: &[&PlayerState],
    team: Team,
    ball: &BallState,
    rule: RoleRule,
) -> Option<PlayerId> {
    alive
        .iter()
        .filter(|p| p.team == team)
        .min_by(|a, b| {
            role_metric(a, ball, rule)
                .total_cmp(&role_metric(b, ball, rule))
                .then(a.player.0.cmp(&b.player.0))
        })
        .map(|p| p.player)
}

/// Lower is "more 1st man". For [`RoleRule::NearestToBall`] this is distance; for
/// [`RoleRule::TimeToBall`] it is distance over closing speed.
fn role_metric(player: &PlayerState, ball: &BallState, rule: RoleRule) -> f32 {
    let dist = player.pose.position.distance(ball.pose.position);
    match rule {
        RoleRule::NearestToBall => dist,
        RoleRule::TimeToBall => {
            if dist <= f32::EPSILON {
                return 0.0;
            }
            // Closing speed: velocity projected onto the unit vector to the ball.
            let to_ball = ball.pose.position;
            let from = player.pose.position;
            let inv = 1.0 / dist;
            let dir_x = (to_ball.x - from.x) * inv;
            let dir_y = (to_ball.y - from.y) * inv;
            let dir_z = (to_ball.z - from.z) * inv;
            let closing =
                player.velocity.x * dir_x + player.velocity.y * dir_y + player.velocity.z * dir_z;
            // A non-closing player gets a large time, deprioritising them. The
            // floor keeps this finite and avoids divide-by-zero.
            dist / closing.max(CLOSING_SPEED_FLOOR)
        }
    }
}

/// Minimum closing speed (uu/s) used in the time-to-ball metric, so a stationary
/// or retreating player yields a large, finite time rather than a divide-by-zero.
const CLOSING_SPEED_FLOOR: f32 = 1.0;

/// Positional possession proxy: the alive player nearest the ball holds it when
/// they are within `control_radius_uu` and clearly closer than any opponent.
fn derive_possession(
    alive: &[&PlayerState],
    ball: &BallState,
    config: &ContextConfig,
) -> Possession {
    let leader = alive.iter().min_by(|a, b| {
        a.pose
            .position
            .distance(ball.pose.position)
            .total_cmp(&b.pose.position.distance(ball.pose.position))
    });
    let Some(leader) = leader else {
        return Possession::Contested;
    };

    let lead_dist = leader.pose.position.distance(ball.pose.position);
    if lead_dist > config.control_radius_uu {
        return Possession::Contested;
    }

    let nearest_opponent = alive
        .iter()
        .filter(|p| p.team != leader.team)
        .map(|p| p.pose.position.distance(ball.pose.position))
        .fold(f32::INFINITY, f32::min);

    if lead_dist + config.control_margin_uu < nearest_opponent {
        Possession::Team(leader.team)
    } else {
        Possession::Contested
    }
}

/// Is `point_y` between `ball_y` and the own goal at `own_goal_y` (strictly
/// goalside of the ball)?
fn is_goalside(point_y: f32, ball_y: f32, own_goal_y: f32) -> bool {
    (point_y - ball_y) * own_goal_y.signum() > 0.0
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Pose, Quat, Vec3};

    fn ball_at(x: f32, y: f32) -> BallState {
        BallState {
            pose: Pose {
                position: Vec3::new(x, y, 93.0),
                rotation: Quat::IDENTITY,
            },
            velocity: Vec3::ZERO,
        }
    }

    fn player(id: u32, team: Team, x: f32, y: f32) -> PlayerState {
        PlayerState {
            player: PlayerId(id),
            team,
            pose: Pose {
                position: Vec3::new(x, y, 17.0),
                rotation: Quat::IDENTITY,
            },
            velocity: Vec3::ZERO,
            boost: 33,
            demolished: false,
            name: None,
        }
    }

    fn frame(ball: BallState, players: Vec<PlayerState>) -> WorldState {
        WorldState {
            t: 0.0,
            ball: Some(ball),
            players,
        }
    }

    /// Derive a single-frame timeline with the given config and hand back the one
    /// frame's facts (cloned, so no borrow of a temporary escapes).
    fn derive_one(world: WorldState, config: ContextConfig) -> FrameContext {
        let timeline = vec![world];
        let ctx = MatchContext::derive(&timeline, config);
        let (_, facts) = ctx.frames().next().unwrap();
        facts.clone()
    }

    fn facts_of(world: WorldState) -> FrameContext {
        derive_one(world, ContextConfig::default())
    }

    #[test]
    fn nearest_blue_player_is_first_man() {
        let fc = facts_of(frame(
            ball_at(0.0, 0.0),
            vec![
                player(0, Team::Blue, 0.0, 500.0),
                player(1, Team::Blue, 0.0, 3000.0),
            ],
        ));

        assert_eq!(fc.player(PlayerId(0)).unwrap().role, Role::FirstMan);
        assert_eq!(fc.player(PlayerId(1)).unwrap().role, Role::SecondMan);
    }

    #[test]
    fn time_to_ball_prefers_the_player_closing_in() {
        // Player 1 is closer but stationary; player 0 is further but driving at
        // the ball, so time-to-ball makes player 0 the 1st man.
        let mut chaser = player(0, Team::Blue, 0.0, 2000.0);
        chaser.velocity = Vec3::new(0.0, -1500.0, 0.0); // toward the ball at y=0
        let near = player(1, Team::Blue, 0.0, 1000.0); // closer, not moving

        let fc = derive_one(
            frame(ball_at(0.0, 0.0), vec![chaser, near]),
            ContextConfig {
                role_rule: RoleRule::TimeToBall,
                ..Default::default()
            },
        );

        assert_eq!(
            fc.player(PlayerId(0)).unwrap().role,
            Role::FirstMan,
            "the closing player"
        );
        assert_eq!(fc.player(PlayerId(1)).unwrap().role, Role::SecondMan);
    }

    #[test]
    fn goalside_is_team_relative() {
        // Ball at y=0. Blue defends -y, orange +y.
        let fc = facts_of(frame(
            ball_at(0.0, 0.0),
            vec![
                player(0, Team::Blue, 0.0, -1000.0),
                player(2, Team::Orange, 0.0, 1000.0),
            ],
        ));

        assert!(
            fc.player(PlayerId(0)).unwrap().goalside,
            "blue behind the ball"
        );
        assert!(
            fc.player(PlayerId(2)).unwrap().goalside,
            "orange behind the ball"
        );
    }

    #[test]
    fn demolished_players_get_no_facts() {
        let mut dead = player(1, Team::Blue, 0.0, 500.0);
        dead.demolished = true;
        let fc = facts_of(frame(
            ball_at(0.0, 0.0),
            vec![player(0, Team::Blue, 0.0, 1000.0), dead],
        ));

        assert!(fc.player(PlayerId(1)).is_none());
        // The lone alive player is, by default, the 1st man.
        assert_eq!(fc.player(PlayerId(0)).unwrap().role, Role::FirstMan);
    }

    #[test]
    fn possession_goes_to_the_clear_nearest_player() {
        let fc = facts_of(frame(
            ball_at(0.0, 0.0),
            vec![
                player(0, Team::Blue, 0.0, 100.0),
                player(2, Team::Orange, 0.0, 2000.0),
            ],
        ));

        assert_eq!(fc.possession(), Possession::Team(Team::Blue));
        assert_eq!(
            fc.possession().relative_to(Team::Blue),
            RelativePossession::Ours
        );
        assert_eq!(
            fc.possession().relative_to(Team::Orange),
            RelativePossession::Theirs
        );
    }

    #[test]
    fn possession_is_contested_when_no_one_is_close() {
        let fc = facts_of(frame(
            ball_at(0.0, 0.0),
            vec![
                player(0, Team::Blue, 0.0, 2000.0),
                player(2, Team::Orange, 0.0, 2100.0),
            ],
        ));
        assert_eq!(fc.possession(), Possession::Contested, "ball is loose");
    }

    #[test]
    fn possession_is_contested_on_an_even_challenge() {
        // Both within range and about equally close → no clear control.
        let fc = facts_of(frame(
            ball_at(0.0, 0.0),
            vec![
                player(0, Team::Blue, 0.0, 150.0),
                player(2, Team::Orange, 0.0, 200.0),
            ],
        ));
        assert_eq!(fc.possession(), Possession::Contested);
    }

    #[test]
    fn possession_counts_tally_across_frames() {
        let blue = frame(
            ball_at(0.0, 0.0),
            vec![
                player(0, Team::Blue, 0.0, 100.0),
                player(2, Team::Orange, 0.0, 2000.0),
            ],
        );
        let loose = frame(
            ball_at(0.0, 0.0),
            vec![
                player(0, Team::Blue, 0.0, 2000.0),
                player(2, Team::Orange, 0.0, 2100.0),
            ],
        );
        let timeline = vec![blue, loose];
        let ctx = MatchContext::derive(&timeline, ContextConfig::default());

        let counts = ctx.possession_counts();
        assert_eq!(counts.blue, 1);
        assert_eq!(counts.contested, 1);
        assert_eq!(counts.orange, 0);
    }

    #[test]
    fn ball_less_frame_has_no_facts_and_is_contested() {
        let fc = facts_of(WorldState {
            t: 0.0,
            ball: None,
            players: vec![player(0, Team::Blue, 0.0, 0.0)],
        });

        assert!(fc.facts().is_empty());
        assert_eq!(fc.possession(), Possession::Contested);
    }
}
