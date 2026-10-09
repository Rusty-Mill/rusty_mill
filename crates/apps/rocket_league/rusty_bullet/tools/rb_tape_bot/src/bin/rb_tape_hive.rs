//! Hivemind tape bot for RLBot v5: one process drives every car of a
//! scenario (`others` in the scenario file), so all their tapes share one
//! clock. It sets the start state of every car and the ball on the first
//! packet where physics ticks, then plays each car's tape one input per game
//! packet, as `rb_tape_bot` does for one car. A bump or demolition test needs
//! this: two bot processes would each find their own start frame.
//!
//! Set `RB_TAPE` to the scenario JSON path before RLBot starts the bot, and
//! start every car of the scenario as a player of one team with the
//! hivemind flag (`rb_run_tapes` does).

use rb_rlbot_client::{run_hivemind, Agent, Connection, Environment, Outbox, StartingInfo};
use rb_rlbot_wire::{ConnectionSettings, GamePacket, MatchPhase, PlayerInput};
use rb_scenario::Scenario;
use rb_tape_bot::{controller, start_state};

struct TapeHive {
    /// Game car indices this process drives, from core.
    indices: Vec<u32>,
    scenario: Scenario,
    /// Physics frame of the previous packet, to tell a ticking game from a
    /// paused one.
    last_frame: Option<u32>,
    /// Physics frame of the first live packet, where the state was set.
    start_frame: Option<u32>,
}

impl TapeHive {
    fn new(info: &StartingInfo) -> Self {
        let path = std::env::var("RB_TAPE").expect("set RB_TAPE to the scenario JSON path");
        let text = std::fs::read_to_string(&path).expect("read the scenario file");
        let scenario = Scenario::from_json(&text).expect("parse the scenario");
        let indices: Vec<u32> = info
            .controllable_team_info
            .controllables
            .iter()
            .map(|controllable| controllable.index)
            .collect();
        println!(
            "tape hive: '{}' ({} ticks), driving cars {:?} of {}",
            scenario.name,
            scenario.total_ticks(),
            indices,
            scenario.car_count()
        );
        Self {
            indices,
            scenario,
            last_frame: None,
            start_frame: None,
        }
    }
}

impl Agent for TapeHive {
    fn tick(&mut self, game_packet: &GamePacket, packet_queue: &mut Outbox) {
        let info = &game_packet.match_info;
        // Same gate as `rb_tape_bot`: act only when the physics frame
        // advanced (core reports `Paused` throughout freeplay).
        if matches!(
            info.match_phase,
            MatchPhase::Replay | MatchPhase::Ended | MatchPhase::GoalScored
        ) {
            return;
        }
        let frame = info.frame_num;
        let Some(last) = self.last_frame.replace(frame) else {
            return;
        };
        if frame == last {
            return;
        }
        let start = match self.start_frame {
            Some(start) => start,
            None => {
                println!(
                    "tape hive: physics ticking at frame {frame} (phase {:?}), setting the start state",
                    info.match_phase
                );
                self.start_frame = Some(frame);
                packet_queue.push(start_state(&self.scenario));
                frame
            }
        };
        // The first live frame only sets the state; the tape starts on the
        // next one, indexed by physics frame so a dropped packet does not
        // shift it.
        let tick = (frame - start).checked_sub(1);
        for &index in &self.indices {
            let input = match tick {
                Some(tick) => self.scenario.input_at_car(index as usize, u64::from(tick)),
                None => Default::default(),
            };
            packet_queue.push(PlayerInput {
                player_index: index,
                controller_state: controller(input),
            });
        }
    }
}

fn main() {
    let Environment {
        server_addr,
        agent_id,
    } = Environment::from_env();
    let agent_id = agent_id.unwrap_or_else(|| "rusty_bullet/tape_hive".into());
    let mut connection = Connection::connect(&server_addr).expect("connect to RLBot core");
    let settings = ConnectionSettings {
        agent_id: agent_id.clone(),
        wants_ball_predictions: false,
        wants_comms: false,
        close_between_matches: true,
    };
    run_hivemind(&mut connection, settings, |info, _| TapeHive::new(info))
        .expect("run_hivemind crashed");
    println!("tape hive `{agent_id}` exited");
}
