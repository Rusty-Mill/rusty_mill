//! A tally of the message and command tags in a recording, to see whether the game sends (or
//! RLBot sends the game) anything this crate does not know about.
//!
//! The decoders turn a tag they do not model into [`GameEvent::Other`](crate::GameEvent), which
//! hides the difference between "known but not decoded" and "not in the format at all". The
//! census reads only the tag of each message wrapper, so it works on any recording and never
//! fails on a body it cannot read.

use std::collections::BTreeMap;

use rusty_flatbuffers::Table;

use crate::game_wire::{WireError, MAX_EVENTS_PER_BUNDLE};

/// The `GameMessage` union as the format note lists it; index = tag, 0 is NONE.
pub const GAME_MESSAGES: [&str; 23] = [
    "NONE",
    "GameSpawn",
    "GameStateChange",
    "GameTimeChange",
    "GoalSpawn",
    "PickupSpawn",
    "PickupStateChange",
    "PlayerSpawn",
    "PlayerPhysicsChange",
    "PlayerBoostChange",
    "BallSpawn",
    "BallPhysicsChange",
    "BallHit",
    "TeamScoreChange",
    "ActorDestroyed",
    "PlayerStatChange",
    "PlayerStatEvent",
    "PlayerSpectate",
    "PlayerInputChange",
    "PlatformSpawn",
    "PlatformDamageStateChange",
    "BallExtraDataChange",
    "PlayerPickupChange",
];

/// The `ControllerCommand` union as the format note lists it; index = tag, 0 is NONE.
pub const CONTROLLER_COMMANDS: [&str; 14] = [
    "NONE",
    "SpawnBot",
    "DestroyActor",
    "PlayerInput",
    "RenderLine3D",
    "RenderText2D",
    "RenderText3D",
    "RemoveRenderItem",
    "Exec",
    "SetPhysics",
    "SetBoost",
    "RenderLineSeries3D",
    "EndMatch",
    "SetPaused",
];

/// Tags seen, by direction, and the frames that could not be read at all.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Census {
    /// Game-to-RLBot message tags.
    pub inbound: BTreeMap<u8, u64>,
    /// RLBot-to-game command tags.
    pub outbound: BTreeMap<u8, u64>,
    /// Frames whose root or message vector could not be read, so no tag was counted.
    pub unreadable_frames: u64,
}

impl Census {
    /// Counts the tags in one `GamePacket` payload from the game.
    pub fn add_game_packet(&mut self, payload: &[u8]) {
        if tags(payload, 1, 0, &mut self.inbound).is_err() {
            self.unreadable_frames += 1;
        }
    }

    /// Counts the tags in one `ControllerPacket` payload sent to the game.
    pub fn add_controller_packet(&mut self, payload: &[u8]) {
        if tags(payload, 0, 1, &mut self.outbound).is_err() {
            self.unreadable_frames += 1;
        }
    }

    /// Inbound tags outside the `GameMessage` union as the format note lists it.
    pub fn unknown_inbound(&self) -> Vec<u8> {
        outside(&self.inbound, GAME_MESSAGES.len())
    }

    /// Outbound tags outside the `ControllerCommand` union as the format note lists it.
    pub fn unknown_outbound(&self) -> Vec<u8> {
        outside(&self.outbound, CONTROLLER_COMMANDS.len())
    }

    /// A readable table of what was seen, naming each tag and ending with the verdict.
    pub fn report(&self) -> String {
        let mut out = String::new();
        section(&mut out, "game to RLBot", &self.inbound, &GAME_MESSAGES);
        section(
            &mut out,
            "RLBot to game",
            &self.outbound,
            &CONTROLLER_COMMANDS,
        );
        out.push_str(&format!("unreadable frames: {}\n", self.unreadable_frames));
        let (inbound, outbound) = (self.unknown_inbound(), self.unknown_outbound());
        if inbound.is_empty() && outbound.is_empty() {
            out.push_str("tags outside the known unions: none\n");
        } else {
            out.push_str(&format!(
                "tags outside the known unions: game to RLBot {inbound:?}, RLBot to game {outbound:?}\n"
            ));
        }
        out
    }
}

fn outside(seen: &BTreeMap<u8, u64>, known: usize) -> Vec<u8> {
    seen.keys()
        .copied()
        .filter(|tag| usize::from(*tag) >= known)
        .collect()
}

fn section(out: &mut String, title: &str, seen: &BTreeMap<u8, u64>, names: &[&str]) {
    out.push_str(&format!("{title}:\n"));
    if seen.is_empty() {
        out.push_str("  (none)\n");
    }
    for (tag, count) in seen {
        let name = names.get(usize::from(*tag)).copied().unwrap_or("UNKNOWN");
        out.push_str(&format!("  {tag:>3} {name:<28} {count}\n"));
    }
}

/// Adds the tag of every wrapper in vector `vector` of the root table (`tag_slot` of each
/// wrapper) to `into`. Counts nothing from a payload it cannot read to the end.
fn tags(
    payload: &[u8],
    vector: usize,
    tag_slot: usize,
    into: &mut BTreeMap<u8, u64>,
) -> Result<(), WireError> {
    let root = Table::root(payload)?;
    let Some(messages) = root.vector(vector)? else {
        return Ok(());
    };
    if messages.len() > MAX_EVENTS_PER_BUNDLE {
        return Err(WireError::TooLarge("events"));
    }
    let mut found = Vec::with_capacity(messages.len());
    for i in 0..messages.len() {
        found.push(messages.table(i)?.scalar(tag_slot, 0u8)?);
    }
    for tag in found {
        *into.entry(tag).or_default() += 1;
    }
    Ok(())
}
