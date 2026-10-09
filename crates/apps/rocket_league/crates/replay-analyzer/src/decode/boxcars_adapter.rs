//! `boxcars` adapter for the [`ReplayParser`] port.
//!
//! Translates the decoded network stream into the neutral [`DecodedReplay`],
//! emitting only the attributes reconstruction consumes (rigid bodies, car->PRI
//! links, PRI names) plus header metadata. All `boxcars`-specific knowledge —
//! object-name lookups, attribute matching — is confined here.

use boxcars::attributes::ActiveActor;
use boxcars::{Attribute, HeaderProp, ObjectId, ParserBuilder, Replay};

use super::{
    ActorClass, ActorUpdate, DecodeError, DecodedReplay, GoalInfo, NewActorEvent, PriStatKind,
    RawFrame, ReplayMeta, ReplayParser,
};
use crate::model::PlayerMeta;
use std::collections::BTreeMap;

/// Object string for a rigid-body (position/velocity) update.
const OBJ_RB_STATE: &str = "TAGame.RBActor_TA:ReplicatedRBState";
/// Object string linking a car (Pawn) to its player-replication-info actor.
const OBJ_CAR_PRI: &str = "Engine.Pawn:PlayerReplicationInfo";
/// Object string carrying a PRI's player name.
const OBJ_PRI_NAME: &str = "Engine.PlayerReplicationInfo:PlayerName";
/// Object string carrying a PRI's team assignment (an [`ActiveActor`] link to a
/// team actor whose archetype is `Archetypes.Teams.Team0`/`Team1`).
const OBJ_PRI_TEAM: &str = "Engine.PlayerReplicationInfo:Team";
/// Object string linking a car component to its car.
const OBJ_COMP_VEHICLE: &str = "TAGame.CarComponent_TA:Vehicle";
/// Object strings carrying a boost amount (byte) — two encodings exist.
const OBJ_BOOST_AMOUNT: &str = "TAGame.CarComponent_Boost_TA:ReplicatedBoostAmount";
const OBJ_BOOST_REPL: &str = "TAGame.CarComponent_Boost_TA:ReplicatedBoost";
/// Object strings carrying a demolition — base and extended encodings.
const OBJ_DEMOLISH: &str = "TAGame.Car_TA:ReplicatedDemolish";
const OBJ_DEMOLISH_EXT: &str = "TAGame.Car_TA:ReplicatedDemolishExtended";
/// Object strings carrying a boost-pad pickup — base and "new" encodings. Both
/// carry an `instigator` (the collecting car) and a picked-up flag; a real
/// collection is one with `Some(instigator)`.
const OBJ_PICKUP: &str = "TAGame.VehiclePickup_TA:ReplicatedPickupData";
const OBJ_PICKUP_NEW: &str = "TAGame.VehiclePickup_TA:NewReplicatedPickupData";
/// Object string carrying a car's replicated handbrake (powerslide) boolean.
const OBJ_HANDBRAKE: &str = "TAGame.Vehicle_TA:bReplicatedHandbrake";
/// Object strings carrying a PRI's loadout: `ClientLoadouts` is a per-team pair
/// (`TeamLoadout`), `ClientLoadout` is a single `Loadout`. Both expose `body`.
const OBJ_LOADOUTS: &str = "TAGame.PRI_TA:ClientLoadouts";
const OBJ_LOADOUT: &str = "TAGame.PRI_TA:ClientLoadout";
/// Object strings for the camera profile (`CamSettings` on a camera-settings
/// actor), that actor's PRI link, and a PRI's steering sensitivity (a float).
const OBJ_CAM_SETTINGS: &str = "TAGame.CameraSettingsActor_TA:ProfileSettings";
const OBJ_CAM_PRI: &str = "TAGame.CameraSettingsActor_TA:PRI";
const OBJ_STEER: &str = "TAGame.PRI_TA:SteeringSensitivity";
/// Object strings carrying the per-player scoreboard counters on the PRI. These
/// are replicated even when the header `PlayerStats[]` array is empty.
const OBJ_PRI_MATCH: [(&str, PriStatKind); 5] = [
    ("TAGame.PRI_TA:MatchScore", PriStatKind::Score),
    ("TAGame.PRI_TA:MatchGoals", PriStatKind::Goals),
    ("TAGame.PRI_TA:MatchSaves", PriStatKind::Saves),
    ("TAGame.PRI_TA:MatchAssists", PriStatKind::Assists),
    ("TAGame.PRI_TA:MatchShots", PriStatKind::Shots),
];

/// The real, resolved `boxcars` version (injected by `build.rs`).
pub const BOXCARS_VERSION: &str = env!("BOXCARS_VERSION");

/// `boxcars`-backed replay parser.
#[derive(Debug, Default, Clone, Copy)]
pub struct BoxcarsParser;

impl BoxcarsParser {
    pub fn new() -> Self {
        BoxcarsParser
    }
}

impl ReplayParser for BoxcarsParser {
    fn parse(&self, data: &[u8]) -> Result<DecodedReplay, DecodeError> {
        let replay = ParserBuilder::new(data)
            .on_error_check_crc()
            .must_parse_network_data()
            .parse()
            .map_err(|e| DecodeError::Parse(e.to_string()))?;

        let meta = extract_meta(&replay);
        let frames = extract_frames(&replay)?;
        Ok(DecodedReplay { meta, frames })
    }
}

/// Convert a rotation quaternion to Euler `[pitch, yaw, roll]` (radians),
/// standard ZYX (yaw-pitch-roll) extraction, with pitch clamped at the poles.
fn quat_to_euler(q: &boxcars::Quaternion) -> [f32; 3] {
    let (x, y, z, w) = (q.x, q.y, q.z, q.w);

    let sinr_cosp = 2.0 * (w * x + y * z);
    let cosr_cosp = 1.0 - 2.0 * (x * x + y * y);
    let roll = sinr_cosp.atan2(cosr_cosp);

    let sinp = 2.0 * (w * y - z * x);
    let pitch = if sinp.abs() >= 1.0 {
        (std::f32::consts::FRAC_PI_2).copysign(sinp)
    } else {
        sinp.asin()
    };

    let siny_cosp = 2.0 * (w * z + x * y);
    let cosy_cosp = 1.0 - 2.0 * (y * y + z * z);
    let yaw = siny_cosp.atan2(cosy_cosp);

    [pitch, yaw, roll]
}

/// Resolve an object name to its [`ObjectId`], if present in this replay.
fn object_id(replay: &Replay, name: &str) -> Option<ObjectId> {
    replay
        .objects
        .iter()
        .position(|o| o == name)
        .map(|i| ObjectId(i as i32))
}

fn header_prop<'a>(replay: &'a Replay, key: &str) -> Option<&'a HeaderProp> {
    replay
        .properties
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
}

fn prop_i32(replay: &Replay, key: &str) -> Option<i32> {
    match header_prop(replay, key)? {
        HeaderProp::Int(v) => Some(*v),
        HeaderProp::QWord(v) => Some(*v as i32),
        _ => None,
    }
}

fn prop_f32(replay: &Replay, key: &str) -> Option<f32> {
    match header_prop(replay, key)? {
        HeaderProp::Float(v) => Some(*v),
        HeaderProp::Int(v) => Some(*v as f32),
        _ => None,
    }
}

fn prop_str(replay: &Replay, key: &str) -> Option<String> {
    match header_prop(replay, key)? {
        HeaderProp::Str(s) | HeaderProp::Name(s) => Some(s.clone()),
        _ => None,
    }
}

/// `YYYY-MM-DD HH-MM-SS` (the header `Date` format) as unix seconds, reading it as UTC.
fn parse_header_date(s: &str) -> Option<u64> {
    let n: Vec<i64> = s
        .split(|c: char| !c.is_ascii_digit())
        .filter(|t| !t.is_empty())
        .map(|t| t.parse().ok())
        .collect::<Option<_>>()?;
    let [y, m, d, hh, mm, ss] = n[..] else {
        return None;
    };
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || hh > 23 || mm > 59 || ss > 59 {
        return None;
    }
    // Days from civil (Howard Hinnant).
    let y = y - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + hh * 3_600 + mm * 60 + ss).ok()
}

/// Extract header-sourced metadata (map, team sizes, scores, player stats).
fn extract_meta(replay: &Replay) -> ReplayMeta {
    let mut team_scores = BTreeMap::new();
    if let Some(s0) = prop_i32(replay, "Team0Score") {
        team_scores.insert(0, s0);
    }
    if let Some(s1) = prop_i32(replay, "Team1Score") {
        team_scores.insert(1, s1);
    }

    ReplayMeta {
        parser_version: format!("boxcars-{BOXCARS_VERSION}"),
        map: prop_str(replay, "MapName"),
        team_size: prop_i32(replay, "TeamSize"),
        record_fps: prop_f32(replay, "RecordFPS"),
        played_at: prop_str(replay, "Date")
            .as_deref()
            .and_then(parse_header_date),
        team_scores,
        players: extract_players(replay),
        goals: extract_goals(replay),
    }
}

/// Parse the header `Goals` array (`frame`, `PlayerName`, `PlayerTeam`).
fn extract_goals(replay: &Replay) -> Vec<GoalInfo> {
    let mut out = Vec::new();
    let Some(HeaderProp::Array(rows)) = header_prop(replay, "Goals") else {
        return out;
    };
    for row in rows {
        let find = |k: &str| row.iter().find(|(rk, _)| rk == k).map(|(_, v)| v);
        let frame = match find("frame") {
            Some(HeaderProp::Int(n)) => *n,
            Some(HeaderProp::QWord(n)) => *n as i32,
            _ => continue,
        };
        let scorer = match find("PlayerName") {
            Some(HeaderProp::Str(s) | HeaderProp::Name(s)) => Some(s.clone()),
            _ => None,
        };
        let team = match find("PlayerTeam") {
            Some(HeaderProp::Int(n)) => Some(*n),
            Some(HeaderProp::QWord(n)) => Some(*n as i32),
            _ => None,
        };
        out.push(GoalInfo {
            frame,
            scorer,
            team,
        });
    }
    out
}

/// Parse the header `PlayerStats` array into [`PlayerMeta`] rows.
fn extract_players(replay: &Replay) -> Vec<PlayerMeta> {
    let mut out = Vec::new();
    let Some(HeaderProp::Array(rows)) = header_prop(replay, "PlayerStats") else {
        return out;
    };
    for row in rows {
        let get_i = |k: &str| -> i32 {
            row.iter()
                .find(|(rk, _)| rk == k)
                .and_then(|(_, v)| match v {
                    HeaderProp::Int(n) => Some(*n),
                    HeaderProp::QWord(n) => Some(*n as i32),
                    _ => None,
                })
                .unwrap_or(0)
        };
        let name = row
            .iter()
            .find(|(rk, _)| rk == "Name")
            .and_then(|(_, v)| match v {
                HeaderProp::Str(s) | HeaderProp::Name(s) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "<unknown>".into());
        out.push(PlayerMeta {
            name,
            team: get_i("Team"),
            score: get_i("Score"),
            goals: get_i("Goals"),
            assists: get_i("Assists"),
            saves: get_i("Saves"),
            shots: get_i("Shots"),
            platform_id: platform_id(row),
            // Car + camera come from the network stream, joined in `build_canonical`.
            car_id: None,
            car_name: None,
            camera: None,
            steering_sensitivity: None,
        });
    }
    out
}

/// The stable `"<platform>:<online id>"` for one `PlayerStats` row, or `None`
/// for bots, unknown platforms, and a zero/absent id.
fn platform_id(row: &[(String, HeaderProp)]) -> Option<String> {
    let field = |k: &str| row.iter().find(|(rk, _)| rk == k).map(|(_, v)| v);
    if matches!(field("bBot"), Some(HeaderProp::Bool(true))) {
        return None;
    }
    let platform = match field("Platform")? {
        HeaderProp::Byte { value: Some(v), .. } => platform_slug(v)?,
        _ => return None,
    };
    let id = match field("OnlineID")? {
        HeaderProp::QWord(n) if *n != 0 => n.to_string(),
        HeaderProp::Str(s) | HeaderProp::Name(s) if !s.is_empty() && s != "0" => s.clone(),
        _ => return None,
    };
    Some(format!("{platform}:{id}"))
}

/// Short lowercase name for a header `OnlinePlatform_*` value. `Dingo` is
/// Rocket League's internal name for Xbox. Unrecognized platforms yield `None`
/// rather than an id whose meaning we would be guessing at.
fn platform_slug(value: &str) -> Option<&'static str> {
    match value.strip_prefix("OnlinePlatform_")? {
        "Steam" => Some("steam"),
        "PS4" | "PS5" | "PS3" => Some("psn"),
        "Dingo" | "XboxOne" | "Xbox" => Some("xbox"),
        "Epic" => Some("epic"),
        "NX" | "Switch" => Some("switch"),
        _ => None,
    }
}

/// Walk the network frames, emitting the neutral event stream.
fn extract_frames(replay: &Replay) -> Result<Vec<RawFrame>, DecodeError> {
    let network = replay
        .network_frames
        .as_ref()
        .ok_or(DecodeError::NoNetworkData)?;

    let rb_state_id = object_id(replay, OBJ_RB_STATE);
    let car_pri_id = object_id(replay, OBJ_CAR_PRI);
    let pri_name_id = object_id(replay, OBJ_PRI_NAME);
    let pri_team_id = object_id(replay, OBJ_PRI_TEAM);
    let comp_vehicle_id = object_id(replay, OBJ_COMP_VEHICLE);
    let boost_amount_id = object_id(replay, OBJ_BOOST_AMOUNT);
    let boost_repl_id = object_id(replay, OBJ_BOOST_REPL);
    let demolish_id = object_id(replay, OBJ_DEMOLISH);
    let demolish_ext_id = object_id(replay, OBJ_DEMOLISH_EXT);
    let pickup_id = object_id(replay, OBJ_PICKUP);
    let pickup_new_id = object_id(replay, OBJ_PICKUP_NEW);
    let handbrake_id = object_id(replay, OBJ_HANDBRAKE);
    let loadouts_id = object_id(replay, OBJ_LOADOUTS);
    let loadout_id = object_id(replay, OBJ_LOADOUT);
    let cam_settings_id = object_id(replay, OBJ_CAM_SETTINGS);
    let cam_pri_id = object_id(replay, OBJ_CAM_PRI);
    let steer_id = object_id(replay, OBJ_STEER);
    // (object id, stat kind) for the per-player scoreboard counters present.
    let pri_match_ids: Vec<(ObjectId, PriStatKind)> = OBJ_PRI_MATCH
        .iter()
        .filter_map(|(name, kind)| object_id(replay, name).map(|id| (id, *kind)))
        .collect();

    let classify = |obj: ObjectId| -> ActorClass {
        let n = replay
            .objects
            .get(obj.0 as usize)
            .map(String::as_str)
            .unwrap_or("");
        if n.contains("Ball") {
            ActorClass::Ball
        } else if n.contains("Car") {
            ActorClass::Car
        } else {
            ActorClass::Other
        }
    };

    // Team actors (`Archetypes.Teams.Team0`/`Team1`) carry the 0/1 index a PRI's
    // `Engine.PlayerReplicationInfo:Team` link points at. Pre-scan every spawn so
    // the index resolves regardless of spawn-vs-link frame ordering.
    let mut team_actor: BTreeMap<i32, i32> = BTreeMap::new();
    for frame in &network.frames {
        for na in &frame.new_actors {
            let n = replay
                .objects
                .get(na.object_id.0 as usize)
                .map(String::as_str)
                .unwrap_or("");
            let idx = if n.ends_with("Teams.Team0") {
                0
            } else if n.ends_with("Teams.Team1") {
                1
            } else {
                continue;
            };
            team_actor.insert(na.actor_id.0, idx);
        }
    }

    let mut frames = Vec::with_capacity(network.frames.len());
    for frame in &network.frames {
        let mut raw = RawFrame {
            time: frame.time,
            delta: frame.delta,
            new_actors: Vec::with_capacity(frame.new_actors.len()),
            updates: Vec::with_capacity(frame.updated_actors.len()),
            deleted: Vec::with_capacity(frame.deleted_actors.len()),
        };

        for na in &frame.new_actors {
            raw.new_actors.push(NewActorEvent {
                actor_id: na.actor_id.0,
                class: classify(na.object_id),
            });
        }

        for ua in &frame.updated_actors {
            let oid = Some(ua.object_id);
            if oid == car_pri_id {
                if let Attribute::ActiveActor(ActiveActor { actor, .. }) = &ua.attribute {
                    if actor.0 >= 0 {
                        raw.updates.push(ActorUpdate::CarPri {
                            car: ua.actor_id.0,
                            pri: actor.0,
                        });
                    }
                }
            } else if oid == pri_name_id {
                if let Attribute::String(name) = &ua.attribute {
                    raw.updates.push(ActorUpdate::PriName {
                        pri: ua.actor_id.0,
                        name: name.clone(),
                    });
                }
            } else if oid == pri_team_id {
                if let Attribute::ActiveActor(ActiveActor { actor, .. }) = &ua.attribute {
                    if let Some(&team) = team_actor.get(&actor.0) {
                        raw.updates.push(ActorUpdate::PriTeam {
                            pri: ua.actor_id.0,
                            team,
                        });
                    }
                }
            } else if oid == rb_state_id {
                if let Attribute::RigidBody(rb) = &ua.attribute {
                    let p = [rb.location.x, rb.location.y, rb.location.z];
                    let v = rb
                        .linear_velocity
                        .map(|lv| [lv.x, lv.y, lv.z])
                        .unwrap_or([0.0; 3]);
                    raw.updates.push(ActorUpdate::RigidBody {
                        actor: ua.actor_id.0,
                        p,
                        v,
                        rot: quat_to_euler(&rb.rotation),
                        sleeping: rb.sleeping,
                    });
                }
            } else if oid == comp_vehicle_id {
                if let Attribute::ActiveActor(ActiveActor { actor, .. }) = &ua.attribute {
                    if actor.0 >= 0 {
                        raw.updates.push(ActorUpdate::CompVehicle {
                            comp: ua.actor_id.0,
                            car: actor.0,
                        });
                    }
                }
            } else if oid == boost_amount_id {
                if let Attribute::Byte(amount) = &ua.attribute {
                    raw.updates.push(ActorUpdate::BoostAmount {
                        comp: ua.actor_id.0,
                        amount: *amount,
                    });
                }
            } else if oid == boost_repl_id {
                if let Attribute::ReplicatedBoost(rb) = &ua.attribute {
                    raw.updates.push(ActorUpdate::BoostAmount {
                        comp: ua.actor_id.0,
                        amount: rb.boost_amount,
                    });
                }
            } else if oid == demolish_id {
                if let Attribute::Demolish(d) = &ua.attribute {
                    if d.attacker.0 >= 0 && d.victim.0 >= 0 && d.attacker != d.victim {
                        raw.updates.push(ActorUpdate::Demolish {
                            attacker_car: d.attacker.0,
                            victim_car: d.victim.0,
                        });
                    }
                }
            } else if oid == demolish_ext_id {
                if let Attribute::DemolishExtended(d) = &ua.attribute {
                    if d.attacker.actor.0 >= 0 && d.victim.actor.0 >= 0 {
                        raw.updates.push(ActorUpdate::Demolish {
                            attacker_car: d.attacker.actor.0,
                            victim_car: d.victim.actor.0,
                        });
                    }
                }
            } else if oid == pickup_id || oid == pickup_new_id {
                // Both pickup encodings carry an `instigator` (the collecting car)
                // and a picked-up flag; the *available-again* updates have no
                // instigator. So a real collection is exactly an update whose
                // instigator is present.
                let instigator = match &ua.attribute {
                    Attribute::Pickup(p) => p.instigator,
                    Attribute::PickupNew(p) => p.instigator,
                    _ => None,
                };
                if let Some(car) = instigator {
                    if car.0 >= 0 {
                        raw.updates.push(ActorUpdate::PickupBoost {
                            instigator_car: car.0,
                        });
                    }
                }
            } else if oid == handbrake_id {
                if let Attribute::Boolean(on) = &ua.attribute {
                    raw.updates.push(ActorUpdate::Handbrake {
                        car: ua.actor_id.0,
                        on: *on,
                    });
                }
            } else if oid == loadouts_id {
                if let Attribute::TeamLoadout(t) = &ua.attribute {
                    raw.updates.push(ActorUpdate::Loadout {
                        pri: ua.actor_id.0,
                        blue_body: t.blue.body,
                        orange_body: t.orange.body,
                    });
                }
            } else if oid == loadout_id {
                if let Attribute::Loadout(l) = &ua.attribute {
                    raw.updates.push(ActorUpdate::Loadout {
                        pri: ua.actor_id.0,
                        blue_body: l.body,
                        orange_body: l.body,
                    });
                }
            } else if oid == cam_settings_id {
                if let Attribute::CamSettings(c) = &ua.attribute {
                    raw.updates.push(ActorUpdate::CameraSettings {
                        cam_actor: ua.actor_id.0,
                        fov: c.fov,
                        height: c.height,
                        angle: c.angle,
                        distance: c.distance,
                        stiffness: c.stiffness,
                        swivel: c.swivel,
                        transition: c.transition.unwrap_or(0.0),
                    });
                }
            } else if oid == cam_pri_id {
                if let Attribute::ActiveActor(ActiveActor { actor, .. }) = &ua.attribute {
                    if actor.0 >= 0 {
                        raw.updates.push(ActorUpdate::CameraPri {
                            cam_actor: ua.actor_id.0,
                            pri: actor.0,
                        });
                    }
                }
            } else if oid == steer_id {
                if let Attribute::Float(v) = &ua.attribute {
                    raw.updates.push(ActorUpdate::SteeringSensitivity {
                        pri: ua.actor_id.0,
                        value: *v,
                    });
                }
            } else if let Some((_, stat)) = pri_match_ids.iter().find(|(id, _)| Some(*id) == oid) {
                if let Attribute::Int(v) = &ua.attribute {
                    raw.updates.push(ActorUpdate::PriStat {
                        pri: ua.actor_id.0,
                        stat: *stat,
                        value: *v,
                    });
                }
            }
        }

        for da in &frame.deleted_actors {
            raw.deleted.push(da.0);
        }

        frames.push(raw);
    }

    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(platform: Option<&str>, id: Option<HeaderProp>, bot: bool) -> Vec<(String, HeaderProp)> {
        let mut r = vec![("Name".to_string(), HeaderProp::Str("p".into()))];
        if let Some(p) = platform {
            r.push((
                "Platform".into(),
                HeaderProp::Byte {
                    kind: "OnlinePlatform".into(),
                    value: Some(p.into()),
                },
            ));
        }
        if let Some(i) = id {
            r.push(("OnlineID".into(), i));
        }
        r.push(("bBot".into(), HeaderProp::Bool(bot)));
        r
    }

    #[test]
    fn platform_id_is_platform_colon_id() {
        let steam = row(
            Some("OnlinePlatform_Steam"),
            Some(HeaderProp::QWord(7656)),
            false,
        );
        assert_eq!(platform_id(&steam).as_deref(), Some("steam:7656"));
        let xbox = row(
            Some("OnlinePlatform_Dingo"),
            Some(HeaderProp::QWord(25)),
            false,
        );
        assert_eq!(
            platform_id(&xbox).as_deref(),
            Some("xbox:25"),
            "Dingo is Xbox"
        );
        let epic = row(
            Some("OnlinePlatform_Epic"),
            Some(HeaderProp::Str("abc123".into())),
            false,
        );
        assert_eq!(platform_id(&epic).as_deref(), Some("epic:abc123"));
    }

    #[test]
    fn no_platform_id_for_bots_zero_ids_or_unknown_platforms() {
        let steam = |id, bot| row(Some("OnlinePlatform_Steam"), Some(id), bot);
        assert_eq!(
            platform_id(&steam(HeaderProp::QWord(7656), true)),
            None,
            "bot"
        );
        assert_eq!(
            platform_id(&steam(HeaderProp::QWord(0), false)),
            None,
            "zero id"
        );
        assert_eq!(
            platform_id(&steam(HeaderProp::Str("0".into()), false)),
            None
        );
        let odd = row(
            Some("OnlinePlatform_Mystery"),
            Some(HeaderProp::QWord(1)),
            false,
        );
        assert_eq!(
            platform_id(&odd),
            None,
            "unknown platform is not guessed at"
        );
        assert_eq!(
            platform_id(&row(None, Some(HeaderProp::QWord(1)), false)),
            None
        );
        assert_eq!(
            platform_id(&row(Some("OnlinePlatform_Steam"), None, false)),
            None
        );
    }
}

#[cfg(test)]
mod date_tests {
    use super::parse_header_date;

    #[test]
    fn header_date_is_unix_seconds() {
        assert_eq!(parse_header_date("1970-01-01 00-00-00"), Some(0));
        assert_eq!(
            parse_header_date("2019-10-14 22-23-14"),
            Some(1_571_091_794)
        );
        assert_eq!(
            parse_header_date("2024-02-29 12-00-00"),
            Some(1_709_208_000)
        );
    }

    #[test]
    fn malformed_dates_are_none() {
        for bad in [
            "",
            "2019-10-14",
            "2019-13-01 00-00-00",
            "2019-10-14 25-00-00",
            "x",
        ] {
            assert_eq!(parse_header_date(bad), None, "{bad}");
        }
    }
}
