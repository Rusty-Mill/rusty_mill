//! Plays a whole Soccar match in the port: two scripted chasers (one per team) with match flow
//! and a five-minute clock, from the intro countdown to `Phase::Ended`, and prints the timeline.
//!
//! `cargo run --release -p rb_env --example play_match [seconds_of_play]` (default 300).
//!
//! The chasers drive at the ball from behind it toward the opposing goal, boosting when pointed
//! at it. They are not good players; the point is that every phase of a match gets exercised
//! with real physics: kickoffs, goals, replays, the clock running out, overtime.

use rb_domain::{BallState, CarState, ControllerInput, PhysicsFrame, Quat, Vec3};
use rb_env::{
    chaser::chase,
    flow::{MatchState, Phase},
    Env,
};

fn describe(state: &MatchState) -> String {
    format!(
        "{:?} score {}-{} clock {:.1}s{}",
        state.phase,
        state.score[0],
        state.score[1],
        state.seconds_remaining().unwrap_or(0.0),
        if state.overtime { " overtime" } else { "" }
    )
}

fn main() {
    let seconds: i64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(300);
    let mut env = Env::new();
    env.set_teams(&[0, 1]);
    env.enable_match_flow(true);
    env.set_match_length(Some(seconds * 120));
    let cars = (0..2)
        .map(|index| CarState {
            player_id: index,
            position: Vec3::new(0.0, if index == 0 { -4000.0 } else { 4000.0 }, 17.0),
            rotation: Quat::IDENTITY,
            velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            boost_amount: 33.3,
            input: None,
        })
        .collect();
    let start = PhysicsFrame {
        timestamp_secs: 0.0,
        ball: BallState {
            position: Vec3::new(0.0, 0.0, 93.0),
            rotation: Quat::IDENTITY,
            velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
        },
        cars,
    };
    env.reset(&start);
    let mut frame = env.start_match();
    let mut last = env.match_state().map(|m| m.phase);
    let started = std::time::Instant::now();
    let mut ticks = 0u64;
    println!(
        "tick 0: {}",
        env.match_state().map_or(String::new(), |m| describe(&m))
    );
    while env.match_state().map(|m| m.phase) != Some(Phase::Ended) {
        // A demolished car is missing from the frame until it respawns; it gets no input.
        let inputs: Vec<ControllerInput> = (0..2)
            .map(|team| {
                frame
                    .cars
                    .iter()
                    .find(|car| car.player_id == team as u32)
                    .map_or_else(ControllerInput::default, |car| {
                        chase(car, &frame.ball, team)
                    })
            })
            .collect();
        frame = env.step(&inputs);
        ticks += 1;
        let Some(state) = env.match_state() else {
            break;
        };
        if Some(state.phase) != last {
            println!("tick {ticks}: {}", describe(&state));
            last = Some(state.phase);
        }
        assert!(
            frame.ball.position.length().is_finite(),
            "the ball went non-finite"
        );
        assert!(ticks < 400_000, "the match never ended");
    }
    println!(
        "{ticks} ticks ({:.0} s of match) in {:.1} s of computing",
        ticks as f32 / 120.0,
        started.elapsed().as_secs_f32()
    );
}
