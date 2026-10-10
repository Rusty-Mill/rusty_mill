//! Tape-player bot for RLBot v5: sets a scenario's start state on its first
//! packet, then replays the scenario's input timeline one input per game
//! packet, so the BakkesMod capture plugin records a repeatable run.
//! See docs/research/BOT-CAPTURE-PLAN.md.
//!
//! Set `RB_TAPE` to the scenario JSON path before RLBot starts the bot.

use rb_rlbot_client::{run_bots, Agent, Connection, Environment, Outbox};
use rb_rlbot_wire::{ConnectionSettings, GamePacket, MatchPhase, PlayerInput};
use rb_scenario::{Input, Scenario};
use rb_tape_bot::{controller, start_state};

struct TapeBot {
    index: u32,
    scenario: Scenario,
    /// Physics frame of the previous packet, to tell a ticking game from a
    /// paused one.
    last_frame: Option<u32>,
    /// Physics frame of the first live packet, where the state was set.
    start_frame: Option<u32>,
}

impl TapeBot {
    fn new(index: u32) -> Self {
        let path = std::env::var("RB_TAPE").expect("set RB_TAPE to the scenario JSON path");
        let text = std::fs::read_to_string(&path).expect("read the scenario file");
        let scenario = Scenario::from_json(&text).expect("parse the scenario");
        println!(
            "tape bot: '{}' ({} ticks)",
            scenario.name,
            scenario.total_ticks()
        );
        Self {
            index,
            scenario,
            last_frame: None,
            start_frame: None,
        }
    }
}

impl Agent for TapeBot {
    fn tick(&mut self, game_packet: &GamePacket, packet_queue: &mut Outbox) {
        // Only the first car plays the tape; any other stays neutral.
        if self.index != 0 {
            return;
        }
        let info = &game_packet.match_info;
        // Freeplay started by RLBot core reports `MatchPhase::Paused` even
        // while physics runs (observed 2026-10-06, core rc17), and a game
        // paused with Escape repeats one packet 240 times a second with the
        // same frame number. So the gate is the physics frame counter, not
        // the phase: act only when the frame advanced since the last packet.
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
                    "tape bot: physics ticking at frame {frame} (phase {:?}), setting the start state",
                    info.match_phase
                );
                self.start_frame = Some(frame);
                packet_queue.push(start_state(&self.scenario));
                frame
            }
        };
        // The first live frame only sets the state; the tape starts on the
        // next one, indexed by physics frame so a dropped packet does not
        // shift the tape.
        let input = match (frame - start).checked_sub(1) {
            Some(tick) => self.scenario.input_at(u64::from(tick)),
            None => Input::default(),
        };
        packet_queue.push(PlayerInput {
            player_index: self.index,
            controller_state: controller(input),
        });
    }
}

fn main() {
    let Environment {
        server_addr,
        agent_id,
    } = Environment::from_env();
    let agent_id = agent_id.unwrap_or_else(|| "rusty_bullet/tape_bot".into());
    let mut connection = Connection::connect(&server_addr).expect("connect to RLBot core");
    let settings = ConnectionSettings {
        agent_id: agent_id.clone(),
        wants_ball_predictions: false,
        wants_comms: false,
        close_between_matches: true,
    };
    run_bots(&mut connection, settings, |init, _| {
        TapeBot::new(init.controllable.index)
    })
    .expect("run_bots crashed");
    println!("tape bot `{agent_id}` exited");
}
