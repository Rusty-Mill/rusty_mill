//! Tells a demolition apart from an actor that was destroyed and spawned anew, in a recording
//! made with `rb_rl_bridge --record-in`.
//!
//! ```text
//! rb_rl_demo_count FILE
//! ```
//!
//! For each map in the recording it prints every player spawn, every destroy of a player actor
//! and every stretch the game reported a car as Demolished (`PlayerPhysicsChange` state 5), with
//! the game's own clock. A demolition that keeps its actor shows as "down" then "back, same
//! actor"; a destroy-and-respawn would show as a destroy followed by a spawn. Players are shown
//! by actor id and team only, never by display name, so the output can be shared as it is.

use std::collections::BTreeMap;
use std::fmt;
use std::process::ExitCode;

use rb_rl_game_bridge::{decode_game_packet_strict, GameBundle, GameEvent};
use rb_rlbot_wire::frame::FrameDecoder;

/// `PlayerPhysicsState` value the game reports for a demolished car
/// (OnGround, Jumped, DoubleJumped, Dodged, InAir, Demolished).
const DEMOLISHED: u8 = 5;

/// One thing that happened to a player actor, at the game's clock in seconds.
#[derive(Debug, Clone, PartialEq)]
enum Event {
    Spawned { actor: u16, team: u8, at: f32 },
    Destroyed { actor: u16, at: f32 },
    Down { actor: u16, at: f32 },
    Back { actor: u16, at: f32, after: f32 },
}

impl fmt::Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Event::Spawned { actor, team, at } => {
                write!(f, "{at:9.3}s spawned    actor {actor} team {team}")
            }
            Event::Destroyed { actor, at } => write!(f, "{at:9.3}s DESTROYED  actor {actor}"),
            Event::Down { actor, at } => write!(f, "{at:9.3}s DEMOLISHED actor {actor}"),
            Event::Back { actor, at, after } => {
                write!(
                    f,
                    "{at:9.3}s back       actor {actor} after {after:.2}s, same actor"
                )
            }
        }
    }
}

/// What one map of the recording held.
#[derive(Debug, Default, PartialEq)]
struct MapReport {
    map: String,
    events: Vec<Event>,
    /// Demolished stretches begun, per actor.
    demolitions: BTreeMap<u16, u32>,
}

impl MapReport {
    fn count(&self, wanted: impl Fn(&Event) -> bool) -> usize {
        self.events.iter().filter(|e| wanted(e)).count()
    }
}

/// Folds bundles into one report per map. A new `GameSpawn` starts a new map and forgets the
/// previous map's actors, because the game numbers actors afresh on each map.
fn summarize(bundles: impl IntoIterator<Item = GameBundle>) -> Vec<MapReport> {
    let mut maps: Vec<MapReport> = Vec::new();
    let mut known: BTreeMap<u16, u8> = BTreeMap::new();
    let mut state: BTreeMap<u16, u8> = BTreeMap::new();
    let mut down_since: BTreeMap<u16, f32> = BTreeMap::new();
    for bundle in bundles {
        let at = bundle.app_seconds;
        for event in bundle.events {
            if let GameEvent::GameSpawn { map_name, .. } = &event {
                maps.push(MapReport {
                    map: map_name.clone(),
                    ..MapReport::default()
                });
                known.clear();
                state.clear();
                down_since.clear();
                continue;
            }
            let Some(map) = maps.last_mut() else { continue };
            match event {
                GameEvent::PlayerSpawn { id, team, .. } => {
                    known.insert(id, team);
                    state.remove(&id);
                    down_since.remove(&id);
                    map.events.push(Event::Spawned {
                        actor: id,
                        team,
                        at,
                    });
                }
                GameEvent::ActorDestroyed { id } if known.contains_key(&id) => {
                    known.remove(&id);
                    state.remove(&id);
                    down_since.remove(&id);
                    map.events.push(Event::Destroyed { actor: id, at });
                }
                GameEvent::PlayerPhysics { id, state: now, .. } if known.contains_key(&id) => {
                    let before = state.insert(id, now);
                    let was_down = before == Some(DEMOLISHED);
                    if now == DEMOLISHED && !was_down {
                        *map.demolitions.entry(id).or_default() += 1;
                        down_since.insert(id, at);
                        map.events.push(Event::Down { actor: id, at });
                    } else if now != DEMOLISHED && was_down {
                        let after = at - down_since.remove(&id).unwrap_or(at);
                        map.events.push(Event::Back {
                            actor: id,
                            at,
                            after,
                        });
                    }
                }
                _ => {}
            }
        }
    }
    maps
}

fn run(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut decoder = FrameDecoder::new();
    decoder.push(&std::fs::read(path)?);
    let (mut bundles, mut undecodable, mut messages) = (Vec::new(), 0u32, 0u32);
    while let Some(payload) = decoder.next_frame() {
        messages += 1;
        match decode_game_packet_strict(&payload) {
            Ok(bundle) => bundles.push(bundle),
            // Counted and reported, not skipped silently: the report is only as complete as this.
            Err(_) => undecodable += 1,
        }
    }
    decoder.finish()?;
    println!("{messages} messages, {undecodable} undecodable");
    for (i, map) in summarize(bundles).iter().enumerate() {
        let episodes: u32 = map.demolitions.values().sum();
        println!(
            "\nmap {} {:?}: {} demolitions, {} player spawns, {} player-actor destroys",
            i + 1,
            map.map,
            episodes,
            map.count(|e| matches!(e, Event::Spawned { .. })),
            map.count(|e| matches!(e, Event::Destroyed { .. })),
        );
        for (actor, n) in &map.demolitions {
            println!("  actor {actor}: demolished {n}x");
        }
        for event in &map.events {
            println!("  {event}");
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: rb_rl_demo_count FILE");
        return ExitCode::FAILURE;
    };
    match run(&path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("rb_rl_demo_count: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rb_rlbot_wire::Physics;

    fn bundle(at: f32, events: Vec<GameEvent>) -> GameBundle {
        GameBundle {
            app_seconds: at,
            delta_physics_frames: 1,
            events,
        }
    }

    fn map(name: &str) -> GameEvent {
        GameEvent::GameSpawn {
            map_name: name.into(),
            mode: 1,
            team_size: 1,
            gravity: None,
        }
    }

    fn spawn(id: u16, team: u8) -> GameEvent {
        GameEvent::PlayerSpawn {
            id,
            command_id: None,
            name: "secret".into(),
            team,
        }
    }

    fn physics(id: u16, state: u8) -> GameEvent {
        GameEvent::PlayerPhysics {
            id,
            physics: Physics::default(),
            state,
        }
    }

    #[test]
    fn a_demolition_that_keeps_its_actor_is_down_then_back() {
        let maps = summarize([
            bundle(0.0, vec![map("m"), spawn(38, 0)]),
            bundle(1.0, vec![physics(38, 0)]),
            bundle(2.0, vec![physics(38, DEMOLISHED)]),
            bundle(2.5, vec![physics(38, DEMOLISHED)]),
            bundle(5.0, vec![physics(38, 0)]),
        ]);
        assert_eq!(maps.len(), 1);
        assert_eq!(
            maps[0].demolitions.get(&38),
            Some(&1),
            "repeated Demolished is one episode"
        );
        assert_eq!(maps[0].count(|e| matches!(e, Event::Destroyed { .. })), 0);
        assert!(maps[0].events.contains(&Event::Back {
            actor: 38,
            at: 5.0,
            after: 3.0
        }));
    }

    #[test]
    fn a_destroy_is_not_a_demolition() {
        let maps = summarize([
            bundle(0.0, vec![map("m"), spawn(38, 0)]),
            bundle(1.0, vec![GameEvent::ActorDestroyed { id: 38 }]),
            bundle(2.0, vec![spawn(38, 0)]),
        ]);
        assert!(maps[0].demolitions.is_empty());
        assert_eq!(maps[0].count(|e| matches!(e, Event::Destroyed { .. })), 1);
        assert_eq!(maps[0].count(|e| matches!(e, Event::Spawned { .. })), 2);
    }

    #[test]
    fn a_new_map_forgets_the_last_maps_actors() {
        let maps = summarize([
            bundle(0.0, vec![map("a"), spawn(38, 0), physics(38, DEMOLISHED)]),
            bundle(1.0, vec![map("b")]),
            // Actor 38 is not a player on map b, so neither of these is counted.
            bundle(
                2.0,
                vec![
                    physics(38, DEMOLISHED),
                    GameEvent::ActorDestroyed { id: 38 },
                ],
            ),
        ]);
        assert_eq!(maps.len(), 2);
        assert_eq!(maps[0].demolitions.get(&38), Some(&1));
        assert!(maps[1].events.is_empty());
    }

    #[test]
    fn non_player_actors_are_ignored_and_events_before_any_map_are_dropped() {
        let maps = summarize([
            bundle(0.0, vec![physics(7, DEMOLISHED)]),
            bundle(
                1.0,
                vec![
                    map("m"),
                    physics(7, DEMOLISHED),
                    GameEvent::ActorDestroyed { id: 7 },
                ],
            ),
        ]);
        assert_eq!(maps.len(), 1);
        assert!(maps[0].events.is_empty());
    }

    #[test]
    fn no_display_name_reaches_the_output() {
        let maps = summarize([bundle(0.0, vec![map("m"), spawn(38, 1)])]);
        let text: Vec<String> = maps[0].events.iter().map(ToString::to_string).collect();
        assert!(text.iter().all(|line| !line.contains("secret")));
        assert!(text[0].contains("actor 38 team 1"));
    }
}
