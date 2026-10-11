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
    time::{Duration, Instant},
};

use rb_rlbot_client::{Connection, CoreMessage};
use rb_rlbot_wire::{
    ConnectionSettings, DebugRendering, DesiredBallState, DesiredGameState, DesiredPhysics,
    ExistingMatchBehavior, GameMode, GamePacket, InitComplete, Launcher, MatchConfiguration,
    MatchLengthMutator, MatchPhase, MutatorSettings, PlayerClass, PlayerConfiguration, PsyonixBot,
    PsyonixSkill, StopCommand,
};
use rb_tape_bot::{
    tie_up::{overtime_goal_due, Action, TieUp},
    vector,
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
        variety: PlayerClass::PsyonixBot(PsyonixBot {
            name: String::new(),
            skill: PsyonixSkill::Beginner,
        }),
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
        mutators: Some(MutatorSettings {
            match_length: length,
        }),
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
            physics: DesiredPhysics {
                location: Some(vector([0.0, 4800.0 * side, 400.0])),
                velocity: Some(vector([0.0, 2500.0 * side, 0.0])),
                angular_velocity: Some(vector([0.0, 0.0, 0.0])),
                rotation: None,
            },
        }],
        ..Default::default()
    }
}

fn main() -> Result<()> {
    let out = arg("--out").unwrap_or_else(|| "match_log.jsonl".into());
    let seconds: u64 = arg("--seconds").and_then(|s| s.parse().ok()).unwrap_or(60);
    let goal_after: Option<f32> = arg("--goal-after").and_then(|s| s.parse().ok());
    let mut conn = Connection::connect(CORE_ADDR)?;
    conn.send(ConnectionSettings {
        wants_ball_predictions: false,
        wants_comms: false,
        close_between_matches: false,
        agent_id: String::new(),
    })?;
    conn.send(InitComplete)?;
    let length = match arg("--length").as_deref() {
        Some("five") => MatchLengthMutator::FiveMinutes,
        _ => MatchLengthMutator::Unlimited,
    };
    let overtime_goal = std::env::args().any(|a| a == "--overtime-goal");
    let mut overtime_goal_sent = false;
    let tie_up = std::env::args().any(|a| a == "--tie-up");
    let mut tie = TieUp::default();
    conn.send(config(length))?;
    let mut file = File::create(&out)?;
    let started = Instant::now();
    let goals: u32 = arg("--goals").and_then(|s| s.parse().ok()).unwrap_or(1);
    let mut active_since: Option<f32> = None;
    let mut goals_sent = 0u32;
    let mut last_phase = MatchPhase::Inactive;
    let mut packets = 0u64;
    while started.elapsed() < Duration::from_secs(seconds) {
        // A short wait stands in for the old non-blocking poll: the loop re-checks its deadline
        // about every 2 ms when core is quiet.
        match conn.recv_timeout(Duration::from_millis(2)) {
            Ok(Some(CoreMessage::GamePacket(p))) => {
                packets += 1;
                writeln!(file, "{}", line(&p))?;
                if p.match_info.match_phase != last_phase {
                    if p.match_info.match_phase == MatchPhase::Active {
                        active_since = Some(p.match_info.seconds_elapsed);
                    }
                    last_phase = p.match_info.match_phase;
                }
                if tie_up
                    && !p.match_info.is_overtime
                    && p.match_info.match_phase == MatchPhase::Active
                    && p.match_info.game_time_remaining < 20.0
                    && p.match_info.game_time_remaining > 8.0
                {
                    let score = |i: usize| p.teams.get(i).map_or(0, |t| t.score);
                    if let Action::Shoot(side) = tie.observe(score(0), score(1)) {
                        conn.send(goal_state(side))?;
                        println!(
                            "tie-up goal sent at remaining {:.2}",
                            p.match_info.game_time_remaining
                        );
                    }
                }
                if overtime_goal
                    && overtime_goal_due(
                        p.match_info.is_overtime,
                        p.match_info.match_phase == MatchPhase::Active,
                        overtime_goal_sent,
                    )
                {
                    conn.send(goal_state(1.0))?;
                    overtime_goal_sent = true;
                    println!(
                        "overtime goal sent at remaining {:.2}",
                        p.match_info.game_time_remaining
                    );
                }
                if let (MatchPhase::Active, Some(since), Some(after)) =
                    (p.match_info.match_phase, active_since, goal_after)
                {
                    if goals_sent < goals && p.match_info.seconds_elapsed - since >= after {
                        let side = if goals_sent.is_multiple_of(2) {
                            1.0
                        } else {
                            -1.0
                        };
                        conn.send(goal_state(side))?;
                        goals_sent += 1;
                        active_since = None;
                        println!(
                            "goal {goals_sent} state sent at elapsed {:.2}",
                            p.match_info.seconds_elapsed
                        );
                    }
                }
            }
            Ok(_) => {}
            Err(e) => return Err(format!("core connection failed: {e}").into()),
        }
    }
    conn.send(StopCommand {
        shutdown_server: false,
    })?;
    println!("{packets} packets written to {out}");
    Ok(())
}
