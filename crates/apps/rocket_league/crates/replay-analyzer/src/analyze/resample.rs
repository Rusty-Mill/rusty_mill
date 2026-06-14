//! Fixed-rate resampling (T2).
//!
//! The native stream is delta/keyframe encoded at a variable rate, so tracks and
//! the ball are sampled irregularly. Resampling places them on a uniform grid
//! (e.g. 30 Hz) by linear interpolation, so every grid frame carries every
//! *live* actor. Dead/respawning actors (inside a [`crate::model::TrackGap`])
//! are absent — interpolation never bridges a gap.

use crate::model::{GridCar, GridFrame, Kin, TrackGap, TrackSample, Vec3};

use super::reconstruct::Reconstruction;

/// Resample the reconstruction onto a fixed `hz` grid, in world coordinates.
///
/// Returns one [`GridFrame`] per grid tick spanning the interval over which any
/// actor produced samples. Empty if nothing was observed.
pub fn resample(recon: &Reconstruction, hz: f32) -> Vec<GridFrame> {
    assert!(hz > 0.0, "resample rate must be positive");
    let dt = 1.0 / hz;

    let Some((t0, t1)) = sample_span(recon) else {
        return Vec::new();
    };

    let mut ball = Cursor::new(&recon.ball_samples, &[]);
    let mut cars: Vec<(i32, Option<i32>, Cursor)> = recon
        .tracks
        .iter()
        .map(|tr| (tr.pri, tr.team, Cursor::new(&tr.samples, &tr.gaps)))
        .collect();

    // Inclusive grid tick count; guard against float drift on the last tick.
    let ticks = (((t1 - t0) / dt).floor() as usize) + 1;
    let mut frames = Vec::with_capacity(ticks);

    for k in 0..ticks {
        let t = t0 + (k as f32) * dt;
        let mut frame_cars = Vec::new();
        for (pri, team, cur) in &mut cars {
            if let Some((kin, boost)) = cur.at(t) {
                frame_cars.push(GridCar {
                    pri: *pri,
                    team: *team,
                    p: kin.p,
                    v: kin.v,
                    boost,
                });
            }
        }
        frame_cars.sort_by_key(|c| c.pri);
        frames.push(GridFrame {
            t,
            ball: ball.at(t).map(|(kin, _)| kin),
            cars: frame_cars,
        });
    }

    frames
}

/// Earliest and latest sample time across the ball and all tracks.
fn sample_span(recon: &Reconstruction) -> Option<(f32, f32)> {
    let mut lo = f32::MAX;
    let mut hi = f32::MIN;
    let mut seen = false;
    let mut fold = |samples: &[TrackSample]| {
        if let (Some(a), Some(b)) = (samples.first(), samples.last()) {
            seen = true;
            lo = lo.min(a.t);
            hi = hi.max(b.t);
        }
    };
    fold(&recon.ball_samples);
    for tr in &recon.tracks {
        fold(&tr.samples);
    }
    (seen && hi >= lo).then_some((lo, hi))
}

/// A monotonic cursor over one actor's samples + gaps, queried at strictly
/// non-decreasing times (as the grid advances).
struct Cursor<'a> {
    samples: &'a [TrackSample],
    gaps: &'a [TrackGap],
    idx: usize,
}

impl<'a> Cursor<'a> {
    fn new(samples: &'a [TrackSample], gaps: &'a [TrackGap]) -> Self {
        Cursor {
            samples,
            gaps,
            idx: 0,
        }
    }

    /// Interpolated kinematics plus carried boost at time `t`, or `None` if the
    /// actor is not live then (before its first / after its last sample, or
    /// inside a gap). Position/velocity are interpolated; boost is carried from
    /// the most recent sample at or before `t` (boost is step-like, not linear).
    fn at(&mut self, t: f32) -> Option<(Kin, Option<u8>)> {
        let first = self.samples.first()?;
        let last = self.samples.last()?;
        if t < first.t || t > last.t {
            return None;
        }
        if self.gaps.iter().any(|g| t > g.start && t < g.end) {
            return None;
        }

        // Advance to the last sample with time <= t.
        while self.idx + 1 < self.samples.len() && self.samples[self.idx + 1].t <= t {
            self.idx += 1;
        }
        let a = &self.samples[self.idx];
        let kin = match self.samples.get(self.idx + 1) {
            Some(b) => lerp_kin(a, b, t),
            None => Kin { p: a.p, v: a.v }, // t == last sample
        };
        Some((kin, a.boost))
    }
}

/// Linearly interpolate position and velocity between two samples at time `t`.
fn lerp_kin(a: &TrackSample, b: &TrackSample, t: f32) -> Kin {
    let span = b.t - a.t;
    let w = if span > f32::EPSILON {
        ((t - a.t) / span).clamp(0.0, 1.0)
    } else {
        0.0
    };
    Kin {
        p: lerp_vec(a.p, b.p, w),
        v: lerp_vec(a.v, b.v, w),
    }
}

fn lerp_vec(a: Vec3, b: Vec3, w: f32) -> Vec3 {
    Vec3 {
        x: a.x + (b.x - a.x) * w,
        y: a.y + (b.y - a.y) * w,
        z: a.z + (b.z - a.z) * w,
    }
}
