//! The simulation loop, porting the shape of
//! `btDiscreteDynamicsWorld::stepSimulation` (predict → collide → solve →
//! integrate) at fixed timestep — no substepping/interpolation yet, since
//! nothing in this scope needs it (no CCD-worthy speeds).

use crate::body::{
    RigidBody, StaticBoundedWall, StaticCornerFillet, StaticGoalWall, StaticPlane,
    StaticQuarterPipe,
};
use crate::collision;
use crate::net::NetMesh;
use crate::solver::ContactCache;
use crate::{drive, integrate, solver};
use rb_domain::{BallState, CarState, ControllerInput, PhysicsFrame, Vec3};
use std::collections::HashMap;

/// Hard cap (uu/s) on the ball's linear speed — confirmed exact against
/// RocketSim's own `RLConst.h` during `RB-PHYSICS-001-FR-061`'s audit:
/// `BALL_MAX_SPEED = 6000.f`. A pure velocity cap, not a torque or force
/// constant, so — like `drive::MAX_CAR_ANGULAR_SPEED` before it (see
/// `RB-PHYSICS-001-FR-057`'s own findings) — it transfers cleanly
/// regardless of this port's ball not being calibrated to real Rocket
/// League's own mass/inertia. Enforced by `clamp_ball_velocity`, called
/// once per step in `PhysicsWorld::step`.
pub const BALL_MAX_SPEED: f32 = 6000.0;

/// Hard cap (rad/s) on the ball's angular speed — confirmed exact against
/// RocketSim's own `RLConst.h` during `RB-PHYSICS-001-FR-061`'s audit:
/// `BALL_MAX_ANG_SPEED = 6.f, // Ball can never exceed this angular
/// velocity (radians/s)`. Enforced by `clamp_ball_velocity`, the same way
/// `BALL_MAX_SPEED` is.
pub const BALL_MAX_ANG_SPEED: f32 = 6.0;

/// Scales `ball.linear_velocity`/`ball.angular_velocity` back down to
/// `BALL_MAX_SPEED`/`BALL_MAX_ANG_SPEED` (preserving direction) if either is
/// exceeded — a genuine clamp, the same kind `drive::clamp_angular_speed`
/// already applies to a car's own angular speed, generalized here to both
/// linear and angular speed since the ball has no drive-input-gated
/// mechanic of its own to house a car-specific version of this in
/// `drive.rs`. Called once per step in `PhysicsWorld::step`, right after
/// this step's contact resolution (including any net) but before the
/// transform integrates — matching real RocketSim's own `_FinishPhysicsTick`
/// placement (fetched and confirmed during `RB-PHYSICS-001-FR-061`'s
/// audit: enforced after collision resolution, at the end of the physics
/// tick) more precisely than `drive::clamp_angular_speed`'s own placement
/// managed for the car (mid-pipeline, before this step's own contact
/// resolution — see that function's own doc comment for why). Like that
/// function, a same-step contact-solver impulse is clamped this same call
/// (not deferred to next step), but a later force/impulse applied after
/// this call within the same step (none currently exists for the ball)
/// wouldn't be re-clamped until the next step's call.
fn clamp_ball_velocity(ball: &mut RigidBody) {
    let speed = ball.linear_velocity.length();
    if speed > BALL_MAX_SPEED {
        ball.linear_velocity *= BALL_MAX_SPEED / speed;
    }
    let angular_speed = ball.angular_velocity.length();
    if angular_speed > BALL_MAX_ANG_SPEED {
        ball.angular_velocity *= BALL_MAX_ANG_SPEED / angular_speed;
    }
}

/// The whole simulated scene: one ball-like sphere, zero or more car-like
/// boxes, one ground plane, zero or more arena walls (`walls`, added via
/// `with_wall` — a plain flat `StaticPlane` each, typically with a
/// horizontal normal), and zero or more curved wall-to-floor/wall-to-ceiling
/// fillets (`curves`, added via `with_curve` — see `RB-PHYSICS-001-FR-020`
/// and `curves`' own doc comment; since `RB-PHYSICS-001-FR-027`, a car is
/// deflected by one too, not just the ball). Every body collides with the
/// ground, every wall, every curve, every compound-corner fillet, every
/// goal wall, and every bounded wall (a wall is just a `StaticPlane` whose
/// normal isn't "up," so the same machinery serves both); every car also
/// collides with the ball and with every other car
/// (`collision::contacts_between`, dispatching to `sphere_vs_box` or
/// `box_vs_box`) — a real N-body scene, not just the one-ball-one-car case
/// `RB-PHYSICS-001-FR-004`/`FR-006` originally scoped. Since
/// `RB-PHYSICS-001-FR-052`, every body's own static-surface contacts and
/// every ball-vs-car/car-vs-car manifold detected in a step are all
/// resolved together via `solver::resolve_manifolds`, one shared iteration
/// budget per body, rather than a body's static contacts (themselves
/// already combined since `RB-PHYSICS-001-FR-051`, and every dynamic
/// manifold already combined since `RB-PHYSICS-001-FR-030`) being two
/// separate solves, one fully resolved and applied before the other's own
/// setup ever reads the shared body's velocity (see `step`'s own doc
/// comment). Each car also has a
/// current `ControllerInput` (`car_inputs`, set via `set_car_input`,
/// `ControllerInput::default()` — neutral — until set), a boost resource
/// (`car_boost`, set via `set_car_boost`, starting full), a remembered base
/// friction (`car_base_friction`, snapshotted from the car's own
/// `RigidBody.friction` when added) that `drive::apply_driven_forces` uses
/// to restore grip after a handbrake-induced reduction, a remembered
/// jump-held state (`car_jump_held`, starting `false`) that
/// `drive::apply_driven_forces` uses to fire jump only on a fresh press,
/// a remembered double-jump-available flag (`car_double_jump_available`,
/// starting `true`) that `drive::apply_driven_forces` resets whenever the
/// car touches the ground *or* a wall and consumes when an airborne fresh
/// press spends it on a plain double jump (not a wall jump), and a
/// remembered jump-hold-window (`car_jump_hold_time_remaining`, starting
/// `0.0`) that `drive::apply_driven_forces` uses to give the ground jump
/// variable height — armed to `drive::JUMP_HOLD_MAX_DURATION` by a fresh
/// ground-jump press, counted down while `jump` stays held, and zeroed
/// immediately on release — and a remembered cancelable-flip flag
/// (`car_dodge_flip_active`, starting `false`) that `drive::apply_driven_forces`
/// sets whenever a dodge fires, clears whenever a plain double jump fires
/// (so a stale flag from an earlier dodge can't leak into a later,
/// unrelated double jump), and spends on a further fresh press to
/// flip-cancel the dodge's spin — all driving the car via
/// `drive::apply_driven_forces`. Since `RB-PHYSICS-001-FR-033`, `nets`
/// (added via `with_net`) gives the ball a real mass-spring net to be
/// caught by, resolved after every other contact each step, and since
/// `RB-PHYSICS-001-FR-038`, every car too — see `nets`' own doc comment.
pub struct PhysicsWorld {
    pub ball: RigidBody,
    pub cars: Vec<RigidBody>,
    car_inputs: Vec<ControllerInput>,
    car_boost: Vec<f32>,
    car_base_friction: Vec<f32>,
    car_jump_held: Vec<bool>,
    car_double_jump_available: Vec<bool>,
    car_jump_hold_time_remaining: Vec<f32>,
    car_dodge_flip_active: Vec<bool>,
    pub ground: StaticPlane,
    pub walls: Vec<StaticPlane>,
    /// Curved wall-to-floor/wall-to-ceiling fillets (`RB-PHYSICS-001-FR-020`),
    /// added via `with_curve` — empty by default, same as `walls`. Both the
    /// ball and a car are deflected by a curve since `RB-PHYSICS-001-FR-027`
    /// — a car's own box is approximated by testing its 8 corners against
    /// the curve (see `collision::box_vs_quarter_pipe`'s own doc comment
    /// for what that does and doesn't catch), not a full convex-vs-curve
    /// narrow phase.
    pub curves: Vec<StaticQuarterPipe>,
    /// Compound-corner fillets (`RB-PHYSICS-001-FR-023`) — the small
    /// spherical patches blending a vertical-edge fillet (`curves`) into a
    /// floor- or ceiling-seam fillet (also `curves`) at each corner wall's
    /// own top/bottom endpoint, added via `with_corner_fillet`. Same
    /// corner-testing deflection convention `curves` uses for a car, since
    /// `RB-PHYSICS-001-FR-027` (`collision::box_vs_corner_fillet`), and
    /// empty by default.
    pub corner_fillets: Vec<StaticCornerFillet>,
    /// Windowed back walls with an actual goal-mouth opening
    /// (`RB-PHYSICS-001-FR-024`), added via `with_goal_wall` — empty by
    /// default. Both the ball and every car are resolved against these;
    /// since `RB-PHYSICS-001-FR-028`, a car passes through the window
    /// exactly like the ball does (`collision::contacts_vs_goal_wall`'s box
    /// path tests each corner against the window, same as the sphere's
    /// single center-point test — see its own doc comment), so a car can
    /// now actually drive into a goal.
    pub goal_walls: Vec<StaticGoalWall>,
    /// The goal box's own side walls and roof (`RB-PHYSICS-001-FR-029`),
    /// added via `with_bounded_wall` — empty by default. Each only
    /// collides within its own rectangular bound (see
    /// `body::StaticBoundedWall`'s own doc comment for why an unbounded
    /// plane there would be wrong); resolved for the ball and every car,
    /// same as `goal_walls`.
    pub bounded_walls: Vec<StaticBoundedWall>,
    /// Real mass-spring net panels (`RB-PHYSICS-001-FR-033`), added via
    /// `with_net` — empty by default. Catches the ball and, since
    /// `RB-PHYSICS-001-FR-038`, every car too (see `net::NetMesh`'s own doc
    /// comment); resolved after every other contact this step, reusing the
    /// same ball-plus-cars snapshot `solver::resolve_manifolds` just
    /// resolved rather than going through that function's own shared solve
    /// (a net's own points aren't part of that scene-wide `bodies` list at
    /// all).
    pub nets: Vec<NetMesh>,
    pub gravity: Vec3,
    elapsed_secs: f32,
    /// Warm-starting's own persistent state (`RB-PHYSICS-001-FR-035`) for
    /// `solver::resolve_manifolds`'s own dynamic-manifold channel, keyed by
    /// (normalized) ball-vs-car/car-vs-car body-index pair — see
    /// `solver::ContactCache`'s own doc comment for what it does and why
    /// only a body's dynamic manifolds (not its static-shape contacts) are
    /// warm-started.
    dynamic_manifold_caches: HashMap<(usize, usize), ContactCache>,
}

/// Borrowed references to every static-shape collection in a `PhysicsWorld`
/// (`RB-PHYSICS-001-FR-051`) — exists purely so `static_contact_manifolds`
/// takes one bundled parameter instead of six separate slice/reference
/// arguments (clippy's `too_many_arguments` threshold), not as a genuine
/// new abstraction; every field here is still borrowed directly from the
/// same `PhysicsWorld` fields it always was.
struct StaticScene<'a> {
    ground: &'a StaticPlane,
    walls: &'a [StaticPlane],
    curves: &'a [StaticQuarterPipe],
    corner_fillets: &'a [StaticCornerFillet],
    goal_walls: &'a [StaticGoalWall],
    bounded_walls: &'a [StaticBoundedWall],
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
            car_double_jump_available: Vec::new(),
            car_jump_hold_time_remaining: Vec::new(),
            car_dodge_flip_active: Vec::new(),
            ground,
            walls: Vec::new(),
            curves: Vec::new(),
            corner_fillets: Vec::new(),
            goal_walls: Vec::new(),
            bounded_walls: Vec::new(),
            nets: Vec::new(),
            gravity: Vec3::new(0.0, 0.0, -650.0),
            elapsed_secs: 0.0,
            dynamic_manifold_caches: HashMap::new(),
        }
    }

    /// Builds a scene bounded by Rocket League's real standard-arena
    /// footprint (`RB-PHYSICS-001-FR-019`/`FR-020`) instead of an empty
    /// `walls`/`curves`/`corner_fillets`/`goal_walls` list a caller
    /// populates itself: the octagonal boundary plus a ceiling from
    /// `arena::standard_walls`, the curved wall-to-floor/wall-to-ceiling,
    /// corner-wall vertical-edge, and goal-cutout-edge fillets from
    /// `arena::standard_curves`/`standard_goal_cutout_fillets`, the
    /// compound-corner fillets from `arena::standard_corner_fillets`/
    /// `standard_goal_corner_fillets`, the windowed goal walls from
    /// `arena::standard_goal_walls`, and the same flat ground
    /// (`arena::standard_ground`) every scene already used.
    /// Equivalent to `PhysicsWorld::new(ball, arena::standard_ground())`
    /// followed by a `with_wall` call for each of `arena::standard_walls()`'s
    /// 7 planes, a `with_curve` call for each of `arena::standard_curves()`'s
    /// 24 fillets and `arena::standard_goal_cutout_fillets()`'s 6, a
    /// `with_corner_fillet` call for each of
    /// `arena::standard_corner_fillets()`'s 16 fillets and
    /// `arena::standard_goal_corner_fillets()`'s 4, a `with_goal_wall`
    /// call for each of `arena::standard_goal_walls()`'s 2 windowed walls,
    /// and, since `RB-PHYSICS-001-FR-029`, a modeled goal interior behind
    /// each window: a `with_wall` call for each of
    /// `arena::standard_goal_back_walls()`'s 2 plain back-of-net planes,
    /// and a `with_bounded_wall` call for each of
    /// `arena::standard_goal_side_walls()`'s 4 and
    /// `arena::standard_goal_roofs()`'s 2 bounded side/roof walls, and,
    /// since `RB-PHYSICS-001-FR-033`, a `with_net` call for each of
    /// `arena::standard_nets()`'s 2 goal net panels. Cars are
    /// added afterward with `with_car`, exactly as with `PhysicsWorld::new`.
    pub fn standard_arena(ball: RigidBody) -> PhysicsWorld {
        let mut world = PhysicsWorld::new(ball, crate::arena::standard_ground());
        for wall in crate::arena::standard_walls() {
            world = world.with_wall(wall);
        }
        for curve in crate::arena::standard_curves() {
            world = world.with_curve(curve);
        }
        for curve in crate::arena::standard_goal_cutout_fillets() {
            world = world.with_curve(curve);
        }
        for corner_fillet in crate::arena::standard_corner_fillets() {
            world = world.with_corner_fillet(corner_fillet);
        }
        for corner_fillet in crate::arena::standard_goal_corner_fillets() {
            world = world.with_corner_fillet(corner_fillet);
        }
        for goal_wall in crate::arena::standard_goal_walls() {
            world = world.with_goal_wall(goal_wall);
        }
        for wall in crate::arena::standard_goal_back_walls() {
            world = world.with_wall(wall);
        }
        for wall in crate::arena::standard_goal_side_walls() {
            world = world.with_bounded_wall(wall);
        }
        for wall in crate::arena::standard_goal_roofs() {
            world = world.with_bounded_wall(wall);
        }
        for net in crate::arena::standard_nets() {
            world = world.with_net(net);
        }
        world
    }

    /// Seeds a `standard_arena` `PhysicsWorld` from a recorded
    /// `PhysicsFrame` — the ball's and every car's position/rotation/
    /// velocity/angular_velocity come directly from the frame (a direct
    /// 1:1 field match with `BallState`/`CarState`), combined with
    /// `RigidBody::standard_ball`/`standard_car`'s confirmed real
    /// shape/mass for what a `PhysicsFrame` doesn't carry at all. Added
    /// for `RB-PHYSICS-001-FR-076`'s candidate-engine plumbing:
    /// `FR-077` uses this to seed a simulation from one of a real
    /// capture's own recorded frames, then drives it forward with
    /// `simulate_recorded` using that same capture's remaining frames.
    ///
    /// The returned world's own `frame().timestamp_secs` starts at
    /// `frame.timestamp_secs` (not `0.0`) — deliberately, so the
    /// candidate trajectory `simulate_recorded` produces lands on the
    /// *same* absolute clock the recorded capture used, letting
    /// `rb_domain::divergence::score`'s nearest-timestamp alignment
    /// actually match frames up; seeding at `0.0` instead would put every
    /// candidate frame outside any real capture's own alignment tolerance.
    ///
    /// Each car's `boost_amount` is seeded from the frame's own recorded
    /// value via `set_car_boost`. Every other per-car runtime state
    /// `PhysicsWorld` tracks but a `PhysicsFrame` doesn't carry at all
    /// (`car_jump_held`, `car_double_jump_available`,
    /// `car_jump_hold_time_remaining`, `car_dodge_flip_active`) is left at
    /// `with_car`'s own fixed defaults (not held, double-jump available,
    /// zero hold time, no dodge in progress) — accurate only if `frame`
    /// captures a genuinely neutral, grounded moment. Choosing such a
    /// frame is the caller's responsibility (see `RB-PHYSICS-001-FR-077`'s
    /// own seed-frame heuristic), not this function's — `PhysicsWorld` has
    /// no public way to seed those four fields directly at all yet.
    ///
    /// Cars are seeded in `frame.cars`'s own order, so a car's resulting
    /// index in `self.cars` (and thus what `set_car_input` expects) always
    /// matches that car's own `player_id` in `frame` — a real capture's
    /// own `player_id` is already a per-session ordinal assigned in
    /// exactly this order (see `RB-VERIFY-002-FR-001`'s plugin), so this
    /// isn't a coincidence to maintain, just a fact to preserve.
    pub fn from_frame(frame: &PhysicsFrame) -> PhysicsWorld {
        let mut ball = RigidBody::standard_ball(frame.ball.position);
        ball.orientation = frame.ball.rotation;
        ball.linear_velocity = frame.ball.velocity;
        ball.angular_velocity = frame.ball.angular_velocity;
        ball.update_inertia_tensor();

        let mut world = PhysicsWorld::standard_arena(ball);
        world.elapsed_secs = frame.timestamp_secs;

        for car_state in &frame.cars {
            let mut car = RigidBody::standard_car(car_state.position);
            car.orientation = car_state.rotation;
            car.linear_velocity = car_state.velocity;
            car.angular_velocity = car_state.angular_velocity;
            car.update_inertia_tensor();
            world = world.with_car(car);
            let index = world.cars.len() - 1;
            world.set_car_boost(index, car_state.boost_amount);
        }

        world
    }

    /// Adds one arena wall to the scene — a flat `StaticPlane`, typically
    /// with a horizontal normal (e.g. `Vec3::new(1.0, 0.0, 0.0)`), though
    /// nothing here actually requires that. Callable more than once to
    /// build a multi-wall arena; a scene with no walls added (the default)
    /// behaves exactly as before walls existed — every body's ground
    /// contact and driven-input behavior is unaffected by an empty
    /// `walls`. Doesn't model a full arena footprint (corners, curvature,
    /// a ceiling) — just the generic per-wall collision and wall-jump
    /// capability; see `RB-PHYSICS-001-FR-013`'s Non-goals.
    pub fn with_wall(mut self, wall: StaticPlane) -> PhysicsWorld {
        self.walls.push(wall);
        self
    }

    /// Adds one curved wall-to-floor/wall-to-ceiling fillet to the scene
    /// (`RB-PHYSICS-001-FR-020`) — callable more than once, same pattern as
    /// `with_wall`; a scene with no curves added (the default) behaves
    /// exactly as before curves existed. Deflects both the ball and a car —
    /// see `curves`' own doc comment.
    pub fn with_curve(mut self, curve: StaticQuarterPipe) -> PhysicsWorld {
        self.curves.push(curve);
        self
    }

    /// Adds one compound-corner fillet to the scene
    /// (`RB-PHYSICS-001-FR-023`) — callable more than once, same pattern as
    /// `with_curve`; a scene with no corner fillets added (the default)
    /// behaves exactly as before they existed. Deflects both the ball and a
    /// car — see `corner_fillets`' own doc comment.
    pub fn with_corner_fillet(mut self, corner_fillet: StaticCornerFillet) -> PhysicsWorld {
        self.corner_fillets.push(corner_fillet);
        self
    }

    /// Adds one windowed back wall to the scene (`RB-PHYSICS-001-FR-024`) —
    /// callable more than once, same pattern as `with_wall`; a scene with
    /// no goal walls added (the default) behaves exactly as before they
    /// existed. Deflects both the ball and, since `RB-PHYSICS-001-FR-028`,
    /// a car too.
    pub fn with_goal_wall(mut self, goal_wall: StaticGoalWall) -> PhysicsWorld {
        self.goal_walls.push(goal_wall);
        self
    }

    /// Adds one bounded wall to the scene (`RB-PHYSICS-001-FR-029`) —
    /// callable more than once, same pattern as `with_wall`; a scene with
    /// no bounded walls added (the default) behaves exactly as before
    /// they existed. Deflects both the ball and every car, same as
    /// `goal_walls`.
    pub fn with_bounded_wall(mut self, bounded_wall: StaticBoundedWall) -> PhysicsWorld {
        self.bounded_walls.push(bounded_wall);
        self
    }

    /// Adds one net panel to the scene (`RB-PHYSICS-001-FR-033`) — callable
    /// more than once, same pattern as `with_bounded_wall`; a scene with no
    /// nets added (the default) behaves exactly as before nets existed.
    /// Catches the ball and every car — see `nets`' own doc comment.
    pub fn with_net(mut self, net: NetMesh) -> PhysicsWorld {
        self.nets.push(net);
        self
    }

    /// Adds one car-shaped body to the scene, with a neutral
    /// (`ControllerInput::default()`) input and a full boost tank
    /// (`drive::MAX_BOOST`) — set a real input afterward with
    /// `set_car_input` if the car should actually drive. `car`'s current
    /// `friction` is snapshotted as its base friction, so handbrake input
    /// (which temporarily lowers `RigidBody.friction`) has a value to
    /// restore to once released; its jump-held state starts `false`, so an
    /// already-`jump: true` initial input still counts as a fresh press; its
    /// double jump starts available (`true`), matching a car that's
    /// effectively "just landed" before its first step; its jump-hold
    /// window starts at `0.0` (no ground jump in flight yet); its
    /// cancelable-flip flag starts `false` (no dodge in flight yet).
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
        self.car_double_jump_available.push(true);
        self.car_jump_hold_time_remaining.push(0.0);
        self.car_dodge_flip_active.push(false);
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
    /// upward velocity change on a fresh press, tracked via `jump_held`;
    /// double jump firing the same kind of impulse on a fresh airborne
    /// press, gated on and consuming `double_jump_available`, restored on
    /// landing; wall jump firing an outward-plus-upward impulse instead,
    /// gated on `wall_normal` — also computed up front from the car's
    /// position at the start of this step, like `on_ground`; the ground
    /// jump's variable height, driven by `jump_hold_time_remaining`; a
    /// dodge's spin flip-canceled by a further press, driven by
    /// `dodge_flip_active`) alongside gravity, so `input`'s forces/impulses
    /// (and friction adjustment) are part of the same velocity-prediction
    /// phase. Since `RB-PHYSICS-001-FR-057`, also calls
    /// `drive::clamp_angular_speed` right after `integrate_velocities`, so
    /// this step's angular velocity — air control torque and any direct
    /// writes (a dodge's kick, the landing-orientation assist) alike —
    /// never leaves this function above `drive::MAX_CAR_ANGULAR_SPEED`.
    #[allow(clippy::too_many_arguments)]
    fn drive_and_integrate_velocities(
        car: &mut RigidBody,
        input: &ControllerInput,
        on_ground: bool,
        wall_normal: Option<Vec3>,
        boost_amount: &mut f32,
        jump_held: &mut bool,
        double_jump_available: &mut bool,
        jump_hold_time_remaining: &mut f32,
        dodge_flip_active: &mut bool,
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
            wall_normal,
            boost_amount,
            jump_held,
            double_jump_available,
            jump_hold_time_remaining,
            dodge_flip_active,
            base_friction,
            dt,
        );
        integrate::apply_damping(car, dt);
        integrate::integrate_velocities(car, dt);
        drive::clamp_angular_speed(car);
    }

    /// Detects every one of `body`'s contacts against every static surface
    /// in the scene — the ground, every wall, every curve
    /// (`RB-PHYSICS-001-FR-020`), every compound-corner fillet
    /// (`RB-PHYSICS-001-FR-023`), every goal wall (`RB-PHYSICS-001-FR-024`),
    /// and every bounded wall (`RB-PHYSICS-001-FR-029`) — returning every
    /// `(restitution, friction, contacts)` group found, rather than
    /// resolving them itself. `step` folds every body's own groups together
    /// with every ball-vs-car/car-vs-car manifold into one combined call to
    /// `solver::resolve_manifolds` (`RB-PHYSICS-001-FR-052`), instead of
    /// this function resolving a body's static contacts on its own via
    /// `solver::resolve_static_manifolds` (`RB-PHYSICS-001-FR-051`) before
    /// `step`'s own separate dynamic-manifold solve ever reads that body's
    /// updated velocity.
    ///
    /// Before `RB-PHYSICS-001-FR-051`, `step` called one dedicated
    /// `resolve_*_contact` helper per static shape type in sequence (ground,
    /// then every wall, then every curve, and so on) — each one fully
    /// resolving and applying its own `SOLVER_ITERATIONS` pass before the
    /// next shape's setup even read `body`'s updated velocity. FR-051 fixed
    /// that for a body touching two different *static* surfaces at once (a
    /// car driving along a wall near the floor, or wedged into a corner)
    /// via `solver::resolve_static_manifolds`'s own combined solve — but
    /// left the exact same gap one level up: a body's now-combined static
    /// resolve was still its own separate call, fully resolved and applied
    /// before `resolve_dynamic_manifolds`'s own setup for that same body
    /// (touching another car, say) ever read the result. `RB-PHYSICS-001-FR-052`
    /// closes that remaining gap by folding this function's own gathered
    /// groups into `solver::resolve_manifolds`'s shared solve instead — see
    /// that function's own doc comment for the exact mechanism and its
    /// dedicated test. `scene` bundles every static-shape slice into one
    /// borrow (clippy's `too_many_arguments` threshold is the only reason
    /// this isn't just six separate parameters — every caller still borrows
    /// the same six `PhysicsWorld` fields directly, same as before
    /// `RB-PHYSICS-001-FR-051`).
    fn static_contact_manifolds(
        body: &RigidBody,
        scene: &StaticScene,
    ) -> Vec<(f32, f32, Vec<collision::Contact>)> {
        let mut manifolds: Vec<(f32, f32, Vec<collision::Contact>)> = Vec::new();

        let ground_contacts = collision::contacts_vs_plane(body, scene.ground);
        if !ground_contacts.is_empty() {
            manifolds.push((
                scene.ground.restitution,
                scene.ground.friction,
                ground_contacts,
            ));
        }
        for wall in scene.walls {
            let contacts = collision::contacts_vs_plane(body, wall);
            if !contacts.is_empty() {
                manifolds.push((wall.restitution, wall.friction, contacts));
            }
        }
        for curve in scene.curves {
            let contacts = collision::contacts_vs_quarter_pipe(body, curve);
            if !contacts.is_empty() {
                manifolds.push((curve.restitution, curve.friction, contacts));
            }
        }
        for corner_fillet in scene.corner_fillets {
            let contacts = collision::contacts_vs_corner_fillet(body, corner_fillet);
            if !contacts.is_empty() {
                manifolds.push((corner_fillet.restitution, corner_fillet.friction, contacts));
            }
        }
        for goal_wall in scene.goal_walls {
            let contacts = collision::contacts_vs_goal_wall(body, goal_wall);
            if !contacts.is_empty() {
                manifolds.push((
                    goal_wall.plane.restitution,
                    goal_wall.plane.friction,
                    contacts,
                ));
            }
        }
        for bounded_wall in scene.bounded_walls {
            let contacts = collision::contacts_vs_bounded_wall(body, bounded_wall);
            if !contacts.is_empty() {
                manifolds.push((
                    bounded_wall.plane.restitution,
                    bounded_wall.plane.friction,
                    contacts,
                ));
            }
        }

        manifolds
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

    /// Advances the whole scene by `dt` seconds, matching
    /// `btDiscreteDynamicsWorld::stepSimulation`'s staged pipeline: predict
    /// every body's unconstrained velocity (for cars, including
    /// `drive::apply_driven_forces` from that car's current input), then
    /// detect and resolve every contact — every body's own static-surface
    /// contacts (`static_contact_manifolds`) and every ball-vs-car/car-vs-car
    /// manifold, all resolved together in one combined solve
    /// (`solver::resolve_manifolds`, since `RB-PHYSICS-001-FR-052`) — then
    /// integrate every body's transform, never resolving one body's
    /// transform before another body's contacts have had a chance to affect
    /// it.
    ///
    /// Before `RB-PHYSICS-001-FR-030`, car-vs-car and ball-vs-car pairs
    /// were each resolved with their own independent call to
    /// `solver::resolve_contacts_between`, fully converged and applied
    /// before the next pair's setup even read a body's velocity — an
    /// approximation once 3+ bodies were mutually touching in the same
    /// step (e.g. a car pinned between the ball and another car), since
    /// the shared body in two pairs never had both contacts reasoned about
    /// together. `solver::resolve_dynamic_manifolds` fixed that by sharing
    /// one `DeltaVelocity` accumulator per body index across every dynamic
    /// manifold that body takes part in. `RB-PHYSICS-001-FR-051` then made
    /// the same fix one level down, for a body's own multiple static-shape
    /// contacts (`solver::resolve_static_manifolds`). But each of those two
    /// combined solves was still its own separate call — a body's static
    /// contacts fully resolved and applied before the dynamic solve's own
    /// setup for that same body (touching another car, say) ever read the
    /// result — the identical order-dependent gap one level up.
    /// `RB-PHYSICS-001-FR-052` closes that: `solver::resolve_manifolds`
    /// folds a step's static and dynamic manifolds into one shared solve,
    /// still simpler than Bullet's actual interleaved-across-islands solver
    /// architecture (no persistent islands), but a genuine combined solve
    /// for everything touching a body this step, not a sequence of
    /// independent ones. Since `RB-PHYSICS-001-FR-035`,
    /// `dynamic_manifold_caches` also carries each dynamic manifold's
    /// converged impulses across steps, so `solver::resolve_manifolds`
    /// warm-starts that channel from last step's answer instead of zero
    /// (a body's static contacts still cold-start every call) — see that
    /// function's and `solver::ContactCache`'s own doc comments. Since
    /// `RB-PHYSICS-001-FR-037`, the ball and every car have their sleep
    /// state (`body::RigidBody::update_sleep_state`) re-evaluated once
    /// every contact above (including the net panels) is resolved but
    /// before the transform integrates, so a body newly asleep this step
    /// freezes in place this same step. Since `RB-PHYSICS-001-FR-061`, the
    /// ball's own linear and angular speed are hard-capped
    /// (`clamp_ball_velocity`) at that same point — after contact
    /// resolution, before sleep evaluation and transform integration —
    /// matching real RocketSim's own placement for this same clamp.
    pub fn step(&mut self, dt: f32) {
        // Ground contact for driving purposes is checked up front, against
        // each car's position at the start of this step (before gravity or
        // driven forces move anything) — `static_contact_manifolds` below
        // re-derives the same contacts for the actual solve; the small
        // duplicated `contacts_vs_plane` call is simpler than threading
        // the manifold through, and cheap (a handful of corner checks).
        let car_on_ground: Vec<bool> = self
            .cars
            .iter()
            .map(|car| !collision::contacts_vs_plane(car, &self.ground).is_empty())
            .collect();
        // Same idea as car_on_ground, but for walls: the outward push-off
        // direction for a wall jump. Since `RB-PHYSICS-001-FR-039`, a car
        // touching two walls at once (a corner — reachable at a diagonal
        // corner wall's own two seams, where it meets a side or back wall)
        // sums every touched wall's normal and normalizes the result,
        // instead of the old "whichever wall comes first in `self.walls`"
        // simplification (RB-PHYSICS-001-FR-013's original Non-goal) — so a
        // corner wall jump pushes diagonally away from the corner, blending
        // both walls, rather than firing along only one of them depending
        // on iteration order. A car touching exactly one wall gets that
        // wall's own normal back unchanged (summing a single unit vector
        // and normalizing is a no-op), so the common single-wall case is
        // unaffected. The only case `normalize` can fail is two touched
        // walls with exactly opposite normals (summing to zero) —
        // geometrically impossible for a convex arena interior, but falls
        // back to the first touched wall's normal rather than panicking if
        // it ever happened.
        let car_wall_normal: Vec<Option<Vec3>> = self
            .cars
            .iter()
            .map(|car| {
                let touched_normals: Vec<Vec3> = self
                    .walls
                    .iter()
                    .filter(|wall| !collision::contacts_vs_plane(car, wall).is_empty())
                    .map(|wall| wall.normal)
                    .collect();
                let mut summed_normal = Vec3::ZERO;
                for normal in &touched_normals {
                    summed_normal += *normal;
                }
                summed_normal
                    .normalize()
                    .or_else(|| touched_normals.first().copied())
            })
            .collect();

        Self::apply_forces_and_integrate_velocities(&mut self.ball, self.gravity, dt);
        for (
            (
                (
                    ((((((car, input), on_ground), wall_normal), boost), base_friction), jump_held),
                    double_jump_available,
                ),
                jump_hold_time_remaining,
            ),
            dodge_flip_active,
        ) in self
            .cars
            .iter_mut()
            .zip(self.car_inputs.iter())
            .zip(car_on_ground.iter())
            .zip(car_wall_normal.iter())
            .zip(self.car_boost.iter_mut())
            .zip(self.car_base_friction.iter())
            .zip(self.car_jump_held.iter_mut())
            .zip(self.car_double_jump_available.iter_mut())
            .zip(self.car_jump_hold_time_remaining.iter_mut())
            .zip(self.car_dodge_flip_active.iter_mut())
        {
            Self::drive_and_integrate_velocities(
                car,
                input,
                *on_ground,
                *wall_normal,
                boost,
                jump_held,
                double_jump_available,
                jump_hold_time_remaining,
                dodge_flip_active,
                *base_friction,
                self.gravity,
                dt,
            );
        }

        let static_scene = StaticScene {
            ground: &self.ground,
            walls: &self.walls,
            curves: &self.curves,
            corner_fillets: &self.corner_fillets,
            goal_walls: &self.goal_walls,
            bounded_walls: &self.bounded_walls,
        };
        // Combined static-and-dynamic solve (RB-PHYSICS-001-FR-052): every
        // body's own static-shape contacts (`static_manifolds`) and every
        // ball-vs-car/car-vs-car manifold (`dynamic_manifolds`) are gathered
        // first and resolved together in one `solver::resolve_manifolds`
        // call, sharing one iteration budget per body — instead of this
        // body's static contacts being fully resolved and applied by their
        // own separate call before the dynamic solve's own setup for that
        // same body ever read the result (see that function's own doc
        // comment). Index 0 is the ball, index `i + 1` is `self.cars[i]`.
        let mut bodies: Vec<RigidBody> = Vec::with_capacity(1 + self.cars.len());
        bodies.push(self.ball);
        bodies.extend(self.cars.iter().copied());

        let mut static_manifolds: Vec<(usize, f32, f32, Vec<collision::Contact>)> = Vec::new();
        for (body_index, body) in bodies.iter().enumerate() {
            for (restitution, friction, contacts) in
                Self::static_contact_manifolds(body, &static_scene)
            {
                static_manifolds.push((body_index, restitution, friction, contacts));
            }
        }

        let mut dynamic_manifolds: Vec<(usize, usize, Vec<collision::Contact>)> = Vec::new();
        for (car_index, car) in self.cars.iter().enumerate() {
            let contacts = collision::contacts_between(&self.ball, car);
            if !contacts.is_empty() {
                dynamic_manifolds.push((0, car_index + 1, contacts));
            }
        }
        for i in 0..self.cars.len() {
            for j in (i + 1)..self.cars.len() {
                let contacts = collision::contacts_between(&self.cars[i], &self.cars[j]);
                if !contacts.is_empty() {
                    dynamic_manifolds.push((i + 1, j + 1, contacts));
                }
            }
        }

        solver::resolve_manifolds(
            &mut bodies,
            &static_manifolds,
            &dynamic_manifolds,
            dt,
            &mut self.dynamic_manifold_caches,
        );

        // Net panels (RB-PHYSICS-001-FR-033, and since RB-PHYSICS-001-FR-038,
        // a car too): each net's own internal physics (spring forces, its own
        // sub-stepped integration) plus every body's contact against it,
        // resolved after every other contact this step so each body's
        // velocity going in already reflects gravity, driven forces, and
        // every static/dynamic contact above — see `nets`' own doc comment
        // for why this isn't part of `resolve_manifolds`' shared solve.
        // Reuses the same `bodies` snapshot `resolve_manifolds` just
        // resolved (index 0 the ball, index `i + 1` `self.cars[i]`,
        // exactly as above) rather than syncing back to `self.ball`/`self.cars`
        // and rebuilding a fresh one, deferring that sync until every net has
        // had its turn.
        for net in &mut self.nets {
            net.step(&mut bodies, self.gravity, dt);
        }

        self.ball = bodies[0];
        for (car, resolved) in self.cars.iter_mut().zip(bodies.iter().skip(1)) {
            *car = *resolved;
        }

        // Hard ball speed/angular-speed caps (RB-PHYSICS-001-FR-061):
        // applied right after this step's contact resolution (including
        // any net, just above), matching real RocketSim's own placement —
        // see `clamp_ball_velocity`'s own doc comment.
        clamp_ball_velocity(&mut self.ball);

        // Sleeping (RB-PHYSICS-001-FR-037): evaluated once every other
        // contact this step has already been resolved (including the net
        // panels just above) but before the transform integrates, so a
        // body that goes to sleep this step also freezes in place this
        // same step instead of drifting one more frame first — see
        // `body::RigidBody::update_sleep_state`'s own doc comment.
        self.ball.update_sleep_state(dt);
        for car in &mut self.cars {
            car.update_sleep_state(dt);
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

/// `simulate`'s own doc comment named this exact next step: "once
/// `RB-VERIFY-002` capture data exists, this signature grows an `inputs`
/// parameter rather than staying input-free." That data now exists
/// (`RB-PHYSICS-001-FR-076`) — this is that grown signature, as a sibling
/// function rather than a breaking change to `simulate` itself (every
/// existing input-free call site — this crate's own tests included — has
/// no recorded input to supply and shouldn't need to fabricate one).
///
/// Drives `world` (typically freshly built by `PhysicsWorld::from_frame`
/// applied to `recorded[0]`) forward using `recorded`'s own per-tick
/// controller input and timestamp spacing: for each consecutive pair of
/// recorded frames, every car's `input` from the *earlier* frame (the
/// input that was actually held going into that tick, matching how
/// `RB-VERIFY-002-FR-001`'s plugin captured it) is applied via
/// `set_car_input`, then `world` steps forward by that pair's own
/// `timestamp_secs` delta — deliberately not a fixed/hardcoded rate,
/// since no confirmed real Rocket League physics-tick-rate constant
/// exists anywhere in this crate (only the empirical ~120Hz implied by a
/// real capture's own line count over its own duration, never a sourced
/// citation) and driving by the recording's own actual spacing sidesteps
/// needing that number at all. A car whose `player_id` doesn't index into
/// `world.cars` (more recorded cars than `world` was seeded with) is
/// silently left undriven that tick rather than panicking — real
/// mid-capture car joins/leaves are unexercised territory this plumbing
/// doesn't need to solve yet (see `RB-PHYSICS-001-FR-077`'s own
/// Non-goals).
///
/// Returns one `PhysicsFrame` per element of `recorded` (the first is
/// `world`'s own starting `frame()`, before any step) — this is the
/// candidate trajectory `rb_domain::divergence::score` compares against
/// `recorded` itself for `RB-PHYSICS-001-FR-077`'s real fidelity number.
pub fn simulate_recorded(mut world: PhysicsWorld, recorded: &[PhysicsFrame]) -> Vec<PhysicsFrame> {
    let mut frames = Vec::with_capacity(recorded.len());
    frames.push(world.frame());
    for pair in recorded.windows(2) {
        let (prev, next) = (&pair[0], &pair[1]);
        for car_state in &prev.cars {
            let Some(input) = car_state.input else {
                continue;
            };
            let index = car_state.player_id as usize;
            if index < world.cars.len() {
                world.set_car_input(index, input);
            }
        }
        let dt = next.timestamp_secs - prev.timestamp_secs;
        world.step(dt);
        frames.push(world.frame());
    }
    frames
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use rb_domain::Quat;

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
        // an average of the two — see solver.rs): before sleeping
        // (`RB-PHYSICS-001-FR-037`) existed, a *bouncy* resting contact
        // legitimately never settled under a naive per-frame sequential
        // impulse solve — each frame's gravity-induced velocity was a fresh
        // "impact" that restitution bounced back up, forever (that's now
        // fixed — see `a_bouncy_resting_ball_actually_settles_once_asleep`).
        // This test only ever needed the inelastic case to settle, which it
        // should regardless of sleeping.
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
    fn clamp_ball_velocity_is_a_no_op_below_both_caps() {
        let mut ball = RigidBody::sphere(1.0, 1.0, Vec3::ZERO);
        ball.linear_velocity = Vec3::new(100.0, 200.0, 0.0);
        ball.angular_velocity = Vec3::new(1.0, 2.0, 0.0);
        clamp_ball_velocity(&mut ball);
        assert_eq!(
            ball.linear_velocity,
            Vec3::new(100.0, 200.0, 0.0),
            "expected an already-under-cap linear velocity to pass through unchanged, got {:?}",
            ball.linear_velocity
        );
        assert_eq!(
            ball.angular_velocity,
            Vec3::new(1.0, 2.0, 0.0),
            "expected an already-under-cap angular velocity to pass through unchanged, got {:?}",
            ball.angular_velocity
        );
    }

    #[test]
    fn clamp_ball_velocity_scales_an_over_cap_linear_velocity_down_to_the_cap_preserving_direction()
    {
        let mut ball = RigidBody::sphere(1.0, 1.0, Vec3::ZERO);
        ball.linear_velocity = Vec3::new(20_000.0, 0.0, 0.0);
        clamp_ball_velocity(&mut ball);
        assert!(
            (ball.linear_velocity.length() - BALL_MAX_SPEED).abs() < 1e-3,
            "expected the clamp to scale magnitude down to exactly BALL_MAX_SPEED, got {:?}",
            ball.linear_velocity
        );
        assert!(
            ball.linear_velocity.y == 0.0 && ball.linear_velocity.z == 0.0,
            "expected the clamp to preserve direction, got {:?}",
            ball.linear_velocity
        );
    }

    #[test]
    fn clamp_ball_velocity_scales_an_over_cap_angular_velocity_down_to_the_cap_preserving_direction(
    ) {
        let mut ball = RigidBody::sphere(1.0, 1.0, Vec3::ZERO);
        ball.angular_velocity = Vec3::new(0.0, 0.0, 50.0);
        clamp_ball_velocity(&mut ball);
        assert!(
            (ball.angular_velocity.length() - BALL_MAX_ANG_SPEED).abs() < 1e-4,
            "expected the clamp to scale magnitude down to exactly BALL_MAX_ANG_SPEED, got {:?}",
            ball.angular_velocity
        );
        assert!(
            ball.angular_velocity.x == 0.0 && ball.angular_velocity.y == 0.0,
            "expected the clamp to preserve direction, got {:?}",
            ball.angular_velocity
        );
    }

    #[test]
    fn a_ball_launched_far_past_ball_max_speed_never_exceeds_it_after_a_step() {
        let mut ball = RigidBody::sphere(1.0, 1.0, Vec3::new(0.0, 0.0, 1000.0));
        ball.linear_velocity = Vec3::new(50_000.0, 0.0, 0.0);
        let mut world = PhysicsWorld::new(ball, flat_ground());
        world.step(1.0 / 60.0);
        assert!(
            world.ball.linear_velocity.length() <= BALL_MAX_SPEED + 1e-3,
            "expected the ball's speed to never exceed BALL_MAX_SPEED after a step, got {}",
            world.ball.linear_velocity.length()
        );
    }

    #[test]
    fn a_bouncy_resting_ball_actually_settles_once_asleep() {
        // RB-PHYSICS-001-FR-037: this is the actual "bouncy resting contact
        // never settles" limitation `resting_ball_stays_at_rest`'s own
        // comment (and this module's/solver's own doc comments) describe —
        // demonstrated directly here with a nonzero-restitution ball/ground
        // pair, instead of only being documented as a known gap.
        let mut ball = RigidBody::sphere(1.0, 1.0, Vec3::new(0.0, 0.0, 1.0));
        ball.restitution = 0.5;
        let ground = StaticPlane {
            restitution: 0.5,
            ..flat_ground()
        };
        let mut world = PhysicsWorld::new(ball, ground);
        let dt = 1.0 / 60.0;
        // SLEEP_TIME_THRESHOLD (0.5s) plus a generous margin for the
        // per-frame gravity/restitution bounce to actually decay under the
        // sleep velocity thresholds before the timer can start counting.
        for _ in 0..300 {
            world.step(dt);
        }
        assert!(
            world.ball.is_sleeping,
            "expected the ball to fall asleep once its bounce settled below threshold"
        );
        assert_eq!(
            world.ball.linear_velocity,
            Vec3::ZERO,
            "a sleeping body's velocity should be forced to exactly zero"
        );
        assert_eq!(world.ball.angular_velocity, Vec3::ZERO);
    }

    #[test]
    fn a_sleeping_car_wakes_up_the_instant_throttle_is_applied() {
        // RB-PHYSICS-001-FR-037: guards against the specific bug this
        // requirement's own design had to avoid — a velocity-only wake
        // check would zero right back out a driving force whose one-frame
        // delta is itself smaller than the sleep threshold, permanently
        // stranding an asleep car. `drive::apply_driven_forces` instead
        // wakes a car unconditionally on any genuinely active input, before
        // that input's own force has had a chance to move it.
        // Seeded already asleep, at rest exactly on the ground, rather than
        // simulated into that state — this test's own claim is about the
        // wake response to input, not about how long settling itself takes
        // (see `dropped_car_settles_flat_on_the_ground_without_tipping_over`
        // and the sleeping tests above for that).
        let mut car = RigidBody::car_box(Vec3::new(1.0, 1.0, 1.0), 1.0, Vec3::new(0.0, 0.0, 1.0));
        car.is_sleeping = true;
        let mut world = PhysicsWorld::new(
            RigidBody::sphere(1.0, 1.0, Vec3::new(10_000.0, 0.0, 1.0)),
            flat_ground(),
        )
        .with_car(car);
        let dt = 1.0 / 60.0;
        world.set_car_input(
            0,
            ControllerInput {
                throttle: 1.0,
                ..Default::default()
            },
        );
        world.step(dt);
        assert!(
            !world.cars[0].is_sleeping,
            "throttle should wake the car immediately"
        );
        assert!(
            world.cars[0].linear_velocity.length() > 0.0,
            "a woken car should actually accelerate under throttle this same step"
        );
    }

    #[test]
    fn a_sleeping_ball_wakes_up_when_a_moving_car_hits_it() {
        // RB-PHYSICS-001-FR-037: no special-case wake logic exists for a
        // contact-driven wake — the ball's own resultant velocity after
        // the collision naturally exceeds the sleep threshold, which
        // `update_sleep_state` reads the same as any other frame.
        let ball = RigidBody::sphere(50.0, 1.0, Vec3::new(0.0, 0.0, 50.0));
        let mut world = PhysicsWorld::new(ball, flat_ground());
        let dt = 1.0 / 60.0;
        // Let the ball (already resting) go to sleep before the car exists
        // at all, so the car's own approach can't be mistaken for having
        // contributed to it.
        for _ in 0..90 {
            world.step(dt);
        }
        assert!(
            world.ball.is_sleeping,
            "expected the ball to be asleep before the car arrives"
        );

        let mut car = RigidBody::car_box(
            Vec3::new(50.0, 50.0, 20.0),
            1.0,
            Vec3::new(-300.0, 0.0, 20.0),
        );
        car.linear_velocity = Vec3::new(2000.0, 0.0, 0.0);
        world = world.with_car(car);
        for _ in 0..60 {
            world.step(dt);
            if !world.ball.is_sleeping {
                break;
            }
        }
        assert!(
            !world.ball.is_sleeping,
            "a moving car's impact should wake the sleeping ball"
        );
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

    fn ball_state_at(position: Vec3) -> BallState {
        BallState {
            position,
            rotation: Quat::IDENTITY,
            velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
        }
    }

    #[test]
    fn simulate_recorded_returns_one_frame_per_recorded_frame() {
        let recorded = vec![
            PhysicsFrame {
                timestamp_secs: 10.0,
                ball: ball_state_at(Vec3::new(0.0, 0.0, 1000.0)),
                cars: vec![],
            },
            PhysicsFrame {
                timestamp_secs: 10.05,
                ball: ball_state_at(Vec3::new(0.0, 0.0, 900.0)),
                cars: vec![],
            },
            PhysicsFrame {
                timestamp_secs: 10.10,
                ball: ball_state_at(Vec3::new(0.0, 0.0, 800.0)),
                cars: vec![],
            },
        ];
        let world = PhysicsWorld::from_frame(&recorded[0]);
        let frames = simulate_recorded(world, &recorded);
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].timestamp_secs, 10.0);
        assert!((frames[2].timestamp_secs - 10.10).abs() < 1e-4);
    }

    #[test]
    fn simulate_recorded_derives_dt_from_each_pairs_own_timestamps_not_a_fixed_rate() {
        // Two consecutive pairs with different spacing (0.05s, then 0.10s),
        // high enough above the ground to stay in free fall throughout:
        // the ball should fall further during the longer second interval,
        // which only happens if dt is actually read per-pair rather than
        // reused from the first. Bypasses from_frame/standard_arena here
        // (a plain flat ground instead) so this is a pure kinematics check,
        // not entangled with any arena-geometry collision.
        let recorded = vec![
            PhysicsFrame {
                timestamp_secs: 0.0,
                ball: ball_state_at(Vec3::new(0.0, 0.0, 1000.0)),
                cars: vec![],
            },
            PhysicsFrame {
                timestamp_secs: 0.05,
                ball: ball_state_at(Vec3::new(0.0, 0.0, 1000.0)),
                cars: vec![],
            },
            PhysicsFrame {
                timestamp_secs: 0.15,
                ball: ball_state_at(Vec3::new(0.0, 0.0, 1000.0)),
                cars: vec![],
            },
        ];
        let world = PhysicsWorld::new(
            RigidBody::sphere(1.0, 1.0, Vec3::new(0.0, 0.0, 1000.0)),
            flat_ground(),
        );
        let frames = simulate_recorded(world, &recorded);
        let fall_1 = frames[0].ball.position.z - frames[1].ball.position.z;
        let fall_2 = frames[1].ball.position.z - frames[2].ball.position.z;
        assert!(
            fall_2 > fall_1 * 1.5,
            "expected the 0.10s second interval to fall further than the \
             0.05s first one, got fall_1={fall_1} fall_2={fall_2}"
        );
    }

    #[test]
    fn simulate_recorded_actually_applies_each_cars_own_recorded_input() {
        let driving_input = ControllerInput {
            throttle: 1.0,
            ..ControllerInput::default()
        };

        let recorded = vec![
            PhysicsFrame {
                timestamp_secs: 0.0,
                ball: ball_state_at(Vec3::new(1000.0, 0.0, 1000.0)),
                cars: vec![CarState {
                    player_id: 0,
                    position: Vec3::new(0.0, 0.0, 17.0),
                    rotation: Quat::IDENTITY,
                    velocity: Vec3::ZERO,
                    angular_velocity: Vec3::ZERO,
                    boost_amount: 0.0,
                    input: Some(driving_input),
                }],
            },
            PhysicsFrame {
                timestamp_secs: 1.0,
                ball: ball_state_at(Vec3::new(1000.0, 0.0, 1000.0)),
                cars: vec![CarState {
                    player_id: 0,
                    position: Vec3::new(0.0, 0.0, 17.0),
                    rotation: Quat::IDENTITY,
                    velocity: Vec3::ZERO,
                    angular_velocity: Vec3::ZERO,
                    boost_amount: 0.0,
                    input: Some(ControllerInput::default()),
                }],
            },
        ];
        let world = PhysicsWorld::from_frame(&recorded[0]);
        let frames = simulate_recorded(world, &recorded);
        assert_eq!(frames.len(), 2);
        assert!(
            frames[1].cars[0].velocity.x.abs() > 1.0,
            "expected recorded throttle input to actually drive the car, got velocity {:?}",
            frames[1].cars[0].velocity
        );
    }

    #[test]
    fn simulate_recorded_skips_a_recorded_car_the_world_was_not_seeded_with() {
        // A car present in the recorded frame but beyond what the seeded
        // world has (e.g. more cars than the seed frame carried) must not
        // panic -- see simulate_recorded's own doc comment.
        let recorded = vec![
            PhysicsFrame {
                timestamp_secs: 0.0,
                ball: ball_state_at(Vec3::ZERO),
                cars: vec![identity_car_state(0, Vec3::new(0.0, 0.0, 17.0), 0.0)],
            },
            PhysicsFrame {
                timestamp_secs: 0.05,
                ball: ball_state_at(Vec3::ZERO),
                cars: vec![
                    identity_car_state(0, Vec3::new(0.0, 0.0, 17.0), 0.0),
                    identity_car_state(1, Vec3::new(100.0, 0.0, 17.0), 0.0),
                ],
            },
        ];
        // Seed from only the first (one-car) frame on purpose.
        let world = PhysicsWorld::from_frame(&recorded[0]);
        let frames = simulate_recorded(world, &recorded);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[1].cars.len(), 1);
    }

    fn identity_car_state(player_id: u32, position: Vec3, boost_amount: f32) -> CarState {
        CarState {
            player_id,
            position,
            rotation: Quat::IDENTITY,
            velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            boost_amount,
            input: Some(ControllerInput::default()),
        }
    }

    #[test]
    fn from_frame_seeds_ball_state_directly_from_the_frame() {
        let frame = PhysicsFrame {
            timestamp_secs: 11.78,
            ball: BallState {
                position: Vec3::new(100.0, 200.0, 300.0),
                rotation: Quat::new(0.5, 0.5, 0.5, 0.5),
                velocity: Vec3::new(10.0, 20.0, 30.0),
                angular_velocity: Vec3::new(1.0, 2.0, 3.0),
            },
            cars: vec![],
        };
        let world = PhysicsWorld::from_frame(&frame);
        assert_eq!(world.ball.position, frame.ball.position);
        assert_eq!(world.ball.orientation, frame.ball.rotation);
        assert_eq!(world.ball.linear_velocity, frame.ball.velocity);
        assert_eq!(world.ball.angular_velocity, frame.ball.angular_velocity);
        assert!(world.cars.is_empty());
    }

    #[test]
    fn from_frame_seeds_the_worlds_own_clock_from_the_frames_own_timestamp() {
        // Critical for rb_domain::divergence::score's nearest-timestamp
        // alignment to actually match candidate frames up against a real
        // capture's own absolute clock -- see from_frame's own doc comment.
        let frame = PhysicsFrame {
            timestamp_secs: 11.78,
            ball: BallState {
                position: Vec3::ZERO,
                rotation: Quat::IDENTITY,
                velocity: Vec3::ZERO,
                angular_velocity: Vec3::ZERO,
            },
            cars: vec![],
        };
        let world = PhysicsWorld::from_frame(&frame);
        assert_eq!(world.frame().timestamp_secs, 11.78);
    }

    #[test]
    fn from_frame_seeds_every_car_in_order_with_its_own_recorded_state() {
        let frame = PhysicsFrame {
            timestamp_secs: 0.0,
            ball: BallState {
                position: Vec3::ZERO,
                rotation: Quat::IDENTITY,
                velocity: Vec3::ZERO,
                angular_velocity: Vec3::ZERO,
            },
            cars: vec![
                identity_car_state(0, Vec3::new(1.0, 0.0, 17.0), 33.0),
                identity_car_state(1, Vec3::new(-1.0, 0.0, 17.0), 100.0),
            ],
        };
        let world = PhysicsWorld::from_frame(&frame);
        assert_eq!(world.cars.len(), 2);
        assert_eq!(world.cars[0].position, Vec3::new(1.0, 0.0, 17.0));
        assert_eq!(world.cars[1].position, Vec3::new(-1.0, 0.0, 17.0));
        let seeded = world.frame();
        assert_eq!(seeded.cars[0].boost_amount, 33.0);
        assert_eq!(seeded.cars[1].boost_amount, 100.0);
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

        let ball_radius = 93.15;
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
    fn a_ball_pinched_between_two_closing_cars_is_resolved_by_a_shared_multi_body_solve() {
        // RB-PHYSICS-001-FR-030: two cars closing symmetrically on a ball
        // squeezed directly between them is the exact "3+ bodies mutually
        // touching in the same step" scenario `step`'s own doc comment
        // calls out. Before the combined solve existed, `step` resolved
        // ball-vs-left to its own full convergence and applied it, then
        // resolved ball-vs-right using the ball's already-updated
        // velocity — which, for this symmetric setup, left the ball at
        // ~99% of the closing speed of whichever car was resolved *last*,
        // as if the first car's contact had barely happened at all (see
        // `solver::tests::resolve_dynamic_manifolds_keeps_more_of_every_bodys_contact_than_resolving_pairs_independently`
        // for the isolated before/after numbers this test's threshold is
        // drawn from). All three bodies float clear of the ground with
        // gravity zeroed, isolating the three-body contact this test
        // checks.
        let car_half_extents = Vec3::new(60.0, 30.0, 18.0);
        let ball_radius = 93.15;
        let gap = car_half_extents.x + ball_radius;

        let mut ball = RigidBody::sphere(ball_radius, 1.0, Vec3::new(0.0, 0.0, 500.0));
        ball.restitution = 0.0;

        let mut left = RigidBody::car_box(car_half_extents, 180.0, Vec3::new(-gap, 0.0, 500.0));
        left.restitution = 0.0;
        left.linear_velocity = Vec3::new(100.0, 0.0, 0.0);

        let mut right = RigidBody::car_box(car_half_extents, 180.0, Vec3::new(gap, 0.0, 500.0));
        right.restitution = 0.0;
        right.linear_velocity = Vec3::new(-100.0, 0.0, 0.0);

        let mut world = PhysicsWorld::new(ball, flat_ground())
            .with_car(left)
            .with_car(right);
        world.gravity = Vec3::ZERO;

        world.step(1.0 / 60.0);

        // A perfectly symmetric pinch's true simultaneous-solve answer is
        // the ball (and both cars) ending near zero net velocity — total
        // momentum is exactly zero and every body is mutually constrained
        // to the others. This port's 10-iteration Gauss-Seidel solve
        // doesn't fully converge to that in one step for such an extreme
        // mass ratio (ball mass 1 vs. car mass 180) — a known, common
        // limitation of projected Gauss-Seidel contact solvers for
        // "sandwiched" configurations, not unique to this port — but it
        // must land meaningfully closer to it than resolving each pair to
        // full, independent convergence would (~99% of a single car's own
        // closing speed, i.e. > 98 units/s, matching the isolated
        // solver-level test).
        assert!(
            world.ball.linear_velocity.x.abs() < 95.0,
            "expected the combined solve to leave the pinched ball noticeably slower than a \
             single car's own closing speed, got vx={}",
            world.ball.linear_velocity.x
        );
        assert!(
            world.ball.position.x.abs() < 10.0,
            "expected the symmetrically pinched ball to stay near-centered rather than being \
             flung toward whichever car happened to be resolved last, got x={}",
            world.ball.position.x
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

        // Holding jump the whole time now also earns the ground jump's
        // variable-height bonus (extra upward accel for
        // drive::JUMP_HOLD_MAX_DURATION), so the car climbs higher and its
        // round trip takes noticeably longer than a bare JUMP_SPEED
        // impulse's ~2*JUMP_SPEED/650 ≈ 0.9s; run well past that with jump
        // still held the entire time.
        let dt = 1.0 / 120.0;
        for _ in 0..(3.0 / dt) as u32 {
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

    #[test]
    fn a_car_with_air_control_input_reorients_itself_midair() {
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(1000.0, 0.0, 93.0));
        let car = some_car(Vec3::new(0.0, 0.0, 1000.0));
        let mut world = PhysicsWorld::new(ball, flat_ground()).with_car(car);
        world.gravity = Vec3::ZERO; // isolate air control from falling
        world.set_car_input(
            0,
            rb_domain::ControllerInput {
                yaw: Some(1.0),
                ..Default::default()
            },
        );

        let dt = 1.0 / 60.0;
        for _ in 0..30 {
            world.step(dt);
        }

        let forward_after = world.cars[0].orientation.rotate(&Vec3::new(1.0, 0.0, 0.0));
        assert!(
            (forward_after - Vec3::new(1.0, 0.0, 0.0)).length() > 0.1,
            "expected air control yaw input to visibly reorient the car mid-air, forward={forward_after:?}"
        );
    }

    #[test]
    fn air_control_does_not_reorient_a_grounded_car() {
        // Regression guard: on the ground, steering already owns yaw —
        // air control must stay a no-op there, or a car resting with
        // stray pitch/yaw/roll input would spuriously spin in place.
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
                pitch: Some(1.0),
                yaw: Some(1.0),
                roll: Some(1.0),
                ..Default::default()
            },
        );

        let dt = 1.0 / 120.0;
        for _ in 0..(1.0 / dt) as u32 {
            world.step(dt);
        }

        let up_after = world.cars[0].orientation.rotate(&Vec3::new(0.0, 0.0, 1.0));
        assert!(
            (up_after - Vec3::new(0.0, 0.0, 1.0)).length() < 0.1,
            "expected a grounded car to stay level despite air control input, up={up_after:?}"
        );
    }

    #[test]
    fn double_jump_after_a_ground_jump_gives_a_second_upward_kick() {
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(1000.0, 0.0, 93.0));
        let mut car = some_car(Vec3::new(0.0, 0.0, 18.0));
        car.restitution = 0.0;
        let ground = StaticPlane {
            restitution: 0.0,
            ..flat_ground()
        };
        let mut world = PhysicsWorld::new(ball, ground).with_car(car);
        world.gravity = Vec3::ZERO; // isolate the jump impulses from falling back down
        let dt = 1.0 / 120.0;

        // Ground jump: a fresh press while grounded.
        world.set_car_input(
            0,
            rb_domain::ControllerInput {
                jump: true,
                ..Default::default()
            },
        );
        world.step(dt);
        let velocity_after_ground_jump = world.cars[0].linear_velocity.z;
        assert!(
            (velocity_after_ground_jump - crate::drive::JUMP_SPEED).abs() < 1.0,
            "expected the ground jump to give ~JUMP_SPEED upward velocity, got {velocity_after_ground_jump}"
        );

        // Release, then let the car actually leave the ground before
        // pressing jump again — a double jump only fires while airborne.
        world.set_car_input(0, rb_domain::ControllerInput::default());
        for _ in 0..12 {
            world.step(dt);
        }
        assert!(
            world.cars[0].position.z > 18.0 + 1.0,
            "expected the car to have left the ground before the double jump, got z={}",
            world.cars[0].position.z
        );

        world.set_car_input(
            0,
            rb_domain::ControllerInput {
                jump: true,
                ..Default::default()
            },
        );
        world.step(dt);
        let velocity_after_double_jump = world.cars[0].linear_velocity.z;
        assert!(
            velocity_after_double_jump > velocity_after_ground_jump + crate::drive::JUMP_SPEED - 1.0,
            "expected the double jump to add a second JUMP_SPEED kick on top of the ground jump, \
             velocity after ground jump={velocity_after_ground_jump}, after double jump={velocity_after_double_jump}"
        );
    }

    #[test]
    fn double_jump_is_not_available_again_mid_air_after_being_used() {
        // Regression guard: once the double jump is spent, releasing and
        // re-pressing jump again while still airborne must not fire a third
        // impulse — it should only become available again after landing.
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(1000.0, 0.0, 93.0));
        let mut car = some_car(Vec3::new(0.0, 0.0, 18.0));
        car.restitution = 0.0;
        let ground = StaticPlane {
            restitution: 0.0,
            ..flat_ground()
        };
        let mut world = PhysicsWorld::new(ball, ground).with_car(car);
        world.gravity = Vec3::ZERO; // isolate the jump impulses from falling back down
        let dt = 1.0 / 120.0;

        world.set_car_input(
            0,
            rb_domain::ControllerInput {
                jump: true,
                ..Default::default()
            },
        );
        world.step(dt);
        world.set_car_input(0, rb_domain::ControllerInput::default());
        for _ in 0..12 {
            world.step(dt);
        }

        // First airborne press: the double jump fires.
        world.set_car_input(
            0,
            rb_domain::ControllerInput {
                jump: true,
                ..Default::default()
            },
        );
        world.step(dt);
        let velocity_after_double_jump = world.cars[0].linear_velocity.z;

        // Release, then press again mid-air — should have no further effect.
        world.set_car_input(0, rb_domain::ControllerInput::default());
        world.step(dt);
        world.set_car_input(
            0,
            rb_domain::ControllerInput {
                jump: true,
                ..Default::default()
            },
        );
        world.step(dt);

        assert!(
            (world.cars[0].linear_velocity.z - velocity_after_double_jump).abs() < 1.0,
            "expected a second airborne jump press to have no effect once the double jump is \
             spent, velocity after double jump={velocity_after_double_jump}, after third press={}",
            world.cars[0].linear_velocity.z
        );
    }

    #[test]
    fn a_car_touching_a_wall_wall_jumps_outward_and_upward() {
        // Wall at x=100, normal (1,0,0): the same convention flat_ground()
        // uses (normal points away from the solid side, into where dynamic
        // bodies live) — free space is x>100, solid x<100.
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(-1000.0, 0.0, 1000.0));
        let wall = StaticPlane::new(Vec3::new(1.0, 0.0, 0.0), 100.0);
        // 60-unit half-extent touching the wall with zero gap: car center
        // at x=160 puts its -x face exactly on the wall's x=100 plane.
        let car = some_car(Vec3::new(160.0, 0.0, 1000.0));
        let mut world = PhysicsWorld::new(ball, flat_ground())
            .with_car(car)
            .with_wall(wall);
        world.gravity = Vec3::ZERO; // isolate the wall jump from falling

        world.set_car_input(
            0,
            rb_domain::ControllerInput {
                jump: true,
                ..Default::default()
            },
        );
        world.step(1.0 / 60.0);

        assert!(
            world.cars[0].linear_velocity.x > 0.0,
            "expected the wall jump to push the car away from the wall (positive x), got {:?}",
            world.cars[0].linear_velocity
        );
        assert!(
            (world.cars[0].linear_velocity.z - crate::drive::JUMP_SPEED).abs() < 1.0,
            "expected roughly JUMP_SPEED upward velocity from the wall jump, got {}",
            world.cars[0].linear_velocity.z
        );
    }

    #[test]
    fn a_car_touching_two_walls_at_a_corner_wall_jumps_diagonally_outward() {
        // RB-PHYSICS-001-FR-039: a car wedged into a corner (touching both
        // walls at once) should push off diagonally, blending both walls'
        // normals, not fire along only one of them depending on which wall
        // happens to come first in `self.walls`. Two perpendicular walls,
        // normals (1,0,0) and (0,1,0): the old "first wall wins" picker
        // would give a wall jump with zero y-velocity (or zero x-velocity,
        // depending on push order); the fix should give roughly equal,
        // both-positive x and y components instead.
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(-1000.0, 0.0, 1000.0));
        let wall_x = StaticPlane::new(Vec3::new(1.0, 0.0, 0.0), 100.0);
        let wall_y = StaticPlane::new(Vec3::new(0.0, 1.0, 0.0), 100.0);
        // Same zero-gap-contact convention as the single-wall test above:
        // 60-unit x half-extent and 30-unit y half-extent, so a car
        // centered at (160, 130, ...) touches both walls exactly.
        let car = some_car(Vec3::new(160.0, 130.0, 1000.0));
        let mut world = PhysicsWorld::new(ball, flat_ground())
            .with_car(car)
            .with_wall(wall_x)
            .with_wall(wall_y);
        world.gravity = Vec3::ZERO; // isolate the wall jump from falling

        world.set_car_input(
            0,
            rb_domain::ControllerInput {
                jump: true,
                ..Default::default()
            },
        );
        world.step(1.0 / 60.0);

        let vx = world.cars[0].linear_velocity.x;
        let vy = world.cars[0].linear_velocity.y;
        assert!(
            vx > 0.0 && vy > 0.0,
            "expected the corner wall jump to push the car away from both walls \
             (positive x and y), got {:?}",
            world.cars[0].linear_velocity
        );
        assert!(
            (vx - vy).abs() < 1.0,
            "expected a symmetric corner (equal-normal walls) to push off with roughly \
             equal x and y components, got vx={vx}, vy={vy}"
        );
        assert!(
            (world.cars[0].linear_velocity.z - crate::drive::JUMP_SPEED).abs() < 1.0,
            "expected roughly JUMP_SPEED upward velocity from the wall jump, got {}",
            world.cars[0].linear_velocity.z
        );
    }

    #[test]
    fn a_ball_wedged_into_a_two_wall_corner_settles_symmetrically_instead_of_favoring_one_wall() {
        // RB-PHYSICS-001-FR-051: the real proof at `PhysicsWorld::step`'s own
        // public level. A ball moving diagonally into a perfectly symmetric
        // two-wall corner (equal restitution/friction, perpendicular
        // normals) has no physical reason to favor either wall — the true
        // answer's x and y velocity components should come out equal.
        // Before this requirement, `step`'s own per-static-shape sequential
        // contact loop (ground, then each wall in `self.walls`' own
        // iteration order) instead left the ball measurably biased toward
        // whichever wall was resolved last, an arbitrary artifact with no
        // physical basis; this test was confirmed to fail under that old
        // sequential loop before `step` was changed to use
        // `solver::resolve_static_manifolds` instead (folded, since
        // `RB-PHYSICS-001-FR-052`, into `solver::resolve_manifolds`'s own
        // wider combined solve — see that function's own doc comment).
        let wall_x = StaticPlane::new(Vec3::new(1.0, 0.0, 0.0), 0.0);
        let wall_y = StaticPlane::new(Vec3::new(0.0, 1.0, 0.0), 0.0);
        let ball_radius = 93.15;
        let mut ball = RigidBody::sphere(
            ball_radius,
            1.0,
            Vec3::new(ball_radius, ball_radius, 1000.0),
        );
        ball.linear_velocity = Vec3::new(-100.0, -100.0, 0.0);
        let mut world = PhysicsWorld::new(ball, flat_ground())
            .with_wall(wall_x)
            .with_wall(wall_y);
        world.gravity = Vec3::ZERO; // isolate the corner impact from falling

        world.step(1.0 / 60.0);

        let vx = world.ball.linear_velocity.x;
        let vy = world.ball.linear_velocity.y;
        assert!(
            (vx - vy).abs() < 5.0,
            "expected a squarely-symmetric two-wall corner impact to leave the ball's x/y \
             velocity components nearly equal, got vx={vx}, vy={vy}"
        );
    }

    #[test]
    fn a_ball_wedged_between_a_wall_and_a_heavy_car_settles_symmetrically_instead_of_favoring_one()
    {
        // RB-PHYSICS-001-FR-052: the real proof at `PhysicsWorld::step`'s
        // own public level, one level up from
        // `a_ball_wedged_into_a_two_wall_corner_settles_symmetrically_instead_of_favoring_one_wall`
        // above. Same symmetric corner setup, except `wall_y` is replaced by
        // a real car in the scene — a very-heavy box (`mass = 1e9`)
        // positioned so its own face is exactly where `wall_y`'s own plane
        // would be, making it a real ball-vs-car dynamic-manifold contact
        // instead of a static one, but as immovable as a real wall for all
        // practical purposes. Before this requirement, `step` resolved the
        // ball's static wall contact fully via `resolve_static_contacts`
        // before ever building the `bodies` array `resolve_dynamic_manifolds`
        // used for the ball-vs-car contact, the same order-dependent gap
        // `RB-PHYSICS-001-FR-030`/`FR-051` already found and fixed
        // elsewhere; this test was confirmed to fail under that old
        // two-call sequence (measurably biased, same as the two-static-wall
        // test's own pre-fix failure) before `step` was changed to route
        // both channels through one `solver::resolve_manifolds` call.
        let wall_x = StaticPlane::new(Vec3::new(1.0, 0.0, 0.0), 0.0);
        let ball_radius = 93.15;
        let mut ball = RigidBody::sphere(
            ball_radius,
            1.0,
            Vec3::new(ball_radius, ball_radius, 1000.0),
        );
        ball.linear_velocity = Vec3::new(-100.0, -100.0, 0.0);
        let heavy_car = RigidBody::car_box(
            Vec3::new(1000.0, 1000.0, 1000.0),
            1.0e9,
            Vec3::new(ball_radius, -1000.0, 1000.0),
        );
        let mut world = PhysicsWorld::new(ball, flat_ground())
            .with_wall(wall_x)
            .with_car(heavy_car);
        world.gravity = Vec3::ZERO; // isolate the corner impact from falling

        world.step(1.0 / 60.0);

        let vx = world.ball.linear_velocity.x;
        let vy = world.ball.linear_velocity.y;
        assert!(
            (vx - vy).abs() < 5.0,
            "expected a squarely-symmetric wall-and-heavy-car corner impact to leave the \
             ball's x/y velocity components nearly equal, got vx={vx}, vy={vy}"
        );
    }

    #[test]
    fn a_ball_bounces_off_a_wall_instead_of_passing_through() {
        // The real end-to-end proof that arena walls are actual physical
        // geometry, not just an input-detection hack: a ball shot at a
        // wall should bounce off it the same way it already does off a
        // car (`ball_bounces_off_a_stationary_car_instead_of_passing_through`),
        // via the same generic `static_contact_manifolds` machinery the
        // ground already uses.
        let wall_x = 100.0;
        let wall = StaticPlane {
            restitution: 0.5,
            ..StaticPlane::new(Vec3::new(1.0, 0.0, 0.0), wall_x)
        };
        let ball_radius = 93.15;
        let mut ball = RigidBody::sphere(
            ball_radius,
            1.0,
            Vec3::new(wall_x + ball_radius + 100.0, 0.0, 1000.0),
        );
        ball.restitution = 0.5;
        ball.linear_velocity = Vec3::new(-300.0, 0.0, 0.0);

        let mut world = PhysicsWorld::new(ball, flat_ground()).with_wall(wall);
        world.gravity = Vec3::ZERO;

        let dt = 1.0 / 120.0;
        for _ in 0..(3.0 / dt) as u32 {
            world.step(dt);
        }

        let contact_surface_x = wall_x + ball_radius;
        assert!(
            world.ball.position.x > contact_surface_x - 1.0,
            "expected the ball to stop at the wall's surface rather than tunnel through, \
             ball x={}, wall surface x={}",
            world.ball.position.x,
            contact_surface_x
        );
        assert!(
            world.ball.linear_velocity.x > 0.0,
            "expected the ball to bounce back, got vx={}",
            world.ball.linear_velocity.x
        );
    }

    #[test]
    fn double_jump_still_fires_when_a_wall_exists_but_is_not_touched() {
        // Regression guard: a wall existing in the scene must not affect a
        // car that isn't actually touching it — car_wall_normal has to be
        // gated on real contact, not just on `walls` being non-empty.
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(-5000.0, 0.0, 1000.0));
        let wall = StaticPlane::new(Vec3::new(1.0, 0.0, 0.0), 100.0);
        let car = some_car(Vec3::new(5000.0, 0.0, 1000.0));
        let mut world = PhysicsWorld::new(ball, flat_ground())
            .with_car(car)
            .with_wall(wall);
        world.gravity = Vec3::ZERO;

        world.set_car_input(
            0,
            rb_domain::ControllerInput {
                jump: true,
                ..Default::default()
            },
        );
        world.step(1.0 / 60.0);

        assert!(
            (world.cars[0].linear_velocity.z - crate::drive::JUMP_SPEED).abs() < 1.0,
            "expected a plain double jump (not a wall jump) when not touching any wall, got {:?}",
            world.cars[0].linear_velocity
        );
        assert_eq!(
            world.cars[0].linear_velocity.x, 0.0,
            "expected no wall-jump horizontal push-off when not touching a wall"
        );
    }

    #[test]
    fn a_car_dodges_forward_after_a_ground_jump_when_pitched_in_the_air() {
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(1000.0, 0.0, 93.0));
        let mut car = some_car(Vec3::new(0.0, 0.0, 18.0));
        car.restitution = 0.0;
        let ground = StaticPlane {
            restitution: 0.0,
            ..flat_ground()
        };
        let mut world = PhysicsWorld::new(ball, ground).with_car(car);
        world.gravity = Vec3::ZERO; // isolate the jump/dodge impulses from falling back down
        let dt = 1.0 / 120.0;

        // Ground jump: a fresh press while grounded.
        world.set_car_input(
            0,
            rb_domain::ControllerInput {
                jump: true,
                ..Default::default()
            },
        );
        world.step(dt);

        // Release, then let the car actually leave the ground before
        // dodging — a dodge, like the plain double jump, only fires while
        // airborne.
        world.set_car_input(0, rb_domain::ControllerInput::default());
        for _ in 0..12 {
            world.step(dt);
        }
        assert!(
            world.cars[0].position.z > 18.0 + 1.0,
            "expected the car to have left the ground before dodging, got z={}",
            world.cars[0].position.z
        );

        world.set_car_input(
            0,
            rb_domain::ControllerInput {
                jump: true,
                pitch: Some(1.0),
                ..Default::default()
            },
        );
        world.step(dt);

        assert!(
            (world.cars[0].linear_velocity.x - crate::drive::DODGE_SPEED).abs() < 1.0,
            "expected the dodge to give ~DODGE_SPEED forward velocity, got {}",
            world.cars[0].linear_velocity.x
        );
        assert!(
            world.cars[0].angular_velocity.y.abs() > 0.0,
            "expected the dodge to give the car a visible flip, got {:?}",
            world.cars[0].angular_velocity
        );
    }

    #[test]
    fn a_wall_jump_dodges_outward_and_upward_with_a_flip_in_a_live_world() {
        // Regression guard for the *reversed* premise: a wall jump used to
        // always ignore stick input; now directional stick input at or
        // above DODGE_DEADZONE fires a wall-jump dodge — the real
        // end-to-end proof that it fires in a live `PhysicsWorld::step`
        // loop, not just in `drive.rs` isolation.
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(-1000.0, 0.0, 1000.0));
        let wall = StaticPlane::new(Vec3::new(1.0, 0.0, 0.0), 100.0);
        let car = some_car(Vec3::new(160.0, 0.0, 1000.0));
        let mut world = PhysicsWorld::new(ball, flat_ground())
            .with_car(car)
            .with_wall(wall);
        world.gravity = Vec3::ZERO;

        world.set_car_input(
            0,
            rb_domain::ControllerInput {
                jump: true,
                pitch: Some(1.0),
                ..Default::default()
            },
        );
        world.step(1.0 / 60.0);

        assert!(
            (world.cars[0].linear_velocity.x
                - (crate::drive::WALL_JUMP_HORIZONTAL_SPEED + crate::drive::DODGE_SPEED))
                .abs()
                < 1.0,
            "expected the wall push-off plus the forward dodge component, got {}",
            world.cars[0].linear_velocity.x
        );
        assert!(
            (world.cars[0].linear_velocity.z - crate::drive::JUMP_SPEED).abs() < 1.0,
            "expected the wall jump's upward component, got {}",
            world.cars[0].linear_velocity.z
        );
        assert!(
            world.cars[0].angular_velocity.length() > 0.0,
            "expected the wall-jump dodge to give the car a visible flip, got {:?}",
            world.cars[0].angular_velocity
        );
    }

    #[test]
    fn a_held_ground_jump_reaches_greater_height_than_a_tapped_one() {
        let peak_height = |held: bool| -> f32 {
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
            let dt = 1.0 / 120.0;
            world.step(dt); // fresh press: fires the base impulse
            if !held {
                world.set_car_input(0, rb_domain::ControllerInput::default());
            }
            let mut peak = world.cars[0].position.z;
            for _ in 0..(2.0 / dt) as u32 {
                world.step(dt);
                peak = peak.max(world.cars[0].position.z);
            }
            peak
        };

        let tapped_peak = peak_height(false);
        let held_peak = peak_height(true);
        assert!(
            held_peak > tapped_peak + 1.0,
            "expected holding jump to reach a greater peak height than tapping it, \
             tapped={tapped_peak}, held={held_peak}"
        );
    }

    #[test]
    fn double_jump_after_a_held_ground_jump_still_gives_exactly_one_more_jump_speed_kick() {
        // Regression guard: holding the ground jump (earning extra height
        // via the new variable-height hold window) must not leak any extra
        // acceleration into a later double jump — variable height is
        // scoped to the ground jump only.
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(1000.0, 0.0, 93.0));
        let mut car = some_car(Vec3::new(0.0, 0.0, 18.0));
        car.restitution = 0.0;
        let ground = StaticPlane {
            restitution: 0.0,
            ..flat_ground()
        };
        let mut world = PhysicsWorld::new(ball, ground).with_car(car);
        world.gravity = Vec3::ZERO; // isolate the jump impulses from falling back down
        let dt = 1.0 / 120.0;

        // Ground jump, held well past drive::JUMP_HOLD_MAX_DURATION so the
        // extra acceleration has fully accrued before releasing.
        world.set_car_input(
            0,
            rb_domain::ControllerInput {
                jump: true,
                ..Default::default()
            },
        );
        for _ in 0..24 {
            world.step(dt);
        }
        let velocity_after_held_ground_jump = world.cars[0].linear_velocity.z;

        // Release, then press again once airborne — a plain double jump.
        world.set_car_input(0, rb_domain::ControllerInput::default());
        world.step(dt);
        world.set_car_input(
            0,
            rb_domain::ControllerInput {
                jump: true,
                ..Default::default()
            },
        );
        world.step(dt);

        let velocity_after_double_jump = world.cars[0].linear_velocity.z;
        assert!(
            (velocity_after_double_jump
                - (velocity_after_held_ground_jump + crate::drive::JUMP_SPEED))
                .abs()
                < 1.0,
            "expected the double jump to add exactly one more JUMP_SPEED kick on top of \
             whatever the held ground jump had already accrued, not an extra variable-height \
             boost, after held ground jump={velocity_after_held_ground_jump}, after double \
             jump={velocity_after_double_jump}"
        );
    }

    #[test]
    fn a_second_jump_press_cancels_a_dodges_spin_in_a_live_world() {
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(1000.0, 0.0, 93.0));
        let mut car = some_car(Vec3::new(0.0, 0.0, 18.0));
        car.restitution = 0.0;
        let ground = StaticPlane {
            restitution: 0.0,
            ..flat_ground()
        };
        let mut world = PhysicsWorld::new(ball, ground).with_car(car);
        world.gravity = Vec3::ZERO; // isolate the jump/dodge/flip-cancel from falling back down
        let dt = 1.0 / 120.0;

        // Ground jump, then leave the ground before dodging.
        world.set_car_input(
            0,
            rb_domain::ControllerInput {
                jump: true,
                ..Default::default()
            },
        );
        world.step(dt);
        world.set_car_input(0, rb_domain::ControllerInput::default());
        for _ in 0..12 {
            world.step(dt);
        }

        // Dodge.
        world.set_car_input(
            0,
            rb_domain::ControllerInput {
                jump: true,
                pitch: Some(1.0),
                ..Default::default()
            },
        );
        world.step(dt);
        assert!(
            world.cars[0].angular_velocity.length() > 0.0,
            "expected the dodge to leave the car spinning, got {:?}",
            world.cars[0].angular_velocity
        );

        // Release, then press again — flip-cancel.
        world.set_car_input(0, rb_domain::ControllerInput::default());
        world.step(dt);
        world.set_car_input(
            0,
            rb_domain::ControllerInput {
                jump: true,
                ..Default::default()
            },
        );
        world.step(dt);

        assert_eq!(
            world.cars[0].angular_velocity,
            Vec3::ZERO,
            "expected the second jump press to cancel the dodge's spin outright, got {:?}",
            world.cars[0].angular_velocity
        );
    }

    #[test]
    fn landing_and_a_new_double_jump_clears_a_stale_dodge_flip_flag_in_a_live_world() {
        // Regression guard: the real end-to-end proof that a dodge's
        // cancelable-flip flag doesn't leak past landing and a later,
        // unrelated plain double jump into a spurious flip-cancel.
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(1000.0, 0.0, 93.0));
        let mut car = some_car(Vec3::new(0.0, 0.0, 18.0));
        car.restitution = 0.0;
        let ground = StaticPlane {
            restitution: 0.0,
            ..flat_ground()
        };
        let mut world = PhysicsWorld::new(ball, ground).with_car(car);
        world.gravity = Vec3::ZERO; // isolate every jump/dodge impulse from falling back down
        let dt = 1.0 / 120.0;

        // Ground jump, leave the ground, dodge.
        world.set_car_input(
            0,
            rb_domain::ControllerInput {
                jump: true,
                ..Default::default()
            },
        );
        world.step(dt);
        world.set_car_input(0, rb_domain::ControllerInput::default());
        for _ in 0..12 {
            world.step(dt);
        }
        world.set_car_input(
            0,
            rb_domain::ControllerInput {
                jump: true,
                pitch: Some(1.0),
                ..Default::default()
            },
        );
        world.step(dt);
        assert!(world.cars[0].angular_velocity.length() > 0.0);

        // Release jump (so the next ground-jump press is a real fresh
        // press), then land: zero out the spin and velocity by hand and
        // put the car back at its resting height, as if it had settled
        // flat — this test only cares about the *later* double jump, not
        // about actually simulating the fall back down. dodge_flip_active
        // is deliberately left stale (`true`) here: landing alone doesn't
        // clear it — see the module doc comment — only a later
        // double-jump-or-dodge press does.
        world.set_car_input(0, rb_domain::ControllerInput::default());
        world.step(dt);
        world.cars[0].angular_velocity = Vec3::ZERO;
        world.cars[0].position = Vec3::new(0.0, 0.0, 18.0);
        world.cars[0].linear_velocity = Vec3::ZERO;
        world.step(dt); // on_ground computed fresh from the reset position

        // Ground jump again, leave the ground, then a plain double jump
        // (no stick input).
        world.set_car_input(
            0,
            rb_domain::ControllerInput {
                jump: true,
                ..Default::default()
            },
        );
        world.step(dt);
        world.set_car_input(0, rb_domain::ControllerInput::default());
        for _ in 0..12 {
            world.step(dt);
        }
        world.set_car_input(
            0,
            rb_domain::ControllerInput {
                jump: true,
                ..Default::default()
            },
        );
        world.step(dt);
        let angular_velocity_after_plain_double_jump = world.cars[0].angular_velocity;

        // Release, then press again — must NOT fire a spurious flip-cancel.
        world.set_car_input(0, rb_domain::ControllerInput::default());
        world.step(dt);
        world.set_car_input(
            0,
            rb_domain::ControllerInput {
                jump: true,
                ..Default::default()
            },
        );
        world.step(dt);

        // A tolerance rather than exact equality: the landing
        // auto-orientation assist (RB-PHYSICS-001-FR-018) now applies a
        // tiny continuous corrective torque on the neutral release step in
        // between, which a real spurious flip-cancel (zeroing the whole
        // angular velocity) would dwarf.
        assert!(
            (world.cars[0].angular_velocity - angular_velocity_after_plain_double_jump).length()
                < 0.01,
            "expected no spurious flip-cancel after an unrelated plain double jump, before \
             release/re-press={angular_velocity_after_plain_double_jump:?}, after={:?}",
            world.cars[0].angular_velocity
        );
    }

    #[test]
    fn a_wall_jump_dodges_spin_can_be_flip_cancelled_in_a_live_world() {
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(-1000.0, 0.0, 1000.0));
        let wall = StaticPlane::new(Vec3::new(1.0, 0.0, 0.0), 100.0);
        let car = some_car(Vec3::new(160.0, 0.0, 1000.0));
        let mut world = PhysicsWorld::new(ball, flat_ground())
            .with_car(car)
            .with_wall(wall);
        world.gravity = Vec3::ZERO;
        let dt = 1.0 / 120.0;

        world.set_car_input(
            0,
            rb_domain::ControllerInput {
                jump: true,
                pitch: Some(1.0),
                ..Default::default()
            },
        );
        world.step(dt);
        assert!(
            world.cars[0].angular_velocity.length() > 0.0,
            "expected the wall-jump dodge to leave the car spinning, got {:?}",
            world.cars[0].angular_velocity
        );

        // Release, then move off the wall and press again — flip-cancel.
        world.set_car_input(0, rb_domain::ControllerInput::default());
        world.cars[0].position = Vec3::new(5000.0, 0.0, 1000.0);
        world.step(dt);
        world.set_car_input(
            0,
            rb_domain::ControllerInput {
                jump: true,
                ..Default::default()
            },
        );
        world.step(dt);

        assert_eq!(
            world.cars[0].angular_velocity,
            Vec3::ZERO,
            "expected the second jump press to cancel the wall-jump dodge's spin outright, got {:?}",
            world.cars[0].angular_velocity
        );
    }

    #[test]
    fn an_airborne_car_gradually_rights_itself_with_no_input_in_a_live_world() {
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(1000.0, 0.0, 1000.0));
        let mut car = some_car(Vec3::new(0.0, 0.0, 1000.0));
        // Tilted 90 degrees about its local forward axis, as if fresh out
        // of an uncanceled dodge, with no stick input to right itself.
        car.orientation = rb_domain::Quat::new(
            std::f32::consts::FRAC_1_SQRT_2,
            0.0,
            0.0,
            std::f32::consts::FRAC_1_SQRT_2,
        );
        car.update_inertia_tensor();
        let mut world = PhysicsWorld::new(ball, flat_ground()).with_car(car);
        world.gravity = Vec3::ZERO; // isolate the assist from falling

        let world_up = Vec3::new(0.0, 0.0, 1.0);
        let alignment_before = world.cars[0].orientation.rotate(&world_up).dot(&world_up);

        let dt = 1.0 / 60.0;
        for _ in 0..120 {
            world.step(dt);
        }

        let alignment_after = world.cars[0].orientation.rotate(&world_up).dot(&world_up);
        assert!(
            alignment_after > alignment_before,
            "expected the landing-orientation assist to trend the car back toward level over \
             time, alignment before={alignment_before}, after={alignment_after}"
        );
    }

    #[test]
    fn standard_arena_has_nine_walls_and_the_standard_ground() {
        // 7 real arena walls (the back walls moved out of `standard_walls`
        // and into `goal_walls` as of RB-PHYSICS-001-FR-024 -- see
        // `standard_arena_has_two_goal_walls`) plus, since
        // RB-PHYSICS-001-FR-029, 2 more plain planes for each goal box's
        // own back-of-net wall (`standard_goal_back_walls`) -- 9 total.
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::ZERO);
        let world = PhysicsWorld::standard_arena(ball);
        assert_eq!(world.walls.len(), 9);
        assert_eq!(world.ground, crate::arena::standard_ground());
    }

    #[test]
    fn standard_arena_has_thirty_curved_transitions() {
        // 24 floor/ceiling-seam and vertical-edge fillets
        // (RB-PHYSICS-001-FR-020/021/022) plus 6 goal-cutout-edge fillets
        // (RB-PHYSICS-001-FR-024), all sharing the same `curves` list.
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::ZERO);
        let world = PhysicsWorld::standard_arena(ball);
        assert_eq!(world.curves.len(), 30);
    }

    #[test]
    fn standard_arena_has_two_goal_walls() {
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::ZERO);
        let world = PhysicsWorld::standard_arena(ball);
        assert_eq!(world.goal_walls.len(), 2);
    }

    #[test]
    fn standard_arena_has_six_bounded_walls() {
        // 4 goal side walls (2 per goal) plus 2 goal roofs (1 per goal),
        // since RB-PHYSICS-001-FR-029.
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::ZERO);
        let world = PhysicsWorld::standard_arena(ball);
        assert_eq!(world.bounded_walls.len(), 6);
    }

    #[test]
    fn standard_arena_has_two_nets() {
        // One net panel per goal, since RB-PHYSICS-001-FR-033.
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::ZERO);
        let world = PhysicsWorld::standard_arena(ball);
        assert_eq!(world.nets.len(), 2);
    }

    #[test]
    fn a_ball_resting_within_a_curved_transitions_footprint_is_pushed_up_off_the_flat_floor_height()
    {
        // Wall at x=1000, fillet radius 292: resting at flat-floor height
        // (z=ball_radius) at x=900 already overlaps the curve's own
        // material (it's within the fillet's footprint, closer to the wall
        // than the fillet's floor-side tangent point at x=708) -- the curve
        // should push the ball up onto its own surface instead of leaving
        // it embedded, the real end-to-end proof that
        // RB-PHYSICS-001-FR-020's curved transition is real physical
        // geometry, not just a detection hack.
        let floor = flat_ground();
        let wall = StaticPlane::new(Vec3::new(-1.0, 0.0, 0.0), -1000.0);
        let curve = crate::body::StaticQuarterPipe::between_planes(
            &floor,
            &wall,
            292.0,
            Vec3::new(0.0, 1.0, 0.0),
        );

        let ball_radius = 93.15;
        let mut ball = RigidBody::sphere(ball_radius, 1.0, Vec3::new(900.0, 0.0, ball_radius));
        ball.restitution = 0.0;

        let mut world = PhysicsWorld::new(ball, floor)
            .with_wall(wall)
            .with_curve(curve);
        world.gravity = Vec3::ZERO;

        let dt = 1.0 / 120.0;
        for _ in 0..60 {
            world.step(dt);
        }

        assert!(
            world.ball.position.z > ball_radius + 10.0,
            "expected the curve to push the ball up off flat-floor height, got z={}",
            world.ball.position.z
        );
    }

    #[test]
    fn a_car_resting_within_a_curved_transitions_footprint_is_pushed_up_off_the_flat_floor_height()
    {
        // The real end-to-end proof of RB-PHYSICS-001-FR-027: a car sitting
        // at the exact same overlapping position the ball test above uses
        // (well within the curve's own footprint, closer to the wall than
        // the fillet's floor-side tangent point) should get pushed up onto
        // the curve's own surface instead of staying embedded, the same
        // live-physics proof already given for the ball, now for a car's
        // own box via `collision::box_vs_quarter_pipe`'s corner-testing
        // approximation.
        let floor = flat_ground();
        let wall = StaticPlane::new(Vec3::new(-1.0, 0.0, 0.0), -1000.0);
        let curve = crate::body::StaticQuarterPipe::between_planes(
            &floor,
            &wall,
            292.0,
            Vec3::new(0.0, 1.0, 0.0),
        );

        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(-5000.0, 0.0, 1000.0));
        let car_half_extents = Vec3::new(60.0, 30.0, 18.0);
        let car = some_car(Vec3::new(900.0, 0.0, car_half_extents.z));
        let mut world = PhysicsWorld::new(ball, floor)
            .with_car(car)
            .with_wall(wall)
            .with_curve(curve);
        world.gravity = Vec3::ZERO;

        let dt = 1.0 / 120.0;
        for _ in 0..60 {
            world.step(dt);
        }

        assert!(
            world.cars[0].position.z > car_half_extents.z + 5.0,
            "expected the curve to push the car up off flat-floor height, got z={}",
            world.cars[0].position.z
        );
    }

    #[test]
    fn a_car_embedded_in_a_compound_corner_fillets_footprint_has_its_penetration_reduced() {
        // The same live-physics proof
        // `a_ball_embedded_in_a_compound_corner_fillets_footprint_is_pushed_toward_the_center`
        // gives for the ball (RB-PHYSICS-001-FR-023), adapted for a car's
        // own box via `collision::box_vs_corner_fillet`'s corner-testing
        // approximation (RB-PHYSICS-001-FR-027). Unlike a sphere (a single
        // point, so "distance to the fillet's center shrinks" is exactly
        // "penetration shrinks"), an axis-aligned box's corners sit at
        // different depths into the fillet at once -- resolving one
        // corner's contact can rotate the box in a way that moves its
        // *center* away from the fillet even as every individual
        // corner's own overlap is being corrected. So this checks the
        // real invariant that generalizes: the worst (deepest) corner
        // penetration this fillet reports should be smaller once the
        // solver has run than it was at the deeply-embedded starting
        // position, not that the box's own center approaches the
        // fillet's.
        let floor = flat_ground();
        let wall_x = StaticPlane::new(Vec3::new(-1.0, 0.0, 0.0), -1000.0);
        let wall_y = StaticPlane::new(Vec3::new(0.0, -1.0, 0.0), -1000.0);
        let radius = 292.0;
        let fillet =
            crate::body::StaticCornerFillet::between_three_planes(&floor, &wall_x, &wall_y, radius);

        let toward_corner = Vec3::new(1.0, 1.0, -1.0)
            .normalize()
            .expect("(1, 1, -1) is nonzero");
        let starting_distance = fillet.radius;
        let car = some_car(fillet.center + toward_corner * starting_distance);
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(-5000.0, -5000.0, 5000.0));

        let max_penetration = |body: &RigidBody| -> f32 {
            collision::contacts_vs_corner_fillet(body, &fillet)
                .iter()
                .map(|c| c.penetration_depth)
                .fold(0.0f32, f32::max)
        };
        let starting_penetration = max_penetration(&car);
        assert!(
            starting_penetration > 0.0,
            "expected the starting position to actually overlap the fillet"
        );

        let mut world = PhysicsWorld::new(ball, floor)
            .with_car(car)
            .with_wall(wall_x)
            .with_wall(wall_y)
            .with_corner_fillet(fillet);
        world.gravity = Vec3::ZERO;

        let dt = 1.0 / 120.0;
        for _ in 0..60 {
            world.step(dt);
        }

        let final_penetration = max_penetration(&world.cars[0]);
        assert!(
            final_penetration < starting_penetration - 5.0,
            "expected the compound-corner fillet to meaningfully reduce the car's worst \
             corner penetration, started at {starting_penetration}, got {final_penetration}"
        );
    }

    #[test]
    fn a_ball_resting_within_a_diagonal_walls_curved_transition_footprint_is_pushed_up() {
        // The real end-to-end proof of RB-PHYSICS-001-FR-021: `between_planes`
        // generalizes to a wall whose normal isn't a coordinate axis (like
        // one of the standard arena's diagonal corner walls) as long as it's
        // still perpendicular to the floor -- this test builds its own
        // diagonal (non-axis-aligned) wall rather than going through
        // `arena::standard_curves` so the fillet's own geometric correctness
        // is checked directly, independent of the arena module's specific
        // corner placement. Same structure as
        // `a_ball_resting_within_a_curved_transitions_footprint_is_pushed_up_off_the_flat_floor_height`,
        // just with a diagonal wall normal instead of an axis-aligned one.
        let floor = flat_ground();
        let wall_normal = Vec3::new(-1.0, -1.0, 0.0) * std::f32::consts::FRAC_1_SQRT_2;
        let wall = StaticPlane::new(wall_normal, -1000.0);
        let axis_direction = floor.normal.cross(&wall.normal);
        let curve =
            crate::body::StaticQuarterPipe::between_planes(&floor, &wall, 292.0, axis_direction);

        let ball_radius = 93.15;
        // 900 units from the origin toward the wall, along the wall's
        // inward direction -- the diagonal analogue of the cardinal test's
        // ball at x=900 for a wall at x=1000.
        let toward_wall = -wall.normal;
        let mut ball = RigidBody::sphere(
            ball_radius,
            1.0,
            toward_wall * 900.0 + Vec3::new(0.0, 0.0, ball_radius),
        );
        ball.restitution = 0.0;

        let mut world = PhysicsWorld::new(ball, floor)
            .with_wall(wall)
            .with_curve(curve);
        world.gravity = Vec3::ZERO;

        let dt = 1.0 / 120.0;
        for _ in 0..60 {
            world.step(dt);
        }

        assert!(
            world.ball.position.z > ball_radius + 10.0,
            "expected the diagonal wall's curve to push the ball up off flat-floor height, got z={}",
            world.ball.position.z
        );
    }

    #[test]
    fn a_ball_embedded_in_a_corner_walls_floor_arch_footprint_is_pushed_toward_the_axis() {
        // The real end-to-end proof of RB-PHYSICS-001-FR-025: a corner
        // wall's own floor-seam arch now uses the larger
        // `arena::CORNER_ARCH_RADIUS`, not the cardinal walls'
        // `arena::FILLET_RADIUS` -- a ball embedded past *that* larger
        // radius (deep enough that it would sit outside a plain
        // `FILLET_RADIUS` fillet's own footprint entirely) should still get
        // pushed back toward the axis, proving the bigger radius is live
        // collision geometry, not just a number in a doc comment. Same
        // diagonal, non-axis-aligned wall setup as
        // `a_ball_resting_within_a_curved_transitions_footprint_is_pushed_up_off_the_flat_floor_height`'s
        // corner-wall variant, and the same weaker "moved meaningfully"
        // assertion as
        // `a_ball_embedded_in_a_vertical_corner_edges_fillet_footprint_is_pushed_toward_the_axis`,
        // for the same residual-velocity reason.
        let floor = flat_ground();
        let wall_normal = Vec3::new(-1.0, -1.0, 0.0) * std::f32::consts::FRAC_1_SQRT_2;
        let wall = StaticPlane::new(wall_normal, -1000.0);
        let axis_direction = floor.normal.cross(&wall.normal);
        let radius = crate::arena::CORNER_ARCH_RADIUS;
        assert!(radius > crate::arena::FILLET_RADIUS);
        let curve =
            crate::body::StaticQuarterPipe::between_planes(&floor, &wall, radius, axis_direction);

        let ball_radius = 93.15;
        let bisector = ((curve.sector_start + curve.sector_end) * 0.5)
            .normalize()
            .expect("sector_start and sector_end aren't exactly opposite, so their sum is nonzero");
        // Overlapping the arch's own material by 10 units (further from the
        // axis than the resting distance, toward the sharp corner the arch
        // replaces) -- well past where a plain FILLET_RADIUS fillet's own
        // footprint would have ended, proving the larger radius is what's
        // actually governing this contact.
        let embedded_distance = radius - ball_radius + 10.0;
        assert!(embedded_distance > crate::arena::FILLET_RADIUS);
        let embedded_position = curve.axis_point + bisector * embedded_distance;
        let mut ball = RigidBody::sphere(ball_radius, 1.0, embedded_position);
        ball.restitution = 0.0;

        let mut world = PhysicsWorld::new(ball, floor)
            .with_wall(wall)
            .with_curve(curve);
        world.gravity = Vec3::ZERO;

        let dt = 1.0 / 120.0;
        for _ in 0..60 {
            world.step(dt);
        }

        let final_horizontal_rel = Vec3::new(
            world.ball.position.x - curve.axis_point.x,
            world.ball.position.y - curve.axis_point.y,
            0.0,
        );
        let final_dist = final_horizontal_rel.length();
        assert!(
            final_dist < embedded_distance - 10.0,
            "expected the corner wall's floor arch to push the ball meaningfully toward the \
             axis, started {embedded_distance} units out, got {final_dist}"
        );
    }

    #[test]
    fn a_ball_embedded_in_a_vertical_corner_edges_fillet_footprint_is_pushed_toward_the_axis() {
        // The real end-to-end proof of RB-PHYSICS-001-FR-022: two vertical
        // walls meeting at a shallow (non-perpendicular, 45-degree-normal)
        // angle, exactly like a diagonal corner wall's own vertical edge
        // where it meets its neighboring side/back wall -- a ball embedded
        // past the fillet's own radius (deep in what would otherwise be the
        // sharp, unrounded corner sliver) should be pushed back toward the
        // axis, the same live-physics proof already given for the
        // floor/wall and diagonal-wall fillets, now for a wall-to-wall
        // corner whose two planes aren't perpendicular. Checks the ball
        // settles at (not past) the fillet's own resting distance: since
        // `RB-PHYSICS-001-FR-034`, penetration correction runs entirely on
        // the split-impulse push channel rather than leaking into the
        // ball's real velocity, so once the overlap resolves there's no
        // residual velocity left to coast onward with (unlike before
        // FR-034, when this same test asserted only "moved meaningfully",
        // since the ball would overshoot the resting distance and keep
        // going).
        let wall_a = StaticPlane::new(Vec3::new(-1.0, 0.0, 0.0), 0.0);
        let wall_b = StaticPlane::new(
            Vec3::new(-1.0, -1.0, 0.0) * std::f32::consts::FRAC_1_SQRT_2,
            0.0,
        );
        let radius = 292.0;
        let curve = crate::body::StaticQuarterPipe::between_planes(
            &wall_a,
            &wall_b,
            radius,
            Vec3::new(0.0, 0.0, 1.0),
        );

        let ball_radius = 93.15;
        let bisector = ((curve.sector_start + curve.sector_end) * 0.5)
            .normalize()
            .expect("sector_start and sector_end aren't exactly opposite, so their sum is nonzero");
        // Overlapping the fillet's own material by 10 units (further from
        // the axis than the resting distance, toward the sharp corner the
        // fillet replaces), well clear of the ground so gravity/floor
        // contact can't interfere.
        let embedded_distance = curve.radius - ball_radius + 10.0;
        let embedded_position =
            curve.axis_point + bisector * embedded_distance + Vec3::new(0.0, 0.0, 500.0);
        let mut ball = RigidBody::sphere(ball_radius, 1.0, embedded_position);
        ball.restitution = 0.0;

        let mut world = PhysicsWorld::new(ball, flat_ground())
            .with_wall(wall_a)
            .with_wall(wall_b)
            .with_curve(curve);
        world.gravity = Vec3::ZERO;

        let dt = 1.0 / 120.0;
        for _ in 0..60 {
            world.step(dt);
        }

        let final_horizontal_rel = Vec3::new(
            world.ball.position.x - curve.axis_point.x,
            world.ball.position.y - curve.axis_point.y,
            0.0,
        );
        let final_dist = final_horizontal_rel.length();
        let resting_distance = curve.radius - ball_radius;
        assert!(
            (final_dist - resting_distance).abs() < 1.0,
            "expected the corner-edge fillet to settle the ball at its resting distance \
             ({resting_distance}), started {embedded_distance} units out, got {final_dist}"
        );
    }

    #[test]
    fn standard_arena_has_twenty_compound_corner_fillets() {
        // 16 arena corners (RB-PHYSICS-001-FR-023) plus 4 goal post-crossbar
        // corners (RB-PHYSICS-001-FR-026, 2 posts times 2 goals).
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::ZERO);
        let world = PhysicsWorld::standard_arena(ball);
        assert_eq!(world.corner_fillets.len(), 20);
    }

    #[test]
    fn a_ball_embedded_in_a_compound_corner_fillets_footprint_is_pushed_toward_the_center() {
        // The real end-to-end proof of RB-PHYSICS-001-FR-023: three planes
        // meeting at a single vertex (here, a floor and two vertical walls
        // meeting at 90 degrees each, like a corner wall's own floor-side
        // endpoint) -- a ball embedded past the fillet's own radius (deep
        // in what would otherwise be the sharp, unrounded corner) should be
        // pushed back toward the fillet's center, the same live-physics
        // proof already given for the edge fillets, now for a compound
        // 3-plane corner. Checks the ball settles at (not past) the
        // fillet's own resting distance, same reasoning as
        // `a_ball_embedded_in_a_vertical_corner_edges_fillet_footprint_is_pushed_toward_the_axis`.
        let floor = flat_ground();
        let wall_x = StaticPlane::new(Vec3::new(-1.0, 0.0, 0.0), -1000.0);
        let wall_y = StaticPlane::new(Vec3::new(0.0, -1.0, 0.0), -1000.0);
        let radius = 292.0;
        let fillet =
            crate::body::StaticCornerFillet::between_three_planes(&floor, &wall_x, &wall_y, radius);

        let ball_radius = 93.15;
        let toward_corner = Vec3::new(1.0, 1.0, -1.0)
            .normalize()
            .expect("(1, 1, -1) is nonzero");
        // Overlapping the fillet's own material by 10 units (further from
        // the center than the resting distance, toward the sharp corner
        // the fillet replaces).
        let embedded_distance = fillet.radius - ball_radius + 10.0;
        let embedded_position = fillet.center + toward_corner * embedded_distance;
        let mut ball = RigidBody::sphere(ball_radius, 1.0, embedded_position);
        ball.restitution = 0.0;

        let mut world = PhysicsWorld::new(ball, floor)
            .with_wall(wall_x)
            .with_wall(wall_y)
            .with_corner_fillet(fillet);
        world.gravity = Vec3::ZERO;

        let dt = 1.0 / 120.0;
        for _ in 0..60 {
            world.step(dt);
        }

        let final_dist = (world.ball.position - fillet.center).length();
        let resting_distance = fillet.radius - ball_radius;
        assert!(
            (final_dist - resting_distance).abs() < 1.0,
            "expected the compound-corner fillet to settle the ball at its resting distance \
             ({resting_distance}), started {embedded_distance} units out, got {final_dist}"
        );
    }

    #[test]
    fn a_ball_bounces_off_the_standard_arenas_side_wall_in_a_live_world() {
        // The same physical proof as a_ball_bounces_off_a_wall_instead_of_
        // passing_through, but against PhysicsWorld::standard_arena's real
        // field-dimension side wall instead of a hand-placed test wall.
        let ball_radius = 93.15;
        let mut ball = RigidBody::sphere(ball_radius, 1.0, Vec3::new(0.0, 0.0, 1000.0));
        ball.restitution = 0.5;
        ball.linear_velocity = Vec3::new(2000.0, 0.0, 0.0);

        let mut world = PhysicsWorld::standard_arena(ball);
        world.gravity = Vec3::ZERO;

        let dt = 1.0 / 120.0;
        for _ in 0..(5.0 / dt) as u32 {
            world.step(dt);
        }

        let side_wall_surface_x = crate::arena::SIDE_WALL_X - ball_radius;
        assert!(
            world.ball.position.x < side_wall_surface_x + 1.0,
            "expected the ball to stop at the standard arena's side wall rather than escape \
             it, ball x={}, wall surface x={}",
            world.ball.position.x,
            side_wall_surface_x
        );
        assert!(
            world.ball.linear_velocity.x <= 0.0,
            "expected the ball to have bounced back off the side wall, got vx={}",
            world.ball.linear_velocity.x
        );
    }

    #[test]
    fn a_ball_is_stopped_by_the_corner_wall_before_reaching_the_true_rectangular_corner() {
        // Fired straight along the diagonal toward the arena's true
        // (uncut) rectangular corner (SIDE_WALL_X, BACK_WALL_Y): if the
        // octagon's corner wall is real physical geometry rather than
        // decoration, the ball must be stopped well before its x or y
        // individually reaches either the side or back wall's own
        // position — proof it's the diagonal corner plane doing the work,
        // not the two cardinal walls.
        let ball_radius = 93.15;
        let mut ball = RigidBody::sphere(ball_radius, 1.0, Vec3::new(0.0, 0.0, 1000.0));
        ball.restitution = 0.5;
        let diag = std::f32::consts::FRAC_1_SQRT_2;
        ball.linear_velocity = Vec3::new(3000.0 * diag, 3000.0 * diag, 0.0);

        let mut world = PhysicsWorld::standard_arena(ball);
        world.gravity = Vec3::ZERO;

        let dt = 1.0 / 120.0;
        for _ in 0..(5.0 / dt) as u32 {
            world.step(dt);
        }

        assert!(
            world.ball.position.x < crate::arena::SIDE_WALL_X - 1.0,
            "expected the corner wall to stop the ball before its x reached the side wall's \
             own position, got x={}",
            world.ball.position.x
        );
        assert!(
            world.ball.position.y < crate::arena::BACK_WALL_Y - 1.0,
            "expected the corner wall to stop the ball before its y reached the back wall's \
             own position, got y={}",
            world.ball.position.y
        );
    }

    #[test]
    fn a_ball_shot_through_the_goal_mouth_passes_the_standard_arenas_back_wall() {
        // The real end-to-end proof of RB-PHYSICS-001-FR-024: a ball fired
        // straight through the center of the goal-mouth window, well clear
        // of the window's own rounded edges, keeps going past the back
        // wall's own y position instead of bouncing off it -- proof the
        // cutout is a genuine opening, not decoration.
        //
        // Only flown for 1.8s (y=5400 unobstructed), comfortably past the
        // back wall at BACK_WALL_Y=5120 but well short of y=~6300: a
        // corner-wall floor-seam arch's own axis (`standard_curves`) is a
        // line that's `StaticQuarterPipe`-documented as infinite along its
        // own length, not clipped to the corner wall's real, finite span, so
        // a ball flying dead down the arena's own center line eventually
        // re-enters *some* corner arch's resting shell far past the goal --
        // already true before RB-PHYSICS-001-FR-025 (verified against this
        // same test with the old, smaller FILLET_RADIUS, where it lands
        // around y=~7650-7930 instead), and unrelated to the goal cutout
        // this test actually exercises. 3.0s used to clear that zone by
        // luck at the old radius; FR-025's bigger CORNER_ARCH_RADIUS moves
        // it closer in (~6300-7700) and turns the same brush from a gentle
        // bounce into a much sharper one, so this test now stops well
        // before reaching it instead of relying on outrunning it.
        let ball_radius = 93.15;
        let mut ball = RigidBody::sphere(
            ball_radius,
            1.0,
            Vec3::new(0.0, 0.0, crate::arena::GOAL_HEIGHT * 0.5),
        );
        ball.restitution = 0.0;
        ball.linear_velocity = Vec3::new(0.0, 3000.0, 0.0);

        let mut world = PhysicsWorld::standard_arena(ball);
        world.gravity = Vec3::ZERO;

        let dt = 1.0 / 120.0;
        for _ in 0..(1.8 / dt) as u32 {
            world.step(dt);
        }

        assert!(
            world.ball.position.y > crate::arena::BACK_WALL_Y + 1.0,
            "expected the ball to pass through the goal mouth rather than bounce off the back \
             wall, got y={}",
            world.ball.position.y
        );
    }

    #[test]
    fn a_car_shot_through_the_goal_mouth_passes_the_standard_arenas_back_wall() {
        // The real end-to-end proof of RB-PHYSICS-001-FR-028: a car aimed
        // straight through the same goal-mouth center position the ball
        // test above uses, well clear of the window's own rounded edges,
        // keeps going past the back wall's own y position instead of being
        // stopped by it -- proof `box_vs_goal_wall`'s per-corner window
        // treatment is live physical geometry for a car, not just a
        // detection hack. Same 1.8s flight-duration bound as the ball's own
        // equivalent test, for the same reason (see that test's own doc
        // comment) -- a car's own small half-extents relative to the goal's
        // real dimensions (`GOAL_HALF_WIDTH`/`GOAL_HEIGHT`) mean it clears
        // the window with room to spare either way.
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(0.0, -3000.0, 1000.0));
        let mut car = some_car(Vec3::new(0.0, 0.0, crate::arena::GOAL_HEIGHT * 0.5));
        car.linear_velocity = Vec3::new(0.0, 3000.0, 0.0);

        let mut world = PhysicsWorld::standard_arena(ball).with_car(car);
        world.gravity = Vec3::ZERO;

        let dt = 1.0 / 120.0;
        for _ in 0..(1.8 / dt) as u32 {
            world.step(dt);
        }

        assert!(
            world.cars[0].position.y > crate::arena::BACK_WALL_Y + 1.0,
            "expected the car to pass through the goal mouth rather than be stopped by the back \
             wall, got y={}",
            world.cars[0].position.y
        );
    }

    #[test]
    fn a_car_aimed_away_from_the_goal_mouth_is_still_stopped_by_the_back_wall() {
        // Regression guard alongside the pass-through proof above: a car
        // aimed at the solid part of the back wall, well outside the
        // goal-mouth window's own half-width, is still stopped by it --
        // `RB-PHYSICS-001-FR-028` only opens the window itself, it doesn't
        // make the rest of the back wall driveable-through.
        let ball = RigidBody::sphere(1.0, 1.0, Vec3::new(0.0, -3000.0, 1000.0));
        let solid_x = crate::arena::GOAL_HALF_WIDTH + 500.0;
        let mut car = some_car(Vec3::new(solid_x, 0.0, 18.0));
        car.linear_velocity = Vec3::new(0.0, 3000.0, 0.0);

        let mut world = PhysicsWorld::standard_arena(ball).with_car(car);
        world.gravity = Vec3::ZERO;

        let dt = 1.0 / 120.0;
        for _ in 0..(3.0 / dt) as u32 {
            world.step(dt);
        }

        assert!(
            world.cars[0].position.y < crate::arena::BACK_WALL_Y - 1.0,
            "expected the car to be stopped by the solid part of the back wall, got y={}",
            world.cars[0].position.y
        );
    }

    #[test]
    fn a_ball_shot_into_the_goal_is_stopped_by_the_goal_back_wall() {
        // The real end-to-end proof of RB-PHYSICS-001-FR-029's own
        // back-of-net wall: a ball fired straight toward the back of the
        // goal box settles there instead of flying forever into unbounded
        // open space the way it did before this requirement -- proof the
        // goal box's own interior is bounded, not just the cutout itself.
        //
        // Isolated to just this one new wall via `PhysicsWorld::new` plus
        // `with_wall`, rather than the full `PhysicsWorld::standard_arena`
        // -- the standard arena's own goal-cutout fillets sit right at the
        // window's edge, close enough to this scene's own path that the
        // pre-existing "quarter-pipe sector membership is angle-only, not
        // radially bounded" limitation (the same category noted in
        // `StaticQuarterPipe`'s own doc comment and the FR-025 test-writing
        // notes) can fire spuriously; a synthetic, single-wall scene proves
        // this wall's own behavior without that unrelated interaction.
        let mut ball = RigidBody::sphere(
            93.15,
            1.0,
            Vec3::new(
                0.0,
                crate::arena::BACK_WALL_Y + 10.0,
                crate::arena::GOAL_HEIGHT * 0.5,
            ),
        );
        ball.restitution = 0.0;
        ball.linear_velocity = Vec3::new(0.0, 400.0, 0.0);

        let mut world = PhysicsWorld::new(ball, crate::arena::standard_ground());
        for mut wall in crate::arena::standard_goal_back_walls() {
            wall.restitution = 0.0;
            world = world.with_wall(wall);
        }
        world.gravity = Vec3::ZERO;

        let dt = 1.0 / 120.0;
        for _ in 0..(3.0 / dt) as u32 {
            world.step(dt);
        }

        let goal_back_wall_y = crate::arena::BACK_WALL_Y + crate::arena::GOAL_DEPTH;
        assert!(
            world.ball.position.y > crate::arena::BACK_WALL_Y
                && world.ball.position.y < goal_back_wall_y + 5.0,
            "expected the ball to settle inside the goal box against its own back wall, got y={}",
            world.ball.position.y
        );
    }

    #[test]
    fn a_ball_shot_sideways_inside_the_goal_is_stopped_by_a_goal_side_wall() {
        // The real end-to-end proof of RB-PHYSICS-001-FR-029's own side
        // walls: a ball fired sideways across the goal's own width (not
        // through the front window) settles against a goal side wall
        // instead of flying through the main field's own much-wider side
        // wall position. Isolated to just the 4 side walls via
        // `with_bounded_wall`, for the same reason the back-wall test
        // above is isolated -- see its own doc comment.
        let mut ball = RigidBody::sphere(
            93.15,
            1.0,
            Vec3::new(
                0.0,
                crate::arena::BACK_WALL_Y + crate::arena::GOAL_DEPTH * 0.5,
                crate::arena::GOAL_HEIGHT * 0.5,
            ),
        );
        ball.restitution = 0.0;
        ball.linear_velocity = Vec3::new(400.0, 0.0, 0.0);

        let mut world = PhysicsWorld::new(ball, crate::arena::standard_ground());
        for mut wall in crate::arena::standard_goal_side_walls() {
            wall.plane.restitution = 0.0;
            world = world.with_bounded_wall(wall);
        }
        world.gravity = Vec3::ZERO;

        let dt = 1.0 / 120.0;
        for _ in 0..(3.0 / dt) as u32 {
            world.step(dt);
        }

        assert!(
            world.ball.position.x > 0.0
                && world.ball.position.x < crate::arena::GOAL_HALF_WIDTH + 5.0,
            "expected the ball to settle inside the goal box against its own side wall, got x={}",
            world.ball.position.x
        );
    }

    #[test]
    fn a_ball_shot_upward_inside_the_goal_is_stopped_by_the_goal_roof() {
        // The real end-to-end proof of RB-PHYSICS-001-FR-029's own roof: a
        // ball fired straight up settles against the goal's own roof
        // instead of flying up to the main arena's much higher real
        // ceiling. Isolated to just the 2 roofs via `with_bounded_wall`,
        // for the same reason the back-wall test above is isolated -- see
        // its own doc comment.
        let mut ball = RigidBody::sphere(
            93.15,
            1.0,
            Vec3::new(
                0.0,
                crate::arena::BACK_WALL_Y + crate::arena::GOAL_DEPTH * 0.5,
                crate::arena::GOAL_HEIGHT * 0.5,
            ),
        );
        ball.restitution = 0.0;
        ball.linear_velocity = Vec3::new(0.0, 0.0, 400.0);

        let mut world = PhysicsWorld::new(ball, crate::arena::standard_ground());
        for mut wall in crate::arena::standard_goal_roofs() {
            wall.plane.restitution = 0.0;
            world = world.with_bounded_wall(wall);
        }
        world.gravity = Vec3::ZERO;

        let dt = 1.0 / 120.0;
        for _ in 0..(3.0 / dt) as u32 {
            world.step(dt);
        }

        assert!(
            world.ball.position.z > crate::arena::GOAL_HEIGHT * 0.5
                && world.ball.position.z < crate::arena::GOAL_HEIGHT + 5.0,
            "expected the ball to settle inside the goal box against its own roof, got z={}",
            world.ball.position.z
        );
    }

    #[test]
    fn a_ball_shot_at_a_goal_net_is_caught_instead_of_passing_through_untouched() {
        // The real end-to-end proof of RB-PHYSICS-001-FR-033's own net: a
        // ball fired straight at a lone net panel loses most of its speed,
        // unlike firing it through the exact same empty space with no net
        // present at all. Isolated to just the one net (`PhysicsWorld::new`
        // plus `with_net`, not `standard_arena`) for the same
        // full-arena-interference reason FR-029's own isolated proofs above
        // are isolated.
        let net_y = crate::arena::BACK_WALL_Y + crate::arena::NET_DEPTH;

        let run = |with_net: bool| -> f32 {
            let ball = RigidBody::sphere(93.15, 1.0, Vec3::new(0.0, net_y - 800.0, 300.0));
            let mut world = PhysicsWorld::new(ball, crate::arena::standard_ground());
            world.ball.linear_velocity = Vec3::new(0.0, 1500.0, 0.0);
            world.gravity = Vec3::ZERO;
            if with_net {
                world = world.with_net(crate::arena::standard_nets().remove(0));
            }
            let dt = 1.0 / 120.0;
            for _ in 0..(1.0 / dt) as u32 {
                world.step(dt);
            }
            world.ball.linear_velocity.y
        };

        let caught_speed = run(true);
        let free_flight_speed = run(false);
        assert!(
            caught_speed.abs() < free_flight_speed.abs() * 0.5,
            "expected the net to catch the ball, losing at least half its speed compared to \
             free flight, caught vy={caught_speed}, free-flight vy={free_flight_speed}"
        );
    }

    #[test]
    fn a_car_shot_at_a_goal_net_is_caught_instead_of_passing_through_untouched() {
        // RB-PHYSICS-001-FR-038: the same "caught vs. free flight" proof as
        // `a_ball_shot_at_a_goal_net_is_caught_instead_of_passing_through_untouched`,
        // but for a car — closing this port's own former Non-goal that "a
        // car still passes straight through a NetMesh's spatial footprint
        // untouched." Uses a car (`with_car`) instead of the scene's own
        // ball, with the ball placed far away so it can't also contact the
        // net and confound the measurement.
        let net_y = crate::arena::BACK_WALL_Y + crate::arena::NET_DEPTH;

        let run = |with_net: bool| -> f32 {
            let ball = RigidBody::sphere(93.15, 1.0, Vec3::new(10_000.0, 10_000.0, 10_000.0));
            let mut world = PhysicsWorld::new(ball, crate::arena::standard_ground());
            // z = 300, not resting on the ground: the net panel is centered
            // at `GOAL_HEIGHT * 0.5` (~321), so a car floating near the
            // panel's own vertical middle (matching the equivalent ball
            // test's own z=300 above) actually overlaps its free interior
            // points — a car resting flat on the ground at car-height would
            // only ever reach the panel's anchored bottom row, which
            // `NetMesh::step`'s own contact-resolution loop deliberately
            // skips (see its own doc comment), passing through untouched
            // for a reason unrelated to this requirement.
            let car = RigidBody::car_box(
                Vec3::new(60.0, 40.0, 20.0),
                1.0,
                Vec3::new(0.0, net_y - 800.0, 300.0),
            );
            world = world.with_car(car);
            world.cars[0].linear_velocity = Vec3::new(0.0, 1500.0, 0.0);
            world.gravity = Vec3::ZERO;
            if with_net {
                world = world.with_net(crate::arena::standard_nets().remove(0));
            }
            let dt = 1.0 / 120.0;
            for _ in 0..(1.0 / dt) as u32 {
                world.step(dt);
            }
            world.cars[0].linear_velocity.y
        };

        let caught_speed = run(true);
        let free_flight_speed = run(false);
        assert!(
            caught_speed.abs() < free_flight_speed.abs() * 0.5,
            "expected the net to catch the car, losing at least half its speed compared to \
             free flight, caught vy={caught_speed}, free-flight vy={free_flight_speed}"
        );
    }

    #[test]
    fn a_ball_embedded_in_a_goal_posts_fillet_footprint_is_pushed_toward_the_axis() {
        // The real end-to-end proof that a goal-cutout edge fillet
        // (RB-PHYSICS-001-FR-024) is live physical geometry, not just a
        // detection hack: a ball embedded past a post fillet's own radius
        // (deep in what would otherwise be the sharp, unrounded corner
        // between the flat back wall and the post's own inward-facing
        // plane) gets pushed back toward the axis -- the same live-physics
        // proof already given for every other fillet in this port. Checks
        // the ball settles at (not past) the fillet's own resting distance,
        // same reasoning as
        // `a_ball_embedded_in_a_vertical_corner_edges_fillet_footprint_is_pushed_toward_the_axis`.
        let wall = StaticPlane::new(Vec3::new(0.0, -1.0, 0.0), -1000.0);
        let post = StaticPlane::new(Vec3::new(-1.0, 0.0, 0.0), -200.0);
        let radius = 292.0;
        let curve = crate::body::StaticQuarterPipe::between_planes(
            &wall,
            &post,
            radius,
            Vec3::new(0.0, 0.0, 1.0),
        );

        let ball_radius = 93.15;
        let bisector = ((curve.sector_start + curve.sector_end) * 0.5)
            .normalize()
            .expect("sector_start and sector_end aren't exactly opposite, so their sum is nonzero");
        // Overlapping the fillet's own material by 10 units (further from
        // the axis than the resting distance, toward the sharp corner the
        // fillet replaces).
        let embedded_distance = curve.radius - ball_radius + 10.0;
        let embedded_position = curve.axis_point + bisector * embedded_distance;
        let mut ball = RigidBody::sphere(ball_radius, 1.0, embedded_position);
        ball.restitution = 0.0;

        let mut world = PhysicsWorld::new(ball, flat_ground())
            .with_wall(wall)
            .with_wall(post)
            .with_curve(curve);
        world.gravity = Vec3::ZERO;

        let dt = 1.0 / 120.0;
        for _ in 0..60 {
            world.step(dt);
        }

        let final_horizontal_rel = Vec3::new(
            world.ball.position.x - curve.axis_point.x,
            world.ball.position.y - curve.axis_point.y,
            0.0,
        );
        let final_dist = final_horizontal_rel.length();
        let resting_distance = curve.radius - ball_radius;
        assert!(
            (final_dist - resting_distance).abs() < 1.0,
            "expected the goal-post fillet to settle the ball at its resting distance \
             ({resting_distance}), started {embedded_distance} units out, got {final_dist}"
        );
    }

    #[test]
    fn a_ball_embedded_in_a_goal_corner_fillets_footprint_is_pushed_toward_the_center() {
        // The real end-to-end proof of RB-PHYSICS-001-FR-026: three planes
        // meeting at a single vertex -- here a back wall, a post plane, and
        // a crossbar plane, exactly like the compound corner where a goal
        // post's own fillet meets the crossbar's -- a ball embedded past
        // the fillet's own radius (deep in what would otherwise be the
        // sharp, unrounded corner) should be pushed back toward the
        // fillet's center, the same live-physics proof
        // `a_ball_embedded_in_a_compound_corner_fillets_footprint_is_pushed_toward_the_center`
        // already gives for the arena's own compound corners, now for a
        // goal's. Checks the ball settles at (not past) the fillet's own
        // resting distance, same reasoning as that test.
        let wall = StaticPlane::new(Vec3::new(0.0, -1.0, 0.0), -1000.0);
        let post = StaticPlane::new(Vec3::new(-1.0, 0.0, 0.0), -200.0);
        let crossbar = StaticPlane::new(Vec3::new(0.0, 0.0, -1.0), -600.0);
        let radius = 292.0;
        let fillet =
            crate::body::StaticCornerFillet::between_three_planes(&wall, &post, &crossbar, radius);

        let ball_radius = 93.15;
        let toward_corner = Vec3::new(1.0, 1.0, 1.0)
            .normalize()
            .expect("(1, 1, 1) is nonzero");
        // Overlapping the fillet's own material by 10 units (further from
        // the center than the resting distance, toward the sharp corner
        // the fillet replaces).
        let embedded_distance = fillet.radius - ball_radius + 10.0;
        let embedded_position = fillet.center + toward_corner * embedded_distance;
        let mut ball = RigidBody::sphere(ball_radius, 1.0, embedded_position);
        ball.restitution = 0.0;

        let mut world = PhysicsWorld::new(ball, flat_ground())
            .with_wall(wall)
            .with_wall(post)
            .with_wall(crossbar)
            .with_corner_fillet(fillet);
        world.gravity = Vec3::ZERO;

        let dt = 1.0 / 120.0;
        for _ in 0..60 {
            world.step(dt);
        }

        let final_dist = (world.ball.position - fillet.center).length();
        let resting_distance = fillet.radius - ball_radius;
        assert!(
            (final_dist - resting_distance).abs() < 1.0,
            "expected the goal corner fillet to settle the ball at its resting distance \
             ({resting_distance}), started {embedded_distance} units out, got {final_dist}"
        );
    }
}
