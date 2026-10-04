//! Derived match events (T4).
//!
//! Touches, possessions, and kickoffs are *inferred* from the resampled
//! kinematics — replays carry no touch/possession markers. Demos and goals are
//! built from authoritative replay attributes (passed in already resolved).
//!
//! Detectors are independent pure functions so each is unit-testable in
//! isolation; [`super::build_canonical`] merges and time-sorts their output.

use crate::decode::PriStatKind;
use crate::model::{Event, GridFrame, PlayerTrack, Resampled, StatKind};
use std::collections::HashMap;

use super::reconstruct::{DemoSample, StatSample};

/// Max ball-center-to-car-center distance for a touch (uu): ball radius plus a
/// car body, with slack for the 30 Hz sampling step.
const TOUCH_RADIUS: f32 = 300.0;
/// Min ball velocity-change magnitude over one grid step to count as a hit
/// (uu/s). Gravity contributes ~20 uu/s per step, far below this; wall/ground
/// bounces clear it but have no car nearby.
const TOUCH_DV: f32 = 300.0;
/// Minimum spacing between two touches by the *same* player (s); suppresses
/// multi-counting a single sustained contact / dribble.
const TOUCH_DEBOUNCE_S: f32 = 0.5;

/// Half-extent for "ball at field center" on x/y (uu) when detecting kickoffs.
const KICKOFF_CENTER_XY: f32 = 6.0;
/// Max ball speed for a kickoff (uu/s): the ball rests at center during the
/// countdown. This (not car spawns) is the robust event signal — post-goal the
/// ball resets to center well before cars finish teleporting to spawns.
const KICKOFF_MAX_SPEED: f32 = 50.0;
/// Minimum spacing between distinct kickoffs (s).
const KICKOFF_SEPARATION_S: f32 = 1.0;

/// Per-victim refractory window for demos (s): a demolished car is out for the
/// ~3 s respawn and must then be driven into again, so a genuine second demo of the
/// same victim comes more than ~3.5 s after the first. The replicated attribute is
/// re-sent up to ~3.5 s after a demolition (duplicates measured 2.6, 3.0 and 3.5 s
/// apart on a real 3v3 match, where 11 events were 8 real demolitions), so the window
/// sits just above that.
const DEMO_REFRACTORY_S: f32 = 4.0;

/// `pri -> (player name, team)` from coalesced tracks.
fn pri_lookup(tracks: &[PlayerTrack]) -> HashMap<i32, (String, Option<i32>)> {
    tracks
        .iter()
        .map(|t| (t.pri, (t.player.clone(), t.team)))
        .collect()
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (dx, dy, dz) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// Detect ball touches: a ball velocity discontinuity with a car close enough to
/// have caused it, attributed to the nearest car. Debounced per player.
pub fn touches(resampled: &Resampled, tracks: &[PlayerTrack]) -> Vec<Event> {
    let lookup = pri_lookup(tracks);
    let mut out = Vec::new();
    let mut last_touch: HashMap<i32, f32> = HashMap::new();

    let frames = &resampled.frames;
    for i in 1..frames.len() {
        let (Some(prev), Some(cur)) = (&frames[i - 1].ball, &frames[i].ball) else {
            continue;
        };
        let dv = dist(cur.v.to_arr(), prev.v.to_arr());
        if dv < TOUCH_DV {
            continue;
        }

        // Nearest car to the ball this frame.
        let bp = cur.p.to_arr();
        let Some(nearest) = frames[i]
            .cars
            .iter()
            .map(|c| (c, dist(c.p.to_arr(), bp)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
        else {
            continue;
        };
        if nearest.1 > TOUCH_RADIUS {
            continue;
        }

        let pri = nearest.0.pri;
        let t = frames[i].t;
        if let Some(prev_t) = last_touch.get(&pri) {
            if t - prev_t < TOUCH_DEBOUNCE_S {
                continue;
            }
        }
        last_touch.insert(pri, t);

        let (player, team) = match lookup.get(&pri) {
            Some((name, team)) => (Some(name.clone()), *team),
            None => (None, nearest.0.team),
        };
        out.push(Event::Touch {
            t,
            pri,
            player,
            team,
        });
    }
    out
}

/// Group consecutive same-team touches into possessions. Touches with no team
/// break a run (and are not themselves a possession).
pub fn possessions(touches: &[Event]) -> Vec<Event> {
    let mut out = Vec::new();
    let mut run: Option<(i32, f32, f32, usize)> = None; // team, start, end, count

    let flush = |run: &mut Option<(i32, f32, f32, usize)>, out: &mut Vec<Event>| {
        if let Some((team, start, end, touches)) = run.take() {
            out.push(Event::Possession {
                team,
                start,
                end,
                touches,
            });
        }
    };

    for ev in touches {
        let Event::Touch { t, team, .. } = ev else {
            continue;
        };
        match team {
            Some(team) => match &mut run {
                Some((rt, _, end, count)) if *rt == *team => {
                    *end = *t;
                    *count += 1;
                }
                _ => {
                    flush(&mut run, &mut out);
                    run = Some((*team, *t, *t, 1));
                }
            },
            None => flush(&mut run, &mut out),
        }
    }
    flush(&mut run, &mut out);
    out
}

/// Detect kickoffs: ball resting at field center with at least two live cars on
/// canonical spawns at ground level. Consecutive matching grid frames collapse
/// into one event at the run's start.
pub fn kickoffs(resampled: &Resampled) -> Vec<Event> {
    let mut out = Vec::new();
    let mut last_t: Option<f32> = None;
    for f in &resampled.frames {
        if is_kickoff(f) {
            let new_kickoff = last_t
                .map(|p| f.t - p > KICKOFF_SEPARATION_S)
                .unwrap_or(true);
            if new_kickoff {
                out.push(Event::Kickoff { t: f.t });
            }
            last_t = Some(f.t);
        }
    }
    out
}

fn is_kickoff(f: &GridFrame) -> bool {
    let Some(b) = &f.ball else { return false };
    let speed = {
        let v = b.v;
        (v.x * v.x + v.y * v.y + v.z * v.z).sqrt()
    };
    b.p.x.abs() < KICKOFF_CENTER_XY
        && b.p.y.abs() < KICKOFF_CENTER_XY
        && (85.0..100.0).contains(&b.p.z)
        && speed < KICKOFF_MAX_SPEED
}

/// Build demo events from captured samples, resolving PRIs to player names.
///
/// The replicated demolish attribute persists and re-replicates, so the same
/// demolition appears many times. A demolished car is gone for the respawn
/// window, so a victim cannot legitimately be re-demoed within it — dedup per
/// victim on that refractory window.
pub fn demos(samples: &[DemoSample], tracks: &[PlayerTrack]) -> Vec<Event> {
    let lookup = pri_lookup(tracks);
    let name_of = |pri: Option<i32>| pri.and_then(|p| lookup.get(&p).map(|(n, _)| n.clone()));

    let mut out = Vec::new();
    let mut last_victim: HashMap<i32, f32> = HashMap::new();
    for d in samples {
        if let Some(victim) = d.victim_pri {
            if let Some(prev_t) = last_victim.get(&victim) {
                if d.t - prev_t < DEMO_REFRACTORY_S {
                    continue;
                }
            }
            last_victim.insert(victim, d.t);
        }
        out.push(Event::Demo {
            t: d.t,
            attacker_pri: d.attacker_pri,
            attacker: name_of(d.attacker_pri),
            victim_pri: d.victim_pri,
            victim: name_of(d.victim_pri),
        });
    }
    out
}

/// Scoreboard counter increments (shots / saves / assists) as timestamped
/// timeline markers, resolved to the player's name + team. Each [`StatSample`]
/// already represents one unit gained on the counter's rising edge.
pub fn stats(samples: &[StatSample], tracks: &[PlayerTrack]) -> Vec<Event> {
    let lookup = pri_lookup(tracks);
    samples
        .iter()
        .filter_map(|s| {
            let kind = match s.kind {
                PriStatKind::Shots => StatKind::Shot,
                PriStatKind::Saves => StatKind::Save,
                PriStatKind::Assists => StatKind::Assist,
                // Score / Goals are not timeline stat markers.
                _ => return None,
            };
            let (player, team) = match lookup.get(&s.pri) {
                Some((n, tm)) => (Some(n.clone()), *tm),
                None => (None, None),
            };
            Some(Event::Stat {
                t: s.t,
                pri: s.pri,
                player,
                team,
                kind,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(pri: i32, name: &str, team: i32) -> PlayerTrack {
        PlayerTrack {
            player: name.into(),
            pri,
            team: Some(team),
            num_segments: 0,
            samples: Vec::new(),
            gaps: Vec::new(),
        }
    }

    #[test]
    fn stats_maps_kinds_resolves_players_and_drops_score_goals() {
        let tracks = [track(7, "Alice", 0), track(9, "Bob", 1)];
        let samples = [
            StatSample {
                t: 10.0,
                pri: 7,
                kind: PriStatKind::Shots,
            },
            StatSample {
                t: 11.0,
                pri: 9,
                kind: PriStatKind::Saves,
            },
            StatSample {
                t: 12.0,
                pri: 7,
                kind: PriStatKind::Assists,
            },
            // Score / Goals never produce timeline markers.
            StatSample {
                t: 13.0,
                pri: 7,
                kind: PriStatKind::Score,
            },
            StatSample {
                t: 14.0,
                pri: 9,
                kind: PriStatKind::Goals,
            },
        ];
        let evs = stats(&samples, &tracks);
        assert_eq!(evs.len(), 3, "only shot/save/assist surface");
        assert!(matches!(
            evs[0],
            Event::Stat {
                kind: StatKind::Shot,
                team: Some(0),
                ref player,
                ..
            } if player.as_deref() == Some("Alice")
        ));
        assert!(matches!(
            evs[1],
            Event::Stat {
                kind: StatKind::Save,
                ..
            }
        ));
        assert!(matches!(
            evs[2],
            Event::Stat {
                kind: StatKind::Assist,
                ..
            }
        ));
    }
}
