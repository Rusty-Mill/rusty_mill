//! The simulation loop, porting the shape of
//! `btDiscreteDynamicsWorld::stepSimulation` (predict → collide → solve →
//! integrate) at fixed timestep — no substepping/interpolation yet, since
//! nothing in this scope needs it (no CCD-worthy speeds).

use crate::body::{RigidBody, StaticPlane};
use crate::collision;
use crate::{integrate, solver};
use rb_domain::{BallState, CarState, PhysicsFrame, Vec3};

/// The whole simulated scene: one ball-like sphere, an optional car-like
/// box, and one ground plane. Ball and car each collide only with the
/// ground in this scope — box-vs-sphere (car-vs-ball) collision is real,
/// not-yet-implemented follow-up work (`RB-PHYSICS-001-FR-004`'s open
/// items), not something silently assumed away.
pub struct PhysicsWorld {
    pub ball: RigidBody,
    pub car: Option<RigidBody>,
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
            car: None,
            ground,
            gravity: Vec3::new(0.0, 0.0, -650.0),
            elapsed_secs: 0.0,
        }
    }

    /// Adds a car-shaped body to the scene, stepped alongside the ball
    /// (against the ground only — see the struct doc comment).
    pub fn with_car(mut self, car: RigidBody) -> PhysicsWorld {
        self.car = Some(car);
        self
    }

    /// Advances one body by `dt` seconds against `ground`: apply forces,
    /// integrate velocities, detect and resolve every ground contact (a
    /// manifold of 1 to 4 points depending on shape/orientation — see
    /// `collision::contacts_vs_plane`), then integrate the transform and
    /// refresh the body's world-space inertia tensor for its new
    /// orientation — the same ordering as
    /// `btDiscreteDynamicsWorld::stepSimulation`'s single-substep path.
    fn step_body(body: &mut RigidBody, ground: &StaticPlane, gravity: Vec3, dt: f32) {
        body.clear_forces();
        integrate::apply_gravity(body, gravity);
        integrate::apply_damping(body, dt);
        integrate::integrate_velocities(body, dt);

        let contacts = collision::contacts_vs_plane(body, ground);
        if !contacts.is_empty() {
            solver::resolve_contacts(body, ground, &contacts, dt);
        }

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

    /// Advances the whole scene by `dt` seconds.
    pub fn step(&mut self, dt: f32) {
        Self::step_body(&mut self.ball, &self.ground, self.gravity, dt);
        if let Some(car) = &mut self.car {
            Self::step_body(car, &self.ground, self.gravity, dt);
        }
        self.elapsed_secs += dt;
    }

    /// The scene's current state as a `PhysicsFrame`, for consumption by
    /// `RB-VERIFY-003`'s divergence scorer. `cars` holds one `CarState`
    /// (`player_id` 0, no recovered input — there's no input source in
    /// this scope) when `car` is present, otherwise it's empty.
    pub fn frame(&self) -> PhysicsFrame {
        let cars = match &self.car {
            Some(car) => vec![CarState {
                player_id: 0,
                position: car.position,
                rotation: car.orientation,
                velocity: car.linear_velocity,
                angular_velocity: car.angular_velocity,
                boost_amount: 0.0,
                input: None,
            }],
            None => Vec::new(),
        };
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
        let car_after = world.car.expect("car should still be present");
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
        let car_after = world.car.expect("car should still be present");
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
}
