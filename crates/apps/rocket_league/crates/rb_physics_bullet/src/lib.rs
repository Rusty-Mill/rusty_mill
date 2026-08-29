//! `rb_physics_bullet` — a from-scratch Rust port of Bullet Physics'
//! (zlib-licensed) rigid-body integration and sequential-impulse contact
//! solver, covering a dynamic sphere (the ball) and a dynamic box (a car),
//! each against a static plane (the ground) and against each other. See
//! `RB-PHYSICS-001` and ADR-0004 for why a port rather than a from-scratch
//! design or an integrated engine like Rapier, and `THIRD_PARTY_NOTICES.md`
//! for the required zlib attribution.
//!
//! Module layout mirrors the pipeline a `stepSimulation` call runs
//! (`btDiscreteDynamicsWorld::stepSimulation`):
//! forces/integration (`integrate`) → collision detection (`collision`) →
//! constraint solving (`solver`) → orchestration (`world`). `mat3`
//! provides the general 3x3 matrix `RigidBody` needs for a box's
//! anisotropic inertia tensor (a sphere's isotropic inertia never actually
//! needed one, but shares the same code path — see `body.rs`).
//!
//! `collision::box_vs_box` (car-vs-car) is implemented and unit-tested, but
//! has no live caller through `PhysicsWorld` yet — this scope has exactly
//! one car, so two cars colliding never actually happens in a real
//! simulated scene. Wiring it in needs `PhysicsWorld` to carry multiple
//! cars, a distinct, larger piece of still-open follow-up work (see
//! `RB-PHYSICS-001`'s open questions), not something done or skipped
//! silently.
//!
//! Not yet in scope (tracked in `RB-PHYSICS-001`, not silently dropped):
//! multi-car `PhysicsWorld` support (see above); split impulse;
//! warm-starting/sleeping; and consuming a recorded input sequence — a car
//! body here is a free rigid box, not a driven vehicle, so `simulate`
//! simulates the scene in isolation from its own initial state.

pub mod body;
pub mod collision;
pub mod integrate;
pub mod mat3;
pub mod solver;
pub mod world;

pub use body::{RigidBody, Shape, StaticPlane};
pub use collision::Contact;
pub use mat3::Mat3;
pub use world::{simulate, PhysicsWorld};
