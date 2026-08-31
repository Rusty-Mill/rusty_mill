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
//! `StaticQuarterPipe` each), compound-corner fillets (`with_corner_fillet`,
//! a `StaticCornerFillet` each — since `RB-PHYSICS-001-FR-027`, both deflect
//! a car too, via testing the car's own 8 corners — exact, not an
//! approximation, for this containment-style contact
//! (`RB-PHYSICS-001-FR-032` rigorously confirmed a once-suspected
//! under-detection gap here doesn't actually exist; see
//! `collision::box_vs_quarter_pipe`'s own doc comment), and windowed goal
//! walls (`with_goal_wall`, a
//! `StaticGoalWall` each — since `RB-PHYSICS-001-FR-028`, both the ball and
//! a car can pass through the window, the car via the same per-corner
//! approach FR-027 established; see `collision::contacts_vs_goal_wall`'s
//! own doc comment), and, since `RB-PHYSICS-001-FR-029`, a modeled goal
//! interior behind each window (`with_bounded_wall`, a `StaticBoundedWall`
//! each — collides only within its own rectangular bound, unlike a plain
//! `StaticPlane`; see `body::StaticBoundedWall`'s own doc comment for why);
//! `arena` builds
//! Rocket League's actual standard-arena octagonal footprint, a ceiling, an
//! actual goal-mouth opening in each back wall, all 30 edge fillets plus 20
//! compound-corner fillets throughout its vertical boundary, and each
//! goal's own bounded interior volume from that same machinery
//! (`PhysicsWorld::standard_arena`) — 16 floor/
//! ceiling-seam fillets (the 4 cardinal walls and, since
//! `RB-PHYSICS-001-FR-021`, the 4 diagonal corner walls too, the latter
//! since `RB-PHYSICS-001-FR-025` using a distinctly larger radius than the
//! former), 8 vertical-edge fillets (since `RB-PHYSICS-001-FR-022`, one per
//! corner wall endpoint, where it meets its neighboring side/back wall), 16
//! compound-corner fillets (since `RB-PHYSICS-001-FR-023`, one per vertex
//! where a vertical-edge fillet meets a floor- or ceiling-seam fillet),
//! since `RB-PHYSICS-001-FR-024`, 6 goal-cutout-edge fillets (two posts and
//! a crossbar per goal) rounding each goal-mouth window's own rim, since
//! `RB-PHYSICS-001-FR-026`, 4 more compound-corner fillets rounding each
//! goal's own post-crossbar vertex, and, since `RB-PHYSICS-001-FR-029`, a
//! bounded box behind each goal's own window (2 plain back-of-net planes,
//! 4 bounded side walls, 2 bounded roofs) so a ball or car passing through
//! no longer sails into unbounded open space.
//!
//! Since `RB-PHYSICS-001-FR-030`, `world::step` resolves every
//! ball-vs-car and car-vs-car manifold in a step together as one combined
//! multi-body solve (`solver::resolve_dynamic_manifolds`), sharing one
//! velocity-change accumulator per body across every manifold that body
//! takes part in, rather than fully resolving and applying each pair
//! independently before the next pair's setup even reads a body's velocity
//! — the fix for a body (e.g. a car) mutually touching two others in the
//! same step (see `world::step`'s own doc comment for what's still
//! simplified relative to Bullet's actual interleaved-across-islands
//! solver).
//!
//! Since `RB-PHYSICS-001-FR-031`, `drive`'s and `arena`'s uncalibrated
//! placeholder constants have each been individually audited against the
//! community reverse-engineering effort (RocketSim, RLUtilities, the RLBot
//! wiki) — some corrected to a real, multi-source-confirmed value
//! (`drive::JUMP_SPEED`, `drive::JUMP_HOLD_ACCELERATION`, and a new
//! `drive::UNBOOSTED_MAX_CAR_SPEED` split from the boosted
//! `drive::MAX_CAR_SPEED`), some confirmed already correct, and the rest
//! explicitly flagged (in their own doc comments and in the FR-031 spec
//! entry) as still uncalibrated rather than left silently ambiguous — see
//! `drive`'s own module doc comment for the full per-constant breakdown.
//! This audit does NOT close `RB-PHYSICS-001-FR-005`'s real-data
//! calibration, which still needs `PHASE-0-EXIT`.
//!
//! Since `RB-PHYSICS-001-FR-033`, each goal also gets a real net
//! (`with_net`, a `net::NetMesh` each — a mass-spring grid, anchored along
//! its own perimeter to the goal frame, catching the *ball* via this
//! crate's existing dynamic-vs-dynamic sequential-impulse solver path
//! rather than a bespoke penalty-force system), replacing part of
//! `RB-PHYSICS-001-FR-029`'s solid-bounding-box stand-in — see `net`'s own
//! module doc comment for the design and for what's explicitly still out
//! of scope (a car's own contact against the net, a full 3D "sock" shape,
//! bending stiffness).
//!
//! Not yet in scope (tracked in `RB-PHYSICS-001`, not silently dropped):
//! split impulse; warm-starting/sleeping; and consuming a *recorded* input
//! sequence — `PhysicsWorld::set_car_input` sets a car's current input
//! (persisting until changed), but nothing yet drives that from real
//! `RB-VERIFY-002` capture data frame-by-frame.

pub mod arena;
pub mod body;
pub mod collision;
pub mod drive;
pub mod integrate;
pub mod mat3;
pub mod net;
pub mod solver;
pub mod world;

pub use body::{
    RigidBody, Shape, StaticBoundedWall, StaticCornerFillet, StaticGoalWall, StaticPlane,
    StaticQuarterPipe,
};
pub use collision::Contact;
pub use mat3::Mat3;
pub use net::NetMesh;
pub use world::{simulate, PhysicsWorld};
