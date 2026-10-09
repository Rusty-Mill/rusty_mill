//! A whole (short) match in the port with two scripted chasers: every phase must be reached in a
//! legal order, the clock and the score must behave, and the match must end.

use rb_domain::{BallState, CarState, ControllerInput, PhysicsFrame, Quat, Vec3};
use rb_env::{chaser::chase, flow::Phase, Env};

fn start() -> PhysicsFrame {
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
    PhysicsFrame {
        timestamp_secs: 0.0,
        ball: BallState {
            position: Vec3::new(0.0, 0.0, 93.0),
            rotation: Quat::IDENTITY,
            velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
        },
        cars,
    }
}

fn legal(from: Phase, to: Phase) -> bool {
    use Phase::*;
    matches!(
        (from, to),
        (Countdown, Kickoff)
            | (Kickoff, Active)
            | (Active, GoalScored)
            | (GoalScored, Replay)
            | (Replay, Countdown)
            | (Replay, Ended)
            | (Active, Ended)
            | (Active, Countdown) // the clock ran out level: overtime
    )
}

#[test]
#[allow(clippy::unwrap_used)]
fn a_thirty_second_match_plays_to_its_end_through_legal_phases() {
    let mut env = Env::new();
    env.set_teams(&[0, 1]);
    env.enable_match_flow(true);
    env.set_match_length(Some(30 * 120));
    env.reset(&start());
    let mut frame = env.start_match();
    let mut state = env.match_state().unwrap();
    assert_eq!(state.phase, Phase::Countdown);
    let mut kickoffs = 1u32;
    let mut ticks = 0u32;
    while state.phase != Phase::Ended {
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
        assert!(ticks < 200_000, "the match never ended");
        assert!(frame.ball.position.length().is_finite());
        let next = env.match_state().unwrap();
        if next.phase != state.phase {
            assert!(
                legal(state.phase, next.phase),
                "{:?} -> {:?}",
                state.phase,
                next.phase
            );
            if next.phase == Phase::Kickoff {
                kickoffs += 1;
            }
        }
        // Goals count up one at a time; the clock never runs backwards.
        assert!(next.score[0] + next.score[1] <= state.score[0] + state.score[1] + 1);
        assert!(next.score[0] >= state.score[0] && next.score[1] >= state.score[1]);
        assert!(next.clock_ticks <= state.clock_ticks);
        state = next;
    }
    // Every goal is followed by a kickoff, and a level finish adds one for overtime.
    let goals = state.score[0] + state.score[1];
    assert!(kickoffs > goals, "{kickoffs} kickoffs for {goals} goals");
    assert!(
        kickoffs <= 2 + goals,
        "{kickoffs} kickoffs for {goals} goals"
    );
}
