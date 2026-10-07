//! A stepping environment over `rb_physics_bullet` (ADR-0060): reset to a
//! frame, apply one controller input per car, step one tick, observe the
//! next frame. The observation is a `PhysicsFrame`, the type every other
//! part of the pipeline already speaks, so there is no second state type.
//!
//! The arena is built once in `Env::new` and cloned on each `reset`; its
//! collision meshes cost far more to build than to copy, and a sweep or a
//! policy resets thousands of times.

use rb_domain::{ControllerInput, PhysicsFrame, Vec3};
use rb_physics_bullet::{CarBallTuning, PhysicsWorld, RigidBody};

/// Seconds per tick: the game's 120 Hz physics rate.
pub const TICK_SECS: f32 = 1.0 / 120.0;

/// One standard-arena simulation, resettable and steppable.
pub struct Env {
    arena: PhysicsWorld,
    world: PhysicsWorld,
    car_ball: CarBallTuning,
    teams: Vec<u32>,
    dodge_forward_from_throttle: bool,
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
            car_ball: CarBallTuning::default(),
            teams: Vec::new(),
            dodge_forward_from_throttle: false,
        }
    }

    /// Sets the car-ball hit numbers every later `reset` starts with
    /// (RocketSim's by default); the current simulation keeps its own.
    pub fn set_car_ball(&mut self, tuning: CarBallTuning) {
        self.car_ball = tuning;
    }

    /// Sets every car's team (in car order) for every later `reset`; cars
    /// on different teams can demolish each other. Teams default to 0.
    pub fn set_teams(&mut self, teams: &[u32]) {
        self.teams = teams.to_vec();
    }

    /// Whether a dodge with the pitch stick centred goes forward by the
    /// throttle, for every later `reset` (`RB-PHYSICS-001-FR-147`): off by
    /// default, as RocketSim; on for the owner's keyboard captures.
    pub fn set_dodge_forward_from_throttle(&mut self, on: bool) {
        self.dodge_forward_from_throttle = on;
    }

    /// Replaces the simulation with `start` (ball and every car), keeping
    /// its timestamp as the clock. Returns the observation, which is
    /// `start` as the port represents it.
    pub fn reset(&mut self, start: &PhysicsFrame) -> PhysicsFrame {
        self.world = PhysicsWorld::from_frame_in(&self.arena, start);
        self.world.car_ball = self.car_ball;
        self.world
            .set_dodge_forward_from_throttle(self.dodge_forward_from_throttle);
        for (index, team) in self.teams.iter().enumerate() {
            self.world.set_car_team(index, *team);
        }
        self.world.frame()
    }

    /// Snaps the ball and every car to `frame`'s state (position, rotation,
    /// velocity, spin, boost) while the simulation's own memory carries on:
    /// solver warm-starts, suspension and ground contact, jump and flip
    /// timers. This is what keeps a prediction made along a recording honest;
    /// `reset` forgets all of that (`PhysicsWorld::snap_to_frame`).
    pub fn snap(&mut self, frame: &PhysicsFrame) {
        self.world.snap_to_frame(frame);
    }

    /// What `step` would produce after each of `inputs` in turn (one slice
    /// per tick, one input per car), without changing this environment: the
    /// rollout runs on a copy. Returns the last observation (the current
    /// state, if `inputs` is empty).
    pub fn peek(&self, inputs: &[&[ControllerInput]]) -> PhysicsFrame {
        let mut ahead = self.world.clone();
        for tick in inputs {
            for (index, input) in tick.iter().enumerate() {
                ahead.set_car_input(index, *input);
            }
            ahead.step(TICK_SECS);
        }
        ahead.frame()
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
    fn car_ball_tuning_survives_a_reset_and_changes_the_run() {
        let mut hit = start();
        hit.ball.position = Vec3::new(160.0, 0.0, 93.0);
        hit.cars[0].velocity = Vec3::new(1500.0, 0.0, 0.0);
        let run = |env: &mut Env| {
            env.reset(&hit);
            (0..30).map(|_| env.step(&[throttle()])).last()
        };
        let mut env = Env::new();
        let normal = run(&mut env);
        env.set_car_ball(CarBallTuning {
            hit_scale: 0.0,
            ..CarBallTuning::default()
        });
        let weak = run(&mut env);
        assert_ne!(normal, weak, "the tuning reached the world after reset");
        env.set_car_ball(CarBallTuning::default());
        assert_eq!(run(&mut env), normal, "and the default restores the run");
    }

    #[test]
    fn peek_matches_stepping_and_leaves_the_env_alone() {
        let mut env = Env::new();
        env.reset(&start());
        let before = env.peek(&[]);
        let tick = [throttle()];
        let seen = env.peek(&[&tick, &tick, &tick]);
        assert_eq!(env.peek(&[]), before, "peek changed nothing");
        let stepped: Vec<_> = (0..3).map(|_| env.step(&tick)).collect();
        assert_eq!(seen, stepped[2]);
    }

    #[test]
    fn snap_moves_the_bodies_but_keeps_the_boost_clock_running() {
        let mut env = Env::new();
        env.reset(&start());
        for _ in 0..10 {
            env.step(&[throttle()]);
        }
        let mut target = start();
        target.cars[0].position = Vec3::new(500.0, 100.0, 17.0);
        target.cars[0].boost_amount = 40.0;
        env.snap(&target);
        let seen = env.peek(&[]);
        assert_eq!(seen.cars[0].position, Vec3::new(500.0, 100.0, 17.0));
        assert_eq!(seen.cars[0].boost_amount, 40.0);
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
