//! Diagnostic: connect to RLBot core as a passive client and print what the
//! game packets say (match phase, frame rate, players, car 0 state), for a
//! few seconds. Use when the tape bot does nothing and you need to know
//! whether the match is active and whether packets flow at 120 per second.

use std::time::{Duration, Instant};

use rlbot::{
    flat::{ConnectionSettings, CoreMessage, InitComplete},
    util::AgentEnvironment,
    RLBotConnection,
};

fn main() {
    let AgentEnvironment { server_addr, .. } = AgentEnvironment::from_env();
    let mut conn = RLBotConnection::new(&server_addr).expect("connect to RLBot core");
    conn.send_packet(ConnectionSettings {
        wants_ball_predictions: false,
        wants_comms: false,
        close_between_matches: true,
        agent_id: String::new(),
    })
    .expect("send connection settings");
    // Core only distributes game packets to a session after InitComplete.
    conn.send_packet(InitComplete {})
        .expect("send init complete");
    println!("probe: connected to {server_addr}");

    let start = Instant::now();
    let mut packets = 0u64;
    let mut last_report = Instant::now();
    let mut first_frame: Option<u32> = None;
    let mut last_frame = 0u32;
    let mut other = 0u64;
    // Optional first argument: how many seconds to listen (default 6).
    let seconds: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(6);
    while start.elapsed() < Duration::from_secs(seconds) {
        match conn.recv_packet() {
            Ok(CoreMessage::GamePacket(gp)) => {
                packets += 1;
                let mi = &gp.match_info;
                first_frame.get_or_insert(mi.frame_num);
                last_frame = mi.frame_num;
                if last_report.elapsed() >= Duration::from_secs(1) {
                    last_report = Instant::now();
                    let p0 = gp.players.first();
                    println!(
                        "t={:.1}s packets={} frame={} phase={:?} elapsed={:.2} players={} balls={} car0={}",
                        start.elapsed().as_secs_f32(),
                        packets,
                        mi.frame_num,
                        mi.match_phase,
                        mi.seconds_elapsed,
                        gp.players.len(),
                        gp.balls.len(),
                        p0.map(|p| format!(
                            "'{}' bot={} loc=({:.1},{:.1},{:.1}) rot=(p{:.3} y{:.3} r{:.3}) air={:?} boost={:.0}",
                            p.name, p.is_bot,
                            p.physics.location.x, p.physics.location.y, p.physics.location.z,
                            p.physics.rotation.pitch, p.physics.rotation.yaw, p.physics.rotation.roll,
                            p.air_state, p.boost
                        )).unwrap_or_else(|| "none".into())
                    );
                }
            }
            Ok(msg) => {
                other += 1;
                let name = format!("{msg:?}");
                println!("other message: {}", &name[..name.len().min(120)]);
            }
            Err(e) => {
                println!("recv error: {e}");
                break;
            }
        }
    }
    let frames = first_frame
        .map(|f| last_frame.saturating_sub(f))
        .unwrap_or(0);
    println!(
        "probe: {packets} game packets, {frames} frames advanced in {:.1}s ({:.1} packets/s, {:.1} frames/s), {other} other messages",
        start.elapsed().as_secs_f32(),
        packets as f32 / start.elapsed().as_secs_f32(),
        frames as f32 / start.elapsed().as_secs_f32()
    );
}
