//! Player identity binding and recycled-actor coalescing (T1).
//!
//! Rocket League recycles car actor ids on every respawn, demo, and goal reset,
//! so a single player shows up as many short-lived car-actor segments — and an
//! id can even be reused later by a *different* player. The stable identity is
//! the player-replication-info (PRI) actor a car is bound to.
//!
//! [`IdentityResolver`] tracks live `car -> PRI` bindings and `PRI -> name`
//! assignments as the event stream is fed in, accumulating one [`CarSegment`]
//! per live car lifetime. [`coalesce`] then groups segments by PRI into a single
//! continuous [`PlayerTrack`] each, with the boundaries between segments marked
//! as explicit gaps (never carry-forward across them).

use crate::model::{GapReason, PlayerTrack, TrackGap, TrackSample};
use std::collections::HashMap;

/// Sentinel PRI for a car whose binding has not yet arrived.
const UNBOUND: i32 = -1;

/// One continuous lifetime of a single car actor: every position sample it
/// produced while bound to one PRI.
#[derive(Debug, Clone, PartialEq)]
pub struct CarSegment {
    /// The car actor id (not stable across the match).
    pub car_actor: i32,
    /// The PRI this car was bound to (the stable player key); [`UNBOUND`] if the
    /// binding never arrived.
    pub pri: i32,
    /// Samples in arrival (time) order.
    pub samples: Vec<TrackSample>,
}

/// Accumulates `car -> PRI` bindings, `PRI -> name`, and per-car-lifetime
/// segments as the neutral event stream is replayed.
#[derive(Debug, Default)]
pub struct IdentityResolver {
    /// Live binding from a currently-alive car actor to its PRI.
    car_to_pri: HashMap<i32, i32>,
    /// Latest known name for each PRI.
    pri_to_name: HashMap<i32, String>,
    /// Open segments for currently-alive cars.
    open: HashMap<i32, CarSegment>,
    /// Finalized segments (car was deleted, or match ended).
    done: Vec<CarSegment>,
}

impl IdentityResolver {
    pub fn new() -> Self {
        Self::default()
    }

    /// The PRI currently bound to a car, if any.
    pub fn current_pri(&self, car: i32) -> Option<i32> {
        self.car_to_pri.get(&car).copied()
    }

    /// Record a `car -> PRI` binding. Updates an already-open segment that was
    /// still waiting on its binding.
    pub fn on_car_pri(&mut self, car: i32, pri: i32) {
        self.car_to_pri.insert(car, pri);
        if let Some(seg) = self.open.get_mut(&car) {
            if seg.pri == UNBOUND {
                seg.pri = pri;
            }
        }
    }

    /// Record a `PRI -> name` assignment.
    pub fn on_pri_name(&mut self, pri: i32, name: String) {
        self.pri_to_name.insert(pri, name);
    }

    /// Append a position sample for a live car, opening its segment if needed.
    pub fn push_sample(&mut self, car: i32, sample: TrackSample) {
        let pri = self.current_pri(car).unwrap_or(UNBOUND);
        let seg = self.open.entry(car).or_insert_with(|| CarSegment {
            car_actor: car,
            pri,
            samples: Vec::new(),
        });
        // Late binding: stamp the PRI if it arrived after the first sample.
        if seg.pri == UNBOUND && pri != UNBOUND {
            seg.pri = pri;
        }
        seg.samples.push(sample);
    }

    /// Finalize a car actor's segment on deletion (respawn/demo/goal reset) and
    /// drop its stale binding so a recycled id starts clean. No-op for actors
    /// that never held an open car segment (balls, PRIs, misc actors).
    pub fn on_delete(&mut self, actor: i32) {
        self.car_to_pri.remove(&actor);
        if let Some(seg) = self.open.remove(&actor) {
            self.done.push(seg);
        }
    }

    /// Close out all still-open segments (match ended with live cars) and return
    /// every segment plus the final `PRI -> name` map.
    pub fn finish(self) -> (Vec<CarSegment>, HashMap<i32, String>) {
        let mut segments = self.done;
        segments.extend(self.open.into_values());
        (segments, self.pri_to_name)
    }
}

/// Coalesce per-car-lifetime segments into one [`PlayerTrack`] per stable
/// identity (PRI), with gaps between segments marked explicitly.
///
/// Empty segments (a car that bound but never reported a position) are dropped.
/// Segments whose binding never arrived are kept but isolated per car actor so
/// distinct unidentified cars never falsely merge.
pub fn coalesce(
    segments: Vec<CarSegment>,
    pri_to_name: &HashMap<i32, String>,
    pri_to_team: &HashMap<i32, i32>,
    name_to_team: &HashMap<String, i32>,
) -> Vec<PlayerTrack> {
    // Group segments under a stable key. Bound cars group by PRI; unbound cars
    // get a synthetic per-actor key so they remain separate tracks.
    #[derive(PartialEq, Eq, Hash, Clone, Copy)]
    enum Key {
        Pri(i32),
        Orphan(i32),
    }

    let mut groups: HashMap<Key, Vec<CarSegment>> = HashMap::new();
    for seg in segments {
        if seg.samples.is_empty() {
            continue;
        }
        let key = if seg.pri >= 0 {
            Key::Pri(seg.pri)
        } else {
            Key::Orphan(seg.car_actor)
        };
        groups.entry(key).or_default().push(seg);
    }

    let mut tracks: Vec<PlayerTrack> = groups
        .into_iter()
        .map(|(key, mut segs)| {
            // Order segments by when they first appeared.
            segs.sort_by(|a, b| {
                a.samples[0]
                    .t
                    .partial_cmp(&b.samples[0].t)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            // Gaps live at segment boundaries: the player had no live car
            // between one segment's last sample and the next's first.
            let mut gaps = Vec::new();
            for pair in segs.windows(2) {
                let prev_end = pair[0].samples.last().unwrap().t;
                let next_start = pair[1].samples[0].t;
                if next_start > prev_end {
                    gaps.push(TrackGap {
                        start: prev_end,
                        end: next_start,
                        reason: GapReason::Respawn,
                    });
                }
            }

            let num_segments = segs.len();
            let mut samples: Vec<TrackSample> = segs.into_iter().flat_map(|s| s.samples).collect();
            samples.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));

            let pri = match key {
                Key::Pri(p) => p,
                Key::Orphan(_) => UNBOUND,
            };
            let player = pri_to_name
                .get(&pri)
                .cloned()
                .unwrap_or_else(|| "<unknown>".to_string());
            // Prefer the network team binding (stable PRI key); fall back to the
            // header name→team join only when the network never bound a team.
            let team = pri_to_team
                .get(&pri)
                .copied()
                .or_else(|| name_to_team.get(&player).copied());

            PlayerTrack {
                player,
                pri,
                team,
                num_segments,
                samples,
                gaps,
            }
        })
        .collect();

    // Deterministic output: by team, then PRI.
    tracks.sort_by(|a, b| {
        a.team
            .cmp(&b.team)
            .then(a.pri.cmp(&b.pri))
            .then(a.player.cmp(&b.player))
    });
    tracks
}
