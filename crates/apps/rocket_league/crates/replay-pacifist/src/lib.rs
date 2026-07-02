//! Pacifist System scoring over the canonical match model.
//!
//! Ported from the standalone PacifistScore repo (consolidated here; see
//! `docs/pacifist-score-design.md` for the design and
//! `docs/pacifist-assessment-criteria.md` for the criteria it will grow to
//! cover). The domain — world types, per-frame context derivation, the
//! [`metrics::MetricExtractor`] dimensions, and the confidence-weighted
//! [`scoring::Analyzer`] — is pure: no I/O and no replay-format dependencies,
//! so it stays testable with hand-built timelines. The one format-facing seam
//! is [`bridge`], which converts this workspace's `CanonicalMatch` (the
//! validated decode/reconstruction layer) into a [`Timeline`], replacing the
//! original repo's own boxcars adapter.

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// A 3D vector in replay (Unreal) units. No unit conversion is applied here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    /// Euclidean distance to another point, in replay units.
    pub fn distance(self, other: Vec3) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        let dz = self.z - other.z;
        (dx * dx + dy * dy + dz * dz).sqrt()
    }
}

/// An orientation quaternion.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Quat {
    /// Identity rotation.
    pub const IDENTITY: Quat = Quat {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    };
}

/// Position + orientation of a physics body.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub position: Vec3,
    pub rotation: Quat,
}

/// The bits of field geometry the analysis needs. `goal_y` is the distance from
/// center to each goal's back wall along the attack axis (standard soccar is
/// 5120 replay units). Blue defends `-goal_y`, orange `+goal_y`.
///
/// This is the single place to flip if a future replay ever disagrees with the
/// verified convention (blue on `-Y` at kickoff).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FieldGeometry {
    pub goal_y: f32,
}

impl Default for FieldGeometry {
    fn default() -> Self {
        Self { goal_y: 5120.0 }
    }
}

impl FieldGeometry {
    /// The y-coordinate of `team`'s own goal.
    pub fn own_goal_y(self, team: Team) -> f32 {
        match team {
            Team::Blue => -self.goal_y,
            Team::Orange => self.goal_y,
        }
    }
}

// ---------------------------------------------------------------------------
// Identifiers
// ---------------------------------------------------------------------------

/// Stable identifier for a player within a single match.
///
/// Assigned by the parser when a player first appears and held constant for the
/// life of that player's replication info. It is *not* meaningful across matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlayerId(pub u32);

/// The two sides. In Rocket League replays, team index `0` is blue and `1` is
/// orange.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Team {
    Blue,
    Orange,
}

impl Team {
    /// Map a raw replay team index to a side. Returns `None` for any value other
    /// than `0` or `1`.
    pub const fn from_side(side: u8) -> Option<Team> {
        match side {
            0 => Some(Team::Blue),
            1 => Some(Team::Orange),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// World state
// ---------------------------------------------------------------------------

/// The ball at a single instant.
#[derive(Debug, Clone, PartialEq)]
pub struct BallState {
    pub pose: Pose,
    pub velocity: Vec3,
}

/// One player's car at a single instant, already resolved to its player and team.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerState {
    pub player: PlayerId,
    pub team: Team,
    pub pose: Pose,
    pub velocity: Vec3,
    /// Boost amount, `0..=100`.
    pub boost: u8,
    /// True for the frame in which a demolition was reported against this car.
    pub demolished: bool,
    /// Display name, if it was present in the replay.
    pub name: Option<String>,
}

/// A normalized snapshot of the world at game time `t` (seconds).
///
/// `ball` is optional because there are frames (kickoff countdown, replays of a
/// goal) where no ball actor is live. `players` contains only cars that have
/// been fully resolved to a player and team; an unresolved car is omitted rather
/// than guessed at.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldState {
    pub t: f32,
    pub ball: Option<BallState>,
    pub players: Vec<PlayerState>,
}

/// An ordered sequence of world snapshots — the analyzer's input.
pub type Timeline = Vec<WorldState>;

// ---------------------------------------------------------------------------
// Roster
// ---------------------------------------------------------------------------

/// A player as resolved across a whole timeline: id, team, and best-known name.
/// Built by [`roster`].
#[derive(Debug, Clone, PartialEq)]
pub struct RosterEntry {
    pub player: PlayerId,
    pub team: Team,
    pub name: Option<String>,
}

/// The set of players that appear anywhere in `timeline`, sorted by [`PlayerId`].
///
/// A player's team is taken from its first appearance; its name from the most
/// recent frame that carried one, since the display name can replicate a few
/// frames after the car resolves. Players the registry never resolved are simply
/// absent — they were omitted upstream, not guessed at.
pub fn roster(timeline: &Timeline) -> Vec<RosterEntry> {
    use std::collections::BTreeMap;

    let mut entries: BTreeMap<PlayerId, RosterEntry> = BTreeMap::new();
    for snapshot in timeline {
        for player in &snapshot.players {
            let entry = entries.entry(player.player).or_insert_with(|| RosterEntry {
                player: player.player,
                team: player.team,
                name: None,
            });
            if player.name.is_some() {
                entry.name = player.name.clone();
            }
        }
    }
    entries.into_values().collect()
}

pub mod bridge;
pub mod context;
pub mod metrics;
pub mod scoring;

// ---------------------------------------------------------------------------
// Match format
// ---------------------------------------------------------------------------

/// How many players each side fielded — used to check whether a replay is the
/// 2v2 format the Pacifist dimensions are tuned for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeamSizes {
    pub blue: usize,
    pub orange: usize,
}

impl TeamSizes {
    /// True when both sides fielded exactly two players.
    pub fn is_two_v_two(self) -> bool {
        self.blue == 2 && self.orange == 2
    }
}

/// Detect team sizes as the peak number of players each side had on the field at
/// once across `timeline`.
///
/// Peak-per-frame (rather than first-frame or whole-match distinct) is robust to
/// two real cases: players that resolve a few frames after kickoff, and a
/// mid-match disconnect/reconnect that would inflate a distinct count past the
/// true team size.
pub fn team_sizes(timeline: &Timeline) -> TeamSizes {
    let mut blue = 0;
    let mut orange = 0;
    for snapshot in timeline {
        let on_field = |team| snapshot.players.iter().filter(|p| p.team == team).count();
        blue = blue.max(on_field(Team::Blue));
        orange = orange.max(on_field(Team::Orange));
    }
    TeamSizes { blue, orange }
}

#[cfg(test)]
mod roster_tests {
    use super::*;

    fn snapshot(t: f32, players: Vec<PlayerState>) -> WorldState {
        WorldState {
            t,
            ball: None,
            players,
        }
    }

    fn player(id: u32, team: Team, name: Option<&str>) -> PlayerState {
        PlayerState {
            player: PlayerId(id),
            team,
            pose: Pose {
                position: Vec3::ZERO,
                rotation: Quat::IDENTITY,
            },
            velocity: Vec3::ZERO,
            boost: 0,
            demolished: false,
            name: name.map(str::to_string),
        }
    }

    #[test]
    fn collects_unique_players_sorted_by_id() {
        let timeline = vec![
            snapshot(
                0.0,
                vec![
                    player(2, Team::Orange, Some("orange")),
                    player(0, Team::Blue, Some("blue")),
                ],
            ),
            snapshot(
                1.0,
                vec![
                    player(0, Team::Blue, Some("blue")),
                    player(2, Team::Orange, Some("orange")),
                ],
            ),
        ];
        let roster = roster(&timeline);

        assert_eq!(roster.len(), 2);
        assert_eq!(roster[0].player, PlayerId(0));
        assert_eq!(roster[0].team, Team::Blue);
        assert_eq!(roster[1].player, PlayerId(2));
    }

    #[test]
    fn name_is_filled_from_a_later_frame() {
        // Player resolves without a name first, then the name replicates.
        let timeline = vec![
            snapshot(0.0, vec![player(0, Team::Blue, None)]),
            snapshot(1.0, vec![player(0, Team::Blue, Some("late_name"))]),
        ];
        let roster = roster(&timeline);

        assert_eq!(roster.len(), 1);
        assert_eq!(roster[0].name.as_deref(), Some("late_name"));
    }

    #[test]
    fn empty_timeline_has_no_roster() {
        assert!(roster(&Vec::new()).is_empty());
    }
}

#[cfg(test)]
mod format_tests {
    use super::*;

    fn snapshot(players: Vec<PlayerState>) -> WorldState {
        WorldState {
            t: 0.0,
            ball: None,
            players,
        }
    }

    fn player(id: u32, team: Team) -> PlayerState {
        PlayerState {
            player: PlayerId(id),
            team,
            pose: Pose {
                position: Vec3::ZERO,
                rotation: Quat::IDENTITY,
            },
            velocity: Vec3::ZERO,
            boost: 0,
            demolished: false,
            name: None,
        }
    }

    #[test]
    fn detects_two_v_two() {
        let timeline = vec![snapshot(vec![
            player(0, Team::Blue),
            player(1, Team::Blue),
            player(2, Team::Orange),
            player(3, Team::Orange),
        ])];
        let sizes = team_sizes(&timeline);
        assert_eq!(sizes, TeamSizes { blue: 2, orange: 2 });
        assert!(sizes.is_two_v_two());
    }

    #[test]
    fn three_v_three_is_not_two_v_two() {
        let timeline = vec![snapshot(vec![
            player(0, Team::Blue),
            player(1, Team::Blue),
            player(2, Team::Blue),
            player(3, Team::Orange),
            player(4, Team::Orange),
            player(5, Team::Orange),
        ])];
        assert!(!team_sizes(&timeline).is_two_v_two());
    }

    #[test]
    fn takes_the_peak_across_frames() {
        // Blue's second player resolves a frame after the first.
        let timeline = vec![
            snapshot(vec![
                player(0, Team::Blue),
                player(2, Team::Orange),
                player(3, Team::Orange),
            ]),
            snapshot(vec![
                player(0, Team::Blue),
                player(1, Team::Blue),
                player(2, Team::Orange),
                player(3, Team::Orange),
            ]),
        ];
        assert_eq!(team_sizes(&timeline), TeamSizes { blue: 2, orange: 2 });
    }

    #[test]
    fn empty_timeline_is_zero() {
        assert_eq!(team_sizes(&Vec::new()), TeamSizes { blue: 0, orange: 0 });
    }
}
