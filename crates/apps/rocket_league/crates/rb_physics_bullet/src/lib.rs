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
//! (throttle, steering), boost, handbrake, a variable-height ground jump, a
//! double jump (plain or a directional, flip-cancelable dodge, depending on
//! stick input), a wall jump (itself dodgeable and flip-cancelable the
//! same way), airborne pitch/yaw/roll (air control), and a gentle landing
//! auto-orientation assist nudging an airborne car back toward level when
//! it isn't otherwise being steered — see `RB-PHYSICS-001` for what's
//! deliberately still out of scope beyond `drive` itself (split impulse,
//! warm-starting, a combined multi-body solve, and constant calibration).
//! `PhysicsWorld` gains arena walls (`with_wall`) as generic flat
//! `StaticPlane` geometry every body collides with, the same way it already
//! collides with the ground, curved edge fillets (`with_curve`, a
//! `StaticQuarterPipe` each), and compound-corner fillets (`with_corner_fillet`,
//! a `StaticCornerFillet` each) — both deflect only the ball, not a car; see
//! `body`'s own doc comment); `arena` builds Rocket League's actual
//! standard-arena octagonal footprint, a ceiling, and all 24 edge fillets
//! plus 16 compound-corner fillets throughout its vertical boundary from
//! that same machinery (`PhysicsWorld::standard_arena`) — 16 floor/
//! ceiling-seam fillets (the 4 cardinal walls and, since
//! `RB-PHYSICS-001-FR-021`, the 4 diagonal corner walls too), 8
//! vertical-edge fillets (since `RB-PHYSICS-001-FR-022`, one per corner wall
//! endpoint, where it meets its neighboring side/back wall), and, since
//! `RB-PHYSICS-001-FR-023`, 16 compound-corner fillets (one per vertex where
//! a vertical-edge fillet meets a floor- or ceiling-seam fillet) — still
//! without goal cutouts.
//!
//! Not yet in scope (tracked in `RB-PHYSICS-001`, not silently dropped):
//! a combined multi-body solve across simultaneous contacts — `world::step`
//! resolves each ball-vs-car and car-vs-car pair independently, one full
//! solver pass at a time, which is an approximation once 3+ bodies are
//! mutually touching in the same step (see `world`'s doc comment); split
//! impulse; warm-starting/sleeping; a car (box) actually being deflected by
//! a curved fillet (needs real support-mapping/SAT-style machinery against
//! curved geometry this port doesn't have); and consuming a *recorded*
//! input sequence — `PhysicsWorld::set_car_input` sets a car's current
//! input (persisting until changed), but nothing yet drives that from real
//! `RB-VERIFY-002` capture data frame-by-frame.

pub mod arena;
pub mod body;
pub mod collision;
pub mod drive;
pub mod integrate;
pub mod mat3;
pub mod solver;
pub mod world;

pub use body::{RigidBody, Shape, StaticCornerFillet, StaticPlane, StaticQuarterPipe};
pub use collision::Contact;
pub use mat3::Mat3;
pub use world::{simulate, PhysicsWorld};
