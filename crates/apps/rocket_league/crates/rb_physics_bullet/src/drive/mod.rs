//! Driven car input: couples `rb_domain::ControllerInput` into forces and
//! torques on a car `RigidBody`. Ground-driving (throttle, steering),
//! boost, handbrake/drift, a single ground jump, and air control in this
//! increment. Throttle, steering, handbrake, and jump are gated on the car
//! actually touching the ground (a free-floating box has no wheels to
//! grip, lock, or push off of, so airborne input does nothing for any of
//! them here); boost is not gated the same way — it's a rocket, not an
//! engine, so it still fires with no ground contact at all, unlike every
//! grounded-only input above. It isn't identical airborne, though: since
//! `RB-PHYSICS-001-FR-056`, its own acceleration magnitude is higher
//! airborne than grounded (`BOOST_ACCELERATION_AIR` vs
//! `BOOST_ACCELERATION_GROUND`), matching real Rocket League's own real
//! split — a claim this doc comment used to get wrong by rounding "not
//! gated on ground contact" up to "identical everywhere". Air control is
//! the mirror image of the *gating*, not the magnitude question above:
//! gated on the car *not* touching the ground (real air control needs no
//! wheels at all — it's pure torque, so it would be redundant with
//! steering while grounded).
//!
//! Tire grip and the handbrake (`RB-PHYSICS-001-FR-081`, ADR-0012): a
//! grounded car's box has no floor friction of its own. `ground`'s tire
//! model grips per axis instead, as RocketSim's wheels do: sideways grip
//! from `LAT_FRICTION_CURVE` by slip ratio, free rolling forward and
//! backward (coast-braking with no throttle). The handbrake cuts sideways
//! grip to a tenth and forward/backward grip to `0.5..0.9`, RocketSim's
//! own anisotropic factors, replacing the old isotropic friction
//! multiplier `RB-PHYSICS-001-FR-066` found had the wrong shape.
//!
//! Jump is a single, fixed-height vertical impulse fired on the *rising
//! edge* of `ControllerInput.jump` (a fresh press, not merely "held") while
//! grounded — holding the button through the resulting airborne period
//! doesn't re-fire it, and releasing then re-pressing while still airborne
//! doesn't fire it either (this increment has no double jump to grant).
//! Edge detection needs one bit of state to remember "was jump held as of
//! last step," carried by the caller in `DriveState::jump_held`, the same
//! pattern `DriveState::boost_amount` already uses for a resource that must
//! persist across calls.
//!
//! Air control (`RB-PHYSICS-001-FR-084`) is RocketSim's `_UpdateAirTorque`:
//! pitch, yaw and roll each give an angular acceleration of
//! `CAR_AIR_CONTROL_TORQUE * CAR_TORQUE_SCALE` per unit stick about the
//! car's -right, up and -forward axes, minus a per-axis damping of the
//! current spin (`CAR_AIR_CONTROL_DAMPING`, faded out for pitch and yaw as
//! their stick is held). A `None` analog value (replay-derived input, see
//! `rb_domain`) counts as a centered stick. Throttle adds a small forward
//! push in the air (`THROTTLE_AIR_ACCELERATION`). Air control is gated by
//! the flip (see **Flip** below). Since `RB-PHYSICS-001-FR-057`, spin is
//! capped at
//! `MAX_CAR_ANGULAR_SPEED`, a real confirmed Rocket League limit, once per
//! step, the same way `MAX_CAR_SPEED` already bounds linear speed.
//!
//! Double jump reuses the ground jump's own rising-edge detection
//! (`jump_pressed`) rather than a second edge-detector: the same fresh
//! press, while airborne, fires one more instantaneous impulse — gated on a
//! per-car `double_jump_available` flag instead of on `on_ground`. Landing
//! (any step where `on_ground` is true) unconditionally restores
//! availability; an airborne fresh press consumes it, so it can fire at
//! most once per airborne period no matter how many times jump is released
//! and re-pressed after that. That impulse is either a plain vertical
//! `JUMP_SPEED` kick, or a directional **dodge**, depending on the car's
//! `pitch`/`roll` stick input at the moment of the press: if either exceeds
//! `DODGE_DEADZONE`, a dodge fires instead — a purely horizontal
//! `DODGE_SPEED` impulse (along `forward_axis` for pitch, `right_axis` for
//! roll), followed by a flip (see **Flip** below), with RocketSim's signs:
//! stick forward (pitch -1) dodges forward and noses down
//! (`RB-PHYSICS-001-FR-082`). Since `RB-PHYSICS-001-FR-073`, the roll axis's own
//! stick value also includes `yaw` input (`roll + yaw`, each clamped to
//! `[-1.0, 1.0]` individually first) — matching RocketSim's own confirmed
//! `dodgeDir = (-pitch, yaw + roll, 0)`, so a yaw-only press (no roll held)
//! now fires a sideways dodge too, the same as a real Rocket League player
//! nudging the right stick purely left/right. Since `RB-PHYSICS-001-FR-075`,
//! `DODGE_DEADZONE`'s own trigger (`dodge_pitch.abs() > DODGE_DEADZONE ||
//! dodge_roll.abs() > DODGE_DEADZONE`) is confirmed the same decision as
//! RocketSim's own real cancellation check, once that fold-in is in place —
//! see `DODGE_DEADZONE`'s own doc comment for the full finding. Both pitch
//! and the combined roll/yaw can contribute at once (a diagonal dodge): since
//! `RB-PHYSICS-001-FR-072`, their combined
//! `(pitch, roll)` direction is normalized to unit length before scaling —
//! matching RocketSim's own confirmed real `dodgeDir.safeNormalized()`
//! step — so a diagonal dodge has the same total magnitude as an
//! axis-aligned one, not the larger, independently-summed magnitude a flat
//! per-axis sum would give; since `RB-PHYSICS-001-FR-074`, a near-axis-
//! aligned diagonal input additionally snaps to a pure single-axis dodge
//! (matching RocketSim's own post-normalization small-component zeroing)
//! instead of leaving a tiny, likely-unintentional perpendicular
//! component — see `normalize_dodge_direction`'s own doc
//! comment for the full finding. Since
//! `RB-PHYSICS-001-FR-059`, though, `DODGE_SPEED`'s own magnitude is no
//! longer flat regardless of direction or current speed: a pitch dodge
//! opposing the car's current forward-velocity direction, or any side
//! (roll) dodge, scales up as current speed rises toward `MAX_CAR_SPEED`
//! — see `dodge_speed_scale`'s own doc comment for the confirmed real
//! ratios and `dodge_pitch_is_backward`'s for the backward classification.
//! A dodge is purely horizontal (no vertical component, unlike the plain double
//! jump) — real Rocket League's dodge impulse does have a small upward
//! component too, not modeled here. Below `DODGE_DEADZONE` on both axes,
//! the plain vertical double jump fires exactly as before dodge existed.
//! Either way, the press still spends the one `double_jump_available` per
//! airborne period — a dodge and a plain double jump share the same
//! resource, matching real Rocket League. A dodge also starts a per-car
//! `DriveState::flip`.
//!
//! Wall jump is a *third* jump variant, alongside the ground jump and the
//! double jump: a fresh press while airborne *and* touching an arena wall
//! (`wall_normal: Some(normal)`, computed by the caller the same way
//! `on_ground` is — see `PhysicsWorld`) fires an impulse combining
//! `WALL_JUMP_HORIZONTAL_SPEED` outward along the wall's normal with
//! `JUMP_SPEED` upward (the same vertical speed the ground jump and double
//! jump use). Touching a wall — whether or not jump is pressed that step —
//! unconditionally restores `double_jump_available`, the same way landing
//! does, matching real Rocket League's "any surface contact refills your
//! second jump" rule; wall jump itself doesn't separately consume or
//! restore it, since contact already did. On a fresh press, wall contact
//! takes priority over consulting `double_jump_available` at all (checked
//! first in the airborne branch), so a player can wall-jump and still have
//! a double jump left afterward.
//! Wall jump has no per-wall-contact limit of its own: touching a (new or
//! the same) wall again always allows another wall jump, unlike the
//! double jump's once-per-airborne-period limit. Since
//! `RB-PHYSICS-001-FR-067`, real Rocket League is confirmed to have no
//! distinct wall-jump mechanic or constant at all — it's the identical
//! grounded-jump impulse applied along the car's own up axis, which tips
//! to match a touched wall via the wheel/suspension system this port's box
//! car doesn't have; see `WALL_JUMP_HORIZONTAL_SPEED`'s own doc comment for
//! the full finding and why this port's two-component substitute isn't
//! adopted away.
//!
//! Wall jump can itself be dodged off of: the same `pitch`/`roll`-vs-
//! `DODGE_DEADZONE` check the ground double jump uses is applied on a wall
//! jump's own fresh press too. Below the deadzone, the plain fixed
//! outward-plus-upward impulse fires exactly as before this existed. At or
//! above it, a **wall-jump dodge** fires instead: the same
//! outward-plus-upward push combined with a horizontal `DODGE_SPEED`
//! impulse and the same flip as a ground dodge. Unlike the plain
//! wall jump, a wall-jump dodge *does* consume `double_jump_available` —
//! the same resource a ground dodge spends — a deliberate simplification:
//! this port has no way to separately account for "a wall touch refilled
//! it, then the wall-jump dodge spent it" versus a genuinely independent
//! wall-dash resource, and real Rocket League's precise accounting here
//! isn't public to the precision this project would need to model that
//! distinction. Since touching a wall unconditionally restores
//! `double_jump_available` first (see above), a wall-jump dodge is never
//! blocked by an already-spent double jump — only its *stick input*, not
//! prior double-jump state, decides whether a wall-jump press dodges.
//!
//! The ground jump has variable height: continuing to hold `jump` after the
//! fresh press that fires it adds a continuous `JUMP_HOLD_ACCELERATION`
//! upward force, for up to `JUMP_HOLD_MAX_DURATION` seconds, on top of the
//! fixed `JUMP_SPEED` impulse — releasing early (or the window simply
//! running out) stops the extra acceleration, matching real Rocket League's
//! held-vs-tapped jump height difference. Since `RB-PHYSICS-001-FR-064`,
//! that release isn't *always* immediate: for the first `JUMP_MIN_TIME`
//! seconds after the press, the acceleration keeps applying regardless of
//! whether `jump` is still held — real Rocket League's own `_UpdateJump` has this same mandatory
//! minimum-hold quirk, so even an instantaneous tap gets a small amount of
//! extra height. Only past that mandatory window does releasing `jump` end
//! it right away. This is scoped to the ground jump alone: the double jump,
//! a dodge, and the wall jump are still each a single fixed instantaneous
//! impulse, completely unaffected by how long jump is held, since firing
//! any of them requires releasing jump first (a fresh press), which itself
//! unconditionally ends the ground jump's hold window (see
//! `apply_driven_forces`'s own doc comment for the exact ordering). Tracked
//! per car via `jump_hold_time_remaining`, the same kind of caller-owned
//! persisted state `jump_held`/`double_jump_available` already are.
//!
//! **Flip** (`RB-PHYSICS-001-FR-083`, ported from RocketSim's
//! `_UpdateAirTorque` and `_UpdateDoubleJumpOrFlip`): a dodge records its
//! direction and starts a flip clock (`DriveState::flip`). From the press
//! tick itself (`RB-PHYSICS-001-FR-085`, one tick earlier than RocketSim's
//! order, per the owner's capture) until `FLIP_TORQUE_TIME` (0.65 s) the flip spins the car at
//! `FLIP_TORQUE_FORWARD`/`FLIP_TORQUE_SIDE` (nose down for forward, right
//! side down for right), air control is off, and from 0.15 s the car's fall
//! is damped (`FLIP_Z_DAMP_*`). Holding pitch against the flip's forward
//! direction scales its pitch spin down by the stick amount and frees air
//! control: the flip cancel. A side flip has no pitch spin to cancel. Air
//! control's pitch stays locked until `FLIP_PITCHLOCK_EXTRA_TIME` after the
//! torque ends. Landing clears the flip. This replaces the port's earlier
//! instant spin kick and its jump-press-again cancel, which Rocket League
//! does not have (FR-069, FR-070).
//!
//! There is no airborne auto-upright: the port's earlier
//! `LANDING_AUTO_UPRIGHT_TORQUE` nudge was removed in
//! `RB-PHYSICS-001-FR-084`, since `RB-PHYSICS-001-FR-060` found Rocket
//! League has no such mechanic (its auto-flip and auto-roll are grounded
//! and input-gated, not modeled yet).
//!
//! A car with no input set (or all-neutral `ControllerInput::default()`)
//! behaves as a free rigid box except for tire grip while grounded (see
//! above) — this module only ever adds force/torque/impulse or a velocity
//! change, never removes physics outright.
//!
//! This is not a Bullet3 port (Bullet has no concept of "a car's engine")
//! — it's this project's own model of Rocket League's driving mechanics,
//! since the real numbers are not public, sourced instead from the
//! community reverse-engineering effort (RocketSim, RLUtilities, and the
//! RLBot wiki's independently-converging "Useful Game Values" — see
//! `RB-PHYSICS-001-FR-031`'s audit for the full source-by-source
//! breakdown). `MAX_CAR_SPEED`, `UNBOOSTED_MAX_CAR_SPEED`, `MAX_BOOST`,
//! `BOOST_ACCELERATION_GROUND`/`BOOST_ACCELERATION_AIR` (since
//! `RB-PHYSICS-001-FR-056` split the single flat `BOOST_ACCELERATION` this
//! bullet used to name into the two distinct values the same sources
//! actually cite), `JUMP_SPEED`, `JUMP_HOLD_MAX_DURATION`,
//! `JUMP_HOLD_ACCELERATION`, (since `RB-PHYSICS-001-FR-057`)
//! `MAX_CAR_ANGULAR_SPEED`, and (since `RB-PHYSICS-001-FR-064`)
//! `JUMP_MIN_TIME` are commonly-cited,
//! multi-source-confirmed community-reverse-engineered approximations (the
//! same body of public research `PhysicsWorld::new`'s gravity constant
//! comes from);
//! `BOOST_CONSUMPTION_RATE` is a simplified constant standing in for Rocket
//! League's real boost-drain behavior; `THROTTLE_ACCELERATION`'s own peak
//! magnitude is likewise a simplified, uncalibrated placeholder, but since
//! `RB-PHYSICS-001-FR-058` it's no longer applied flat — `drive_speed_taper`
//! scales it by RocketSim's own confirmed real curve shape as speed rises,
//! tapering smoothly to zero at `UNBOOSTED_MAX_CAR_SPEED` instead of a hard
//! cutoff (see that function's own doc comment for why the curve's shape,
//! unlike its peak magnitude, transfers cleanly). `DODGE_SPEED`'s own base
//! magnitude is likewise still an uncalibrated placeholder, but since
//! `RB-PHYSICS-001-FR-059` its per-direction scaling (a backward dodge
//! opposing current motion, or any side dodge, growing stronger as current
//! speed rises) matches RocketSim's own confirmed real ratios via
//! `dodge_speed_scale` — the same "shape confirmed, magnitude not" split
//! `THROTTLE_ACCELERATION` already has. Steering no longer uses a
//! placeholder torque: since `RB-PHYSICS-001-FR-086` the front wheels turn
//! by RocketSim's real steer-angle curves (`ground::steer_angle`) and each
//! wheel's side impulse yaws the car (ADR-0016, superseding FR-080's
//! kinematic yaw rate), resolving the wrong-shape finding
//! `RB-PHYSICS-001-FR-065` recorded.
//! The handbrake no longer uses a placeholder friction multiplier: since
//! `RB-PHYSICS-001-FR-081` it scales the tire model's per-axis grip by
//! RocketSim's real factors, resolving `RB-PHYSICS-001-FR-066`'s
//! wrong-shape finding. `WALL_JUMP_HORIZONTAL_SPEED` remains an uncalibrated
//! placeholder too, but since `RB-PHYSICS-001-FR-067` real Rocket League is
//! confirmed to have no distinct wall-jump mechanic or constant to
//! calibrate against at all — see that requirement's own entry and
//! `WALL_JUMP_HORIZONTAL_SPEED`'s own doc comment for the full finding.
//! Air control's magnitudes are RocketSim's own since
//! `RB-PHYSICS-001-FR-084` (FR-068 had adopted only their ratios).
//! (Historical, before `RB-PHYSICS-001-FR-083` replaced it with the real
//! flip torque:) `DODGE_ANGULAR_SPEED` remained an uncalibrated placeholder too,
//! but since `RB-PHYSICS-001-FR-069` real Rocket League's own dodge spin
//! is confirmed to be a continuous per-axis torque over a fixed 0.65s
//! window, not this port's own single instantaneous shared kick — a
//! confirmed-but-not-adopted finding in the same category as
//! `WALL_JUMP_HORIZONTAL_SPEED`,
//! since adopting the real shape would mean new per-car elapsed-flip-time
//! state, a substantially larger redesign `RB-PHYSICS-001-FR-059`'s own
//! Non-goals already flagged as out of scope — see that requirement's own
//! entry and `DODGE_ANGULAR_SPEED`'s own doc comment for the full finding.
//! `RB-PHYSICS-001-FR-031`'s audit
//! found real reference numbers for some of these (a dodge's real ~500
//! uu/s base impulse; a wall jump reusing the plain jump impulse rather
//! than its own faster speed, confirmed exact by `RB-PHYSICS-001-FR-067`;
//! real air-control torque/damping coefficients, whose per-axis ratio
//! `RB-PHYSICS-001-FR-068` later confirmed and adopted; a dodge's real
//! spin torque and duration, whose exact mechanism `RB-PHYSICS-001-FR-069`
//! later confirmed), but none of the
//! remaining raw absolute values port directly: they're expressed as
//! torques or velocity-dependent curves calibrated against real Rocket
//! League's own specific car mass/inertia tensor and mechanic shape,
//! neither of which this port's own placeholder car body or simplified
//! single-impulse mechanics are calibrated to match, so adopting the raw
//! numbers here would be false precision, not a real fix — see the
//! audit's own findings for detail. `MAX_CAR_ANGULAR_SPEED` (and, since
//! `RB-PHYSICS-001-FR-059`, `DODGE_SPEED`'s own per-direction scale
//! ratios) don't have that problem even though they also bound
//! rotation/velocity: they cap or scale the *result* (a rad/s or uu/s
//! quantity) rather than prescribing the torque or force that produces
//! it, so they transfer cleanly regardless of this port's own car
//! body/inertia tensor not matching real Rocket League's — see
//! `RB-PHYSICS-001-FR-057`'s own findings for why that distinction let
//! these constants clear the bar the torque-based placeholders above
//! couldn't. Aside from those exceptions, none of these are independently
//! confirmed by this project — see `RB-PHYSICS-001-FR-005`/`FR-031`.

mod air;
mod boost;
mod ground;
mod jump;
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;
mod wheels;

pub use boost::MAX_BOOST;
pub use wheels::{cast_wheels, is_on_ground, WheelContact, WheelContacts, NO_WHEEL_CONTACTS};

/// The no-slip bicycle-model yaw rate for `ground::steer_angle` over the
/// Octane wheelbase: the turn rate grip-limited steering approaches, for
/// sibling modules' tests only.
#[cfg(test)]
pub(crate) fn bicycle_yaw_rate_for_tests(
    forward_speed: f32,
    steer: f32,
    handbrake_amount: f32,
) -> f32 {
    let wheelbase = ground::FRONT_AXLE_X - ground::REAR_AXLE_X;
    forward_speed * ground::steer_angle(forward_speed, steer, handbrake_amount).tan() / wheelbase
}
pub use jump::{FlipState, DODGE_SPEED, JUMP_SPEED, WALL_JUMP_HORIZONTAL_SPEED};

use crate::body::RigidBody;
use rb_domain::{ControllerInput, Vec3};

/// Commonly-cited boosted top speed (uu/s), boost's own speed cap here —
/// confirmed against real Rocket League's own reverse-engineered constants
/// during `RB-PHYSICS-001-FR-031`'s audit (`CAR_MAX_SPEED = 2300.f` in the
/// RocketSim project's `RLConst.h`; matched independently by RLUtilities'
/// `Car::v_max` and the RLBot community wiki's "Useful Game Values" page).
pub const MAX_CAR_SPEED: f32 = 2300.0;

/// Hard cap (rad/s) on a car's angular speed, enforced by
/// `clamp_angular_speed` once per step, right after
/// `integrate::integrate_velocities` — a genuine clamp that scales
/// `angular_velocity` back down if it's exceeded, unlike `MAX_CAR_SPEED`/
/// `UNBOOSTED_MAX_CAR_SPEED` above (which only gate *new* throttle/boost
/// force, never reduce velocity already past the cap). Confirmed exact
/// against RocketSim's own `RLConst.h` during `RB-PHYSICS-001-FR-057`'s
/// audit: `CAR_MAX_ANG_SPEED = 5.5f, // Car can never exceed this angular
/// velocity (radians/s)`.
///
/// A flip's torque (`jump::FLIP_TORQUE_*`, `RB-PHYSICS-001-FR-083`)
/// reaches this cap within a few ticks, as in RocketSim.
///
/// Only covers this port's own driven-forces sources (continuous air
/// control torque integrated this step, plus any single-step direct
/// `angular_velocity` write like the flip torque or the wheels' side
/// impulses) — a same-step contact-solver impulse (e.g. a hard collision
/// imparting spin) isn't re-clamped until the *next* step's call, so it
/// could in principle transiently exceed this for one step, unlike
/// RocketSim's own "can never exceed" phrasing suggests for its engine.
/// Closing that remaining gap would mean clamping again after the solver
/// too, which this port doesn't do — out of scope for FR-057.
pub const MAX_CAR_ANGULAR_SPEED: f32 = 5.5;

/// Commonly-cited *unboosted* top speed (uu/s) — throttle's own speed cap,
/// distinct from `MAX_CAR_SPEED`. Before `RB-PHYSICS-001-FR-031`'s audit,
/// throttle alone could push a car all the way to `MAX_CAR_SPEED` (2300),
/// which is real Rocket League's *boosted* cap, not its unboosted one; the
/// audit found a consistent, independently-corroborated real value (1410)
/// across RocketSim's `RLConst.h` — whose `DRIVE_SPEED_TORQUE_FACTOR_CURVE`
/// drives available drive torque to exactly zero at 1410 uu/s — and the
/// RLBot community wiki's "Useful Game Values" page, so throttle now caps
/// here instead.
pub const UNBOOSTED_MAX_CAR_SPEED: f32 = 1410.0;

fn forward_axis(car: &RigidBody) -> Vec3 {
    car.orientation.rotate(&Vec3::new(1.0, 0.0, 0.0))
}

fn up_axis(car: &RigidBody) -> Vec3 {
    car.orientation.rotate(&Vec3::new(0.0, 0.0, 1.0))
}

/// The car's local "right" axis (local +Y) — completes the right-handed
/// (forward, right, up) basis `up_axis × forward_axis` gives: the wheels'
/// axles, air-control pitch, and the flip's pitch torque.
fn right_axis(car: &RigidBody) -> Vec3 {
    car.orientation.rotate(&Vec3::new(0.0, 1.0, 0.0))
}

/// `RB-PHYSICS-001-FR-037` — whether `input` represents genuine driving
/// intent, for waking a sleeping car (see `apply_driven_forces`'s own doc
/// comment). Treats an unrecovered analog channel (`None`, from a replay
/// that never had one) the same as a recovered-but-literally-neutral one
/// (`Some(0.0)`, from a capture) — both mean "no analog input this tick" —
/// rather than the simpler `*input != ControllerInput::default()` (which
/// would treat any `Some(0.0)` as active purely because it's not `None`,
/// keeping a car receiving a real recorded input stream that always
/// resolves every channel — even at rest — from ever sleeping at all).
fn input_is_active(input: &ControllerInput) -> bool {
    input.throttle != 0.0
        || input.steer != 0.0
        || input.pitch.unwrap_or(0.0) != 0.0
        || input.yaw.unwrap_or(0.0) != 0.0
        || input.roll.unwrap_or(0.0) != 0.0
        || input.jump
        || input.boost
        || input.handbrake
}

/// Per-car state `apply_driven_forces` carries from one step to the next.
/// `PhysicsWorld` keeps one per car; `DriveState::new` gives the defaults
/// for a freshly added car (full boost, jump released, double jump
/// available, no hold window, no dodge flip).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DriveState {
    /// Remaining boost fuel, `0.0..=MAX_BOOST`. Drained while boost is held,
    /// even when the force itself doesn't apply.
    pub boost_amount: f32,
    /// The car's `input.jump` as of the *previous* call. Every jump variant
    /// fires only on a rising edge (`input.jump && !jump_held`), so a
    /// continued press doesn't re-fire every step; updated on every call,
    /// including while airborne, so a fresh press is still required for a
    /// double or wall jump even if the button was never released after the
    /// ground jump.
    pub jump_held: bool,
    /// Whether the car still has a double jump (plain or dodge) to spend
    /// this airborne period. Landing or merely touching a wall (no jump
    /// press required) both set it back to `true`; only an airborne fresh
    /// press that fires the double jump or a dodge (including a wall-jump
    /// dodge, not a plain wall jump) sets it to `false`.
    pub double_jump_available: bool,
    /// How much longer, in seconds, continuing to hold `jump` keeps adding
    /// extra upward acceleration to a ground jump. Checked and decremented
    /// *before* this call's own ground-jump press can re-arm it; the press
    /// re-arms it to `JUMP_HOLD_MAX_DURATION` and spends its first tick
    /// (`RB-PHYSICS-001-FR-091`). Since `RB-PHYSICS-001-FR-064`, releasing
    /// `jump` inside the first `JUMP_MIN_TIME` seconds doesn't zero it: that
    /// mandatory window keeps applying the full acceleration.
    /// Untouched by the double jump, a dodge, or the wall jump.
    pub jump_hold_time_remaining: f32,
    /// The dodge flip since the last dodge press, until landing
    /// (`RB-PHYSICS-001-FR-083`): drives the flip torque, vertical damping,
    /// and air-control lock. `None` on the ground and before any dodge.
    pub flip: Option<FlipState>,
    /// How engaged the handbrake is, `0.0..=1.0`: ramps up while held and
    /// down once released (`ground::POWERSLIDE_RISE_RATE`/`FALL_RATE`),
    /// scaling the tire grip reduction and the powerslide steer blend.
    pub handbrake_amount: f32,
}

impl DriveState {
    /// Defaults for a freshly added car.
    pub fn new() -> DriveState {
        DriveState {
            boost_amount: MAX_BOOST,
            jump_held: false,
            double_jump_available: true,
            jump_hold_time_remaining: 0.0,
            flip: None,
            handbrake_amount: 0.0,
        }
    }
}

impl Default for DriveState {
    fn default() -> DriveState {
        DriveState::new()
    }
}

/// Applies throttle, steering, boost, handbrake, jump, double jump, wall
/// jump, and air control as forces/torques/impulses (or, for tire grip,
/// direct velocity changes) on `car`, reading and updating `state`
/// (see each `DriveState` field for its rules). `wheels` are the wheel
/// rays' hits cast at the start of the step (`cast_wheels`); the car is on
/// the ground when at least three touch (`is_on_ground`); their suspension
/// is `apply_wheel_forces`, called separately. Throttle, steering,
/// handbrake, and the ground jump are a no-op unless `on_ground`; air
/// control, double jump, and wall jump are the reverse — a no-op unless
/// *not* `on_ground`; boost isn't gated on ground contact at all, but is a
/// no-op once `state.boost_amount` reaches zero. `wall_normal` is the
/// outward normal of the wall the car is currently touching, if any
/// (computed by the caller the same way `on_ground` is — see
/// `PhysicsWorld`); a fresh press while airborne and touching a wall fires
/// a wall jump instead of consulting `double_jump_available` at all. Since
/// `RB-PHYSICS-001-FR-037`, any genuinely active `input` (see
/// `input_is_active`) wakes `car` unconditionally before anything else in
/// this call runs, regardless of whether `car` was already asleep or what
/// velocity results this step — see this crate's own
/// `body::RigidBody::wake` doc comment for why a velocity-only wake check
/// isn't enough here. Call once per step, before
/// `integrate::integrate_velocities`, alongside `apply_gravity`; follow it
/// with `clamp_angular_speed` right *after* that same
/// `integrate_velocities` call, so `MAX_CAR_ANGULAR_SPEED` sees this step's
/// fully-integrated angular velocity, torque contributions included.
pub fn apply_driven_forces(
    car: &mut RigidBody,
    input: &ControllerInput,
    wheels: &WheelContacts,
    wall_normal: Option<Vec3>,
    state: &mut DriveState,
    dt: f32,
) {
    let on_ground = is_on_ground(wheels);
    // RB-PHYSICS-001-FR-037: any genuinely active input wakes the car
    // unconditionally, before that input's own force/impulse has a chance
    // to move it — a resultant-velocity-only wake check would zero right
    // back out a driving force whose one-frame delta is itself smaller
    // than `body::LINEAR_SLEEP_VELOCITY_THRESHOLD` (e.g. one frame of
    // throttle from a dead stop at a very small `dt`), permanently
    // stranding an asleep car that should be free to start moving.
    if input_is_active(input) {
        car.wake();
    }

    let forward = forward_axis(car);
    let jump_pressed = input.jump && !state.jump_held;
    state.jump_held = input.jump;

    jump::apply_jump_hold(car, input.jump, &mut state.jump_hold_time_remaining, dt);
    state.handbrake_amount = ground::ramp_handbrake(state.handbrake_amount, input.handbrake, dt);

    let throttle = effective_throttle(input, state);
    if on_ground {
        // Landing (or simply resting) always restores the double jump,
        // regardless of this step's input.
        state.double_jump_available = true;
        state.flip = None;
        ground::apply_ground_control(
            car,
            wheels,
            input,
            throttle,
            forward,
            state.handbrake_amount,
            dt,
        );
        if jump_pressed {
            jump::ground_jump(car, &mut state.jump_hold_time_remaining, dt);
        }
    } else {
        if wall_normal.is_some() {
            // Touching a wall restores the double jump unconditionally —
            // the same "any surface contact refills your second jump"
            // rule landing uses — regardless of whether jump is pressed.
            state.double_jump_available = true;
        }
        // The jump press first, so a dodge's flip torque acts on the press
        // tick itself: the owner's capture shows the real game's spin jump
        // by one tick of flip torque on that tick (RB-PHYSICS-001-FR-085),
        // one tick earlier than RocketSim's order. Then flip torque and air
        // control, then the flip clock and damping.
        if jump_pressed {
            jump::airborne_jump_press(
                car,
                input,
                forward,
                wall_normal,
                &mut state.double_jump_available,
                &mut state.flip,
            );
        }
        let gate = jump::apply_flip_torque(car, input, state.flip);
        if gate.enabled {
            air::apply_air_control(car, input, gate.pitch_scale, dt);
        }
        air::apply_air_throttle(car, input.throttle.clamp(-1.0, 1.0), forward);
        jump::advance_flip(car, &mut state.flip, dt);
    }

    boost::apply_boost(
        car,
        input.boost,
        on_ground,
        forward,
        &mut state.boost_amount,
        dt,
    );
}

/// Throttle as the pedals see it: RocketSim treats a boosting car with
/// boost left as full throttle.
fn effective_throttle(input: &ControllerInput, state: &DriveState) -> f32 {
    if input.boost && state.boost_amount > 0.0 {
        1.0
    } else {
        input.throttle.clamp(-1.0, 1.0)
    }
}

/// Every touching wheel's suspension and the sticky force for one step
/// (`RB-PHYSICS-001-FR-090`, `wheels::apply_wheel_forces`), from `wheels`
/// cast at the start of the step. Call after `apply_driven_forces`, as
/// RocketSim's `updateVehicleSecond` follows its drive logic.
pub fn apply_wheel_forces(
    car: &mut RigidBody,
    input: &ControllerInput,
    wheels: &WheelContacts,
    state: &DriveState,
    dt: f32,
) {
    wheels::apply_wheel_forces(car, wheels, effective_throttle(input, state) != 0.0, dt);
}

/// Scales `car.angular_velocity` back down to `MAX_CAR_ANGULAR_SPEED` if
/// its length exceeds it, preserving direction — a no-op otherwise. Call
/// once per step at its very end, after the transform has integrated
/// (`RB-PHYSICS-001-FR-087`): RocketSim clamps in `Car::_PostTickUpdate`,
/// after Bullet's step, so a step's orientation moves with the unclamped
/// spin. The real capture confirms it: a flipping car turns at ~7.6 rad/s
/// (5.5 plus one tick of flip torque) while reporting 5.5.
pub fn clamp_angular_speed(car: &mut RigidBody) {
    let speed = car.angular_velocity.length();
    if speed > MAX_CAR_ANGULAR_SPEED {
        car.angular_velocity *= MAX_CAR_ANGULAR_SPEED / speed;
    }
}
