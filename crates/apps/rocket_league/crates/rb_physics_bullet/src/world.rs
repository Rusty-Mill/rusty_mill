//! The simulation loop, porting the shape of
//! `btDiscreteDynamicsWorld::stepSimulation` (predict → collide → solve →
//! integrate) at fixed timestep — no substepping/interpolation yet, since
//! v0 has nothing that needs it (single ball, no CCD-worthy speeds).

use crate::body::{Sphere, StaticPlane};
use crate::collision::sphere_vs_plane;
use crate::{integrate, solver};
use rb_domain::{BallState, PhysicsFrame, Vec3};

/// The whole simulated scene for v0: one ball-like sphere against one
/// ground plane. Multiple spheres (e.g. future car bodies) are a
/// straightforward extension of `bodies: Vec<Sphere>`, deferred until
/// there's a second real body type to justify it (car boxes need general
/// inertia tensors anyway — see `RB-PHYSICS-001`).
pub struct PhysicsWorld {
    pub ball: Sphere,
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
    pub fn new(ball: Sphere, ground: StaticPlane) -> PhysicsWorld {
        PhysicsWorld {
            ball,
            ground,
            gravity: Vec3::new(0.0, 0.0, -650.0),
            elapsed_secs: 0.0,
        }
    }

    /// Advances the simulation by `dt` seconds: apply forces, integrate
    /// velocities, detect and resolve the ball/ground contact (if any),
    /// then integrate the transform — the same ordering as
    /// `btDiscreteDynamicsWorld::stepSimulation`'s single-substep path.
    pub fn step(&mut self, dt: f32) {
        self.ball.clear_forces();
        integrate::apply_gravity(&mut self.ball, self.gravity);
        integrate::apply_damping(&mut self.ball, dt);
        integrate::integrate_velocities(&mut self.ball, dt);

        if let Some(contact) = sphere_vs_plane(&self.ball, &self.ground) {
            solver::resolve_contact(&mut self.ball, &self.ground, &contact, dt);
        }

        let (position, orientation) = integrate::integrate_transform(
            self.ball.position,
            self.ball.orientation,
            self.ball.linear_velocity,
            self.ball.angular_velocity,
            dt,
        );
        self.ball.position = position;
        self.ball.orientation = orientation;

        self.elapsed_secs += dt;
    }

    /// The ball's current state as a `PhysicsFrame`, for consumption by
    /// `RB-VERIFY-003`'s divergence scorer. `cars` is always empty — v0
    /// has no car bodies (see `RB-PHYSICS-001`).
    pub fn frame(&self) -> PhysicsFrame {
        PhysicsFrame {
            timestamp_secs: self.elapsed_secs,
            ball: BallState {
                position: self.ball.position,
                rotation: self.ball.orientation,
                velocity: self.ball.linear_velocity,
                angular_velocity: self.ball.angular_velocity,
            },
            cars: Vec::new(),
        }
    }
}

/// Runs `PhysicsWorld` for `duration_secs` at fixed `dt`, recording one
/// `PhysicsFrame` per step. This is `RB-PHYSICS-001-FR-001`'s "produce a
/// `Vec<PhysicsFrame>` the divergence scorer can consume" — the candidate
/// trajectory `rb_verify_cli` compares against recorded ground truth.
///
/// Doesn't yet consume a recorded input sequence (no car exists to receive
/// throttle/steer/boost input in v0) — it simulates the ball in isolation
/// from its initial state. Once `RB-VERIFY-002` capture data and a car body
/// both exist, this signature grows an `inputs` parameter rather than
/// staying ball-only.
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
mod tests {
    use super::*;

    fn flat_ground() -> StaticPlane {
        StaticPlane::new(Vec3::new(0.0, 0.0, 1.0), 0.0)
    }

    #[test]
    fn ball_in_free_fall_matches_kinematics_before_impact() {
        let ball = Sphere::new(1.0, 1.0, Vec3::new(0.0, 0.0, 1000.0));
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
        let mut ball = Sphere::new(1.0, 1.0, Vec3::new(0.0, 0.0, 1.0));
        // Inelastic on purpose, on *both* surfaces (combined restitution is
        // an average of the two — see solver.rs): v0 has no warm-starting
        // or sleeping, so a *bouncy* resting contact legitimately never
        // settles under a naive per-frame sequential impulse solve — each
        // frame's gravity-induced velocity is a fresh "impact" that
        // restitution bounces back up, forever. That's a real, known
        // limitation (tracked in RB-PHYSICS-001), not covered by this
        // test; this test checks the inelastic case actually settles,
        // which it should regardless.
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
        let ball = Sphere::new(1.0, 1.0, Vec3::new(0.0, 0.0, 50.0));
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
        let ball = Sphere::new(1.0, 1.0, Vec3::new(0.0, 0.0, 100.0));
        let world = PhysicsWorld::new(ball, flat_ground());
        let frames = simulate(world, 1.0, 1.0 / 60.0);
        assert_eq!(frames.len(), 61);
        assert_eq!(frames[0].timestamp_secs, 0.0);
    }
}
