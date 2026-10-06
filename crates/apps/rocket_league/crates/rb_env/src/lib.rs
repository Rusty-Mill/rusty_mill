//! A stepping environment over `rb_physics_bullet` (ADR-0060): reset to a
//! frame, apply one controller input per car, step one tick, observe the
//! next frame. The observation is a `PhysicsFrame`, the type every other
//! part of the pipeline already speaks, so there is no second state type.
//!
//! The arena is built once in `Env::new` and cloned on each `reset`; its
//! collision meshes cost far more to build than to copy, and a sweep or a
//! policy resets thousands of times.

use rb_domain::{ControllerInput, PhysicsFrame, Vec3};
use rb_physics_bullet::{PhysicsWorld, RigidBody};

/// Seconds per tick: the game's 120 Hz physics rate.
pub const TICK_SECS: f32 = 1.0 / 120.0;

/// One standard-arena simulation, resettable and steppable.
pub struct Env {
    arena: PhysicsWorld,
    world: PhysicsWorld,
}

impl Env {
    /// Builds the arena once. The environment starts empty-handed: call
    /// `reset` before `step`.
    pub fn new() -> Env {
        let arena =
            PhysicsWorld::standard_arena(RigidBody::standard_ball(Vec3::new(0.0, 0.0, 93.0)));
        Env {
            world: arena.clone(),
            arena,
        }
    }

    /// Replaces the simulation with `start` (ball and every car), keeping
    /// its timestamp as the clock. Returns the observation, which is
    /// `start` as the port represents it.
    pub fn reset(&mut self, start: &PhysicsFrame) -> PhysicsFrame {
        self.world = PhysicsWorld::from_frame_in(&self.arena, start);
        self.world.frame()
    }

    /// Applies `inputs` (one per car, in the order `reset` was given; a car
    /// without one keeps its previous input), advances one tick and returns
    /// the observation. Each returned car carries the input that produced
    /// the frame.
    pub fn step(&mut self, inputs: &[ControllerInput]) -> PhysicsFrame {
        for (index, input) in inputs.iter().enumerate() {
            self.world.set_car_input(index, *input);
        }
        self.world.step(TICK_SECS);
        self.world.frame()
    }
}

impl Default for Env {
    fn default() -> Env {
        Env::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rb_domain::{BallState, CarState, Quat};

    fn start() -> PhysicsFrame {
        PhysicsFrame {
            timestamp_secs: 0.0,
            ball: BallState {
                position: Vec3::new(2000.0, 0.0, 93.0),
                rotation: Quat::IDENTITY,
                velocity: Vec3::new(0.0, 0.0, 0.0),
                angular_velocity: Vec3::new(0.0, 0.0, 0.0),
            },
            cars: vec![CarState {
                player_id: 0,
                position: Vec3::new(0.0, 0.0, 17.0),
                rotation: Quat::IDENTITY,
                velocity: Vec3::new(500.0, 0.0, 0.0),
                angular_velocity: Vec3::new(0.0, 0.0, 0.0),
                boost_amount: 100.0,
                input: None,
            }],
        }
    }

    fn throttle() -> ControllerInput {
        ControllerInput {
            throttle: 1.0,
            boost: true,
            ..ControllerInput::default()
        }
    }

    fn run(env: &mut Env, ticks: usize) -> Vec<PhysicsFrame> {
        env.reset(&start());
        (0..ticks).map(|_| env.step(&[throttle()])).collect()
    }

    #[test]
    fn step_matches_a_world_stepped_directly() {
        let mut world = PhysicsWorld::from_frame(&start());
        let mut env = Env::new();
        env.reset(&start());
        for _ in 0..60 {
            world.set_car_input(0, throttle());
            world.step(TICK_SECS);
            assert_eq!(env.step(&[throttle()]), world.frame());
        }
    }

    #[test]
    fn reset_makes_a_run_repeatable_on_a_reused_env() {
        let mut env = Env::new();
        let first = run(&mut env, 90);
        let second = run(&mut env, 90);
        assert_eq!(first, second);
        assert_ne!(first[0], first[89], "the car actually moved");
    }

    #[test]
    fn a_fresh_env_and_a_used_one_agree() {
        let mut used = Env::new();
        run(&mut used, 30);
        assert_eq!(run(&mut used, 30), run(&mut Env::new(), 30));
    }

    #[test]
    fn reset_observes_the_start_and_keeps_its_clock() {
        let mut frame = start();
        frame.timestamp_secs = 12.5;
        let seen = Env::new().reset(&frame);
        assert_eq!(seen.timestamp_secs, 12.5);
        assert_eq!(seen.cars.len(), 1);
        assert_eq!(seen.ball.position, frame.ball.position);
    }

    #[test]
    fn step_with_fewer_inputs_than_cars_keeps_the_rest() {
        let mut frame = start();
        let mut second = frame.cars[0];
        second.player_id = 1;
        second.position = Vec3::new(0.0, 800.0, 17.0);
        frame.cars.push(second);
        let mut env = Env::new();
        env.reset(&frame);
        let seen = env.step(&[throttle()]);
        assert_eq!(seen.cars.len(), 2);
        assert_eq!(seen.cars[0].input, Some(throttle()));
        assert_eq!(seen.cars[1].input, Some(ControllerInput::default()));
    }
}
