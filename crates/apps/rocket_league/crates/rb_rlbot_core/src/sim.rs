//! The match, with no sockets: `rb_env` behind the shapes RLBot core hands out.
//!
//! [`Simulation`] owns an [`Env`] with match flow on, one held input per car, and a frame
//! counter. It answers the questions core answers (who controls which cars, what the field is,
//! what the current [`GamePacket`] is) and takes the two things a client can change: a car's
//! input and a [`DesiredGameState`]. Everything here is deterministic: the same configuration
//! and the same inputs per tick give the same packets.

use rb_domain::{BallState, CarState, ControllerInput, PhysicsFrame, Quat, Vec3};
use rb_env::flow::{self, MatchState, Phase};
use rb_env::Env;
use rb_physics_bullet::arena::BACK_WALL_Y;
use rb_physics_bullet::BoostPads;
use rb_rlbot_bridge::{controller_input, physics};
use rb_rlbot_wire::{
    AirState, BallInfo, BoostPad, BoostPadState, ControllableInfo, ControllableTeamInfo,
    ControllerState, CustomBot, DesiredGameState, DesiredPhysics, FieldInfo, GamePacket, GoalInfo,
    MatchConfiguration, MatchInfo, MatchLengthMutator, MatchPhase, MutatorSettings, PartialVec3,
    PlayerClass, PlayerConfiguration, PlayerInfo, TeamInfo, Vec3 as WireVec3,
};
use rb_scenario::{quat_to_rotator, rotator_to_quat};

/// A reason a match cannot be set up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupError {
    /// The configuration has no players, so there is nothing to simulate.
    NoPlayers,
    /// A player is on a team other than 0 (blue) or 1 (orange).
    BadTeam { player: usize, team: u32 },
    /// A custom bot's agent id is empty or only whitespace, so no client could ever claim it.
    BlankAgentId { player: usize },
}

impl std::fmt::Display for SetupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SetupError::NoPlayers => f.write_str("the match configuration has no players"),
            SetupError::BadTeam { player, team } => {
                write!(f, "player {player} is on team {team}; only 0 and 1 exist")
            }
            SetupError::BlankAgentId { player } => {
                write!(f, "player {player} is a custom bot with a blank agent id")
            }
        }
    }
}

impl std::error::Error for SetupError {}

/// A goal as the real game announces it in `GoalSpawn` (see `Simulation::field_info`).
const GOAL_WIDTH: f32 = 1920.0;
const GOAL_HEIGHT_ANNOUNCED: f32 = 752.0;
const GOAL_CENTRE_Z: f32 = 312.0;

/// The ticks of play in a match of `length`; `None` for an unlimited one.
fn match_ticks(length: MatchLengthMutator) -> Option<i64> {
    match length {
        MatchLengthMutator::FiveMinutes => Some(flow::FIVE_MINUTES),
        MatchLengthMutator::TenMinutes => Some(2 * flow::FIVE_MINUTES),
        MatchLengthMutator::TwentyMinutes => Some(4 * flow::FIVE_MINUTES),
        MatchLengthMutator::Unlimited => None,
    }
}

/// The agent ids a configuration waits for: every custom bot's, once each, in player order.
pub fn expected_agents(configuration: &MatchConfiguration) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for p in &configuration.player_configurations {
        // Core trims the id a client sends, so the configured one is held trimmed too.
        if let PlayerClass::CustomBot(bot) = &p.variety {
            let id = bot.agent_id.trim();
            if !id.is_empty() && !ids.iter().any(|known| known == id) {
                ids.push(id.to_string());
            }
        }
    }
    ids
}

/// The cars `agent_id` controls under `configuration` (car index = player order), and their team.
pub fn controllable_team_info(
    configuration: &MatchConfiguration,
    agent_id: &str,
) -> ControllableTeamInfo {
    let mut team = 0;
    let mut controllables = Vec::new();
    for (index, p) in configuration.player_configurations.iter().enumerate() {
        let PlayerClass::CustomBot(bot) = &p.variety else {
            continue;
        };
        if bot.agent_id.trim() != agent_id.trim() {
            continue;
        }
        team = p.team;
        controllables.push(ControllableInfo {
            index: index as u32,
            identifier: p.player_id,
        });
    }
    ControllableTeamInfo {
        team,
        controllables,
    }
}

/// A configuration of one custom bot per `(agent_id, team)`, in that order, with `length` as the
/// match length. The same agent id twice makes a hivemind's two cars.
pub fn bot_match(players: &[(String, u32)], length: MatchLengthMutator) -> MatchConfiguration {
    MatchConfiguration {
        player_configurations: players
            .iter()
            .enumerate()
            .map(|(index, (id, team))| PlayerConfiguration {
                variety: PlayerClass::CustomBot(CustomBot {
                    name: id.clone(),
                    root_dir: String::new(),
                    run_command: String::new(),
                    agent_id: id.clone(),
                    hivemind: false,
                    environment: None,
                }),
                team: *team,
                player_id: index as i32 + 100,
            })
            .collect(),
        mutators: Some(MutatorSettings {
            match_length: length,
        }),
        ..MatchConfiguration::default()
    }
}

/// One RLBot match played on `rb_env`.
pub struct Simulation {
    env: Env,
    configuration: MatchConfiguration,
    teams: Vec<u32>,
    inputs: Vec<ControllerInput>,
    frame: PhysicsFrame,
    frame_num: u32,
    count_up_ticks: u64,
}

impl Simulation {
    /// Sets up the match `configuration` describes and starts its first kickoff.
    ///
    /// Cars are the player configurations in order, so a car's index is its `player_index`.
    /// Honoured: the players and their teams, `mutators.match_length` (five minutes when the
    /// mutators are absent, the protocol's default), `skip_replays`. Everything else (map,
    /// launcher, rendering, the other mutators) has no meaning here and is ignored.
    pub fn new(configuration: MatchConfiguration) -> Result<Simulation, SetupError> {
        if configuration.player_configurations.is_empty() {
            return Err(SetupError::NoPlayers);
        }
        let mut teams = Vec::new();
        for (player, p) in configuration.player_configurations.iter().enumerate() {
            if p.team > 1 {
                return Err(SetupError::BadTeam {
                    player,
                    team: p.team,
                });
            }
            if matches!(&p.variety, PlayerClass::CustomBot(bot) if bot.agent_id.trim().is_empty()) {
                return Err(SetupError::BlankAgentId { player });
            }
            teams.push(p.team);
        }

        let length = configuration
            .mutators
            .map_or(MatchLengthMutator::FiveMinutes, |m| m.match_length);
        let mut env = Env::new();
        env.set_teams(&teams);
        env.enable_match_flow(true);
        env.set_match_length(match_ticks(length));
        if configuration.skip_replays {
            env.set_replay_ticks(Some(1));
        }
        // `start_match` places everything; this only has to name the cars.
        let cars = (0..teams.len())
            .map(|index| CarState {
                player_id: index as u32,
                position: Vec3::new(index as f32 * 300.0, 0.0, 17.0),
                rotation: Quat::IDENTITY,
                velocity: Vec3::ZERO,
                angular_velocity: Vec3::ZERO,
                boost_amount: 33.0,
                input: None,
            })
            .collect();
        env.reset(&PhysicsFrame {
            timestamp_secs: 0.0,
            ball: BallState {
                position: Vec3::new(0.0, 0.0, 93.0),
                rotation: Quat::IDENTITY,
                velocity: Vec3::ZERO,
                angular_velocity: Vec3::ZERO,
            },
            cars,
        });
        let frame = env.start_match();
        Ok(Simulation {
            env,
            inputs: vec![ControllerInput::default(); teams.len()],
            teams,
            configuration,
            frame,
            frame_num: 0,
            count_up_ticks: 0,
        })
    }

    pub fn configuration(&self) -> &MatchConfiguration {
        &self.configuration
    }

    /// The agent ids the configuration waits for: every custom bot's, once each.
    pub fn expected_agents(&self) -> Vec<String> {
        expected_agents(&self.configuration)
    }

    /// What core tells an agent about the cars it controls: those configured with its id, which
    /// are all on one team (a hivemind controls a team's cars). An id that no car carries gets
    /// no cars, which a client takes as "nothing to do".
    pub fn controllable_team_info(&self, agent_id: &str) -> ControllableTeamInfo {
        controllable_team_info(&self.configuration, agent_id)
    }

    /// The static field: Soccar's 34 pads in the game's order and the two goals.
    pub fn field_info(&self) -> FieldInfo {
        let pads = BoostPads::standard();
        let boost_pads = (0..pads.len())
            .filter_map(|i| pads.pad(i))
            .map(|pad| BoostPad {
                location: WireVec3 {
                    x: pad.position.x,
                    y: pad.position.y,
                    z: pad.position.z,
                },
                is_full_boost: pad.big,
            })
            .collect();
        // A goal is named for the team that defends it; its direction points into the field.
        // Width, height and centre height are what the real game announced for a standard map
        // (live capture, TrainStation_P, 2026-10-11): 1920 x 752 centred at z 312. They are larger
        // than the goal mouth the physics uses, so the physics constants are not reused here.
        let goal = |team_num: i32, sign: f32| GoalInfo {
            team_num,
            location: WireVec3 {
                x: 0.0,
                y: sign * BACK_WALL_Y,
                z: GOAL_CENTRE_Z,
            },
            direction: WireVec3 {
                x: 0.0,
                y: -sign,
                z: 0.0,
            },
            width: GOAL_WIDTH,
            height: GOAL_HEIGHT_ANNOUNCED,
        };
        FieldInfo {
            boost_pads,
            goals: vec![goal(0, -1.0), goal(1, 1.0)],
        }
    }

    /// Holds `state` as car `player_index`'s input until it is replaced. `false` (and nothing
    /// changed) if there is no such car.
    pub fn set_input(&mut self, player_index: u32, state: &ControllerState) -> bool {
        match self.inputs.get_mut(player_index as usize) {
            Some(slot) => {
                *slot = controller_input(&crate::validation::sanitise_controller(state));
                true
            }
            None => false,
        }
    }

    /// Advances one physics tick (1/120 s) with the held inputs.
    pub fn step(&mut self) {
        let before = self.env.match_state();
        self.frame = self.env.step(&self.inputs);
        if before
            .is_some_and(|s| s.phase == Phase::Active && (s.overtime || s.clock_ticks.is_none()))
        {
            self.count_up_ticks += 1;
        }
        if before.is_some_and(|s| !s.overtime) && self.env.match_state().is_some_and(|s| s.overtime)
        {
            self.count_up_ticks = 0;
        }
        self.frame_num += 1;
    }

    /// Applies a client's state-setting message: the fields it names change, the rest stay.
    /// Only the first ball and the cars it lists by index are touched. Match info (gravity,
    /// game speed) cannot be changed in `rb_env` and is ignored. Rejects nonfinite requests atomically with `InvalidInput`. Returns whether the message
    /// named a car that does not exist (the rest of it was still applied).
    pub fn apply_desired(&mut self, desired: &DesiredGameState) -> std::io::Result<bool> {
        crate::validation::validate_desired(desired)?;
        let mut frame = self.env.peek(&[]);
        if let Some(ball) = desired.ball_states.first() {
            let b = &mut frame.ball;
            (b.position, b.rotation, b.velocity, b.angular_velocity) = patch(
                &ball.physics,
                (b.position, b.rotation, b.velocity, b.angular_velocity),
            );
        }
        let mut unknown_car = false;
        for (index, want) in desired.car_states.iter().enumerate() {
            let Some(car) = frame.cars.iter_mut().find(|c| c.player_id == index as u32) else {
                unknown_car = true;
                continue;
            };
            if let Some(desired_physics) = &want.physics {
                (
                    car.position,
                    car.rotation,
                    car.velocity,
                    car.angular_velocity,
                ) = patch(
                    desired_physics,
                    (
                        car.position,
                        car.rotation,
                        car.velocity,
                        car.angular_velocity,
                    ),
                );
            }
            if let Some(boost) = want.boost_amount {
                car.boost_amount = boost.clamp(0.0, 100.0);
            }
        }
        self.env.snap(&frame);
        self.frame = self.env.peek(&[]);
        Ok(unknown_car)
    }

    /// The match as it stands, in the shape core sends every tick.
    pub fn packet(&self) -> GamePacket {
        let state = self.env.match_state();
        let boost_pads = self
            .env
            .boost_pads()
            .map(|pads| {
                (0..pads.len())
                    .map(|i| BoostPadState {
                        is_active: pads.is_active(i),
                        timer: if pads.is_active(i) {
                            0.0
                        } else {
                            let duration = if pads.pad(i).is_some_and(|p| p.big) {
                                10.0
                            } else {
                                4.0
                            };
                            (duration - pads.cooldown_left(i)).max(0.0)
                        },
                    })
                    .collect()
            })
            .unwrap_or_default();
        let configured = &self.configuration.player_configurations;
        // Observations omit demolished cars; their player_id remains the physical slot.
        // Keep a packet entry for every configured slot so controls and state targets never shift.
        let players = configured
            .iter()
            .enumerate()
            .map(|(index, p)| {
                let car = self.frame.cars.iter().find(|c| c.player_id == index as u32);
                PlayerInfo {
                    physics: car
                        .map(|c| physics(c.position, c.rotation, c.velocity, c.angular_velocity))
                        .unwrap_or_default(),
                    air_state: car.map(air_state).unwrap_or(AirState::OnGround),
                    // The observation omits the respawn timer; zero denotes demolished, duration unknown.
                    demolished_timeout: if car.is_some() { -1.0 } else { 0.0 },
                    is_bot: !matches!(p.variety, PlayerClass::Human),
                    name: name_of(p),
                    team: self.teams[index],
                    boost: car.map_or(0.0, |c| c.boost_amount),
                    player_id: p.player_id,
                }
            })
            .collect();
        let score = state.map_or([0, 0], |s| s.score);
        GamePacket {
            players,
            boost_pads,
            balls: vec![BallInfo {
                physics: physics(
                    self.frame.ball.position,
                    self.frame.ball.rotation,
                    self.frame.ball.velocity,
                    self.frame.ball.angular_velocity,
                ),
            }],
            match_info: MatchInfo {
                seconds_elapsed: self.frame_num as f32 / 120.0,
                game_time_remaining: if state.is_some_and(|s| s.overtime || s.clock_ticks.is_none())
                {
                    self.count_up_ticks as f32 / 120.0
                } else {
                    state
                        .and_then(|s| s.seconds_remaining())
                        .unwrap_or(0.0)
                        .max(0.0)
                },
                is_overtime: state.is_some_and(|s| s.overtime),
                is_unlimited_time: state.is_none_or(|s| s.clock_ticks.is_none()),
                match_phase: state.map_or(MatchPhase::Active, wire_phase),
                frame_num: self.frame_num,
            },
            teams: vec![
                TeamInfo {
                    team_index: 0,
                    score: score[0],
                },
                TeamInfo {
                    team_index: 1,
                    score: score[1],
                },
            ],
        }
    }

    /// Where the match is (match flow is always on for a `Simulation`).
    pub fn match_state(&self) -> Option<MatchState> {
        self.env.match_state()
    }
}

fn name_of(p: &PlayerConfiguration) -> String {
    match &p.variety {
        PlayerClass::Human => "Human".into(),
        PlayerClass::CustomBot(bot) => bot.name.clone(),
        PlayerClass::PsyonixBot(bot) => bot.name.clone(),
    }
}

fn wire_phase(state: MatchState) -> MatchPhase {
    match state.phase {
        Phase::Countdown => MatchPhase::Countdown,
        Phase::Kickoff => MatchPhase::Kickoff,
        Phase::Active => MatchPhase::Active,
        Phase::GoalScored => MatchPhase::GoalScored,
        Phase::Replay => MatchPhase::Replay,
        Phase::Ended => MatchPhase::Ended,
    }
}

/// `rb_env` does not report whether a car's wheels touch the ground; the height of its origin
/// is the stand-in. A car resting on its wheels has its origin about 17 uu up.
fn air_state(car: &CarState) -> AirState {
    if car.position.z < 25.0 {
        AirState::OnGround
    } else {
        AirState::InAir
    }
}

type Body = (Vec3, Quat, Vec3, Vec3);

/// `current` with the fields `want` names replaced. A rotation naming only some of pitch,
/// yaw and roll keeps the others from the current orientation.
fn patch(want: &DesiredPhysics, current: Body) -> Body {
    let (position, rotation, velocity, angular_velocity) = current;
    let vec = |w: &Option<PartialVec3>, old: Vec3| match w {
        Some(w) => Vec3::new(
            w.x.unwrap_or(old.x),
            w.y.unwrap_or(old.y),
            w.z.unwrap_or(old.z),
        ),
        None => old,
    };
    let rotation = match &want.rotation {
        Some(r) => {
            let [pitch, yaw, roll] = quat_to_rotator(rotation);
            rotator_to_quat([
                r.pitch.unwrap_or(pitch),
                r.yaw.unwrap_or(yaw),
                r.roll.unwrap_or(roll),
            ])
        }
        None => rotation,
    };
    (
        vec(&want.location, position),
        rotation,
        vec(&want.velocity, velocity),
        vec(&want.angular_velocity, angular_velocity),
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod regressions {
    use super::*;
    use rb_rlbot_wire::DesiredCarState;

    fn active(length: MatchLengthMutator) -> Simulation {
        let mut sim = Simulation::new(bot_match(
            &[("blue".into(), 0), ("orange".into(), 1)],
            length,
        ))
        .unwrap();
        sim.frame = sim.env.reset(&sim.frame);
        sim
    }

    #[test]
    fn demolition_preserves_slots_policy_input_and_state_targets_through_respawn() {
        let mut sim = active(MatchLengthMutator::FiveMinutes);
        let mut frame = sim.frame.clone();
        for car in &mut frame.cars {
            car.position = Vec3::new(0.0, 0.0, 500.0);
            car.rotation = rotator_to_quat([0.0, std::f32::consts::FRAC_PI_2, 0.0]);
            car.velocity = Vec3::ZERO;
        }
        frame.cars[1].position.y = -130.0;
        frame.cars[1].velocity.y = 2300.0;
        sim.frame = sim.env.reset(&frame);
        for _ in 0..4 {
            sim.step();
        }
        assert_eq!(sim.frame.cars.len(), 1);
        assert_eq!(sim.frame.cars[0].player_id, 1);
        let packet = sim.packet();
        assert_eq!(packet.players.len(), 2);
        assert_eq!(packet.players[0].demolished_timeout, 0.0);
        assert_eq!(
            (packet.players[1].player_id, packet.players[1].team),
            (101, 1)
        );
        let mut bot = rb_rlbot_bridge::PolicyBot::new(
            |_: &PhysicsFrame, index: usize| {
                assert_eq!(index, 1);
                ControllerInput::default()
            },
            1,
        );
        assert_eq!(bot.input(&packet).unwrap().player_index, 1);
        let before = packet.players[1].physics;
        assert!(sim
            .apply_desired(&DesiredGameState {
                car_states: vec![
                    DesiredCarState {
                        boost_amount: Some(1.0),
                        ..Default::default()
                    },
                    DesiredCarState {
                        boost_amount: Some(77.0),
                        ..Default::default()
                    }
                ],
                ..Default::default()
            })
            .unwrap());
        assert_eq!(sim.packet().players[1].boost, 77.0);
        assert_eq!(sim.packet().players[1].physics, before);
        for _ in 0..361 {
            sim.step();
        }
        assert_eq!(sim.packet().players[0].demolished_timeout, -1.0);
        assert_eq!(sim.packet().players[1].player_id, 101);
    }

    #[test]
    fn elapsed_pad_time_increases_then_resets_on_respawn() {
        for (x, y) in [(0.0, -1024.0), (-3584.0, 0.0)] {
            let mut sim = active(MatchLengthMutator::FiveMinutes);
            let mut frame = sim.frame.clone();
            frame.cars[0].position = Vec3::new(x, y, 17.0);
            frame.cars[0].boost_amount = 0.0;
            sim.frame = sim.env.reset(&frame);
            sim.step();
            let index = sim
                .packet()
                .boost_pads
                .iter()
                .position(|p| !p.is_active)
                .unwrap();
            assert!(sim.packet().boost_pads[index].timer < 0.02);
            let mut away = sim.frame.clone();
            away.cars[0].position.x = 700.0;
            sim.env.snap(&away);
            for _ in 0..120 {
                sim.step();
            }
            assert!((sim.packet().boost_pads[index].timer - 1.0).abs() < 0.02);
            for _ in 0..1200 {
                sim.step();
            }
            assert_eq!(sim.packet().boost_pads[index].timer, 0.0);
            assert!(sim.packet().boost_pads[index].is_active);
        }
    }

    #[test]
    fn replay_elapsed_time_advances_while_match_clock_holds_and_unlimited_counts_play() {
        let mut sim = active(MatchLengthMutator::Unlimited);
        for _ in 0..120 {
            sim.step();
        }
        assert_eq!(sim.packet().match_info.game_time_remaining, 1.0);
        let mut goal = sim.frame.clone();
        goal.ball.position = Vec3::new(0.0, 5300.0, 100.0);
        sim.env.snap(&goal);
        sim.step();
        while sim.packet().match_info.match_phase != MatchPhase::Replay {
            sim.step();
        }
        let before = sim.packet();
        for _ in 0..120 {
            sim.step();
        }
        let after = sim.packet();
        assert!(
            (after.match_info.seconds_elapsed - before.match_info.seconds_elapsed - 1.0).abs()
                < 1e-5
        );
        assert_eq!(
            after.match_info.game_time_remaining,
            before.match_info.game_time_remaining
        );
        assert_eq!(after.balls, before.balls);
    }

    #[test]
    fn overtime_starts_at_zero_and_counts_up_only_in_play() {
        let mut sim = active(MatchLengthMutator::FiveMinutes);
        sim.env.set_match_length(Some(1));
        let mut frame = sim.frame.clone();
        frame.ball.position = Vec3::new(0.0, 0.0, 93.0);
        sim.frame = sim.env.reset(&frame);
        for _ in 0..120 {
            sim.step();
            if sim.packet().match_info.is_overtime {
                break;
            }
        }
        assert!(sim.packet().match_info.is_overtime);
        assert_eq!(sim.packet().match_info.game_time_remaining, 0.0);
        while sim.packet().match_info.match_phase != MatchPhase::Active {
            sim.step();
        }
        for _ in 0..120 {
            sim.step();
        }
        assert_eq!(sim.packet().match_info.game_time_remaining, 1.0);
    }
}
