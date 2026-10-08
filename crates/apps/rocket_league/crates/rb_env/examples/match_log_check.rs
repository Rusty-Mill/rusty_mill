//! Replays a game match log (`rb_match_log`, one JSON line per packet) through the match-flow
//! phase machine and compares it with what the game did: the phase on every frame, the clock,
//! the score. The ball and the frame numbers are the log's; everything else (goals, the end of
//! the replay, the countdown, touches, the clock running out, overtime, the end) is `Flow`'s call.
//!
//! `cargo run -p rb_env --example match_log_check -- replays/match/log6.jsonl [five]`
//! (`five`: a five-minute clock; without it the match is unlimited).

use std::{error::Error, fs};

use rb_domain::Vec3;
use rb_env::flow::{Flow, Phase, Transition, FIRST_COUNTDOWN_TICKS, FIVE_MINUTES};

/// Seconds the clock may differ by: the game's started up to 12 ticks before its first `Active`.
const CLOCK_TOLERANCE: f32 = 0.15;

fn phase_of(name: &str) -> Phase {
    match name {
        "Kickoff" => Phase::Kickoff,
        "Active" => Phase::Active,
        "GoalScored" => Phase::GoalScored,
        "Replay" => Phase::Replay,
        "Ended" => Phase::Ended,
        _ => Phase::Countdown, // Countdown and the match intro (Paused)
    }
}

fn vec3(value: &serde_json::Value) -> Vec3 {
    let at = |i: usize| value[i].as_f64().unwrap_or(0.0) as f32;
    Vec3::new(at(0), at(1), at(2))
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("usage: match_log_check LOG [five]")?;
    let five = args.next().as_deref() == Some("five");
    let rows: Vec<serde_json::Value> = fs::read_to_string(&path)?
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    // A log may start with the last packets of the previous match; the match proper starts where the
    // frame counter resets.
    let start = (1..rows.len())
        .rev()
        .find(|&i| rows[i]["frame"].as_u64() < rows[i - 1]["frame"].as_u64())
        .unwrap_or(0);
    let rows = &rows[start..];
    let first = rows.first().ok_or("empty log")?;
    let frame0 = first["frame"].as_u64().unwrap_or(0);
    // The intro ends at a fixed frame of the match (about 850), wherever the log's first packet is.
    let intro = (FIRST_COUNTDOWN_TICKS as u64).saturating_sub(frame0) as u32;
    let mut flow = Flow::new(phase_of(first["phase"].as_str().unwrap_or(""))).with_countdown(intro);
    if five {
        flow = flow.with_clock(FIVE_MINUTES);
    }
    // The game's replays vary from 1076 to 1560 ticks for reasons the packets do not show: give
    // each goal the length its replay had, so the rest of the match can be compared.
    let mut replay_lengths: Vec<u32> = Vec::new();
    let mut replay_start: Option<u64> = None;
    for row in rows {
        let frame = row["frame"].as_u64().unwrap_or(0);
        match (row["phase"].as_str(), replay_start) {
            (Some("Replay"), None) => replay_start = Some(frame),
            (Some(name), Some(from)) if name != "Replay" => {
                replay_lengths.push((frame - from) as u32);
                replay_start = None;
            }
            _ => {}
        }
    }
    let mut replays_seen = 0usize;
    let (mut ball, mut velocity) = (Vec3::ZERO, Vec3::ZERO);
    let mut index = 0;
    let last_frame = rows.last().map_or(0, |r| r["frame"].as_u64().unwrap_or(0));
    let (mut compared, mut phase_misses, mut clock_misses) = (0u64, 0u64, 0u64);
    let mut max_clock = 0.0f32;
    let mut changes: Vec<(u64, String, Option<u64>)> = Vec::new();
    let mut predicted_changes: Vec<(u64, Phase)> = Vec::new();
    let mut previous = flow.state().phase;
    let mut recorded_previous = String::new();
    for frame in frame0..=last_frame {
        while rows
            .get(index)
            .is_some_and(|r| r["frame"].as_u64() < Some(frame))
        {
            index += 1;
        }
        let row = rows
            .get(index)
            .filter(|r| r["frame"].as_u64() == Some(frame));
        if let Some(row) = row {
            if let (Some(p), Some(v)) = (row["ball"].get("p"), row["ball"].get("v")) {
                ball = vec3(p);
                velocity = vec3(v);
            } else {
                ball = ball + velocity * (1.0 / 120.0);
            }
        } else {
            ball = ball + velocity * (1.0 / 120.0);
        }
        if frame > frame0 {
            if flow.after_step(ball, velocity) == Transition::ReplayStarted {
                if let Some(&ticks) = replay_lengths.get(replays_seen) {
                    flow = flow.with_replay(ticks);
                }
                replays_seen += 1;
            }
        }
        let state = flow.state();
        if state.phase != previous {
            predicted_changes.push((frame, state.phase));
            previous = state.phase;
        }
        let Some(row) = row else { continue };
        let name = row["phase"].as_str().unwrap_or("");
        if name != recorded_previous {
            if name == "GoalScored" && std::env::var("CLOCK_TRACE").is_ok() {
                println!(
                    "goal at {frame}: port clock {:?}, game {}",
                    state.seconds_remaining(),
                    row["remaining"]
                );
            }
            changes.push((frame, name.to_string(), None));
            recorded_previous = name.to_string();
        }
        compared += 1;
        if state.phase != phase_of(name) {
            if phase_misses < 3 {
                println!("phase: frame {frame}: port {:?}, game {name}", state.phase);
            }
            phase_misses += 1;
        }
        if let (Some(seconds), Some(recorded)) =
            (state.seconds_remaining(), row["remaining"].as_f64())
        {
            if name != "Ended" {
                let diff = (seconds - recorded as f32).abs();
                max_clock = max_clock.max(diff);
                if diff > CLOCK_TOLERANCE {
                    if clock_misses < 3 {
                        println!(
                            "clock: frame {frame} {name}: port {seconds:.3}, game {recorded:.3}"
                        );
                    }
                    clock_misses += 1;
                }
            }
        }
    }
    println!(
        "{compared} frames compared: phase differs on {phase_misses}, clock off by over {CLOCK_TOLERANCE} s on {clock_misses} (max {max_clock:.3} s)"
    );
    println!(
        "final score {:?}, overtime {}",
        flow.state().score,
        flow.state().overtime
    );
    println!("phase changes (game frame / port frame):");
    for (frame, name) in changes.iter().map(|(f, n, _)| (f, n)) {
        let target = phase_of(name);
        let port = predicted_changes
            .iter()
            .find(|(f, p)| *p == target && f.abs_diff(*frame) < 600)
            .map(|(f, _)| *f);
        println!(
            "  {name:<11} {frame:>6}  port {}",
            port.map_or("-".to_string(), |f| format!(
                "{f} ({:+})",
                f as i64 - *frame as i64
            ))
        );
    }
    Ok(())
}
