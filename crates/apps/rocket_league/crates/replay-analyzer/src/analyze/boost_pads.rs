//! **Boost-pad pickup attribution** (ballchasing parity item #2).
//!
//! ballchasing splits boost collection by pad — big vs small, and "stolen" (a pad
//! taken in the opponent's half). The replay doesn't record pad pickups, but in
//! standard Soccar the only way the boost gauge *rises* is a pad, and there are 34
//! fixed pad locations ([`crate::field`]). So every positive boost-gauge step in a
//! player's track is a pickup, attributed to the nearest pad: its **kind** comes
//! from the step size (only a big pad can add more than a small's 12%) backed by
//! proximity, and **stolen** from whether that pad sits past midfield in the
//! player's attack frame.
//!
//! This is the per-pickup primitive; [`crate::analyze::bcstats`] aggregates it into
//! the per-player big/small/stolen/overfill totals, and a future viewer overlay can
//! reuse the timestamped [`PadPickup`] stream to light pads as they're taken.
//!
//! Caveats (shared with the rest of `bcstats`): boost is sampled, so a step that
//! brackets two pads reads as one larger pickup; non-standard modes with passive
//! boost regen would mis-read regen as pickups (we only target standard Soccar).

use crate::field::{self, PadKind};
use crate::model::PlayerTrack;

/// Minimum boost-gauge gain (percent) to count a step as a pad pickup. The
/// replicated gauge jitters by 1–2 bytes (≤0.8%) between observations; real
/// pickups add ≥ ~5% (a small pad is 12%), so this cleanly drops the jitter
/// without losing genuine pickups (there is a wide empty gap between the two).
const MIN_PICKUP_PCT: f32 = 2.0;
/// Above this gain (percent) a step can only be a big pad — a small grants 12%, so
/// anything well past that is unambiguous even when the sampled position is noisy.
const BIG_GAIN_MIN: f32 = 18.0;
/// A pad sitting beyond this normalized `y` (uu, attack frame) is in the
/// opponent's half ⇒ a "stolen" pickup. Midline pads (`y≈0`) are not stolen.
const STOLEN_Y: f32 = 1.0;

/// One attributed boost-pad pickup.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PadPickup {
    pub t: f32,
    pub kind: PadKind,
    /// World `(x, y)` of the pad this pickup was attributed to (for the viewer
    /// pickup-map overlay).
    pub pad: (f32, f32),
    /// Boost actually added to the tank (percent) — caps at the tank, so a pickup
    /// at high boost adds less than the pad's nominal value.
    pub gain: f32,
    /// Boost wasted because the tank was already partly full (`nominal − gain`).
    pub overfill: f32,
    /// The pad was in the opponent's half (attack frame).
    pub stolen: bool,
}

/// Attribute every boost-gauge increase in a track to a pad. `sign` is the
/// player's team attack sign (`+1`/`-1` from [`crate::model::Resampled`]); it puts
/// the matched pad into the attack frame to decide "stolen".
pub fn pad_pickups(track: &PlayerTrack, sign: i32) -> Vec<PadPickup> {
    let mut out = Vec::new();
    for pair in track.samples.windows(2) {
        let (prev, cur) = (&pair[0], &pair[1]);
        // A step across a respawn gap is a fresh car's spawn boost, not a pickup.
        let across_gap = track
            .gaps
            .iter()
            .any(|g| prev.t <= g.start && cur.t >= g.end);
        if across_gap {
            continue;
        }
        let (Some(pb), Some(cb)) = (prev.boost, cur.boost) else {
            continue;
        };
        if cb <= pb {
            continue;
        }
        let gain = field::boost_percent(cb) - field::boost_percent(pb);
        // Drop sub-pickup gauge jitter (see MIN_PICKUP_PCT).
        if gain < MIN_PICKUP_PCT {
            continue;
        }

        // Kind: a clearly-large step must be a big pad; otherwise trust the nearer
        // pad to the pickup position (handles a big pad grabbed at high boost,
        // where the step is small but the car is sitting on a big pad).
        let p = cur.p.to_arr();
        let kind = if gain > BIG_GAIN_MIN {
            PadKind::Big
        } else {
            field::nearest_pad_any(p).0
        };
        let (pad_pos, _) = field::nearest_pad(p, kind);

        let nominal = field::pad_nominal(kind);
        let overfill = (nominal - gain).max(0.0);
        // Pad y in the player's attack frame (+Y attacking).
        let pad_y = if sign >= 0 { pad_pos.1 } else { -pad_pos.1 };

        out.push(PadPickup {
            t: cur.t,
            kind,
            pad: pad_pos,
            gain,
            overfill,
            stolen: pad_y > STOLEN_Y,
        });
    }
    out
}

/// Per-player pad-pickup aggregates (the ballchasing boost-pad fields).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PadStats {
    pub amount_collected_big: f32,
    pub amount_collected_small: f32,
    pub amount_stolen: f32,
    pub amount_stolen_big: f32,
    pub amount_stolen_small: f32,
    pub count_collected_big: u32,
    pub count_collected_small: u32,
    pub count_stolen_big: u32,
    pub count_stolen_small: u32,
    pub amount_overfill: f32,
    pub amount_overfill_stolen: f32,
}

/// A player's boost economy: pad-pickup breakdown plus jitter-free collected /
/// used totals (percent).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BoostEconomy {
    pub pads: PadStats,
    /// Total boost collected from pads (= big + small) — the real boost gained.
    pub collected: f32,
    /// Total boost used (consumed while boosting), from the conservation budget.
    pub used: f32,
    /// Boost consumed while **supersonic and on the ground** — a best-effort
    /// approximation of ballchasing's `amount_used_while_supersonic`. Summed as raw
    /// gauge decreases over supersonic+grounded sample steps (not the conservation
    /// budget), so it carries the reconstruction noise of the speed/height tracks:
    /// near-exact on clean replays, but ±~25% (occasionally 2×) on sparse ones.
    pub used_supersonic: f32,
}

/// Height band (uu) below which a car centre counts as on the ground — the same
/// `50` floor band `bcstats` uses for the ground/air movement split.
const GROUND_Z: f32 = 50.0;

/// Compute a track's boost economy. `collected` is the sum of attributed pad
/// pickups (so it ignores gauge jitter); `used` falls out of conservation —
/// within a car life `end = start + collected − used`, so `used = collected −
/// Σ(net gauge change)` over non-gap sample steps. This is jitter-free because the
/// net change telescopes (the ±1-byte wiggle cancels), unlike summing raw
/// decreases.
pub fn boost_economy(track: &PlayerTrack, sign: i32) -> BoostEconomy {
    let pads = pad_stats(track, sign);
    let collected = pads.amount_collected_big + pads.amount_collected_small;
    let mut net = 0.0f32;
    let mut used_supersonic = 0.0f32;
    for pair in track.samples.windows(2) {
        let (prev, cur) = (&pair[0], &pair[1]);
        let across_gap = track
            .gaps
            .iter()
            .any(|g| prev.t <= g.start && cur.t >= g.end);
        if across_gap {
            continue;
        }
        if let (Some(pb), Some(cb)) = (prev.boost, cur.boost) {
            let (pp, cc) = (field::boost_percent(pb), field::boost_percent(cb));
            net += cc - pp;
            // Boost burned while supersonic on the ground: a gauge *decrease* at a
            // sample where the car is at supersonic speed and near the floor. The
            // `< 40` guard drops any non-boost reset the gap filter missed.
            let drop = pp - cc;
            let speed = (cur.v.x * cur.v.x + cur.v.y * cur.v.y + cur.v.z * cur.v.z).sqrt();
            if (0.0..40.0).contains(&drop) && speed >= field::SUPERSONIC_SPEED && cur.p.z < GROUND_Z
            {
                used_supersonic += drop;
            }
        }
    }
    BoostEconomy {
        pads,
        collected,
        used: (collected - net).max(0.0),
        used_supersonic,
    }
}

/// Reduce a track's pickups to per-player totals.
pub fn pad_stats(track: &PlayerTrack, sign: i32) -> PadStats {
    let mut s = PadStats::default();
    for p in pad_pickups(track, sign) {
        let big = p.kind == PadKind::Big;
        if big {
            s.amount_collected_big += p.gain;
            s.count_collected_big += 1;
        } else {
            s.amount_collected_small += p.gain;
            s.count_collected_small += 1;
        }
        s.amount_overfill += p.overfill;
        if p.stolen {
            s.amount_stolen += p.gain;
            s.amount_overfill_stolen += p.overfill;
            if big {
                s.amount_stolen_big += p.gain;
                s.count_stolen_big += 1;
            } else {
                s.amount_stolen_small += p.gain;
                s.count_stolen_small += 1;
            }
        }
    }
    s
}
