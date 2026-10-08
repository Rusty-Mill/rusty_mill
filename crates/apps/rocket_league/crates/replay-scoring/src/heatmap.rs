//! Position-occupancy and touch heatmaps as dependency-free SVG.
//!
//! Everything is rendered in the player's **attacking frame** (they attack `+Y`,
//! drawn upward), so a heatmap reads the same regardless of which side the player
//! started on and is directly comparable across both teams. Pure functions over
//! the canonical match — no image crates; SVG is text and embeds straight into
//! the HTML report (and any later headless-Chromium/weasyprint PDF step).

use std::fmt::Write;

use replay_analyzer::field::{BACK_WALL_Y, SIDE_WALL_X};
use replay_analyzer::model::CanonicalMatch;
use replay_analyzer::model::Event;

/// A binned field occupancy histogram for one player, row-major in `y`
/// (`counts[iy * nx + ix]`; `iy = 0` is the player's *own* goal end).
#[derive(Debug, Clone, PartialEq)]
pub struct Occupancy {
    pub nx: usize,
    pub ny: usize,
    pub counts: Vec<u32>,
    pub max: u32,
    pub samples: u32,
}

impl Occupancy {
    /// Occupancy count of cell `(ix, iy)`.
    pub fn cell(&self, ix: usize, iy: usize) -> u32 {
        self.counts[iy * self.nx + ix]
    }
}

/// The attack-frame sign for a player's team (`+1`/`-1`), defaulting to `+1`.
fn team_sign(m: &CanonicalMatch, pri: i32) -> f32 {
    m.tracks
        .iter()
        .find(|t| t.pri == pri)
        .and_then(|t| t.team)
        .and_then(|tm| m.resampled.team_attack_sign.get(&tm).copied())
        .unwrap_or(1) as f32
}

/// Map an oriented field position to a grid cell, clamping cars that ride the
/// walls / sit in the goal recess into the edge bins rather than dropping them.
fn bin(x: f32, y: f32, nx: usize, ny: usize) -> Option<(usize, usize)> {
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    let fx = ((x + SIDE_WALL_X) / (2.0 * SIDE_WALL_X)).clamp(0.0, 0.999_9);
    let fy = ((y + BACK_WALL_Y) / (2.0 * BACK_WALL_Y)).clamp(0.0, 0.999_9);
    Some(((fx * nx as f32) as usize, (fy * ny as f32) as usize))
}

/// Bin a player's per-frame positions into an `nx × ny` attack-frame histogram.
pub fn occupancy(m: &CanonicalMatch, pri: i32, nx: usize, ny: usize) -> Occupancy {
    let sign = team_sign(m, pri);
    let mut counts = vec![0u32; nx * ny];
    let (mut max, mut samples) = (0u32, 0u32);
    for f in &m.resampled.frames {
        let Some(c) = f.cars.iter().find(|c| c.pri == pri) else {
            continue;
        };
        if let Some((ix, iy)) = bin(c.p.x, c.p.y * sign, nx, ny) {
            let k = iy * nx + ix;
            counts[k] += 1;
            samples += 1;
            max = max.max(counts[k]);
        }
    }
    Occupancy {
        nx,
        ny,
        counts,
        max,
        samples,
    }
}

/// Attack-frame ball positions at the player's own touches (for overlay markers).
pub fn touch_points(m: &CanonicalMatch, pri: i32) -> Vec<(f32, f32)> {
    let sign = team_sign(m, pri);
    let frames = &m.resampled.frames;
    let mut out = Vec::new();
    for e in &m.events {
        let Event::Touch { t, pri: tp, .. } = e else {
            continue;
        };
        if *tp != pri {
            continue;
        }
        if let Some(i) = frame_index_at(frames, *t) {
            if let Some(k) = &frames[i].ball {
                out.push((k.p.x, k.p.y * sign));
            }
        }
    }
    out
}

/// Index of the grid frame nearest `t` (frames are time-sorted ascending).
fn frame_index_at(frames: &[replay_analyzer::model::GridFrame], t: f32) -> Option<usize> {
    if frames.is_empty() {
        return None;
    }
    let i = frames.partition_point(|f| f.t < t);
    if i == 0 {
        Some(0)
    } else if i >= frames.len() {
        Some(frames.len() - 1)
    } else if (t - frames[i - 1].t).abs() <= (frames[i].t - t).abs() {
        Some(i - 1)
    } else {
        Some(i)
    }
}

const CELL: u32 = 20;

/// Render an occupancy grid (+ touch markers) as an inline SVG snippet. The
/// player attacks upward; their own goal sits at the bottom.
pub fn render_svg(occ: &Occupancy, touches: &[(f32, f32)]) -> String {
    let (w, h) = (occ.nx as u32 * CELL, occ.ny as u32 * CELL);
    let (wf, hf) = (w as f32, h as f32);
    let mut s = String::with_capacity(2048 + occ.nx * occ.ny * 80);
    let _ = write!(
        s,
        "<svg viewBox=\"0 0 {w} {h}\" width=\"{w}\" height=\"{h}\" \
         xmlns=\"http://www.w3.org/2000/svg\" role=\"img\">"
    );
    let _ = write!(s, "<rect width=\"{w}\" height=\"{h}\" fill=\"#10151c\"/>");

    // Occupancy cells: hotter = more time spent. iy=0 (own goal) drawn at bottom.
    let denom = occ.max.max(1) as f32;
    for iy in 0..occ.ny {
        for ix in 0..occ.nx {
            let c = occ.cell(ix, iy);
            if c == 0 {
                continue;
            }
            let intensity = (c as f32 / denom).clamp(0.0, 1.0);
            let op = 0.12 + 0.88 * intensity;
            let x = ix as u32 * CELL;
            let y = (occ.ny - 1 - iy) as u32 * CELL;
            let _ = write!(
                s,
                "<rect x=\"{x}\" y=\"{y}\" width=\"{CELL}\" height=\"{CELL}\" \
                 fill=\"#ff5a3c\" fill-opacity=\"{op:.3}\"/>"
            );
        }
    }

    // Field furniture: outline, halfway line, centre mark, both goal mouths.
    let mid = h / 2;
    let gw = wf * 0.36;
    let gx = (wf - gw) / 2.0;
    let gh = hf * 0.05;
    let _ = write!(
        s,
        "<g fill=\"none\" stroke=\"#cdd6e0\" stroke-opacity=\"0.5\" stroke-width=\"1.5\">\
         <rect x=\"0.75\" y=\"0.75\" width=\"{:.1}\" height=\"{:.1}\"/>\
         <line x1=\"0\" y1=\"{mid}\" x2=\"{w}\" y2=\"{mid}\"/>\
         <circle cx=\"{:.1}\" cy=\"{mid}\" r=\"{:.1}\"/>\
         <rect x=\"{gx:.1}\" y=\"0\" width=\"{gw:.1}\" height=\"{gh:.1}\"/>\
         <rect x=\"{gx:.1}\" y=\"{:.1}\" width=\"{gw:.1}\" height=\"{gh:.1}\"/></g>",
        wf - 1.5,
        hf - 1.5,
        wf / 2.0,
        wf * 0.10,
        hf - gh,
    );

    // Touch markers (attack-frame ball positions).
    for (x, y) in touches {
        let sx = (x + SIDE_WALL_X) / (2.0 * SIDE_WALL_X) * wf;
        let sy = (1.0 - (y + BACK_WALL_Y) / (2.0 * BACK_WALL_Y)) * hf;
        let _ = write!(
            s,
            "<circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"2.6\" fill=\"#ffe08a\" \
             stroke=\"#1a1f27\" stroke-width=\"0.6\"/>",
            sx.clamp(0.0, wf),
            sy.clamp(0.0, hf)
        );
    }

    s.push_str("</svg>");
    s
}
