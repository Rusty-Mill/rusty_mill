//! The slot layout of every table: field `n` of a table is vtable slot `n` (a union takes two,
//! its type then its value). Defaults follow the schema, which is why a few `true`s appear.

use rusty_flatbuffers::{Builder, Offset, Table};

use crate::codec::{
    add_offset, add_opt_struct, add_struct, enum_field, opt_struct, opt_table, opt_table_vec,
    req_table, required_table_vec_offset, string, string_offset, struct_or_default, struct_vec,
    struct_vec_offset, table_offset, table_vec, table_vec_offset, TableCodec, R,
};
use crate::types::*;
use crate::Error;

/// Starts a table, lets `fill` add fields, and finishes it.
fn build(
    b: &mut Builder,
    fill: impl FnOnce(&mut rusty_flatbuffers::TableBuilder<'_>) -> R<()>,
) -> R<Offset> {
    let mut t = b.start_table();
    fill(&mut t)?;
    Ok(t.finish()?)
}

fn float(t: &Table<'_>, slot: usize) -> R<Option<f32>> {
    Ok(opt_struct::<Float>(t, slot)?.map(|f| f.0))
}

fn add_float(t: &mut rusty_flatbuffers::TableBuilder<'_>, slot: usize, v: Option<f32>) -> R<()> {
    add_opt_struct(t, slot, &v.map(Float))
}

// ---- messages the client sends ----

/// First message on a connection.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConnectionSettings {
    pub agent_id: String,
    pub wants_ball_predictions: bool,
    pub wants_comms: bool,
    pub close_between_matches: bool,
}

impl TableCodec for ConnectionSettings {
    fn write(&self, b: &mut Builder) -> R<Offset> {
        let id = string_offset(b, &self.agent_id)?;
        build(b, |t| {
            add_offset(t, 0, id)?;
            t.add_scalar(1, self.wants_ball_predictions, false)?;
            t.add_scalar(2, self.wants_comms, false)?;
            Ok(t.add_scalar(3, self.close_between_matches, false)?)
        })
    }
    fn read(t: &Table<'_>) -> R<Self> {
        Ok(ConnectionSettings {
            agent_id: string(t, 0)?,
            wants_ball_predictions: t.scalar(1, false)?,
            wants_comms: t.scalar(2, false)?,
            close_between_matches: t.scalar(3, false)?,
        })
    }
}

/// Sent once the client has finished its own start-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InitComplete;

impl TableCodec for InitComplete {
    fn write(&self, b: &mut Builder) -> R<Offset> {
        build(b, |_| Ok(()))
    }
    fn read(_: &Table<'_>) -> R<Self> {
        Ok(InitComplete)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StopCommand {
    pub shutdown_server: bool,
}

impl TableCodec for StopCommand {
    fn write(&self, b: &mut Builder) -> R<Offset> {
        build(b, |t| Ok(t.add_scalar(0, self.shutdown_server, false)?))
    }
    fn read(t: &Table<'_>) -> R<Self> {
        Ok(StopCommand {
            shutdown_server: t.scalar(0, false)?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PlayerInput {
    pub player_index: u32,
    pub controller_state: ControllerState,
}

impl TableCodec for PlayerInput {
    fn write(&self, b: &mut Builder) -> R<Offset> {
        build(b, |t| {
            t.add_scalar(0, self.player_index, 0)?;
            add_struct(t, 1, &self.controller_state)
        })
    }
    fn read(t: &Table<'_>) -> R<Self> {
        Ok(PlayerInput {
            player_index: t.scalar(0, 0)?,
            controller_state: struct_or_default(t, 1)?,
        })
    }
}

// ---- match setup ----

impl TableCodec for EnvironmentVariable {
    fn write(&self, b: &mut Builder) -> R<Offset> {
        let (name, value) = (
            string_offset(b, &self.name)?,
            string_offset(b, &self.value)?,
        );
        build(b, |t| {
            add_offset(t, 0, name)?;
            add_offset(t, 1, value)
        })
    }
    fn read(t: &Table<'_>) -> R<Self> {
        Ok(EnvironmentVariable {
            name: string(t, 0)?,
            value: string(t, 1)?,
        })
    }
}

impl TableCodec for CustomBot {
    fn write(&self, b: &mut Builder) -> R<Offset> {
        let name = string_offset(b, &self.name)?;
        let root_dir = string_offset(b, &self.root_dir)?;
        let run_command = string_offset(b, &self.run_command)?;
        let agent_id = string_offset(b, &self.agent_id)?;
        let environment = table_vec_offset(b, self.environment.as_deref())?;
        build(b, |t| {
            add_offset(t, 0, name)?;
            add_offset(t, 1, root_dir)?;
            add_offset(t, 2, run_command)?;
            // slot 3: loadout, not modelled
            add_offset(t, 4, agent_id)?;
            t.add_scalar(5, self.hivemind, false)?;
            add_offset(t, 6, environment)
        })
    }
    fn read(t: &Table<'_>) -> R<Self> {
        Ok(CustomBot {
            name: string(t, 0)?,
            root_dir: string(t, 1)?,
            run_command: string(t, 2)?,
            agent_id: string(t, 4)?,
            hivemind: t.scalar(5, false)?,
            environment: opt_table_vec(t, 6)?,
        })
    }
}

impl TableCodec for PsyonixBot {
    fn write(&self, b: &mut Builder) -> R<Offset> {
        let name = string_offset(b, &self.name)?;
        build(b, |t| {
            add_offset(t, 0, name)?;
            // slot 1: loadout, not modelled
            Ok(t.add_scalar(2, self.skill as u8, 0)?)
        })
    }
    fn read(t: &Table<'_>) -> R<Self> {
        Ok(PsyonixBot {
            name: string(t, 0)?,
            skill: enum_field(t, 2, PsyonixSkill::from_u8)?,
        })
    }
}

/// The `PlayerClass` union member tags (0 is "none").
const HUMAN: u8 = 1;
const CUSTOM_BOT: u8 = 2;
const PSYONIX_BOT: u8 = 3;

impl TableCodec for PlayerConfiguration {
    fn write(&self, b: &mut Builder) -> R<Offset> {
        let (tag, value) = match &self.variety {
            PlayerClass::Human => (HUMAN, build(b, |_| Ok(()))?),
            PlayerClass::CustomBot(bot) => (CUSTOM_BOT, bot.write(b)?),
            PlayerClass::PsyonixBot(bot) => (PSYONIX_BOT, bot.write(b)?),
        };
        build(b, |t| {
            t.add_scalar(0, tag, 0)?;
            t.add_offset(1, value)?;
            t.add_scalar(2, self.team, 0)?;
            Ok(t.add_scalar(3, self.player_id, 0)?)
        })
    }
    fn read(t: &Table<'_>) -> R<Self> {
        let tag = t.scalar(0, 0u8)?;
        let variety = match tag {
            HUMAN => PlayerClass::Human,
            CUSTOM_BOT => PlayerClass::CustomBot(req_table(t, 1, "variety")?),
            PSYONIX_BOT => PlayerClass::PsyonixBot(req_table(t, 1, "variety")?),
            0 => return Err(Error::Missing("variety")),
            tag => {
                return Err(Error::UnknownUnion {
                    name: "PlayerClass",
                    tag,
                })
            }
        };
        Ok(PlayerConfiguration {
            variety,
            team: t.scalar(2, 0)?,
            player_id: t.scalar(3, 0)?,
        })
    }
}

impl TableCodec for MutatorSettings {
    fn write(&self, b: &mut Builder) -> R<Offset> {
        build(b, |t| Ok(t.add_scalar(0, self.match_length as u8, 0)?))
    }
    fn read(t: &Table<'_>) -> R<Self> {
        Ok(MutatorSettings {
            match_length: enum_field(t, 0, MatchLengthMutator::from_u8)?,
        })
    }
}

impl TableCodec for MatchConfiguration {
    fn write(&self, b: &mut Builder) -> R<Offset> {
        let launcher_arg = string_offset(b, &self.launcher_arg)?;
        let map = string_offset(b, &self.game_map_upk)?;
        let players = required_table_vec_offset(b, &self.player_configurations)?;
        let scripts = required_table_vec_offset::<PlayerConfiguration>(b, &[])?;
        let mutators = table_offset(b, &self.mutators)?;
        build(b, |t| {
            t.add_scalar(0, self.launcher as u8, 0)?;
            add_offset(t, 1, launcher_arg)?;
            t.add_scalar(2, self.auto_start_agents, true)?;
            t.add_scalar(3, self.wait_for_agents, true)?;
            add_offset(t, 4, map)?;
            add_offset(t, 5, players)?;
            add_offset(t, 6, scripts)?; // script configurations: never modelled, always empty
            t.add_scalar(7, self.game_mode as u8, 0)?;
            t.add_scalar(8, self.skip_replays, false)?;
            t.add_scalar(9, self.instant_start, false)?;
            add_offset(t, 10, mutators)?;
            t.add_scalar(11, self.existing_match_behavior as u8, 0)?;
            t.add_scalar(12, self.enable_rendering as u8, 0)?;
            t.add_scalar(13, self.enable_state_setting, true)?;
            t.add_scalar(14, self.auto_save_replay, false)?;
            t.add_scalar(15, self.freeplay, false)?;
            Ok(t.add_scalar(16, self.performance_monitor as u8, 0)?)
        })
    }
    fn read(t: &Table<'_>) -> R<Self> {
        Ok(MatchConfiguration {
            launcher: enum_field(t, 0, Launcher::from_u8)?,
            launcher_arg: string(t, 1)?,
            auto_start_agents: t.scalar(2, true)?,
            wait_for_agents: t.scalar(3, true)?,
            game_map_upk: string(t, 4)?,
            player_configurations: table_vec(t, 5)?,
            game_mode: enum_field(t, 7, GameMode::from_u8)?,
            skip_replays: t.scalar(8, false)?,
            instant_start: t.scalar(9, false)?,
            mutators: opt_table(t, 10)?,
            existing_match_behavior: enum_field(t, 11, ExistingMatchBehavior::from_u8)?,
            enable_rendering: enum_field(t, 12, DebugRendering::from_u8)?,
            enable_state_setting: t.scalar(13, true)?,
            auto_save_replay: t.scalar(14, false)?,
            freeplay: t.scalar(15, false)?,
            performance_monitor: enum_field(t, 16, PerformanceMonitor::from_u8)?,
        })
    }
}

// ---- state setting ----

impl TableCodec for PartialVec3 {
    fn write(&self, b: &mut Builder) -> R<Offset> {
        build(b, |t| {
            add_float(t, 0, self.x)?;
            add_float(t, 1, self.y)?;
            add_float(t, 2, self.z)
        })
    }
    fn read(t: &Table<'_>) -> R<Self> {
        Ok(PartialVec3 {
            x: float(t, 0)?,
            y: float(t, 1)?,
            z: float(t, 2)?,
        })
    }
}

impl TableCodec for PartialRotator {
    fn write(&self, b: &mut Builder) -> R<Offset> {
        build(b, |t| {
            add_float(t, 0, self.pitch)?;
            add_float(t, 1, self.yaw)?;
            add_float(t, 2, self.roll)
        })
    }
    fn read(t: &Table<'_>) -> R<Self> {
        Ok(PartialRotator {
            pitch: float(t, 0)?,
            yaw: float(t, 1)?,
            roll: float(t, 2)?,
        })
    }
}

impl TableCodec for DesiredPhysics {
    fn write(&self, b: &mut Builder) -> R<Offset> {
        let location = table_offset(b, &self.location)?;
        let rotation = table_offset(b, &self.rotation)?;
        let velocity = table_offset(b, &self.velocity)?;
        let spin = table_offset(b, &self.angular_velocity)?;
        build(b, |t| {
            add_offset(t, 0, location)?;
            add_offset(t, 1, rotation)?;
            add_offset(t, 2, velocity)?;
            add_offset(t, 3, spin)
        })
    }
    fn read(t: &Table<'_>) -> R<Self> {
        Ok(DesiredPhysics {
            location: opt_table(t, 0)?,
            rotation: opt_table(t, 1)?,
            velocity: opt_table(t, 2)?,
            angular_velocity: opt_table(t, 3)?,
        })
    }
}

impl TableCodec for DesiredBallState {
    fn write(&self, b: &mut Builder) -> R<Offset> {
        let physics = self.physics.write(b)?;
        build(b, |t| Ok(t.add_offset(0, physics)?))
    }
    fn read(t: &Table<'_>) -> R<Self> {
        Ok(DesiredBallState {
            physics: req_table(t, 0, "physics")?,
        })
    }
}

impl TableCodec for DesiredCarState {
    fn write(&self, b: &mut Builder) -> R<Offset> {
        let physics = table_offset(b, &self.physics)?;
        build(b, |t| {
            add_offset(t, 0, physics)?;
            add_float(t, 1, self.boost_amount)
        })
    }
    fn read(t: &Table<'_>) -> R<Self> {
        Ok(DesiredCarState {
            physics: opt_table(t, 0)?,
            boost_amount: float(t, 1)?,
        })
    }
}

impl TableCodec for DesiredMatchInfo {
    fn write(&self, b: &mut Builder) -> R<Offset> {
        build(b, |t| {
            add_float(t, 0, self.world_gravity_z)?;
            add_float(t, 1, self.game_speed)
        })
    }
    fn read(t: &Table<'_>) -> R<Self> {
        Ok(DesiredMatchInfo {
            world_gravity_z: float(t, 0)?,
            game_speed: float(t, 1)?,
        })
    }
}

impl TableCodec for DesiredGameState {
    fn write(&self, b: &mut Builder) -> R<Offset> {
        let balls = required_table_vec_offset(b, &self.ball_states)?;
        let cars = required_table_vec_offset(b, &self.car_states)?;
        let info = table_offset(b, &self.match_info)?;
        build(b, |t| {
            add_offset(t, 0, balls)?;
            add_offset(t, 1, cars)?;
            add_offset(t, 2, info)
        })
    }
    fn read(t: &Table<'_>) -> R<Self> {
        Ok(DesiredGameState {
            ball_states: table_vec(t, 0)?,
            car_states: table_vec(t, 1)?,
            match_info: opt_table(t, 2)?,
        })
    }
}

// ---- what core reports ----

impl TableCodec for MatchInfo {
    fn write(&self, b: &mut Builder) -> R<Offset> {
        build(b, |t| {
            t.add_scalar(0, self.seconds_elapsed, 0.0)?;
            t.add_scalar(1, self.game_time_remaining, 0.0)?;
            t.add_scalar(2, self.is_overtime, false)?;
            t.add_scalar(3, self.is_unlimited_time, false)?;
            t.add_scalar(4, self.match_phase as u8, 0)?;
            // slots 5-7: world gravity, game speed, last spectated, not modelled
            Ok(t.add_scalar(8, self.frame_num, 0)?)
        })
    }
    fn read(t: &Table<'_>) -> R<Self> {
        Ok(MatchInfo {
            seconds_elapsed: t.scalar(0, 0.0)?,
            game_time_remaining: t.scalar(1, 0.0)?,
            is_overtime: t.scalar(2, false)?,
            is_unlimited_time: t.scalar(3, false)?,
            match_phase: enum_field(t, 4, MatchPhase::from_u8)?,
            frame_num: t.scalar(8, 0)?,
        })
    }
}

impl TableCodec for PlayerInfo {
    fn write(&self, b: &mut Builder) -> R<Offset> {
        let name = string_offset(b, &self.name)?;
        build(b, |t| {
            add_struct(t, 0, &self.physics)?;
            // slots 1-4: score, hitbox, hitbox offset, latest touch, not modelled
            t.add_scalar(5, self.air_state as u8, 0)?;
            // slot 6: dodge timeout; slot 8: supersonic
            t.add_scalar(7, self.demolished_timeout, 0.0)?;
            t.add_scalar(9, self.is_bot, false)?;
            add_offset(t, 10, name)?;
            t.add_scalar(11, self.team, 0)?;
            t.add_scalar(12, self.boost, 0.0)?;
            Ok(t.add_scalar(13, self.player_id, 0)?)
        })
    }
    fn read(t: &Table<'_>) -> R<Self> {
        Ok(PlayerInfo {
            physics: struct_or_default(t, 0)?,
            air_state: enum_field(t, 5, AirState::from_u8)?,
            demolished_timeout: t.scalar(7, 0.0)?,
            is_bot: t.scalar(9, false)?,
            name: string(t, 10)?,
            team: t.scalar(11, 0)?,
            boost: t.scalar(12, 0.0)?,
            player_id: t.scalar(13, 0)?,
        })
    }
}

impl TableCodec for BallInfo {
    fn write(&self, b: &mut Builder) -> R<Offset> {
        // slots 1-2 (the collision shape union), 3, 4: not modelled
        build(b, |t| add_struct(t, 0, &self.physics))
    }
    fn read(t: &Table<'_>) -> R<Self> {
        Ok(BallInfo {
            physics: struct_or_default(t, 0)?,
        })
    }
}

impl TableCodec for GamePacket {
    fn write(&self, b: &mut Builder) -> R<Offset> {
        let players = required_table_vec_offset(b, &self.players)?;
        let pads = struct_vec_offset(b, &self.boost_pads)?;
        let balls = required_table_vec_offset(b, &self.balls)?;
        let info = self.match_info.write(b)?;
        let teams = struct_vec_offset(b, &self.teams)?;
        build(b, |t| {
            add_offset(t, 0, players)?;
            add_offset(t, 1, pads)?;
            add_offset(t, 2, balls)?;
            t.add_offset(3, info)?;
            add_offset(t, 4, teams)
        })
    }
    fn read(t: &Table<'_>) -> R<Self> {
        Ok(GamePacket {
            players: table_vec(t, 0)?,
            boost_pads: struct_vec(t, 1)?,
            balls: table_vec(t, 2)?,
            match_info: req_table(t, 3, "match_info")?,
            teams: struct_vec(t, 4)?,
        })
    }
}

impl TableCodec for BoostPad {
    fn write(&self, b: &mut Builder) -> R<Offset> {
        build(b, |t| {
            add_struct(t, 0, &self.location)?;
            Ok(t.add_scalar(1, self.is_full_boost, false)?)
        })
    }
    fn read(t: &Table<'_>) -> R<Self> {
        Ok(BoostPad {
            location: struct_or_default(t, 0)?,
            is_full_boost: t.scalar(1, false)?,
        })
    }
}

impl TableCodec for GoalInfo {
    fn write(&self, b: &mut Builder) -> R<Offset> {
        build(b, |t| {
            t.add_scalar(0, self.team_num, 0)?;
            add_struct(t, 1, &self.location)?;
            add_struct(t, 2, &self.direction)?;
            t.add_scalar(3, self.width, 0.0)?;
            Ok(t.add_scalar(4, self.height, 0.0)?)
        })
    }
    fn read(t: &Table<'_>) -> R<Self> {
        Ok(GoalInfo {
            team_num: t.scalar(0, 0)?,
            location: struct_or_default(t, 1)?,
            direction: struct_or_default(t, 2)?,
            width: t.scalar(3, 0.0)?,
            height: t.scalar(4, 0.0)?,
        })
    }
}

impl TableCodec for FieldInfo {
    fn write(&self, b: &mut Builder) -> R<Offset> {
        let pads = required_table_vec_offset(b, &self.boost_pads)?;
        let goals = required_table_vec_offset(b, &self.goals)?;
        build(b, |t| {
            add_offset(t, 0, pads)?;
            add_offset(t, 1, goals)
        })
    }
    fn read(t: &Table<'_>) -> R<Self> {
        Ok(FieldInfo {
            boost_pads: table_vec(t, 0)?,
            goals: table_vec(t, 1)?,
        })
    }
}

impl TableCodec for ControllableInfo {
    fn write(&self, b: &mut Builder) -> R<Offset> {
        build(b, |t| {
            t.add_scalar(0, self.index, 0)?;
            Ok(t.add_scalar(1, self.identifier, 0)?)
        })
    }
    fn read(t: &Table<'_>) -> R<Self> {
        Ok(ControllableInfo {
            index: t.scalar(0, 0)?,
            identifier: t.scalar(1, 0)?,
        })
    }
}

impl TableCodec for ControllableTeamInfo {
    fn write(&self, b: &mut Builder) -> R<Offset> {
        let controllables = required_table_vec_offset(b, &self.controllables)?;
        build(b, |t| {
            t.add_scalar(0, self.team, 0)?;
            add_offset(t, 1, controllables)
        })
    }
    fn read(t: &Table<'_>) -> R<Self> {
        Ok(ControllableTeamInfo {
            team: t.scalar(0, 0)?,
            controllables: table_vec(t, 1)?,
        })
    }
}
