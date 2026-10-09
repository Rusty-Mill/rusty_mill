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

use rb_scenario::Scenario;
use rb_tape_bot::{controller, start_state};
use rlbot::{
    agents::{run_hivemind_agent, HivemindAgent},
    flat::{
        ControllableTeamInfo, FieldInfo, GamePacket, MatchConfiguration, MatchPhase, PlayerInput,
    },
    util::{AgentEnvironment, PacketQueue},
    RLBotConnection,
};

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

impl HivemindAgent for TapeHive {
    fn new(
        controllable_team_info: ControllableTeamInfo,
        _match_configuration: MatchConfiguration,
        _field_info: FieldInfo,
        _packet_queue: &mut PacketQueue,
    ) -> Self {
        let path = std::env::var("RB_TAPE").expect("set RB_TAPE to the scenario JSON path");
        let text = std::fs::read_to_string(&path).expect("read the scenario file");
        let scenario = Scenario::from_json(&text).expect("parse the scenario");
        let indices: Vec<u32> = controllable_team_info
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

    fn tick(&mut self, game_packet: GamePacket, packet_queue: &mut PacketQueue) {
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
    let AgentEnvironment {
        server_addr,
        agent_id,
    } = AgentEnvironment::from_env();
    let agent_id = agent_id.unwrap_or_else(|| "rusty_bullet/tape_hive".into());
    let connection = RLBotConnection::new(&server_addr).expect("connect to RLBot core");
    run_hivemind_agent::<TapeHive>(agent_id.clone(), false, false, connection)
        .expect("run_hivemind_agent crashed");
    println!("tape hive `{agent_id}` exited");
}
