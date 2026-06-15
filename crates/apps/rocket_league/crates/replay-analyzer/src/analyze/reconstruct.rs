//! Carry-forward world-state reconstruction.
//!
//! The network stream is delta/keyframe encoded: an actor only re-reports its
//! rigid body when it changes (a stationary car may go silent for many frames).
//! Reconstruction maintains the last-known position/velocity of every live
//! actor and emits a uniform per-frame snapshot ([`FrameOut`]). In the same
//! pass it feeds the [`IdentityResolver`] so car segments can be coalesced into
//! identity-stable [`PlayerTrack`]s.

use crate::decode::{ActorClass, ActorUpdate, DecodedReplay};
use crate::model::{CarState, FrameOut, PlayerTrack, Rot3, TrackSample, Vec3};
use std::collections::{HashMap, HashSet};

use super::identity::{coalesce, IdentityResolver};

/// Output of a reconstruction pass.
#[derive(Debug, Clone, PartialEq)]
pub struct Reconstruction {
    /// Per-frame world state, one entry per network frame.
    pub frames: Vec<FrameOut>,
    /// Coalesced per-player tracks (T1).
    pub tracks: Vec<PlayerTrack>,
    /// Raw ball rigid-body observations (t, p, v), in observation order, for
    /// resampling. Carry-forward is *not* applied here.
    pub ball_samples: Vec<TrackSample>,
    /// Demolitions captured with the players (PRIs) bound at the demo time.
    pub demos: Vec<DemoSample>,
}

/// A demolition with attacker/victim resolved to their bound PRI at demo time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DemoSample {
    pub t: f32,
    pub attacker_pri: Option<i32>,
    pub victim_pri: Option<i32>,
}

/// Reconstruct per-frame world state and coalesced player tracks from a neutral
/// decoded replay.
pub fn reconstruct(decoded: &DecodedReplay) -> Reconstruction {
    let name_to_team: HashMap<String, i32> = decoded
        .meta
        .players
        .iter()
        .map(|p| (p.name.clone(), p.team))
        .collect();

    // Network-sourced team binding (`Engine.PlayerReplicationInfo:Team`), keyed by
    // the stable PRI — preferred over the fragile header name→team join below.
    let mut pri_to_team: HashMap<i32, i32> = HashMap::new();

    let mut actor_kind: HashMap<i32, ActorClass> = HashMap::new();
    let mut ball_pv: Option<([f32; 3], [f32; 3])> = None;
    let mut car_pv: HashMap<i32, ([f32; 3], [f32; 3])> = HashMap::new();
    let mut car_rot: HashMap<i32, [f32; 3]> = HashMap::new();
    // Boost (T3): boost component actor -> its car, and car -> last boost byte.
    let mut comp_to_car: HashMap<i32, i32> = HashMap::new();
    let mut car_boost: HashMap<i32, u8> = HashMap::new();
    let mut ball_samples: Vec<TrackSample> = Vec::new();
    let mut demos: Vec<DemoSample> = Vec::new();
    let mut resolver = IdentityResolver::new();

    let mut frames: Vec<FrameOut> = Vec::with_capacity(decoded.frames.len());

    for frame in &decoded.frames {
        for na in &frame.new_actors {
            actor_kind.insert(na.actor_id, na.class);
        }

        for upd in &frame.updates {
            match upd {
                ActorUpdate::CarPri { car, pri } => resolver.on_car_pri(*car, *pri),
                ActorUpdate::PriName { pri, name } => resolver.on_pri_name(*pri, name.clone()),
                ActorUpdate::PriTeam { pri, team } => {
                    pri_to_team.insert(*pri, *team);
                }
                ActorUpdate::CompVehicle { comp, car } => {
                    comp_to_car.insert(*comp, *car);
                }
                ActorUpdate::BoostAmount { comp, amount } => {
                    if let Some(car) = comp_to_car.get(comp) {
                        car_boost.insert(*car, *amount);
                    }
                }
                ActorUpdate::Demolish {
                    attacker_car,
                    victim_car,
                } => {
                    demos.push(DemoSample {
                        t: frame.time,
                        attacker_pri: resolver.current_pri(*attacker_car),
                        victim_pri: resolver.current_pri(*victim_car),
                    });
                }
                ActorUpdate::RigidBody {
                    actor, p, v, rot, ..
                } => {
                    match actor_kind.get(actor).copied() {
                        // Latest ball-classified rigid body wins per frame.
                        // Static secondary `Ball` actors (some non-Soccar modes)
                        // are dropped after the pass (see `moving_ball_actors`).
                        Some(ActorClass::Ball) => {
                            ball_pv = Some((*p, *v));
                            ball_samples.push(TrackSample {
                                t: frame.time,
                                actor_id: *actor,
                                p: Vec3::from_arr(*p),
                                v: Vec3::from_arr(*v),
                                boost: None,
                                rot: None,
                            });
                        }
                        Some(ActorClass::Car) => {
                            car_pv.insert(*actor, (*p, *v));
                            car_rot.insert(*actor, *rot);
                            resolver.push_sample(
                                *actor,
                                TrackSample {
                                    t: frame.time,
                                    actor_id: *actor,
                                    p: Vec3::from_arr(*p),
                                    v: Vec3::from_arr(*v),
                                    boost: car_boost.get(actor).copied(),
                                    rot: Some(Rot3::from_arr(*rot)),
                                },
                            );
                        }
                        _ => {}
                    }
                }
            }
        }

        for da in &frame.deleted {
            car_pv.remove(da);
            car_rot.remove(da);
            car_boost.remove(da);
            comp_to_car.remove(da);
            resolver.on_delete(*da);
            actor_kind.remove(da);
        }

        let ball = ball_pv.map(|(p, _)| Vec3::from_arr(p));
        let mut cars: Vec<CarState> = car_pv
            .iter()
            .map(|(aid, (p, v))| CarState {
                actor_id: *aid,
                pri: resolver.current_pri(*aid).unwrap_or(-1),
                player: None, // resolved after the walk, once names are final
                p: Vec3::from_arr(*p),
                v: Vec3::from_arr(*v),
                boost: car_boost.get(aid).copied(),
                rot: car_rot.get(aid).map(|r| Rot3::from_arr(*r)),
            })
            .collect();
        cars.sort_by_key(|c| c.actor_id);

        frames.push(FrameOut {
            t: frame.time,
            ball,
            cars,
        });
    }

    let (segments, pri_to_name) = resolver.finish();

    // Resolve per-frame car names now that PRI->name is final. The car's PRI was
    // captured per frame, so this stays correct even when an actor id is recycled
    // by a different player later in the match.
    for frame in &mut frames {
        for car in &mut frame.cars {
            car.player = pri_to_name.get(&car.pri).cloned();
        }
    }

    // Multi-ball robustness: the ball actor id recycles across goals (several
    // ids, all moving). Some non-Soccar modes additionally spawn a *stationary*
    // secondary `Ball`-named actor (a decoy near origin) that the substring
    // classifier also flags. Keep only ball actors that actually move, then
    // rebuild each frame's ball by carry-forward over them. No-op for Soccar.
    let moving = moving_ball_actors(&ball_samples, 250.0);
    if !moving.is_empty() {
        ball_samples.retain(|s| moving.contains(&s.actor_id));
        let mut si = 0;
        let mut cur: Option<Vec3> = None;
        for frame in &mut frames {
            while si < ball_samples.len() && ball_samples[si].t <= frame.t {
                cur = Some(ball_samples[si].p);
                si += 1;
            }
            frame.ball = cur;
        }
    }

    let tracks = coalesce(segments, &pri_to_name, &pri_to_team, &name_to_team);

    Reconstruction {
        frames,
        tracks,
        ball_samples,
        demos,
    }
}

/// Ball-classified actor ids whose observed positions span at least `min_diag`
/// (uu) — i.e. that actually move. Filters out stationary secondary `Ball`
/// actors while keeping every recycled life of the real ball.
fn moving_ball_actors(samples: &[TrackSample], min_diag: f32) -> HashSet<i32> {
    let mut bb: HashMap<i32, ([f32; 3], [f32; 3])> = HashMap::new();
    for s in samples {
        let p = s.p.to_arr();
        let e = bb
            .entry(s.actor_id)
            .or_insert(([f32::MAX; 3], [f32::MIN; 3]));
        for ((lo, hi), &pi) in e.0.iter_mut().zip(e.1.iter_mut()).zip(p.iter()) {
            *lo = lo.min(pi);
            *hi = hi.max(pi);
        }
    }
    bb.into_iter()
        .filter_map(|(id, (mn, mx))| {
            let d = ((mx[0] - mn[0]).powi(2) + (mx[1] - mn[1]).powi(2) + (mx[2] - mn[2]).powi(2))
                .sqrt();
            (d >= min_diag).then_some(id)
        })
        .collect()
}

/// Min/max of every reconstructed ball position, for arena-bounds validation.
/// Returns `None` if no frame ever observed the ball.
pub fn ball_bounds(frames: &[FrameOut]) -> Option<([f32; 3], [f32; 3])> {
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    let mut seen = false;
    for f in frames {
        if let Some(b) = &f.ball {
            seen = true;
            let p = b.to_arr();
            for i in 0..3 {
                min[i] = min[i].min(p[i]);
                max[i] = max[i].max(p[i]);
            }
        }
    }
    seen.then_some((min, max))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(actor: i32, x: f32) -> TrackSample {
        TrackSample {
            t: 0.0,
            actor_id: actor,
            p: Vec3 { x, y: 0.0, z: 0.0 },
            v: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            boost: None,
            rot: None,
        }
    }

    #[test]
    fn moving_ball_actors_drops_static_decoy() {
        // Actor 1 ranges across the field; actor 2 is a near-origin decoy.
        let samples = vec![
            sample(1, -3000.0),
            sample(1, 3000.0),
            sample(2, 1.0),
            sample(2, 2.0),
        ];
        let moving = moving_ball_actors(&samples, 250.0);
        assert!(moving.contains(&1), "the real ball must be kept");
        assert!(!moving.contains(&2), "the static decoy must be dropped");
    }

    #[test]
    fn moving_ball_actors_keeps_recycled_lives() {
        // Two ids, both moving (the real ball recycled across a goal).
        let samples = vec![
            sample(1, -2000.0),
            sample(1, 2000.0),
            sample(2, -1500.0),
            sample(2, 1500.0),
        ];
        let moving = moving_ball_actors(&samples, 250.0);
        assert!(
            moving.contains(&1) && moving.contains(&2),
            "both lives kept"
        );
    }
}
