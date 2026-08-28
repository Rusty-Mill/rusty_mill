//! `rb_physics_bullet` — v0 physics core, a from-scratch Rust port of
//! Bullet Physics' (zlib-licensed) rigid-body integration and
//! sequential-impulse contact solver, scoped to a dynamic sphere (the
//! ball) against a static plane (the ground). See `RB-PHYSICS-001` and
//! ADR-0004 for why a port rather than a from-scratch design or an
//! integrated engine like Rapier, and `THIRD_PARTY_NOTICES.md` for the
//! required zlib attribution.
//!
//! Module layout mirrors the pipeline a `stepSimulation` call runs
//! (`btDiscreteDynamicsWorld::stepSimulation`):
//! forces/integration (`integrate`) → collision detection (`collision`) →
//! constraint solving (`solver`) → orchestration (`world`).
//!
//! Not yet in scope (tracked in `RB-PHYSICS-001`, not silently dropped):
//! car-shaped (box) rigid bodies, general 3x3 inertia tensors, split
//! impulse, and consuming a recorded input sequence — v0 simulates the
//! ball in isolation from its own initial state.

pub mod body;
pub mod collision;
pub mod integrate;
pub mod solver;
pub mod world;

pub use body::{Sphere, StaticPlane};
pub use collision::Contact;
pub use world::{simulate, PhysicsWorld};
