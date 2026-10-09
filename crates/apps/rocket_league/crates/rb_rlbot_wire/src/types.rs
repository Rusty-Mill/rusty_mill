//! Plain data the messages carry: inline structs, enums, and the match-setup records.

use crate::codec::{f32_at, u32_at, wire_enum, Struct};

fn put_f32(out: &mut Vec<u8>, v: f32) {
    out.extend_from_slice(&v.to_le_bytes());
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Struct for Vec3 {
    const SIZE: usize = 12;
    const ALIGN: usize = 4;
    fn write(&self, out: &mut Vec<u8>) {
        [self.x, self.y, self.z]
            .into_iter()
            .for_each(|v| put_f32(out, v));
    }
    fn read(b: &[u8]) -> Option<Vec3> {
        Some(Vec3 {
            x: f32_at(b, 0)?,
            y: f32_at(b, 4)?,
            z: f32_at(b, 8)?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rotator {
    pub pitch: f32,
    pub yaw: f32,
    pub roll: f32,
}

impl Struct for Rotator {
    const SIZE: usize = 12;
    const ALIGN: usize = 4;
    fn write(&self, out: &mut Vec<u8>) {
        [self.pitch, self.yaw, self.roll]
            .into_iter()
            .for_each(|v| put_f32(out, v));
    }
    fn read(b: &[u8]) -> Option<Rotator> {
        Some(Rotator {
            pitch: f32_at(b, 0)?,
            yaw: f32_at(b, 4)?,
            roll: f32_at(b, 8)?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Physics {
    pub location: Vec3,
    pub rotation: Rotator,
    pub velocity: Vec3,
    pub angular_velocity: Vec3,
}

impl Struct for Physics {
    const SIZE: usize = 48;
    const ALIGN: usize = 4;
    fn write(&self, out: &mut Vec<u8>) {
        self.location.write(out);
        self.rotation.write(out);
        self.velocity.write(out);
        self.angular_velocity.write(out);
    }
    fn read(b: &[u8]) -> Option<Physics> {
        Some(Physics {
            location: Vec3::read(b.get(0..12)?)?,
            rotation: Rotator::read(b.get(12..24)?)?,
            velocity: Vec3::read(b.get(24..36)?)?,
            angular_velocity: Vec3::read(b.get(36..48)?)?,
        })
    }
}

/// What a bot presses for one car on one tick.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ControllerState {
    pub throttle: f32,
    pub steer: f32,
    pub pitch: f32,
    pub yaw: f32,
    pub roll: f32,
    pub jump: bool,
    pub boost: bool,
    pub handbrake: bool,
    pub use_item: bool,
}

impl Struct for ControllerState {
    const SIZE: usize = 24;
    const ALIGN: usize = 4;
    fn write(&self, out: &mut Vec<u8>) {
        [self.throttle, self.steer, self.pitch, self.yaw, self.roll]
            .into_iter()
            .for_each(|v| put_f32(out, v));
        out.extend([self.jump, self.boost, self.handbrake, self.use_item].map(u8::from));
    }
    fn read(b: &[u8]) -> Option<ControllerState> {
        Some(ControllerState {
            throttle: f32_at(b, 0)?,
            steer: f32_at(b, 4)?,
            pitch: f32_at(b, 8)?,
            yaw: f32_at(b, 12)?,
            roll: f32_at(b, 16)?,
            jump: *b.get(20)? != 0,
            boost: *b.get(21)? != 0,
            handbrake: *b.get(22)? != 0,
            use_item: *b.get(23)? != 0,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BoostPadState {
    pub is_active: bool,
    /// Seconds until the pad is active again.
    pub timer: f32,
}

impl Struct for BoostPadState {
    const SIZE: usize = 8;
    const ALIGN: usize = 4;
    fn write(&self, out: &mut Vec<u8>) {
        out.extend([u8::from(self.is_active), 0, 0, 0]);
        put_f32(out, self.timer);
    }
    fn read(b: &[u8]) -> Option<BoostPadState> {
        Some(BoostPadState {
            is_active: *b.first()? != 0,
            timer: f32_at(b, 4)?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TeamInfo {
    pub team_index: u32,
    pub score: u32,
}

impl Struct for TeamInfo {
    const SIZE: usize = 8;
    const ALIGN: usize = 4;
    fn write(&self, out: &mut Vec<u8>) {
        out.extend(self.team_index.to_le_bytes());
        out.extend(self.score.to_le_bytes());
    }
    fn read(b: &[u8]) -> Option<TeamInfo> {
        Some(TeamInfo {
            team_index: u32_at(b, 0)?,
            score: u32_at(b, 4)?,
        })
    }
}

/// A single optional `f32`, stored as the protocol's one-field `Float` struct.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Float(pub f32);

impl Struct for Float {
    const SIZE: usize = 4;
    const ALIGN: usize = 4;
    fn write(&self, out: &mut Vec<u8>) {
        put_f32(out, self.0);
    }
    fn read(b: &[u8]) -> Option<Float> {
        f32_at(b, 0).map(Float)
    }
}

wire_enum! {
    Launcher { Steam = 0, Epic = 1, Custom = 2, NoLaunch = 3 }
}
wire_enum! {
    GameMode {
        Soccar = 0, Hoops = 1, Dropshot = 2, Snowday = 3,
        Rumble = 4, Heatseeker = 5, Gridiron = 6, Knockout = 7,
    }
}
wire_enum! {
    MatchPhase {
        Inactive = 0, Countdown = 1, Kickoff = 2, Active = 3,
        GoalScored = 4, Replay = 5, Paused = 6, Ended = 7,
    }
}
wire_enum! {
    MatchLengthMutator { FiveMinutes = 0, TenMinutes = 1, TwentyMinutes = 2, Unlimited = 3 }
}
wire_enum! {
    ExistingMatchBehavior { Restart = 0, ContinueAndSpawn = 1, RestartIfDifferent = 2 }
}
wire_enum! {
    DebugRendering { OffByDefault = 0, OnByDefault = 1, AlwaysOff = 2 }
}
wire_enum! {
    PerformanceMonitor { ShowWhenSuboptimal = 0, AlwaysShow = 1, NeverShow = 2 }
}
wire_enum! {
    PsyonixSkill { Beginner = 0, Rookie = 1, Pro = 2, AllStar = 3 }
}
wire_enum! {
    AirState { OnGround = 0, Jumping = 1, DoubleJumping = 2, Dodging = 3, InAir = 4 }
}

// ---- match setup ----

/// Only `match_length` of the protocol's 38 mutators is modelled; the rest are written as
/// their defaults and ignored when reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MutatorSettings {
    pub match_length: MatchLengthMutator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentVariable {
    pub name: String,
    pub value: String,
}

/// A bot RLBot core launches as a process. Loadout is not modelled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomBot {
    pub name: String,
    pub root_dir: String,
    pub run_command: String,
    pub agent_id: String,
    pub hivemind: bool,
    pub environment: Option<Vec<EnvironmentVariable>>,
}

/// A built-in Psyonix bot. Loadout is not modelled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PsyonixBot {
    pub name: String,
    pub skill: PsyonixSkill,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerClass {
    Human,
    CustomBot(CustomBot),
    PsyonixBot(PsyonixBot),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerConfiguration {
    pub variety: PlayerClass,
    pub team: u32,
    pub player_id: i32,
}

/// How to start a match. Script configurations are not modelled (never written, read as none).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchConfiguration {
    pub launcher: Launcher,
    pub launcher_arg: String,
    pub auto_start_agents: bool,
    pub wait_for_agents: bool,
    pub game_map_upk: String,
    pub player_configurations: Vec<PlayerConfiguration>,
    pub game_mode: GameMode,
    pub skip_replays: bool,
    pub instant_start: bool,
    pub mutators: Option<MutatorSettings>,
    pub existing_match_behavior: ExistingMatchBehavior,
    pub enable_rendering: DebugRendering,
    pub enable_state_setting: bool,
    pub auto_save_replay: bool,
    pub freeplay: bool,
    pub performance_monitor: PerformanceMonitor,
}

impl Default for MatchConfiguration {
    /// The protocol's defaults: agents auto-started and waited for, state setting on.
    fn default() -> MatchConfiguration {
        MatchConfiguration {
            launcher: Launcher::Steam,
            launcher_arg: String::new(),
            auto_start_agents: true,
            wait_for_agents: true,
            game_map_upk: String::new(),
            player_configurations: Vec::new(),
            game_mode: GameMode::Soccar,
            skip_replays: false,
            instant_start: false,
            mutators: None,
            existing_match_behavior: ExistingMatchBehavior::Restart,
            enable_rendering: DebugRendering::OffByDefault,
            enable_state_setting: true,
            auto_save_replay: false,
            freeplay: false,
            performance_monitor: PerformanceMonitor::ShowWhenSuboptimal,
        }
    }
}

// ---- state setting ----

/// Vector with each component optionally set (the rest are left as the game has them).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PartialVec3 {
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub z: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PartialRotator {
    pub pitch: Option<f32>,
    pub yaw: Option<f32>,
    pub roll: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DesiredPhysics {
    pub location: Option<PartialVec3>,
    pub rotation: Option<PartialRotator>,
    pub velocity: Option<PartialVec3>,
    pub angular_velocity: Option<PartialVec3>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DesiredBallState {
    pub physics: DesiredPhysics,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DesiredCarState {
    pub physics: Option<DesiredPhysics>,
    pub boost_amount: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DesiredMatchInfo {
    pub world_gravity_z: Option<f32>,
    pub game_speed: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DesiredGameState {
    pub ball_states: Vec<DesiredBallState>,
    pub car_states: Vec<DesiredCarState>,
    pub match_info: Option<DesiredMatchInfo>,
}

// ---- what core reports ----

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatchInfo {
    pub seconds_elapsed: f32,
    pub game_time_remaining: f32,
    pub is_overtime: bool,
    pub is_unlimited_time: bool,
    pub match_phase: MatchPhase,
    pub frame_num: u32,
}

/// One car. The protocol sends 25 fields; these are the ones callers have used.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerInfo {
    pub physics: Physics,
    pub air_state: AirState,
    pub demolished_timeout: f32,
    pub is_bot: bool,
    pub name: String,
    pub team: u32,
    pub boost: f32,
    pub player_id: i32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BallInfo {
    pub physics: Physics,
}

/// One tick of the world, as core sends it.
#[derive(Debug, Clone, PartialEq)]
pub struct GamePacket {
    pub players: Vec<PlayerInfo>,
    pub boost_pads: Vec<BoostPadState>,
    pub balls: Vec<BallInfo>,
    pub match_info: MatchInfo,
    pub teams: Vec<TeamInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoostPad {
    pub location: Vec3,
    pub is_full_boost: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GoalInfo {
    pub team_num: i32,
    pub location: Vec3,
    pub direction: Vec3,
    pub width: f32,
    pub height: f32,
}

/// Static geometry of the arena.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FieldInfo {
    pub boost_pads: Vec<BoostPad>,
    pub goals: Vec<GoalInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControllableInfo {
    pub index: u32,
    pub identifier: i32,
}

/// Which cars this connection controls.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ControllableTeamInfo {
    pub team: u32,
    pub controllables: Vec<ControllableInfo>,
}
