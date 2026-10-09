//! Match-flow probe (PARITY-PLAN workstream E): starts a real (not freeplay) match with one
//! Psyonix bot per team through RLBot core's socket and writes one JSON line per game packet
//! to a file: frame, match phase, clock, score, the ball and every car. After `--goal-after`
//! seconds of the Active phase it sets the ball inside the orange goal (state setting), to
//! record the goal, replay, reset and the next kickoff.
//!
//! Needs the game up as for `rb_run_tapes` (RLBot GUI Start Match once, core listening).
//! Usage: `rb_match_log [--out FILE] [--seconds N] [--goal-after S]`.

use std::{
    error::Error,
    fs::File,
    io::Write as _,
    thread::sleep,
    time::{Duration, Instant},
};

use rb_tape_bot::vector;
use rlbot::{
    flat::{
        ConnectionSettings, CoreMessage, DebugRendering, DesiredBallState, DesiredGameState,
        DesiredPhysics, ExistingMatchBehavior, GameMode, GamePacket, InitComplete, Launcher,
        MatchConfiguration, MatchLengthMutator, MatchPhase, MutatorSettings, PlayerClass,
        PlayerConfiguration, PsyonixBot, StopCommand,
    },
    RLBotConnection,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const CORE_ADDR: &str = "127.0.0.1:23234";

fn arg(name: &str) -> Option<String> {
    let mut args = std::env::args();
    while let Some(a) = args.next() {
        if a == name {
            return args.next();
        }
    }
    None
}

fn bot(team: u32, player_id: i32) -> PlayerConfiguration {
    PlayerConfiguration {
        variety: PlayerClass::PsyonixBot(Box::new(PsyonixBot::default())),
        team,
        player_id,
    }
}

fn config(length: MatchLengthMutator) -> MatchConfiguration {
    MatchConfiguration {
        launcher: Launcher::Epic,
        auto_start_agents: true,
        wait_for_agents: true,
        game_map_upk: "Stadium_P".into(),
        player_configurations: vec![bot(0, 0), bot(1, 1)],
        game_mode: GameMode::Soccar,
        mutators: Some(Box::new(MutatorSettings {
            match_length: length,
            ..Default::default()
        })),
        existing_match_behavior: ExistingMatchBehavior::Restart,
        enable_rendering: DebugRendering::OffByDefault,
        enable_state_setting: true,
        freeplay: false,
        ..Default::default()
    }
}

fn line(p: &GamePacket) -> String {
    let m = &p.match_info;
    let score = |i: usize| p.teams.get(i).map_or(0, |t| t.score);
    let ball = p.balls.first().map_or_else(String::new, |b| {
        let (l, v) = (&b.physics.location, &b.physics.velocity);
        format!(
            r#""ball":{{"p":[{:.2},{:.2},{:.2}],"v":[{:.2},{:.2},{:.2}]}},"#,
            l.x, l.y, l.z, v.x, v.y, v.z
        )
    });
    let cars: Vec<String> = p
        .players
        .iter()
        .map(|c| {
            let (l, v) = (&c.physics.location, &c.physics.velocity);
            format!(
                r#"{{"p":[{:.2},{:.2},{:.2}],"v":[{:.2},{:.2},{:.2}],"boost":{:.2},"demo":{}}}"#,
                l.x, l.y, l.z, v.x, v.y, v.z, c.boost, c.demolished_timeout
            )
        })
        .collect();
    format!(
        r#"{{"frame":{},"phase":"{:?}","elapsed":{:.4},"remaining":{:.3},"overtime":{},"blue":{},"orange":{},"pads_active":{},{}"cars":[{}]}}"#,
        m.frame_num,
        m.match_phase,
        m.seconds_elapsed,
        m.game_time_remaining,
        m.is_overtime,
        score(0),
        score(1),
        p.boost_pads.iter().filter(|pad| pad.is_active).count(),
        ball,
        cars.join(",")
    )
}

/// The ball just in front of a goal mouth, flying in: `side` +1 is the orange goal (+y).
fn goal_state(side: f32) -> DesiredGameState {
    DesiredGameState {
        ball_states: vec![DesiredBallState {
            physics: Box::new(DesiredPhysics {
                location: vector([0.0, 4800.0 * side, 400.0]),
                velocity: vector([0.0, 2500.0 * side, 0.0]),
                angular_velocity: vector([0.0, 0.0, 0.0]),
                rotation: None,
            }),
        }],
        ..Default::default()
    }
}

fn main() -> Result<()> {
    let out = arg("--out").unwrap_or_else(|| "match_log.jsonl".into());
    let seconds: u64 = arg("--seconds").and_then(|s| s.parse().ok()).unwrap_or(60);
    let goal_after: Option<f32> = arg("--goal-after").and_then(|s| s.parse().ok());
    let mut conn = RLBotConnection::new(CORE_ADDR)?;
    conn.send_packet(ConnectionSettings {
        wants_ball_predictions: false,
        wants_comms: false,
        close_between_matches: false,
        agent_id: String::new(),
    })?;
    conn.send_packet(InitComplete {})?;
    conn.set_nonblocking(true)?;
    let length = match arg("--length").as_deref() {
        Some("five") => MatchLengthMutator::FiveMinutes,
        _ => MatchLengthMutator::Unlimited,
    };
    let overtime_goal = std::env::args().any(|a| a == "--overtime-goal");
    let mut overtime_goal_sent = false;
    let tie_up = std::env::args().any(|a| a == "--tie-up");
    let mut tied = false;
    conn.send_packet(config(length))?;
    let mut file = File::create(&out)?;
    let started = Instant::now();
    let goals: u32 = arg("--goals").and_then(|s| s.parse().ok()).unwrap_or(1);
    let mut active_since: Option<f32> = None;
    let mut goals_sent = 0u32;
    let mut last_phase = MatchPhase::Inactive;
    let mut packets = 0u64;
    while started.elapsed() < Duration::from_secs(seconds) {
        match conn.recv_packet() {
            Ok(CoreMessage::GamePacket(p)) => {
                packets += 1;
                writeln!(file, "{}", line(&p))?;
                if p.match_info.match_phase != last_phase {
                    if p.match_info.match_phase == MatchPhase::Active {
                        active_since = Some(p.match_info.seconds_elapsed);
                    }
                    last_phase = p.match_info.match_phase;
                }
                if tie_up
                    && !tied
                    && !p.match_info.is_overtime
                    && p.match_info.match_phase == MatchPhase::Active
                    && p.match_info.game_time_remaining < 20.0
                    && p.match_info.game_time_remaining > 8.0
                {
                    let (blue, orange) = (p.teams.first().map_or(0, |t| t.score), p.teams.get(1).map_or(0, |t| t.score));
                    if blue == orange {
                        tied = true;
                    } else {
                        conn.send_packet(goal_state(if blue < orange { 1.0 } else { -1.0 }))?;
                        println!("tie-up goal sent at remaining {:.2}", p.match_info.game_time_remaining);
                        tied = true;
                    }
                }
                if overtime_goal
                    && !overtime_goal_sent
                    && p.match_info.is_overtime
                    && p.match_info.match_phase == MatchPhase::Active
                    && p.match_info.game_time_remaining > 3.0
                {
                    conn.send_packet(goal_state(1.0))?;
                    overtime_goal_sent = true;
                    println!("overtime goal sent at remaining {:.2}", p.match_info.game_time_remaining);
                }
                if let (MatchPhase::Active, Some(since), Some(after)) =
                    (p.match_info.match_phase, active_since, goal_after)
                {
                    if goals_sent < goals && p.match_info.seconds_elapsed - since >= after {
                        let side = if goals_sent % 2 == 0 { 1.0 } else { -1.0 };
                        conn.send_packet(goal_state(side))?;
                        goals_sent += 1;
                        active_since = None;
                        println!("goal {goals_sent} state sent at elapsed {:.2}", p.match_info.seconds_elapsed);
                    }
                }
            }
            Ok(_) => {}
            Err(rlbot::RLBotError::Connection(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                sleep(Duration::from_millis(2));
            }
            Err(e) => return Err(format!("core connection failed: {e}").into()),
        }
    }
    conn.send_packet(StopCommand {
        shutdown_server: false,
    })?;
    println!("{packets} packets written to {out}");
    Ok(())
}
