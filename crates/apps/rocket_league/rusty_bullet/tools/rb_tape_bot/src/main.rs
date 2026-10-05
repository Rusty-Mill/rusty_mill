//! Tape-player bot for RLBot v5: sets a scenario's start state on its first
//! packet, then replays the scenario's input timeline one input per game
//! packet, so the BakkesMod capture plugin records a repeatable run.
//! See docs/research/BOT-CAPTURE-PLAN.md.
//!
//! Set `RB_TAPE` to the scenario JSON path before RLBot starts the bot.

use std::sync::Arc;

use rb_tape_bot::{BallStart, CarStart, Input, Scenario};
use rlbot::{
    agents::{run_bot_agents, BotAgent},
    flat::{
        ControllableInfo, ControllerState, DesiredBallState, DesiredCarState, DesiredGameState,
        DesiredPhysics, FieldInfo, Float, GamePacket, MatchConfiguration, MatchPhase, PlayerInput,
        RotatorPartial, Vector3Partial,
    },
    util::{AgentEnvironment, PacketQueue},
    RLBotConnection,
};

struct TapeBot {
    index: u32,
    scenario: Scenario,
    /// Active packets seen so far.
    packets: u64,
}

fn float(val: f32) -> Option<Float> {
    Some(Float { val })
}

fn vector(v: [f32; 3]) -> Option<Box<Vector3Partial>> {
    Some(Box::new(Vector3Partial {
        x: float(v[0]),
        y: float(v[1]),
        z: float(v[2]),
    }))
}

fn rotator(r: [f32; 3]) -> Option<Box<RotatorPartial>> {
    Some(Box::new(RotatorPartial {
        pitch: float(r[0]),
        yaw: float(r[1]),
        roll: float(r[2]),
    }))
}

fn car_state(car: &CarStart) -> DesiredCarState {
    DesiredCarState {
        physics: Some(Box::new(DesiredPhysics {
            location: car.location.and_then(vector),
            rotation: car.rotation.and_then(rotator),
            velocity: car.velocity.and_then(vector),
            angular_velocity: car.angular_velocity.and_then(vector),
        })),
        boost_amount: car.boost.and_then(float),
    }
}

fn ball_state(ball: &BallStart) -> DesiredBallState {
    DesiredBallState {
        physics: Box::new(DesiredPhysics {
            location: ball.location.and_then(vector),
            rotation: None,
            velocity: ball.velocity.and_then(vector),
            angular_velocity: ball.angular_velocity.and_then(vector),
        }),
    }
}

fn controller(input: Input) -> ControllerState {
    ControllerState {
        throttle: input.throttle,
        steer: input.steer,
        pitch: input.pitch,
        yaw: input.yaw,
        roll: input.roll,
        jump: input.jump,
        boost: input.boost,
        handbrake: input.handbrake,
        ..Default::default()
    }
}

impl BotAgent for TapeBot {
    fn new(
        _team: u32,
        controllable_info: ControllableInfo,
        _match_config: Arc<MatchConfiguration>,
        _field_info: Arc<FieldInfo>,
        _packet_queue: &mut PacketQueue,
    ) -> Self {
        let path = std::env::var("RB_TAPE").expect("set RB_TAPE to the scenario JSON path");
        let text = std::fs::read_to_string(&path).expect("read the scenario file");
        let scenario = Scenario::from_json(&text).expect("parse the scenario");
        println!(
            "tape bot: '{}' ({} ticks)",
            scenario.name,
            scenario.total_ticks()
        );
        Self {
            index: controllable_info.index,
            scenario,
            packets: 0,
        }
    }

    fn tick(&mut self, game_packet: &GamePacket, packet_queue: &mut PacketQueue) {
        // Only the first car plays the tape; any other stays neutral.
        if self.index != 0 || game_packet.match_info.match_phase != MatchPhase::Active {
            return;
        }
        let packet = self.packets;
        self.packets += 1;
        if packet == 0 {
            packet_queue.push(DesiredGameState {
                ball_states: self.scenario.ball.iter().map(ball_state).collect(),
                car_states: vec![car_state(&self.scenario.car)],
                ..Default::default()
            });
        }
        // The first packet only sets the state; the tape starts on the next.
        let input = match packet.checked_sub(1) {
            Some(tick) => self.scenario.input_at(tick),
            None => Input::default(),
        };
        packet_queue.push(PlayerInput {
            player_index: self.index,
            controller_state: controller(input),
        });
    }
}

fn main() {
    let AgentEnvironment {
        server_addr,
        agent_id,
    } = AgentEnvironment::from_env();
    let agent_id = agent_id.unwrap_or_else(|| "rusty_bullet/tape_bot".into());
    let connection = RLBotConnection::new(&server_addr).expect("connect to RLBot core");
    run_bot_agents::<TapeBot>(agent_id.clone(), false, false, connection)
        .expect("run_bot_agents crashed");
    println!("tape bot `{agent_id}` exited");
}
