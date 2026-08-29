//! `rb_physics_bullet` — a from-scratch Rust port of Bullet Physics'
//! (zlib-licensed) rigid-body integration and sequential-impulse contact
//! solver, covering a dynamic sphere (the ball) and zero or more dynamic
//! boxes (cars), each against a static plane (the ground) and against
//! every other dynamic body in the scene. See `RB-PHYSICS-001` and
//! ADR-0004 for why a port rather than a from-scratch design or an
//! integrated engine like Rapier, and `THIRD_PARTY_NOTICES.md` for the
//! required zlib attribution.
//!
//! Module layout mirrors the pipeline a `stepSimulation` call runs
//! (`btDiscreteDynamicsWorld::stepSimulation`):
//! forces/integration (`integrate`) → collision detection (`collision`) →
//! constraint solving (`solver`) → orchestration (`world`). `mat3`
//! provides the general 3x3 matrix `RigidBody` needs for a box's
//! anisotropic inertia tensor (a sphere's isotropic inertia never actually
//! needed one, but shares the same code path — see `body.rs`).
//!
//! `drive` couples `rb_domain::ControllerInput` into ground-driving forces
//! (throttle, steering), boost, handbrake, a single ground jump, and
//! airborne pitch/yaw/roll (air control) on a car — see its own module doc
//! for what's deliberately still out of scope (double jump/dodge,
//! variable jump height, wall jump).
//!
//! Not yet in scope (tracked in `RB-PHYSICS-001`, not silently dropped):
//! a combined multi-body solve across simultaneous contacts — `world::step`
//! resolves each ball-vs-car and car-vs-car pair independently, one full
//! solver pass at a time, which is an approximation once 3+ bodies are
//! mutually touching in the same step (see `world`'s doc comment); split
//! impulse; warm-starting/sleeping; and consuming a *recorded* input
//! sequence — `PhysicsWorld::set_car_input` sets a car's current input
//! (persisting until changed), but nothing yet drives that from real
//! `RB-VERIFY-002` capture data frame-by-frame.

pub mod body;
pub mod collision;
pub mod drive;
pub mod integrate;
pub mod mat3;
pub mod solver;
pub mod world;

pub use body::{RigidBody, Shape, StaticPlane};
pub use collision::Contact;
pub use mat3::Mat3;
pub use world::{simulate, PhysicsWorld};
