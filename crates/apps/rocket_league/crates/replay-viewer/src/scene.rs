//! The **playback scene**: a compact, viewer-ready distillation of the canonical
//! match model.
//!
//! The full [`CanonicalMatch`] carries native frames, per-player track samples,
//! and the resampled grid — far more than a 3D viewer needs, and large. This
//! module projects it down to exactly what playback consumes: per-frame ball +
//! car kinematics off the **resampled grid** (uniform rate, every live actor
//! present), a player roster, the field box, and a merged, time-sorted list of
//! match events and detected skills to annotate the timeline.
//!
//! Coordinates stay in Rocket League unreal units (Z-up); the web layer scales
//! and orients. Values are rounded (positions to 1 uu, rotations to 1e-3 rad) so
//! the embedded JSON stays small without any visible loss.

use std::collections::BTreeMap;

use replay_analyzer::field;
use replay_analyzer::model::{CanonicalMatch, Event};
use replay_skills::SkillInstance;
use serde::{Deserialize, Serialize};

/// Goal-mouth half-width (uu) — not in `field.rs`; standard Soccar net is ~1786
/// wide, ~642.78 tall. Used only to draw the goal markers.
const GOAL_HALF_WIDTH: f32 = 892.755;
const GOAL_HEIGHT: f32 = 642.775;

/// Arena dimensions the viewer draws (uu).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field {
    pub side_wall_x: f32,
    pub back_wall_y: f32,
    pub ceiling_z: f32,
    pub goal_half_width: f32,
    pub goal_height: f32,
    pub ball_radius: f32,
}

impl Default for Field {
    fn default() -> Self {
        Field {
            side_wall_x: field::SIDE_WALL_X,
            back_wall_y: field::BACK_WALL_Y,
            ceiling_z: field::CEILING_Z,
            goal_half_width: GOAL_HALF_WIDTH,
            goal_height: GOAL_HEIGHT,
            ball_radius: field::BALL_RADIUS,
        }
    }
}

/// A player in the roster, keyed by the stable PRI the frames reference, with
/// header end-of-match stats (joined by name).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScenePlayer {
    pub pri: i32,
    pub name: String,
    pub team: Option<i32>,
    #[serde(default)]
    pub goals: i32,
    #[serde(default)]
    pub assists: i32,
    #[serde(default)]
    pub saves: i32,
    /// Total ΔV (scoring-probability swing) this player's touches caused — an
    /// "impact" readout. Filled by [`crate::impact::attach_impact`]; `None` until.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub impact: Option<f32>,
}

/// One car's pose at a frame. `rot` is `[pitch, yaw, roll]` (rad); `boost` is a
/// percent (0–100). `role` is `1` (1st man) / `2` (2nd man) when the optional
/// scoring overlay is attached (see [`crate::roles::attach_roles`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneCar {
    pub pri: i32,
    pub p: [f32; 3],
    pub rot: [f32; 3],
    pub boost: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<u8>,
}

/// One playback frame: time, ball position (absent if the ball isn't live), and
/// every live car.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneFrame {
    pub t: f32,
    pub ball: Option<[f32; 3]>,
    pub cars: Vec<SceneCar>,
}

/// A timeline annotation: a match event or a detected skill.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneEvent {
    /// `goal` | `demo` | `kickoff` | `touch` | `skill`.
    pub kind: String,
    pub t: f32,
    pub pri: Option<i32>,
    pub team: Option<i32>,
    /// Human-readable one-liner for the ticker.
    pub label: String,
    /// Scoring-probability swing (ΔV) of the touch this annotates — signed
    /// (+ helped the toucher's team). Set on `touch` events by
    /// [`crate::impact::attach_impact`]; `None` otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dv: Option<f32>,
}

/// A boost-pad pickup, for the viewer pickup-map overlay: which pad lit up, when,
/// and for whom (from [`replay_analyzer::analyze::boost_pads`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScenePadPickup {
    pub t: f32,
    /// World `(x, y)` of the pad (matches the viewer's drawn pad positions).
    pub pad: [f32; 2],
    /// Collecting player's team (tints the flash).
    pub team: Option<i32>,
    /// A big (full) pad vs a small one.
    pub big: bool,
    /// Collected in the opponent's half.
    pub stolen: bool,
}

/// Compact per-player aggregates for the viewer's analysis strips, distilled from
/// [`replay_analyzer::analyze::bcstats`]. Percents are 0–100. Keyed by `pri` so the
/// viewer joins them to the roster rows. Filled by
/// [`crate::playerstats::attach_player_stats`]; absent until.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScenePlayerStat {
    pub pri: i32,
    /// Movement time split: `[slow, boost-speed, supersonic]` as percents.
    pub speed: [f32; 3],
    /// Field occupancy: `[defensive, neutral, offensive]` third, as percents.
    pub thirds: [f32; 3],
    /// Time airborne (low + high air), as a percent.
    #[serde(default)]
    pub air: f32,
    /// Share of time this player was the most-back on their team (percent).
    pub most_back: f32,
    /// Boost collected per minute.
    #[serde(default)]
    pub bpm: f32,
    /// Mean boost gauge held (0–100).
    #[serde(default)]
    pub avg_boost: f32,
    /// Mean distance to the ball (uu).
    #[serde(default)]
    pub dist_to_ball: f32,
    /// Share of driving time spent in reverse (percent).
    #[serde(default)]
    pub reverse: f32,
    /// Share of ball-present time facing the ball (percent).
    #[serde(default)]
    pub facing: f32,
    /// Powerslide count over the match.
    #[serde(default)]
    pub powerslides: u32,
    /// Demolitions inflicted / taken.
    #[serde(default)]
    pub demos_for: u32,
    #[serde(default)]
    pub demos_against: u32,
}

/// The full viewer payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub replay_id: String,
    pub map: Option<String>,
    /// True if the map is a recognized non-standard arena/mode, so the drawn
    /// field and geometry-derived overlays are approximate (see
    /// [`replay_analyzer::field::classify_map`]).
    pub non_standard_map: bool,
    pub hz: f32,
    pub duration_s: f32,
    /// Team id (as string key in JSON) -> final score.
    pub team_scores: BTreeMap<i32, i32>,
    /// Team id -> attack-direction sign (+1 attacks +Y), so the viewer can show
    /// which end each team defends.
    pub attack_sign: BTreeMap<i32, i32>,
    pub field: Field,
    pub players: Vec<ScenePlayer>,
    pub frames: Vec<SceneFrame>,
    /// Events + skills, time-sorted.
    pub events: Vec<SceneEvent>,
    /// Boost-pad pickups, time-sorted (for the pickup-map overlay). Empty unless
    /// any were attributed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pad_pickups: Vec<ScenePadPickup>,
    /// Optional coarse momentum curve: P(team 0 scores the next goal) sampled
    /// evenly across the match (see [`crate::winprob::attach_winprob`]). Empty
    /// unless attached.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub win_prob: Vec<f32>,
    /// Per-player movement/positioning aggregates for the analysis strips. Empty
    /// unless [`crate::playerstats::attach_player_stats`] ran.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub player_stats: Vec<ScenePlayerStat>,
}

fn round(x: f32, places: i32) -> f32 {
    let f = 10f32.powi(places);
    (x * f).round() / f
}

fn pos(p: replay_analyzer::model::Vec3) -> [f32; 3] {
    [round(p.x, 0), round(p.y, 0), round(p.z, 0)]
}

/// Build the playback scene from the canonical match and (optionally) detected
/// skills. Pass an empty slice for `skills` to annotate match events only.
pub fn build_scene(m: &CanonicalMatch, skills: &[SkillInstance]) -> Scene {
    let players = m
        .tracks
        .iter()
        .map(|t| {
            let meta = m.players.iter().find(|p| p.name == t.player);
            ScenePlayer {
                pri: t.pri,
                name: t.player.clone(),
                team: t.team,
                goals: meta.map(|p| p.goals).unwrap_or(0),
                assists: meta.map(|p| p.assists).unwrap_or(0),
                saves: meta.map(|p| p.saves).unwrap_or(0),
                impact: None,
            }
        })
        .collect();

    let frames = m
        .resampled
        .frames
        .iter()
        .map(|f| SceneFrame {
            t: round(f.t, 2),
            ball: f.ball.map(|b| pos(b.p)),
            cars: f
                .cars
                .iter()
                .map(|c| SceneCar {
                    pri: c.pri,
                    p: pos(c.p),
                    rot: c
                        .rot
                        .map(|r| [round(r.pitch, 3), round(r.yaw, 3), round(r.roll, 3)])
                        .unwrap_or([0.0, 0.0, 0.0]),
                    boost: c
                        .boost
                        .map(|b| field::boost_percent(b).round() as u8)
                        .unwrap_or(0),
                    role: None,
                })
                .collect(),
        })
        .collect();

    // Boost-pad pickups per player, tagged with the collecting team.
    let mut pad_pickups: Vec<ScenePadPickup> = m
        .tracks
        .iter()
        .flat_map(|t| {
            let sign = t
                .team
                .and_then(|tm| m.resampled.team_attack_sign.get(&tm).copied())
                .unwrap_or(1);
            replay_analyzer::analyze::boost_pads::pad_pickups(t, sign)
                .into_iter()
                .map(move |pk| ScenePadPickup {
                    t: round(pk.t, 2),
                    pad: [round(pk.pad.0, 0), round(pk.pad.1, 0)],
                    team: t.team,
                    big: matches!(pk.kind, replay_analyzer::field::PadKind::Big),
                    stolen: pk.stolen,
                })
        })
        .collect();
    pad_pickups.sort_by(|a, b| a.t.total_cmp(&b.t));

    let mut events = match_events(m);
    events.extend(skills.iter().map(|s| SceneEvent {
        kind: "skill".into(),
        t: round(s.t, 2),
        pri: Some(s.pri),
        team: s.team,
        label: match &s.player {
            Some(p) => format!("{} — {}", s.skill.display_name(), p),
            None => s.skill.display_name().to_string(),
        },
        dv: None,
    }));
    events.sort_by(|a, b| a.t.total_cmp(&b.t));

    Scene {
        replay_id: m.replay_id.clone(),
        map: m.map.clone(),
        non_standard_map: !field::is_standard_geometry(m.map.as_deref()),
        hz: m.resampled.hz,
        duration_s: round(m.duration_s, 2),
        team_scores: m.team_scores.clone(),
        attack_sign: m.resampled.team_attack_sign.clone(),
        field: field_for(m.map.as_deref()),
        players,
        frames,
        events,
        pad_pickups,
        win_prob: Vec::new(),
        player_stats: Vec::new(),
    }
}

/// The drawn-arena dimensions for a map (envelope from
/// [`field::geometry_for_map`]; goal mouth + ball radius are constants).
fn field_for(map: Option<&str>) -> Field {
    let g = field::geometry_for_map(map);
    Field {
        side_wall_x: g.side_wall_x,
        back_wall_y: g.back_wall_y,
        ceiling_z: g.ceiling_z,
        goal_half_width: GOAL_HALF_WIDTH,
        goal_height: GOAL_HEIGHT,
        ball_radius: field::BALL_RADIUS,
    }
}

/// Thin the playback frames toward `target_hz` (keep every k-th frame) to shrink
/// the embedded payload. No-op if `target_hz` is non-positive or already ≥ the
/// scene's rate. Updates [`Scene::hz`]; events are untouched (they carry their
/// own times).
pub fn downsample(scene: &mut Scene, target_hz: f32) {
    if target_hz <= 0.0 || target_hz >= scene.hz {
        return;
    }
    let k = (scene.hz / target_hz).round().max(1.0) as usize;
    if k <= 1 {
        return;
    }
    let mut i = 0usize;
    scene.frames.retain(|_| {
        let keep = i.is_multiple_of(k);
        i += 1;
        keep
    });
    scene.hz /= k as f32;
}

/// Project the canonical match events into point annotations (possessions, being
/// intervals, are dropped — the timeline shows discrete moments).
fn match_events(m: &CanonicalMatch) -> Vec<SceneEvent> {
    let mut out = Vec::new();
    for e in &m.events {
        let ev = match e {
            Event::Goal { t, scorer, team } => SceneEvent {
                kind: "goal".into(),
                t: round(*t, 2),
                pri: None,
                team: *team,
                label: match scorer {
                    Some(s) => format!("GOAL — {s}"),
                    None => "GOAL".into(),
                },
                dv: None,
            },
            Event::Demo {
                t,
                attacker,
                victim,
                attacker_pri,
                ..
            } => SceneEvent {
                kind: "demo".into(),
                t: round(*t, 2),
                pri: *attacker_pri,
                team: None,
                label: format!(
                    "DEMO — {} ▸ {}",
                    attacker.as_deref().unwrap_or("?"),
                    victim.as_deref().unwrap_or("?")
                ),
                dv: None,
            },
            Event::Kickoff { t } => SceneEvent {
                kind: "kickoff".into(),
                t: round(*t, 2),
                pri: None,
                team: None,
                label: "Kickoff".into(),
                dv: None,
            },
            Event::Touch {
                t,
                player,
                team,
                pri,
            } => SceneEvent {
                kind: "touch".into(),
                t: round(*t, 2),
                pri: Some(*pri),
                team: *team,
                label: format!("touch — {}", player.as_deref().unwrap_or("?")),
                dv: None,
            },
            Event::Stat {
                t,
                player,
                team,
                pri,
                kind,
            } => SceneEvent {
                kind: "stat".into(),
                t: round(*t, 2),
                pri: Some(*pri),
                team: *team,
                label: format!("{kind:?} — {}", player.as_deref().unwrap_or("?")),
                dv: None,
            },
            Event::Possession { .. } => continue,
        };
        out.push(ev);
    }
    out
}
