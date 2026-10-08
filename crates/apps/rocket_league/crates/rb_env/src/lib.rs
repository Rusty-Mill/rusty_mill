//! A stepping environment over `rb_physics_bullet` (ADR-0060): reset to a
//! frame, apply one controller input per car, step one tick, observe the
//! next frame. The observation is a `PhysicsFrame`, the type every other
//! part of the pipeline already speaks, so there is no second state type.
//!
//! The arena is built once in `Env::new` and cloned on each `reset`; its
//! collision meshes cost far more to build than to copy, and a sweep or a
//! policy resets thousands of times.

pub mod flow;

use flow::{Flow, MatchState, Phase, Transition};
use rb_domain::{ControllerInput, PhysicsFrame, Vec3};
use rb_physics_bullet::{CarBallTuning, PhysicsWorld, RigidBody};

/// Height of the ball's centre where it touches the floor (uu).
const BALL_FLOOR_CONTACT_HEIGHT: f32 = 93.2;

/// Seconds per tick: the game's 120 Hz physics rate.
pub const TICK_SECS: f32 = 1.0 / 120.0;

/// One standard-arena simulation, resettable and steppable.
pub struct Env {
    arena: PhysicsWorld,
    world: PhysicsWorld,
    car_ball: CarBallTuning,
    teams: Vec<u32>,
    dodge_forward_from_throttle: bool,
    boost_pads: bool,
    /// The match phase machine, when match flow is on.
    flow: Option<Flow>,
    /// The spawn slots (0 to 4) of the next kickoff, car by car, if given.
    kickoff_slots: Option<Vec<usize>>,
    /// Ticks of play in a match, if it has a clock.
    match_ticks: Option<i64>,
    /// Kickoffs started so far, which rotates the default slots.
    kickoffs: usize,
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
            boost_pads: true,
            flow: None,
            kickoff_slots: None,
            match_ticks: None,
            kickoffs: 0,
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

    /// Whether every later `reset` puts Soccar's 34 boost pads in the arena
    /// (`RB-PHYSICS-001-FR-149`, ADR-0072): on by default, unlike
    /// `PhysicsWorld::new`, because a policy plays on a field with pads.
    pub fn set_boost_pads(&mut self, on: bool) {
        self.boost_pads = on;
    }

    /// The boost pads of the current simulation and their cooldowns, if on.
    pub fn boost_pads(&self) -> Option<&rb_physics_bullet::BoostPads> {
        self.world.boost_pads()
    }

    /// Replaces the simulation with `start` (ball and every car), keeping
    /// its timestamp as the clock. Returns the observation, which is
    /// `start` as the port represents it.
    pub fn reset(&mut self, start: &PhysicsFrame) -> PhysicsFrame {
        self.world = PhysicsWorld::from_frame_in(&self.arena, start);
        self.world.car_ball = self.car_ball;
        self.world.set_boost_pads(self.boost_pads);
        self.world
            .set_dodge_forward_from_throttle(self.dodge_forward_from_throttle);
        for (index, team) in self.teams.iter().enumerate() {
            self.world.set_car_team(index, *team);
        }
        if self.flow.is_some() {
            self.flow = Some(self.new_flow(Phase::Active));
        }
        self.world.frame()
    }

    /// Turns match flow on or off for the current and later simulations (`RB-PHYSICS-001-FR-160`,
    /// ADR-0082). Off by default. On, a reset starts in `Phase::Active`; a ball past a goal line
    /// scores, the world carries on for three seconds, freezes for the nine of the replay,
    /// then `kickoff`s: cars at their spawn slots with inputs ignored for four seconds, the ball
    /// held on the centre spot until the first touch.
    pub fn enable_match_flow(&mut self, on: bool) {
        self.flow = on.then(|| self.new_flow(Phase::Active));
    }

    fn new_flow(&self, phase: Phase) -> Flow {
        let flow = Flow::new(phase);
        match self.match_ticks {
            Some(ticks) => flow.with_clock(ticks),
            None => flow,
        }
    }

    /// Gives matches a clock of `ticks` of play (`flow::FIVE_MINUTES` for the standard five
    /// minutes), counted down while the ball is in play, for every later `reset` or
    /// `enable_match_flow`; `None` (the default) is an unlimited match. At zero play goes on until
    /// the ball is low, then a lead ends the match and a tie starts overtime.
    pub fn set_match_length(&mut self, ticks: Option<i64>) {
        self.match_ticks = ticks;
    }

    /// Starts a match from its first kickoff: like `start_kickoff`, but with the intro that makes
    /// the first countdown 826 ticks.
    pub fn start_match(&mut self) -> PhysicsFrame {
        self.flow = Some(
            self.new_flow(Phase::Countdown)
                .with_countdown(flow::FIRST_COUNTDOWN_TICKS),
        );
        self.kickoffs = 0;
        self.place_kickoff()
    }

    /// The match phase, ticks in it and the score, if match flow is on.
    pub fn match_state(&self) -> Option<MatchState> {
        self.flow.map(|flow| flow.state())
    }

    /// Chooses the spawn slots (0 to 4, car by car) of the next kickoff; later ones rotate
    /// from the default again. The game picks at random.
    pub fn set_kickoff_slots(&mut self, slots: Option<Vec<usize>>) {
        self.kickoff_slots = slots;
    }

    /// Starts a kickoff now (match flow on): places the cars and the ball and begins the
    /// countdown. Returns the first observation.
    pub fn start_kickoff(&mut self) -> PhysicsFrame {
        self.flow = Some(match self.flow {
            Some(flow) => flow.restarted(Phase::Countdown),
            None => self.new_flow(Phase::Countdown),
        });
        self.place_kickoff()
    }

    fn place_kickoff(&mut self) -> PhysicsFrame {
        let slots = self
            .kickoff_slots
            .take()
            .unwrap_or_else(|| self.default_slots());
        self.kickoffs += 1;
        self.world.kickoff(&slots);
        self.world.frame()
    }

    /// Default slots: teammates take consecutive slots, starting one further along at each
    /// kickoff.
    fn default_slots(&self) -> Vec<usize> {
        let cars = self.world.frame().cars.len();
        let mut seen = [0usize; 2];
        (0..cars)
            .map(|index| {
                let team = usize::from(self.teams.get(index).copied().unwrap_or(0) != 0);
                let slot = (self.kickoffs + seen[team]) % 5;
                seen[team] += 1;
                slot
            })
            .collect()
    }

    /// Runs the phase machine after a tick and does what the new phase asks of the world.
    fn advance_flow(&mut self) {
        let Some(mut flow) = self.flow else {
            return;
        };
        let frame = self.world.frame();
        let transition = flow.after_step(frame.ball.position, frame.ball.velocity);
        self.flow = Some(flow);
        if matches!(
            transition,
            Transition::CountdownStarted | Transition::OvertimeStarted
        ) {
            self.start_kickoff();
            return;
        }
        if flow.state().phase == Phase::Countdown {
            self.world.hold_cars_on_their_spots();
        }
        let held = match flow.state().phase {
            // The falling ball is caught where it would meet the floor (its radius, 93.15).
            Phase::Countdown => frame.ball.position.z <= BALL_FLOOR_CONTACT_HEIGHT,
            Phase::Kickoff => true,
            _ => false,
        };
        if held {
            self.world.pin_ball_to_centre_spot();
        }
    }

    /// Chooses the spawn point (an index into `rb_physics_bullet::respawn::SPAWN_POINTS`)
    /// car `index` respawns at after its next demolition in the current simulation;
    /// `None` takes the default stand-in for the game's random pick.
    pub fn set_respawn_point(&mut self, index: usize, point: Option<usize>) {
        self.world.set_respawn_point(index, point);
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
        let phase = self.flow.map(|flow| flow.state().phase);
        if matches!(phase, Some(Phase::Replay | Phase::Ended)) {
            // The replay shows the goal again: the clock runs, the world does not.
            self.advance_flow();
            return self.world.frame();
        }
        let countdown = phase == Some(Phase::Countdown);
        for (index, input) in inputs.iter().enumerate() {
            let input = if countdown {
                ControllerInput::default()
            } else {
                *input
            };
            self.world.set_car_input(index, input);
        }
        self.world.step(TICK_SECS);
        self.advance_flow();
        self.world.frame()
    }
}

impl Default for Env {
    fn default() -> Env {
        Env::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
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

    /// A parked car on small pad 14 at (0, -1024) with 20 boost.
    fn on_pad() -> PhysicsFrame {
        let mut frame = start();
        frame.cars[0].position = Vec3::new(0.0, -1024.0, 17.0);
        frame.cars[0].velocity = Vec3::new(0.0, 0.0, 0.0);
        frame.cars[0].boost_amount = 20.0;
        frame
    }

    #[test]
    fn boost_pads_are_on_by_default_and_a_pickup_shows_in_the_observation() {
        let mut env = Env::new();
        env.reset(&on_pad());
        assert_eq!(env.boost_pads().map(|pads| pads.len()), Some(34));
        let seen = env.step(&[ControllerInput::default()]);
        assert_eq!(seen.cars[0].boost_amount, 32.0);
        assert_eq!(env.boost_pads().map(|pads| pads.is_active(14)), Some(false));
    }

    #[test]
    fn boost_pads_can_be_switched_off() {
        let mut env = Env::new();
        env.set_boost_pads(false);
        env.reset(&on_pad());
        assert!(env.boost_pads().is_none());
        let seen = env.step(&[ControllerInput::default()]);
        assert_eq!(seen.cars[0].boost_amount, 20.0);
    }

    #[test]
    fn a_reset_makes_every_pad_active_again() {
        let mut env = Env::new();
        env.reset(&on_pad());
        env.step(&[ControllerInput::default()]);
        env.reset(&on_pad());
        assert_eq!(env.boost_pads().map(|pads| pads.is_active(14)), Some(true));
        let seen = env.step(&[ControllerInput::default()]);
        assert_eq!(seen.cars[0].boost_amount, 32.0);
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

    /// A car at rest by the touchline and the ball launched into the orange goal.
    fn goal_shot() -> PhysicsFrame {
        let mut frame = start();
        frame.cars[0].position = Vec3::new(-3000.0, -3000.0, 17.0);
        frame.cars[0].velocity = Vec3::ZERO;
        frame.ball.position = Vec3::new(0.0, 4800.0, 400.0);
        frame.ball.velocity = Vec3::new(0.0, 2500.0, 0.0);
        frame
    }

    fn idle() -> [ControllerInput; 1] {
        [ControllerInput::default()]
    }

    #[test]
    fn match_flow_is_off_unless_asked_for() {
        let mut env = Env::new();
        env.reset(&goal_shot());
        for _ in 0..60 {
            env.step(&idle());
        }
        assert_eq!(env.match_state(), None);
    }

    /// The measured sequence (`log2.jsonl`): a goal, 3.0 s of play, a 9.0 s frozen replay, a
    /// 4.0 s countdown with the field reset, then the kickoff waits for a touch.
    #[test]
    fn a_goal_leads_to_a_replay_a_countdown_and_a_kickoff() {
        let mut env = Env::new();
        env.enable_match_flow(true);
        // The centre slot faces the ball squarely; the others pass it by.
        env.set_kickoff_slots(Some(vec![4]));
        env.reset(&goal_shot());
        assert_eq!(env.match_state().map(|m| m.phase), Some(Phase::Active));
        let mut ticks = 0;
        while env.match_state().map(|m| m.phase) == Some(Phase::Active) {
            env.step(&idle());
            ticks += 1;
            assert!(ticks < 100, "the shot never scored");
        }
        let state = env.match_state().unwrap();
        assert_eq!((state.phase, state.score), (Phase::GoalScored, [1, 0]));
        // About (5215 - 4800) / 20.8 uu per tick.
        assert!((18..=22).contains(&ticks), "scored after {ticks} ticks");

        for _ in 0..flow::GOAL_SCORED_TICKS {
            env.step(&idle());
        }
        assert_eq!(env.match_state().map(|m| m.phase), Some(Phase::Replay));
        // The replay freezes the world.
        let frozen = env.step(&idle());
        let later = env.step(&idle());
        assert_eq!(frozen, later);
        for _ in 2..flow::REPLAY_TICKS {
            env.step(&idle());
        }
        let first = env.match_state().unwrap();
        assert_eq!(first.phase, Phase::Countdown);

        // The field is reset: ball at the centre, the car at a blue spawn slot (slot 4), a third of a tank once it has dropped.
        let frame = env.step(&idle());
        assert_eq!((frame.ball.position.x, frame.ball.position.y), (0.0, 0.0));
        let spawn = rb_physics_bullet::respawn::SPAWN_POINTS[4];
        assert_eq!(
            (frame.cars[0].position.x, frame.cars[0].position.y),
            (spawn.x, spawn.y)
        );
        for _ in 0..flow::COUNTDOWN_TICKS - 1 {
            let throttled = env.step(&[throttle()]);
            // Inputs are ignored through the countdown.
            let v = throttled.cars[0].velocity;
            assert!(v.x.hypot(v.y) < 1.0, "{v:?}");
        }
        assert_eq!(env.match_state().map(|m| m.phase), Some(Phase::Kickoff));
        let held = env.step(&idle());
        assert_eq!(
            held.ball.position.z,
            rb_physics_bullet::respawn::KICKOFF_BALL_REST_HEIGHT
        );
        assert_eq!(env.match_state().map(|m| m.score), Some([1, 0]));

        // Driving at the ball starts play.
        let mut touched = false;
        for _ in 0..600 {
            env.step(&[throttle()]);
            if env.match_state().map(|m| m.phase) == Some(Phase::Active) {
                touched = true;
                break;
            }
        }
        assert!(touched, "a car driving at the ball never touched it");
    }

    /// The ball of a countdown falls from 100.49 as the game's does (`log2.jsonl`, countdown of
    /// the second goal, tick k after the first frame) and is then held at 92.75.
    #[test]
    fn the_countdown_ball_falls_as_recorded_and_is_held_at_the_rest_height() {
        let mut env = Env::new();
        env.enable_match_flow(true);
        env.reset(&start());
        let first = env.start_kickoff();
        assert_eq!(first.ball.position.z, 100.49);
        let recorded = [
            (0, 100.440),
            (1, 100.350),
            (2, 100.210),
            (3, 100.030),
            (4, 99.800),
            (5, 99.530),
            (6, 99.210),
            (8, 98.440),
            (10, 97.490),
            (15, 94.330),
        ];
        let frames: Vec<PhysicsFrame> = (0..40).map(|_| env.step(&idle())).collect();
        // The game's falling ball shows no drag; the port's 3 percent per second adds up to a tenth of
        // a unit by tick 15.
        for (k, z) in recorded {
            let port = frames[k].ball.position.z;
            assert!((port - z).abs() < 0.1, "tick {k}: port {port}, game {z}");
        }
        assert_eq!(
            frames[39].ball.position.z,
            rb_physics_bullet::respawn::KICKOFF_BALL_REST_HEIGHT
        );
        assert_eq!(frames[39].ball.velocity, Vec3::ZERO);
    }

    /// A match from its first kickoff has the 826-tick intro countdown and a clock that starts
    /// when the ball is first touched.
    #[test]
    fn a_match_starts_with_the_long_countdown_and_a_clock_that_waits_for_play() {
        let mut env = Env::new();
        env.enable_match_flow(true);
        env.set_match_length(Some(flow::FIVE_MINUTES));
        env.reset(&start());
        env.set_kickoff_slots(Some(vec![4]));
        env.start_match();
        for _ in 0..flow::FIRST_COUNTDOWN_TICKS - 1 {
            env.step(&idle());
        }
        assert_eq!(env.match_state().map(|m| m.phase), Some(Phase::Countdown));
        env.step(&idle());
        let state = env.match_state().unwrap();
        assert_eq!(state.phase, Phase::Kickoff);
        assert_eq!(state.seconds_remaining(), Some(300.0));
        // Play starts the clock.
        for _ in 0..600 {
            env.step(&[throttle()]);
            if env.match_state().map(|m| m.phase) == Some(Phase::Active) {
                break;
            }
        }
        env.step(&[throttle()]);
        assert!(env
            .match_state()
            .and_then(|m| m.seconds_remaining())
            .is_some_and(|s| s < 300.0));
    }
}
