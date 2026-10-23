//! Determinism: two `Env`s given the same start and the same inputs produce the same frames, bit
//! for bit, every tick. Rollback and the loopback netcode experiment both rest on this: a client
//! that re-simulates must land exactly where the server did.
#![allow(clippy::unwrap_used, clippy::panic)]

use rb_domain::{BallState, CarState, ControllerInput, PhysicsFrame, Quat, Vec3};
use rb_env::{chaser::chase, Env};

/// Two cars a side at kickoff-like spots, the ball dropping onto the centre spot.
fn start() -> PhysicsFrame {
    let spots = [
        (-2048.0, -2560.0),
        (256.0, -3840.0),
        (2048.0, 2560.0),
        (-256.0, 3840.0),
    ];
    let cars = spots
        .iter()
        .enumerate()
        .map(|(index, &(x, y))| CarState {
            player_id: index as u32,
            position: Vec3::new(x, y, 17.0),
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
            position: Vec3::new(0.0, 0.0, 120.0),
            rotation: Quat::IDENTITY,
            velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
        },
        cars,
    }
}

const TEAMS: [u32; 4] = [0, 0, 1, 1];

/// A fixed input for car `car` at `tick` that exercises driving, steering, boost, handbrake,
/// jumps, dodges and air control: a pure function of its arguments, so both runs get it exactly.
fn scripted(car: usize, tick: u32) -> ControllerInput {
    let phase = tick + 37 * car as u32;
    let wave =
        |period: u32| (((phase % period) as f32 / period as f32) * 2.0 - 1.0).abs() * 2.0 - 1.0;
    let in_window = |period: u32, from: u32, to: u32| (from..to).contains(&(phase % period));
    ControllerInput {
        throttle: if in_window(400, 300, 340) { -1.0 } else { 1.0 },
        steer: wave(240),
        pitch: Some(wave(90)),
        yaw: Some(wave(130)),
        roll: Some(if in_window(300, 0, 40) { 1.0 } else { 0.0 }),
        // A jump, then a second press in the air (a dodge) a few ticks later.
        jump: in_window(180, 0, 8) || in_window(180, 20, 24),
        boost: in_window(150, 40, 110),
        handbrake: in_window(200, 150, 170),
    }
}

/// Every number in `frame`, as raw bits: `==` on floats would call 0.0 and -0.0 the same and
/// never match a NaN, and the claim is bit-identical frames.
fn bits(frame: &PhysicsFrame) -> Vec<u32> {
    fn vec3(out: &mut Vec<u32>, v: Vec3) {
        out.extend([v.x, v.y, v.z].map(f32::to_bits));
    }
    fn quat(out: &mut Vec<u32>, q: Quat) {
        out.extend([q.x, q.y, q.z, q.w].map(f32::to_bits));
    }
    fn axis(out: &mut Vec<u32>, a: Option<f32>) {
        out.extend(a.map_or([0, 0], |a| [1, a.to_bits()]));
    }
    let mut out = vec![frame.timestamp_secs.to_bits()];
    let ball = &frame.ball;
    vec3(&mut out, ball.position);
    quat(&mut out, ball.rotation);
    vec3(&mut out, ball.velocity);
    vec3(&mut out, ball.angular_velocity);
    for car in &frame.cars {
        out.push(car.player_id);
        vec3(&mut out, car.position);
        quat(&mut out, car.rotation);
        vec3(&mut out, car.velocity);
        vec3(&mut out, car.angular_velocity);
        out.push(car.boost_amount.to_bits());
        match car.input {
            None => out.push(0),
            Some(i) => {
                out.extend([1, i.throttle.to_bits(), i.steer.to_bits()]);
                axis(&mut out, i.pitch);
                axis(&mut out, i.yaw);
                axis(&mut out, i.roll);
                out.extend([i.jump, i.boost, i.handbrake].map(u32::from));
            }
        }
    }
    out
}

fn assert_same(tick: u32, a: &PhysicsFrame, b: &PhysicsFrame) {
    assert!(
        bits(a) == bits(b),
        "first divergence at tick {tick}:\n{a:?}\n{b:?}"
    );
}

fn env() -> Env {
    let mut env = Env::new();
    env.set_teams(&TEAMS);
    env
}

/// Plays the scripted tape for `ticks` on `env` from `start`, returning every frame.
fn tape(env: &mut Env, start: &PhysicsFrame, ticks: u32) -> Vec<PhysicsFrame> {
    let mut frames = vec![env.reset(start)];
    for tick in 0..ticks {
        let inputs: Vec<_> = (0..start.cars.len())
            .map(|car| scripted(car, tick))
            .collect();
        frames.push(env.step(&inputs));
    }
    frames
}

/// 20 s of play.
const TICKS: u32 = 2400;

#[test]
fn two_envs_fed_the_same_tape_give_bit_identical_frames() {
    let a = tape(&mut env(), &start(), TICKS);
    let b = tape(&mut env(), &start(), TICKS);
    for (tick, (a, b)) in a.iter().zip(&b).enumerate() {
        assert_same(tick as u32, a, b);
    }
    // The tape is not a car sitting still: the run went somewhere worth comparing.
    let last = a.last().unwrap();
    assert!(last.cars.iter().all(|c| c.position.length() > 0.0));
    assert!(last.ball.velocity.length() > 0.0 || last.ball.position != start().ball.position);
}

#[test]
fn an_env_that_already_ran_something_else_matches_a_fresh_one_after_reset() {
    let mut used = env();
    let mut elsewhere = start();
    for car in &mut elsewhere.cars {
        car.position.x += 500.0;
    }
    tape(&mut used, &elsewhere, 600);
    let fresh = tape(&mut env(), &start(), TICKS);
    let reused = tape(&mut used, &start(), TICKS);
    for (tick, (a, b)) in fresh.iter().zip(&reused).enumerate() {
        assert_same(tick as u32, a, b);
    }
}

#[test]
fn two_envs_playing_a_chaser_match_with_match_flow_stay_identical() {
    let match_env = || {
        let mut env = env();
        env.enable_match_flow(true);
        env.set_match_length(Some(60 * 120));
        env.reset(&start());
        env
    };
    let (mut a, mut b) = (match_env(), match_env());
    let (mut frame_a, frame_b) = (a.start_match(), b.start_match());
    assert_same(0, &frame_a, &frame_b);
    // A minute of clock plus kickoffs and goal replays, or the match's end.
    for tick in 1..=12_000 {
        let inputs: Vec<_> = frame_a
            .cars
            .iter()
            .map(|car| chase(car, &frame_a.ball, TEAMS[car.player_id as usize] as usize))
            .collect();
        frame_a = a.step(&inputs);
        let frame_b = b.step(&inputs);
        assert_same(tick, &frame_a, &frame_b);
        assert_eq!(
            a.match_state(),
            b.match_state(),
            "match state at tick {tick}"
        );
        if a.match_state().unwrap().phase == rb_env::flow::Phase::Ended {
            break;
        }
    }
}
