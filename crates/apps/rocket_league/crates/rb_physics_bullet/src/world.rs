//! The simulation loop, porting the shape of
//! `btDiscreteDynamicsWorld::stepSimulation` (predict → collide → solve →
//! integrate) at fixed timestep — no substepping/interpolation yet, since
//! nothing in this scope needs it (no CCD-worthy speeds).

use crate::body::{RigidBody, StaticPlane};
use crate::collision;
use crate::{drive, integrate, solver};
use rb_domain::{BallState, CarState, ControllerInput, PhysicsFrame, Vec3};

/// The whole simulated scene: one ball-like sphere, zero or more car-like
/// boxes, and one ground plane. Every body collides with the ground; every
/// car also collides with the ball and with every other car
/// (`collision::contacts_between`, dispatching to `sphere_vs_box` or
/// `box_vs_box`) — a real N-body scene, not just the one-ball-one-car case
/// `RB-PHYSICS-001-FR-004`/`FR-006` originally scoped. Each car also has a
/// current `ControllerInput` (`car_inputs`, set via `set_car_input`,
/// `ControllerInput::default()` — neutral — until set), a boost resource
/// (`car_boost`, set via `set_car_boost`, starting full), a remembered base
/// friction (`car_base_friction`, snapshotted from the car's own
/// `RigidBody.friction` when added) that `drive::apply_driven_forces` uses
/// to restore grip after a handbrake-induced reduction, and a remembered
/// jump-held state (`car_jump_held`, starting `false`) that
/// `drive::apply_driven_forces` uses to fire jump only on a fresh press —
/// all driving the car via `drive::apply_driven_forces`.
pub struct PhysicsWorld {
    pub ball: RigidBody,
    pub cars: Vec<RigidBody>,
    car_inputs: Vec<ControllerInput>,
    car_boost: Vec<f32>,
    car_base_friction: Vec<f32>,
    car_jump_held: Vec<bool>,
    pub ground: StaticPlane,
    pub gravity: Vec3,
    elapsed_secs: f32,
}

impl PhysicsWorld {
    /// `gravity` defaults to -650 Unreal units/s^2 on Z, a commonly-cited
    /// community-measured approximation of Rocket League's ball gravity —
    /// not a value this project has independently confirmed, and not
    /// Earth gravity (the two diverge enough to matter for a divergence
    /// metric scored against real matches). Treat this default as a
    /// placeholder to calibrate, not settled fact: `RB-VERIFY-001`/`002`
    /// data should be used to fit the real constant once available (see
    /// `RB-PHYSICS-001` open questions). Overridable via the `gravity`
    /// field in the meantime.
    pub fn new(ball: RigidBody, ground: StaticPlane) -> PhysicsWorld {
        PhysicsWorld {
            ball,
            cars: Vec::new(),
            car_inputs: Vec::new(),
            car_boost: Vec::new(),
            car_base_friction: Vec::new(),
            car_jump_held: Vec::new(),
            ground,
            gravity: Vec3::new(0.0, 0.0, -650.0),
            elapsed_secs: 0.0,
        }
    }

    /// Adds one car-shaped body to the scene, with a neutral
    /// (`ControllerInput::default()`) input and a full boost tank
    /// (`drive::MAX_BOOST`) — set a real input afterward with
    /// `set_car_input` if the car should actually drive. `car`'s current
    /// `friction` is snapshotted as its base friction, so handbrake input
    /// (which temporarily lowers `RigidBody.friction`) has a value to
    /// restore to once released; its jump-held state starts `false`, so an
    /// already-`jump: true` initial input still counts as a fresh press.
    /// Callable more than once —
    /// `PhysicsWorld::new(ball, ground).with_car(a).with_car(b)` builds a
    /// two-car scene — since a car's `player_id` in `frame()` is just its
    /// index in `cars`, added cars are always appended, never inserted.
    pub fn with_car(mut self, car: RigidBody) -> PhysicsWorld {
        self.car_base_friction.push(car.friction);
        self.cars.push(car);
        self.car_inputs.push(ControllerInput::default());
        self.car_boost.push(drive::MAX_BOOST);
        self.car_jump_held.push(false);
        self
    }

    /// Sets car `index`'s current controller input, which persists across
    /// steps until changed again (matching how a real controller's state
    /// holds between frames). Panics if `index` is out of bounds — an
    /// invalid index is a programming error, not a recoverable runtime
    /// condition (see the crate's "trust internal callers" convention).
    pub fn set_car_input(&mut self, index: usize, input: ControllerInput) {
        self.car_inputs[index] = input;
    }

    /// Sets car `index`'s current boost amount, clamped to
    /// `[0, drive::MAX_BOOST]`. Panics if `index` is out of bounds (see
    /// `set_car_input`).
    pub fn set_car_boost(&mut self, index: usize, amount: f32) {
        self.car_boost[index] = amount.clamp(0.0, drive::MAX_BOOST);
    }

    /// Applies forces and integrates velocities for one body — the first
    /// phase of `btDiscreteDynamicsWorld::stepSimulation`
    /// (`predictUnconstrainedMotion`, run for every body before any
    /// collision detection happens).
    fn apply_forces_and_integrate_velocities(body: &mut RigidBody, gravity: Vec3, dt: f32) {
        body.clear_forces();
        integrate::apply_gravity(body, gravity);
        integrate::apply_damping(body, dt);
        integrate::integrate_velocities(body, dt);
    }

    /// Like `apply_forces_and_integrate_velocities`, but for a car: also
    /// applies `drive::apply_driven_forces` (throttle/steer/handbrake/jump
    /// gated on `on_ground`, computed from the car's position at the start
    /// of this step, before anything moves; boost not gated on it, but
    /// draining `boost_amount`; handbrake temporarily lowering
    /// `car.friction` below `base_friction`; jump firing an instantaneous
    /// upward velocity change on a fresh press, tracked via `jump_held`)
    /// alongside gravity, so `input`'s forces/impulses (and friction
    /// adjustment) are part of the same velocity-prediction phase.
    #[allow(clippy::too_many_arguments)]
    fn drive_and_integrate_velocities(
        car: &mut RigidBody,
        input: &ControllerInput,
        on_ground: bool,
        boost_amount: &mut f32,
        jump_held: &mut bool,
        base_friction: f32,
        gravity: Vec3,
        dt: f32,
    ) {
        car.clear_forces();
        integrate::apply_gravity(car, gravity);
        drive::apply_driven_forces(
            car,
            input,
            on_ground,
            boost_amount,
            jump_held,
            base_friction,
            dt,
        );
        integrate::apply_damping(car, dt);
        integrate::integrate_velocities(car, dt);
    }

    /// Detects and resolves `body`'s ground contact (a manifold of 1 to 4
    /// points depending on shape/orientation — see
    /// `collision::contacts_vs_plane`), if any.
    fn resolve_ground_contact(body: &mut RigidBody, ground: &StaticPlane, dt: f32) {
        let contacts = collision::contacts_vs_plane(body, ground);
        if !contacts.is_empty() {
            solver::resolve_contacts(body, ground, &contacts, dt);
        }
    }

    /// Integrates `body`'s transform from its (already-resolved) velocity,
    /// then refreshes its world-space inertia tensor for the new
    /// orientation — the last phase of `stepSimulation`
    /// (`integrateTransforms`), run once every contact this step has been
    /// resolved.
    fn integrate_transform_and_refresh_inertia(body: &mut RigidBody, dt: f32) {
        let (position, orientation) = integrate::integrate_transform(
            body.position,
            body.orientation,
            body.linear_velocity,
            body.angular_velocity,
            dt,
        );
        body.position = position;
        body.orientation = orientation;
        body.update_inertia_tensor();
    }

    /// Detects and resolves the contact manifold, if any, between two
    /// dynamic bodies already known not to alias — the shared step for
    /// ball-vs-car and car-vs-car resolution.
    fn resolve_dynamic_contact(a: &mut RigidBody, b: &mut RigidBody, dt: f32) {
        let contacts = collision::contacts_between(a, b);
        if !contacts.is_empty() {
            solver::resolve_contacts_between(a, b, &contacts, dt);
        }
    }

    /// Advances the whole scene by `dt` seconds, matching
    /// `btDiscreteDynamicsWorld::stepSimulation`'s staged pipeline: predict
    /// every body's unconstrained velocity (for cars, including
    /// `drive::apply_driven_forces` from that car's current input), then
    /// detect and resolve every contact (ground contacts for every body,
    /// then every ball-vs-car pair, then every car-vs-car pair), then
    /// integrate every body's transform — never resolving one body's
    /// transform before another body's contacts have had a chance to
    /// affect it.
    ///
    /// Car-vs-car and ball-vs-car pairs are resolved one pair at a time
    /// (each running its own full `SOLVER_ITERATIONS` pass), not as one
    /// combined multi-body solve across every simultaneous contact —
    /// simpler than Bullet's actual interleaved-across-islands solver, and
    /// a real approximation once 3+ bodies are mutually touching in the
    /// same step (e.g. a car pinned between the ball and another car),
    /// tracked as open follow-up in `RB-PHYSICS-001`, not hidden.
    pub fn step(&mut self, dt: f32) {
        // Ground contact for driving purposes is checked up front, against
        // each car's position at the start of this step (before gravity or
        // driven forces move anything) — `resolve_ground_contact` below
        // re-derives the same contacts for the actual solve; the small
        // duplicated `contacts_vs_plane` call is simpler than threading
        // the manifold through, and cheap (a handful of corner checks).
        let car_on_ground: Vec<bool> = self
            .cars
            .iter()
            .map(|car| !collision::contacts_vs_plane(car, &self.ground).is_empty())
            .collect();

        Self::apply_forces_and_integrate_velocities(&mut self.ball, self.gravity, dt);
        for (((((car, input), on_ground), boost), base_friction), jump_held) in self
            .cars
            .iter_mut()
            .zip(self.car_inputs.iter())
            .zip(car_on_ground.iter())
            .zip(self.car_boost.iter_mut())
            .zip(self.car_base_friction.iter())
            .zip(self.car_jump_held.iter_mut())
        {
            Self::drive_and_integrate_velocities(
                car,
                input,
                *on_ground,
                boost,
                jump_held,
                *base_friction,
                self.gravity,
                dt,
            );
        }

        Self::resolve_ground_contact(&mut self.ball, &self.ground, dt);
        for car in &mut self.cars {
            Self::resolve_ground_contact(car, &self.ground, dt);
        }

        for car in &mut self.cars {
            Self::resolve_dynamic_contact(&mut self.ball, car, dt);
        }

        for i in 0..self.cars.len() {
            for j in (i + 1)..self.cars.len() {
                let (left, right) = self.cars.split_at_mut(j);
                Self::resolve_dynamic_contact(&mut left[i], &mut right[0], dt);
            }
        }

        Self::integrate_transform_and_refresh_inertia(&mut self.ball, dt);
        for car in &mut self.cars {
            Self::integrate_transform_and_refresh_inertia(car, dt);
        }

        self.elapsed_secs += dt;
    }

    /// The scene's current state as a `PhysicsFrame`, for consumption by
    /// `RB-VERIFY-003`'s divergence scorer. One `CarState` per car in
    /// `self.cars`, `player_id` set to each car's index, `input` set to
    /// that car's current `ControllerInput` (the one actually driving it
    /// — not "recovered" the way `rb_replay_ingest`/`rb_capture_ingest`
    /// use the field, but the same data), `boost_amount` its current fuel.
    pub fn frame(&self) -> PhysicsFrame {
        let cars = self
            .cars
            .iter()
            .zip(self.car_inputs.iter())
            .zip(self.car_boost.iter())
            .enumerate()
            .map(|(i, ((car, input), boost))| CarState {
                player_id: i as u32,
                position: car.position,
                rotation: car.orientation,
                velocity: car.linear_velocity,
                angular_velocity: car.angular_velocity,
                boost_amount: *boost,
                input: Some(*input),
            })
            .collect();
        PhysicsFrame {
            timestamp_secs: self.elapsed_secs,
            ball: BallState {
                position: self.ball.position,
                rotation: self.ball.orientation,
                velocity: self.ball.linear_velocity,
                angular_velocity: self.ball.angular_velocity,
            },
            cars,
        }
    }
}

/// Runs `PhysicsWorld` for `duration_secs` at fixed `dt`, recording one
/// `PhysicsFrame` per step. This is `RB-PHYSICS-001-FR-001`'s "produce a
/// `Vec<PhysicsFrame>` the divergence scorer can consume" — the candidate
/// trajectory `rb_verify_cli` compares against recorded ground truth.
///
/// Doesn't yet consume a recorded input sequence (no throttle/steer/boost
/// coupling exists — a car body here is a free rigid box, not a driven
/// vehicle) — it simulates the scene in isolation from its initial state.
/// Once `RB-VERIFY-002` capture data exists, this signature grows an
/// `inputs` parameter rather than staying input-free.
pub fn simulate(mut world: PhysicsWorld, duration_secs: f32, dt: f32) -> Vec<PhysicsFrame> {
    let steps = (duration_secs / dt).round() as u32;
    let mut frames = Vec::with_capacity(steps as usize + 1);
    frames.push(world.frame());
    for _ in 0..steps {
        world.step(dt);
        frames.push(world.frame());
    }
    frames
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn flat_ground() -> StaticPlane {
        StaticPlane::new(Vec3::new(0.0, 0.0, 1.0), 0.0)
    }

    #[test]
    fn ball_in_free_fall_matches_kinematics_before_impact() {
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(0.0, 0.0, 1000.0));
        let mut world = PhysicsWorld::new(ball, flat_ground());
        let dt: f32 = 1.0 / 240.0; // fine timestep to keep semi-implicit Euler error small
        let t: f32 = 0.2;
        let steps = (t / dt).round() as u32;
        for _ in 0..steps {
            world.step(dt);
        }
        // Semi-implicit Euler's known one-step lag vs. exact kinematics:
        // expected velocity is exact, position is off by ~0.5*g*dt*t.
        let expected_vz = world.gravity.z * t;
        assert!(
            (world.ball.linear_velocity.z - expected_vz).abs() < 1.0,
            "expected vz ~= {expected_vz}, got {}",
            world.ball.linear_velocity.z
        );
        assert!(world.ball.position.z < 1000.0, "ball should have fallen");
    }

    #[test]
    fn resting_ball_stays_at_rest() {
        let mut ball = RigidBody::sphere(1.0, 1.0, Vec3::new(0.0, 0.0, 1.0));
        // Inelastic on purpose, on *both* surfaces (combined restitution is
        // an average of the two — see solver.rs): this port has no
        // warm-starting or sleeping, so a *bouncy* resting contact
        // legitimately never settles under a naive per-frame sequential
        // impulse solve — each frame's gravity-induced velocity is a fresh
        // "impact" that restitution bounces back up, forever. That's a
        // real, known limitation (tracked in RB-PHYSICS-001), not covered
        // by this test; this test checks the inelastic case actually
        // settles, which it should regardless.
        ball.restitution = 0.0;
        let ground = StaticPlane {
            restitution: 0.0,
            ..flat_ground()
        };
        let mut world = PhysicsWorld::new(ball, ground);
        let dt = 1.0 / 60.0;
        for _ in 0..120 {
            world.step(dt);
        }
        assert!(
            (world.ball.position.z - 1.0).abs() < 0.05,
            "z drifted to {}",
            world.ball.position.z
        );
        assert!(world.ball.linear_velocity.length() < 1.0);
    }

    #[test]
    fn dropped_ball_eventually_settles_on_the_ground() {
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(0.0, 0.0, 50.0));
        let mut world = PhysicsWorld::new(ball, flat_ground());
        world.ball.restitution = 0.3;
        let dt = 1.0 / 120.0;
        for _ in 0..(6.0 / dt) as u32 {
            world.step(dt);
        }
        assert!(
            (world.ball.position.z - 1.0).abs() < 0.2,
            "expected to settle near z=1.0, got {}",
            world.ball.position.z
        );
    }

    #[test]
    fn simulate_returns_one_frame_per_step_plus_the_initial_frame() {
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(0.0, 0.0, 100.0));
        let world = PhysicsWorld::new(ball, flat_ground());
        let frames = simulate(world, 1.0, 1.0 / 60.0);
        assert_eq!(frames.len(), 61);
        assert_eq!(frames[0].timestamp_secs, 0.0);
    }

    #[test]
    fn frame_has_no_cars_when_no_car_is_present() {
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(0.0, 0.0, 100.0));
        let world = PhysicsWorld::new(ball, flat_ground());
        assert!(world.frame().cars.is_empty());
    }

    #[test]
    fn car_in_free_fall_matches_kinematics_before_impact() {
        // The general-inertia box path should integrate translationally
        // identically to the sphere path — same semi-implicit Euler
        // kinematics, independent of shape.
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(1000.0, 0.0, 1000.0));
        let car = RigidBody::car_box(
            Vec3::new(60.0, 30.0, 18.0),
            180.0,
            Vec3::new(0.0, 0.0, 1000.0),
        );
        let mut world = PhysicsWorld::new(ball, flat_ground()).with_car(car);
        let dt: f32 = 1.0 / 240.0;
        let t: f32 = 0.2;
        let steps = (t / dt).round() as u32;
        for _ in 0..steps {
            world.step(dt);
        }
        let expected_vz = world.gravity.z * t;
        let car_after = *world.cars.first().expect("car should still be present");
        assert!(
            (car_after.linear_velocity.z - expected_vz).abs() < 1.0,
            "expected vz ~= {expected_vz}, got {}",
            car_after.linear_velocity.z
        );
        assert!(car_after.position.z < 1000.0, "car should have fallen");
    }

    #[test]
    fn dropped_car_settles_flat_on_the_ground_without_tipping_over() {
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(1000.0, 0.0, 93.0));
        let mut car = RigidBody::car_box(
            Vec3::new(60.0, 30.0, 18.0),
            180.0,
            Vec3::new(0.0, 0.0, 100.0),
        );
        car.restitution = 0.0;
        let ground = StaticPlane {
            restitution: 0.0,
            ..flat_ground()
        };
        let mut world = PhysicsWorld::new(ball, ground).with_car(car);
        let dt = 1.0 / 120.0;
        for _ in 0..(6.0 / dt) as u32 {
            world.step(dt);
        }
        let car_after = *world.cars.first().expect("car should still be present");
        assert!(
            (car_after.position.z - 18.0).abs() < 0.5,
            "expected the car to settle resting on its 18-unit half-height, got z={}",
            car_after.position.z
        );
        assert!(
            car_after.linear_velocity.length() < 1.0,
            "expected the car to have settled, got velocity {:?}",
            car_after.linear_velocity
        );
        // A car dropped flat, with no sideways forces, shouldn't tip onto
        // an edge or corner — its orientation should stay close to level.
        let up_after_rotation = car_after.orientation.rotate(&Vec3::new(0.0, 0.0, 1.0));
        assert!(
            (up_after_rotation - Vec3::new(0.0, 0.0, 1.0)).length() < 0.1,
            "expected the car to stay level, got local +Z pointing toward {up_after_rotation:?}"
        );
    }

    #[test]
    fn car_frame_reports_player_id_zero() {
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(1000.0, 0.0, 93.0));
        let car = RigidBody::car_box(
            Vec3::new(60.0, 30.0, 18.0),
            180.0,
            Vec3::new(0.0, 0.0, 18.0),
        );
        let world = PhysicsWorld::new(ball, flat_ground()).with_car(car);
        let frame = world.frame();
        assert_eq!(frame.cars.len(), 1);
        assert_eq!(frame.cars[0].player_id, 0);
    }

    #[test]
    fn ball_bounces_off_a_stationary_car_instead_of_passing_through() {
        // Both bodies float well above the ground and gravity is zeroed,
        // isolating the ball-vs-car collision this test actually checks
        // from ground contact — a real end-to-end proof that
        // `PhysicsWorld::step` now resolves the two dynamic bodies against
        // each other, not just each against the ground.
        let car_position = Vec3::new(300.0, 0.0, 100.0);
        let car_half_extents = Vec3::new(60.0, 30.0, 18.0);
        let mut car = RigidBody::car_box(car_half_extents, 180.0, car_position);
        car.restitution = 0.5;

        let ball_radius = 92.75;
        let mut ball = RigidBody::sphere(
            ball_radius,
            1.0,
            Vec3::new(
                car_position.x - car_half_extents.x - ball_radius - 100.0,
                0.0,
                100.0,
            ),
        );
        ball.restitution = 0.5;
        ball.linear_velocity = Vec3::new(300.0, 0.0, 0.0);

        let mut world = PhysicsWorld::new(ball, flat_ground()).with_car(car);
        world.gravity = Vec3::ZERO;

        let dt = 1.0 / 120.0;
        for _ in 0..(3.0 / dt) as u32 {
            world.step(dt);
        }

        let car_after = *world.cars.first().expect("car should still be present");
        let contact_surface_x = car_after.position.x - car_half_extents.x - ball_radius;
        assert!(
            world.ball.position.x < contact_surface_x + 1.0,
            "expected the ball to stop at the car's surface rather than tunnel through, \
             ball x={}, car surface x={}",
            world.ball.position.x,
            contact_surface_x
        );
        assert!(
            world.ball.linear_velocity.x < 0.0,
            "expected the ball to bounce back, got vx={}",
            world.ball.linear_velocity.x
        );
    }

    fn some_car(position: Vec3) -> RigidBody {
        RigidBody::car_box(Vec3::new(60.0, 30.0, 18.0), 180.0, position)
    }

    #[test]
    fn with_car_called_twice_builds_a_two_car_scene() {
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(1000.0, 0.0, 93.0));
        let world = PhysicsWorld::new(ball, flat_ground())
            .with_car(some_car(Vec3::new(0.0, 0.0, 18.0)))
            .with_car(some_car(Vec3::new(500.0, 0.0, 18.0)));
        assert_eq!(world.cars.len(), 2);
    }

    #[test]
    fn frame_assigns_sequential_player_ids_across_multiple_cars() {
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(1000.0, 0.0, 93.0));
        let world = PhysicsWorld::new(ball, flat_ground())
            .with_car(some_car(Vec3::new(0.0, 0.0, 18.0)))
            .with_car(some_car(Vec3::new(500.0, 0.0, 18.0)))
            .with_car(some_car(Vec3::new(1000.0, 0.0, 18.0)));
        let frame = world.frame();
        assert_eq!(frame.cars.len(), 3);
        let ids: Vec<u32> = frame.cars.iter().map(|c| c.player_id).collect();
        assert_eq!(ids, vec![0, 1, 2]);
    }

    #[test]
    fn cars_bounce_off_each_other_instead_of_passing_through() {
        // The real end-to-end proof of multi-car support: two cars,
        // floating well clear of the ground with gravity zeroed (isolating
        // the car-vs-car collision this test checks), closing head-on.
        // Before multi-car PhysicsWorld support, box_vs_box had no live
        // caller at all — this exercises it for real for the first time.
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(-5000.0, 0.0, 1000.0));

        let mut car_a = some_car(Vec3::new(-100.0, 0.0, 500.0));
        car_a.restitution = 0.5;
        car_a.linear_velocity = Vec3::new(200.0, 0.0, 0.0);

        let mut car_b = some_car(Vec3::new(100.0, 0.0, 500.0));
        car_b.restitution = 0.5;
        car_b.linear_velocity = Vec3::new(-200.0, 0.0, 0.0);

        let mut world = PhysicsWorld::new(ball, flat_ground())
            .with_car(car_a)
            .with_car(car_b);
        world.gravity = Vec3::ZERO;

        let dt = 1.0 / 120.0;
        for _ in 0..(3.0 / dt) as u32 {
            world.step(dt);
        }

        let a_after = world.cars[0];
        let b_after = world.cars[1];
        assert!(
            a_after.position.x < b_after.position.x,
            "expected car a to stay left of car b (no tunnelling), a.x={}, b.x={}",
            a_after.position.x,
            b_after.position.x
        );
        assert!(
            a_after.linear_velocity.x < 0.0,
            "expected car a to bounce back (negative x velocity), got {}",
            a_after.linear_velocity.x
        );
        assert!(
            b_after.linear_velocity.x > 0.0,
            "expected car b to bounce back (positive x velocity), got {}",
            b_after.linear_velocity.x
        );
    }

    #[test]
    fn a_car_with_throttle_input_drives_forward_across_the_ground() {
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(1000.0, 0.0, 93.0));
        let car = some_car(Vec3::new(0.0, 0.0, 18.0));
        let mut world = PhysicsWorld::new(ball, flat_ground()).with_car(car);
        world.set_car_input(
            0,
            rb_domain::ControllerInput {
                throttle: 1.0,
                ..Default::default()
            },
        );

        let start_x = world.cars[0].position.x;
        let dt = 1.0 / 60.0;
        for _ in 0..120 {
            world.step(dt);
        }

        assert!(
            world.cars[0].position.x > start_x + 1.0,
            "expected the car to drive forward under throttle, start={start_x}, end={}",
            world.cars[0].position.x
        );
        assert_eq!(
            world.frame().cars[0].input,
            Some(rb_domain::ControllerInput {
                throttle: 1.0,
                ..Default::default()
            }),
            "expected frame() to report the car's actual driving input"
        );
    }

    #[test]
    fn a_car_with_no_input_set_drives_exactly_like_before_driven_input_existed() {
        // Regression guard: with_car's default (neutral) input must not
        // change any existing free-rigid-box behavior.
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(1000.0, 0.0, 93.0));
        let mut car = some_car(Vec3::new(0.0, 0.0, 100.0));
        car.restitution = 0.0;
        let ground = StaticPlane {
            restitution: 0.0,
            ..flat_ground()
        };
        let mut world = PhysicsWorld::new(ball, ground).with_car(car);
        let dt = 1.0 / 120.0;
        for _ in 0..(6.0 / dt) as u32 {
            world.step(dt);
        }
        let settled = world.cars[0];
        assert!((settled.position.z - 18.0).abs() < 0.5);
        assert!(settled.linear_velocity.length() < 1.0);
    }

    #[test]
    fn a_car_with_boost_input_drives_forward_while_airborne() {
        // Unlike throttle, boost must work with no ground contact at all
        // — this is the real end-to-end proof that PhysicsWorld actually
        // threads a car's boost resource through drive::apply_driven_forces.
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(1000.0, 0.0, 93.0));
        let car = some_car(Vec3::new(0.0, 0.0, 1000.0));
        let mut world = PhysicsWorld::new(ball, flat_ground()).with_car(car);
        world.gravity = Vec3::ZERO; // isolate the boost force from gravity's fall
        world.set_car_input(
            0,
            rb_domain::ControllerInput {
                boost: true,
                ..Default::default()
            },
        );

        let start_x = world.cars[0].position.x;
        let dt = 1.0 / 60.0;
        for _ in 0..60 {
            world.step(dt);
        }

        assert!(
            world.cars[0].position.x > start_x + 1.0,
            "expected boost to drive the airborne car forward, start={start_x}, end={}",
            world.cars[0].position.x
        );
        let boost_after = world.frame().cars[0].boost_amount;
        assert!(
            boost_after < crate::drive::MAX_BOOST,
            "expected a held boost to have drained some fuel, got {boost_after}"
        );
    }

    #[test]
    fn a_new_car_starts_with_a_full_boost_tank() {
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(1000.0, 0.0, 93.0));
        let car = some_car(Vec3::new(0.0, 0.0, 18.0));
        let world = PhysicsWorld::new(ball, flat_ground()).with_car(car);
        assert_eq!(world.frame().cars[0].boost_amount, crate::drive::MAX_BOOST);
    }

    #[test]
    fn handbrake_restores_a_cars_own_base_friction_not_a_hardcoded_default() {
        // with_car snapshots whatever friction the car was constructed with
        // as its base — releasing handbrake must restore that value, not
        // some crate-wide default, even when it differs from one. Both
        // restitutions are zeroed so the car stays in continuous ground
        // contact frame-to-frame (a bouncy resting contact never fully
        // settles under this port's solver — see `resting_ball_stays_at_rest`
        // — which would otherwise flicker `on_ground` off for a step).
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(1000.0, 0.0, 93.0));
        let mut car = some_car(Vec3::new(0.0, 0.0, 18.0));
        car.friction = 0.9;
        car.restitution = 0.0;
        let ground = StaticPlane {
            restitution: 0.0,
            ..flat_ground()
        };
        let mut world = PhysicsWorld::new(ball, ground).with_car(car);
        let dt = 1.0 / 60.0;

        world.set_car_input(
            0,
            rb_domain::ControllerInput {
                handbrake: true,
                ..Default::default()
            },
        );
        world.step(dt);
        assert!(
            world.cars[0].friction < 0.9,
            "expected handbrake to reduce friction below the car's own 0.9 base, got {}",
            world.cars[0].friction
        );

        world.set_car_input(0, rb_domain::ControllerInput::default());
        world.step(dt);
        assert!(
            (world.cars[0].friction - 0.9).abs() < 1e-6,
            "expected releasing handbrake to restore the car's own 0.9 base friction, got {}",
            world.cars[0].friction
        );
    }

    #[test]
    fn a_handbraking_car_retains_more_sideways_slide_than_a_gripping_car() {
        // The real end-to-end proof: ground friction decelerates a body's
        // tangential (sliding) velocity — the same mechanism
        // `solver::tests::sliding_sphere_decelerates_due_to_friction`
        // already proves works for the ball. A car already sliding
        // sideways (as if mid-drift) should keep more of that sideways
        // speed under handbrake's reduced friction than it would under
        // normal grip — this is the actual mechanism `drive.rs` implements
        // handbrake with, exercised here through a live `PhysicsWorld`
        // rather than in isolation.
        let run = |handbrake: bool| -> f32 {
            let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(1000.0, 0.0, 93.0));
            let mut car = some_car(Vec3::new(0.0, 0.0, 18.0));
            car.linear_velocity = Vec3::new(0.0, 1000.0, 0.0);
            // Zeroed so the car stays in continuous ground contact frame to
            // frame — a bouncy resting contact (this port's known
            // no-warm-starting limitation, see `resting_ball_stays_at_rest`)
            // would otherwise flicker `on_ground` off for a step, silently
            // skipping that step's handbrake input entirely.
            car.restitution = 0.0;
            let ground = StaticPlane {
                restitution: 0.0,
                ..flat_ground()
            };
            let mut world = PhysicsWorld::new(ball, ground).with_car(car);
            world.set_car_input(
                0,
                rb_domain::ControllerInput {
                    handbrake,
                    ..Default::default()
                },
            );
            let dt = 1.0 / 120.0;
            for _ in 0..(0.5 / dt) as u32 {
                world.step(dt);
            }
            world.cars[0].linear_velocity.y.abs()
        };

        let gripping_remaining_slide = run(false);
        let handbraking_remaining_slide = run(true);
        assert!(
            handbraking_remaining_slide > gripping_remaining_slide,
            "expected handbrake's reduced friction to decelerate a sideways slide less than \
             normal grip, gripping={gripping_remaining_slide}, \
             handbrake={handbraking_remaining_slide}"
        );
    }

    #[test]
    fn a_car_with_jump_input_leaves_the_ground() {
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(1000.0, 0.0, 93.0));
        let mut car = some_car(Vec3::new(0.0, 0.0, 18.0));
        car.restitution = 0.0;
        let ground = StaticPlane {
            restitution: 0.0,
            ..flat_ground()
        };
        let mut world = PhysicsWorld::new(ball, ground).with_car(car);
        world.set_car_input(
            0,
            rb_domain::ControllerInput {
                jump: true,
                ..Default::default()
            },
        );

        let start_z = world.cars[0].position.z;
        let dt = 1.0 / 120.0;
        for _ in 0..12 {
            world.step(dt);
        }

        assert!(
            world.cars[0].position.z > start_z + 1.0,
            "expected jump input to lift the car off the ground, start={start_z}, end={}",
            world.cars[0].position.z
        );
    }

    #[test]
    fn holding_jump_does_not_repeatedly_relaunch_the_car() {
        // The real end-to-end proof that PhysicsWorld's car_jump_held
        // wiring actually prevents re-firing: hold jump for the whole
        // flight (never released), let the car arc up and land again, and
        // confirm it settles instead of being relaunched every time it
        // touches back down.
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(1000.0, 0.0, 93.0));
        let mut car = some_car(Vec3::new(0.0, 0.0, 18.0));
        car.restitution = 0.0;
        let ground = StaticPlane {
            restitution: 0.0,
            ..flat_ground()
        };
        let mut world = PhysicsWorld::new(ball, ground).with_car(car);
        world.set_car_input(
            0,
            rb_domain::ControllerInput {
                jump: true,
                ..Default::default()
            },
        );

        // A single JUMP_SPEED impulse under this world's gravity returns to
        // the ground in ~2*JUMP_SPEED/650 ≈ 0.9s; run well past that with
        // jump still held the entire time.
        let dt = 1.0 / 120.0;
        for _ in 0..(1.5 / dt) as u32 {
            world.step(dt);
        }

        let settled = world.cars[0];
        assert!(
            (settled.position.z - 18.0).abs() < 1.0,
            "expected the car to land and settle near its resting height instead of being \
             relaunched, got z={}",
            settled.position.z
        );
        assert!(
            settled.linear_velocity.length() < 5.0,
            "expected the car to have settled, got velocity {:?}",
            settled.linear_velocity
        );
    }
}
