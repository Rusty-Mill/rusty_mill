//! Carry-forward world-state reconstruction.
//!
//! The network stream is delta/keyframe encoded: an actor only re-reports its
//! rigid body when it changes (a stationary car may go silent for many frames).
//! Reconstruction maintains the last-known position/velocity of every live
//! actor and emits a uniform per-frame snapshot ([`FrameOut`]). In the same
//! pass it feeds the [`IdentityResolver`] so car segments can be coalesced into
//! identity-stable [`PlayerTrack`]s.

use crate::decode::{ActorClass, ActorUpdate, DecodedReplay, PriStatKind};
use crate::field::{self, PadKind};
use crate::model::{
    Camera, CarState, FrameOut, PadPickupEvent, PlayerTrack, PowerslideInterval, Rot3, TrackSample,
    Vec3,
};
use std::collections::{HashMap, HashSet};

use super::identity::{coalesce, IdentityResolver};

/// Per-player scoreboard counters captured from the network `PRI_TA:Match*`
/// attributes (keyed by PRI). Authoritative even when the header `PlayerStats[]`
/// array is empty.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PriScore {
    pub score: i32,
    pub goals: i32,
    pub saves: i32,
    pub assists: i32,
    pub shots: i32,
}

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
    /// Authoritative boost-pad pickups (T6), attributed to the collecting PRI.
    pub pickups: Vec<PadPickupEvent>,
    /// Powerslide (handbrake) intervals (T6), per PRI.
    pub powerslides: Vec<PowerslideInterval>,
    /// Per-player scoreboard counters from the network `PRI_TA:Match*` attributes,
    /// keyed by PRI — the fallback when the header `PlayerStats[]` is empty.
    pub pri_scores: HashMap<i32, PriScore>,
    /// Rising edges of the network `PRI_TA:Match{Shots,Saves,Assists}` counters,
    /// one per unit increment, timestamped — the substrate for the Game Timeline.
    pub stat_events: Vec<StatSample>,
    /// Per-PRI car-body product ids from the loadout, as `(blue_body, orange_body)`
    /// — pick by the player's team. Empty when the replay carries no loadout.
    pub pri_body: HashMap<i32, (u32, u32)>,
    /// Per-PRI camera profile (`Camera`), resolved through the camera-settings
    /// actor → PRI link.
    pub pri_camera: HashMap<i32, Camera>,
    /// Per-PRI steering sensitivity (`TAGame.PRI_TA:SteeringSensitivity`).
    pub pri_steer: HashMap<i32, f32>,
}

/// A demolition with attacker/victim resolved to their bound PRI at demo time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DemoSample {
    pub t: f32,
    pub attacker_pri: Option<i32>,
    pub victim_pri: Option<i32>,
}

/// A single scoreboard-counter increment (one shot / save / assist) for a PRI,
/// timestamped at the network update that raised it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StatSample {
    pub t: f32,
    pub pri: i32,
    pub kind: PriStatKind,
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
    let mut pickups: Vec<PadPickupEvent> = Vec::new();
    let mut powerslides: Vec<PowerslideInterval> = Vec::new();
    let mut pri_scores: HashMap<i32, PriScore> = HashMap::new();
    let mut stat_events: Vec<StatSample> = Vec::new();
    let mut pri_body: HashMap<i32, (u32, u32)> = HashMap::new();
    // Camera profile keyed by the camera-settings actor, plus that actor's PRI
    // link and each PRI's steering sensitivity. Joined into pri_camera at the end
    // (decoupled from frame ordering of the settings vs the PRI link).
    let mut cam_actor_settings: HashMap<i32, Camera> = HashMap::new();
    let mut cam_actor_pri: HashMap<i32, i32> = HashMap::new();
    let mut pri_steer: HashMap<i32, f32> = HashMap::new();
    // Car actor -> time its current powerslide (handbrake-held) began.
    let mut handbrake_start: HashMap<i32, f32> = HashMap::new();
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
                ActorUpdate::PickupBoost { instigator_car } => {
                    // The collector is sitting on the pad, so the nearest pad to
                    // its current position identifies which pad (big vs small +
                    // location); the gauge before the pickup gives the real gain.
                    if let Some((p, _)) = car_pv.get(instigator_car) {
                        let before = car_boost
                            .get(instigator_car)
                            .map(|b| field::boost_percent(*b))
                            .unwrap_or(0.0);
                        let (kind, pad, _d) = field::nearest_pad_any(*p);
                        let nominal = field::pad_nominal(kind);
                        let gain = nominal.min((100.0 - before).max(0.0));
                        pickups.push(PadPickupEvent {
                            t: frame.time,
                            pri: resolver.current_pri(*instigator_car).unwrap_or(-1),
                            pad: [pad.0, pad.1],
                            big: matches!(kind, PadKind::Big),
                            gain,
                            overfill: (nominal - gain).max(0.0),
                        });
                    }
                }
                ActorUpdate::PriStat { pri, stat, value } => {
                    let e = pri_scores.entry(*pri).or_default();
                    let prev = match stat {
                        PriStatKind::Score => &mut e.score,
                        PriStatKind::Goals => &mut e.goals,
                        PriStatKind::Saves => &mut e.saves,
                        PriStatKind::Assists => &mut e.assists,
                        PriStatKind::Shots => &mut e.shots,
                    };
                    // Counters are cumulative and replicated on each change; emit a
                    // timeline marker per unit gained on the rising edge (shots /
                    // saves / assists only — goals come from the header).
                    if matches!(
                        stat,
                        PriStatKind::Shots | PriStatKind::Saves | PriStatKind::Assists
                    ) {
                        for _ in *prev..*value {
                            stat_events.push(StatSample {
                                t: frame.time,
                                pri: *pri,
                                kind: *stat,
                            });
                        }
                    }
                    *prev = *value;
                }
                ActorUpdate::Loadout {
                    pri,
                    blue_body,
                    orange_body,
                } => {
                    pri_body.insert(*pri, (*blue_body, *orange_body));
                }
                ActorUpdate::CameraSettings {
                    cam_actor,
                    fov,
                    height,
                    angle,
                    distance,
                    stiffness,
                    swivel,
                    transition,
                } => {
                    cam_actor_settings.insert(
                        *cam_actor,
                        Camera {
                            fov: *fov,
                            height: *height,
                            pitch: *angle,
                            distance: *distance,
                            stiffness: *stiffness,
                            swivel_speed: *swivel,
                            transition_speed: *transition,
                        },
                    );
                }
                ActorUpdate::CameraPri { cam_actor, pri } => {
                    cam_actor_pri.insert(*cam_actor, *pri);
                }
                ActorUpdate::SteeringSensitivity { pri, value } => {
                    pri_steer.insert(*pri, *value);
                }
                ActorUpdate::Handbrake { car, on } => {
                    if *on {
                        handbrake_start.entry(*car).or_insert(frame.time);
                    } else if let Some(start) = handbrake_start.remove(car) {
                        if frame.time > start {
                            powerslides.push(PowerslideInterval {
                                pri: resolver.current_pri(*car).unwrap_or(-1),
                                start,
                                end: frame.time,
                            });
                        }
                    }
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
            // Close an open powerslide before identity forgets this car's PRI.
            if let Some(start) = handbrake_start.remove(da) {
                if frame.time > start {
                    powerslides.push(PowerslideInterval {
                        pri: resolver.current_pri(*da).unwrap_or(-1),
                        start,
                        end: frame.time,
                    });
                }
            }
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

    // Close powerslides still held at the final frame (cars never deleted).
    let last_t = decoded.frames.last().map(|f| f.time).unwrap_or(0.0);
    for (car, start) in handbrake_start.drain() {
        if last_t > start {
            powerslides.push(PowerslideInterval {
                pri: resolver.current_pri(car).unwrap_or(-1),
                start,
                end: last_t,
            });
        }
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

    // Join each camera-settings actor's profile to its PRI.
    let mut pri_camera: HashMap<i32, Camera> = HashMap::new();
    for (cam_actor, cam) in &cam_actor_settings {
        if let Some(&pri) = cam_actor_pri.get(cam_actor) {
            pri_camera.insert(pri, *cam);
        }
    }

    Reconstruction {
        frames,
        tracks,
        ball_samples,
        demos,
        pickups,
        powerslides,
        pri_scores,
        stat_events,
        pri_body,
        pri_camera,
        pri_steer,
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
