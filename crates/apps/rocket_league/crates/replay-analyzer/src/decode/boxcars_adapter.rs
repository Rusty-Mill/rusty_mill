//! `boxcars` adapter for the [`ReplayParser`] port.
//!
//! Translates the decoded network stream into the neutral [`DecodedReplay`],
//! emitting only the attributes reconstruction consumes (rigid bodies, car->PRI
//! links, PRI names) plus header metadata. All `boxcars`-specific knowledge —
//! object-name lookups, attribute matching — is confined here.

use boxcars::attributes::ActiveActor;
use boxcars::{Attribute, HeaderProp, ObjectId, ParserBuilder, Replay};

use super::{
    ActorClass, ActorUpdate, DecodeError, DecodedReplay, NewActorEvent, RawFrame, ReplayMeta,
    ReplayParser,
};
use crate::model::PlayerMeta;
use std::collections::BTreeMap;

/// Object string for a rigid-body (position/velocity) update.
const OBJ_RB_STATE: &str = "TAGame.RBActor_TA:ReplicatedRBState";
/// Object string linking a car (Pawn) to its player-replication-info actor.
const OBJ_CAR_PRI: &str = "Engine.Pawn:PlayerReplicationInfo";
/// Object string carrying a PRI's player name.
const OBJ_PRI_NAME: &str = "Engine.PlayerReplicationInfo:PlayerName";
/// Object string linking a car component to its car.
const OBJ_COMP_VEHICLE: &str = "TAGame.CarComponent_TA:Vehicle";
/// Object strings carrying a boost amount (byte) — two encodings exist.
const OBJ_BOOST_AMOUNT: &str = "TAGame.CarComponent_Boost_TA:ReplicatedBoostAmount";
const OBJ_BOOST_REPL: &str = "TAGame.CarComponent_Boost_TA:ReplicatedBoost";

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
        team_scores,
        players: extract_players(replay),
    }
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
        });
    }
    out
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
    let comp_vehicle_id = object_id(replay, OBJ_COMP_VEHICLE);
    let boost_amount_id = object_id(replay, OBJ_BOOST_AMOUNT);
    let boost_repl_id = object_id(replay, OBJ_BOOST_REPL);

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
            }
        }

        for da in &frame.deleted_actors {
            raw.deleted.push(da.0);
        }

        frames.push(raw);
    }

    Ok(frames)
}
