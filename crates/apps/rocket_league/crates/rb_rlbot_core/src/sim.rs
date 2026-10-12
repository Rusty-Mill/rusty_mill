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
}

impl std::fmt::Display for SetupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SetupError::NoPlayers => f.write_str("the match configuration has no players"),
            SetupError::BadTeam { player, team } => {
                write!(f, "player {player} is on team {team}; only 0 and 1 exist")
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
        if let PlayerClass::CustomBot(bot) = &p.variety {
            if !ids.contains(&bot.agent_id) {
                ids.push(bot.agent_id.clone());
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
        if bot.agent_id != agent_id {
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
                *slot = controller_input(state);
                true
            }
            None => false,
        }
    }

    /// Advances one physics tick (1/120 s) with the held inputs.
    pub fn step(&mut self) {
        self.frame = self.env.step(&self.inputs);
        self.frame_num += 1;
    }

    /// Applies a client's state-setting message: the fields it names change, the rest stay.
    /// Only the first ball and the cars it lists by index are touched. Match info (gravity,
    /// game speed) cannot be changed in `rb_env` and is ignored. Returns whether the message
    /// named a car that does not exist (the rest of it was still applied).
    pub fn apply_desired(&mut self, desired: &DesiredGameState) -> bool {
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
            let Some(car) = frame.cars.get_mut(index) else {
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
        unknown_car
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
                        timer: pads.cooldown_left(i),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let configured = &self.configuration.player_configurations;
        let players = self
            .frame
            .cars
            .iter()
            .enumerate()
            .map(|(index, car)| PlayerInfo {
                physics: physics(
                    car.position,
                    car.rotation,
                    car.velocity,
                    car.angular_velocity,
                ),
                air_state: air_state(car),
                demolished_timeout: -1.0,
                is_bot: configured
                    .get(index)
                    .is_some_and(|p| !matches!(p.variety, PlayerClass::Human)),
                name: configured.get(index).map(name_of).unwrap_or_default(),
                team: self.teams[index],
                boost: car.boost_amount,
                player_id: configured.get(index).map_or(index as i32, |p| p.player_id),
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
                seconds_elapsed: self.frame.timestamp_secs,
                game_time_remaining: state
                    .and_then(|s| s.seconds_remaining())
                    .unwrap_or(0.0)
                    .max(0.0),
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
