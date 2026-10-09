//! Coordinate-convention sanity check.
//!
//! Position-based metrics (the Pacifist depth/lateral dimensions in particular)
//! assume the standard Soccar convention: **blue defends `-Y`**, the goal line
//! sits at `|Y| = 5120`, and positions are in uu. If a replay violates that —
//! a flipped map, a unit change in a future decoder version — every such score
//! would come out mirrored or scaled *without any error*. This module checks the
//! assumption against the replay itself and reports what it found.
//!
//! Ported from the standalone PacifistScore repo's `coordinate_report`. Two
//! deliberate differences: kickoffs are taken from the analyzer's own
//! [`Event::Kickoff`] times (not re-detected from ball position), and the
//! convention is voted over *every* kickoff rather than the first, so one odd
//! frame cannot flip the verdict.
//!
//! Pure: takes the resampled grid and kickoff times, does no I/O.

use serde::Serialize;

use crate::field::{BACK_WALL_Y, CEILING_Z, GOAL_DEPTH_Y, SIDE_WALL_X};
use crate::model::{CanonicalMatch, Event, GridFrame};

/// Slack past the arena planes before a position counts as out of bounds (uu).
/// Cars ride walls and the ball briefly intrudes into goals, so this is generous:
/// it should trip on a wrong unit or axis, not on a rebound.
const BOUNDS_SLACK: f32 = 300.0;

/// Blue's mean kickoff `y` must clear orange's by this much before the
/// convention is called either way; closer than this is "can't tell".
const MIN_TEAM_SEPARATION_Y: f32 = 500.0;

/// What the replay's coordinates look like.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoordinateReport {
    /// Kickoffs with a live car on each side that contributed to the means.
    pub kickoffs_checked: usize,
    /// Mean car `y` at kickoff, averaged over the kickoffs checked.
    pub blue_kickoff_mean_y: Option<f32>,
    pub orange_kickoff_mean_y: Option<f32>,
    /// Extents over every ball and car sample, for the bounds check.
    pub max_abs_x: f32,
    pub max_abs_y: f32,
    pub max_z: f32,
}

/// Something that would silently corrupt position-based scores.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CoordinateIssue {
    /// Blue spawns on the `+Y` side: depth/lateral metrics would be mirrored.
    ConventionFlipped { blue_y: f32, orange_y: f32 },
    /// Positions beyond the standard arena: wrong units or axes.
    OutOfBounds {
        axis: &'static str,
        max: f32,
        limit: f32,
    },
}

impl CoordinateIssue {
    /// One-line, user-facing description.
    pub fn message(&self) -> String {
        match self {
            Self::ConventionFlipped { blue_y, orange_y } => format!(
                "Blue spawns on the +Y side (mean kickoff y {blue_y:.0} vs orange {orange_y:.0}); \
                 Pacifist depth and lateral scores assume blue defends -Y and may be mirrored."
            ),
            Self::OutOfBounds { axis, max, limit } => format!(
                "Positions reach {axis} = {max:.0} uu, beyond the standard arena (~{limit:.0}); \
                 the replay may use different units or a non-standard map."
            ),
        }
    }
}

impl CoordinateReport {
    /// Does blue sit on the negative-`y` side at kickoff, as the metrics assume?
    /// `None` when no kickoff had both teams, or the teams are too close to tell.
    pub fn blue_defends_negative_y(&self) -> Option<bool> {
        let (blue, orange) = (self.blue_kickoff_mean_y?, self.orange_kickoff_mean_y?);
        ((orange - blue).abs() >= MIN_TEAM_SEPARATION_Y).then_some(blue < orange)
    }

    /// Everything wrong with the coordinates. `check_bounds` should be `false`
    /// for non-standard maps, whose arenas differ from the Soccar constants.
    pub fn issues(&self, check_bounds: bool) -> Vec<CoordinateIssue> {
        let mut out = Vec::new();
        if let (Some(false), Some(blue_y), Some(orange_y)) = (
            self.blue_defends_negative_y(),
            self.blue_kickoff_mean_y,
            self.orange_kickoff_mean_y,
        ) {
            out.push(CoordinateIssue::ConventionFlipped { blue_y, orange_y });
        }
        if check_bounds {
            let limits = [
                ("x", self.max_abs_x, SIDE_WALL_X + BOUNDS_SLACK),
                (
                    "y",
                    self.max_abs_y,
                    BACK_WALL_Y + GOAL_DEPTH_Y + BOUNDS_SLACK,
                ),
                ("z", self.max_z, CEILING_Z + BOUNDS_SLACK),
            ];
            out.extend(
                limits
                    .into_iter()
                    .filter(|(_, max, limit)| max > limit)
                    .map(|(axis, max, limit)| CoordinateIssue::OutOfBounds { axis, max, limit }),
            );
        }
        out
    }
}

/// Build the report for a whole match, using its own kickoff events.
pub fn coordinate_report(m: &CanonicalMatch) -> CoordinateReport {
    let kickoffs: Vec<f32> = m
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Kickoff { t } => Some(*t),
            _ => None,
        })
        .collect();
    report_from_grid(&m.resampled.frames, &kickoffs)
}

/// Core of [`coordinate_report`], over a grid and kickoff times (oldest first).
pub fn report_from_grid(frames: &[GridFrame], kickoff_times: &[f32]) -> CoordinateReport {
    let (mut max_abs_x, mut max_abs_y, mut max_z) = (0.0_f32, 0.0_f32, 0.0_f32);
    let mut observe = |p: crate::model::Vec3| {
        max_abs_x = max_abs_x.max(p.x.abs());
        max_abs_y = max_abs_y.max(p.y.abs());
        max_z = max_z.max(p.z);
    };
    for f in frames {
        if let Some(b) = &f.ball {
            observe(b.p);
        }
        for c in &f.cars {
            observe(c.p);
        }
    }

    let per_kickoff: Vec<(f32, f32)> = kickoff_times
        .iter()
        .filter_map(|&t| {
            let i = frames.partition_point(|f| f.t < t);
            let f = frames.get(i)?;
            Some((team_mean_y(f, 0)?, team_mean_y(f, 1)?))
        })
        .collect();
    let mean = |pick: fn(&(f32, f32)) -> f32| {
        (!per_kickoff.is_empty())
            .then(|| per_kickoff.iter().map(pick).sum::<f32>() / per_kickoff.len() as f32)
    };

    CoordinateReport {
        kickoffs_checked: per_kickoff.len(),
        blue_kickoff_mean_y: mean(|p| p.0),
        orange_kickoff_mean_y: mean(|p| p.1),
        max_abs_x,
        max_abs_y,
        max_z,
    }
}

fn team_mean_y(f: &GridFrame, team: i32) -> Option<f32> {
    let ys: Vec<f32> = f
        .cars
        .iter()
        .filter(|c| c.team == Some(team))
        .map(|c| c.p.y)
        .collect();
    (!ys.is_empty()).then(|| ys.iter().sum::<f32>() / ys.len() as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{GridCar, Kin, Vec3};

    fn v(x: f32, y: f32, z: f32) -> Vec3 {
        Vec3 { x, y, z }
    }

    fn car(pri: i32, team: i32, p: Vec3) -> GridCar {
        GridCar {
            pri,
            team: Some(team),
            p,
            v: v(0.0, 0.0, 0.0),
            boost: None,
            rot: None,
        }
    }

    fn frame(t: f32, blue_y: f32, orange_y: f32) -> GridFrame {
        GridFrame {
            t,
            ball: Some(Kin {
                p: v(0.0, 0.0, 93.0),
                v: v(0.0, 0.0, 0.0),
            }),
            cars: vec![
                car(0, 0, v(-256.0, blue_y, 17.0)),
                car(1, 0, v(256.0, blue_y, 17.0)),
                car(2, 1, v(0.0, orange_y, 17.0)),
            ],
        }
    }

    #[test]
    fn standard_convention_is_clean() {
        let r = report_from_grid(&[frame(0.5, -2900.0, 2900.0)], &[0.5]);
        assert_eq!(r.kickoffs_checked, 1);
        assert_eq!(r.blue_defends_negative_y(), Some(true));
        assert!(r.issues(true).is_empty());
    }

    #[test]
    fn flipped_convention_is_reported_with_the_numbers() {
        let r = report_from_grid(&[frame(0.5, 2900.0, -2900.0)], &[0.5]);
        assert_eq!(r.blue_defends_negative_y(), Some(false));
        let issues = r.issues(true);
        assert_eq!(
            issues,
            vec![CoordinateIssue::ConventionFlipped {
                blue_y: 2900.0,
                orange_y: -2900.0
            }]
        );
        assert!(issues[0].message().contains("mirrored"));
    }

    #[test]
    fn teams_too_close_to_tell_is_not_a_verdict() {
        let r = report_from_grid(&[frame(0.5, -100.0, 100.0)], &[0.5]);
        assert_eq!(r.blue_defends_negative_y(), None);
        assert!(r.issues(true).is_empty(), "unknown is not a failure");
    }

    #[test]
    fn no_kickoff_means_no_convention_verdict() {
        let r = report_from_grid(&[frame(0.5, 2900.0, -2900.0)], &[]);
        assert_eq!(r.kickoffs_checked, 0);
        assert_eq!(r.blue_defends_negative_y(), None);
        assert!(r.issues(true).is_empty());
    }

    #[test]
    fn kickoff_is_read_at_the_first_frame_at_or_after_it() {
        let frames = [frame(0.0, 0.0, 0.0), frame(1.0, -2900.0, 2900.0)];
        let r = report_from_grid(&frames, &[0.4]);
        assert_eq!(r.blue_kickoff_mean_y, Some(-2900.0));
    }

    #[test]
    fn a_kickoff_missing_a_team_is_skipped_not_fatal() {
        let mut f = frame(0.5, -2900.0, 2900.0);
        f.cars.retain(|c| c.team == Some(0));
        let r = report_from_grid(&[f, frame(1.5, -2900.0, 2900.0)], &[0.5, 1.5]);
        assert_eq!(r.kickoffs_checked, 1);
        assert_eq!(r.blue_defends_negative_y(), Some(true));
    }

    #[test]
    fn kickoffs_are_averaged_so_one_odd_frame_cannot_flip_the_verdict() {
        let frames = [
            frame(0.5, -2900.0, 2900.0),
            frame(10.5, -2900.0, 2900.0),
            frame(20.5, 2900.0, -2900.0), // one glitched kickoff
        ];
        let r = report_from_grid(&frames, &[0.5, 10.5, 20.5]);
        assert_eq!(r.blue_defends_negative_y(), Some(true));
    }

    #[test]
    fn extents_track_the_max_and_gate_on_bounds() {
        let mut f = frame(0.5, -2900.0, 2900.0);
        f.cars[0].p = v(-4096.0, -5900.0, 1800.0);
        let r = report_from_grid(&[f.clone()], &[0.5]);
        assert_eq!(
            (r.max_abs_x, r.max_abs_y, r.max_z),
            (4096.0, 5900.0, 1800.0)
        );
        assert!(
            r.issues(true).is_empty(),
            "a car on the wall/in the goal is legal"
        );

        // Centimetres instead of uu: every extent is 10x too big.
        f.cars[0].p = v(-40960.0, -59000.0, 18000.0);
        let r = report_from_grid(&[f], &[0.5]);
        let axes: Vec<_> = r
            .issues(true)
            .into_iter()
            .filter_map(|i| match i {
                CoordinateIssue::OutOfBounds { axis, .. } => Some(axis),
                _ => None,
            })
            .collect();
        assert_eq!(axes, ["x", "y", "z"]);
        assert!(
            r.issues(false).is_empty(),
            "non-standard maps skip the bounds check"
        );
    }
}
