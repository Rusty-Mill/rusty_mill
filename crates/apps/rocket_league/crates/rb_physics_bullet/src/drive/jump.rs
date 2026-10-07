//! Jumping: the variable-height hold window, the ground jump's impulse, and
//! the airborne presses (wall jump, wall-jump dodge, double jump, dodge),
//! and the dodge's flip (torque, cancel, vertical damping). See the parent
//! module doc comment for the full rule set.

use super::{forward_axis, right_axis, up_axis, MAX_CAR_SPEED};
use crate::body::RigidBody;
use rb_domain::{ControllerInput, Vec3};

/// Jump impulse speed (uu/s), applied as an instantaneous vertical
/// velocity change (not a continuous force) on a fresh grounded jump
/// press — a flat speed regardless of the car's mass, matching how the
/// real jump impulse doesn't scale with car mass either. Also reused as
/// the double jump's impulse magnitude (see the module doc comment) —
/// `pub` so `world.rs`'s end-to-end tests can assert against it directly,
/// the same way `MAX_CAR_SPEED`/`MAX_BOOST` already are. Refined from an
/// earlier `292.0` approximation to the precise value during
/// `RB-PHYSICS-001-FR-031`'s audit: RocketSim's `RLConst.h` defines
/// `JUMP_IMMEDIATE_FORCE = 875.f/3.f`, and RLUtilities independently
/// hardcodes `Jump::speed = 291.667f` — the same number both projects
/// also apply, unmodified, to the double jump.
pub const JUMP_SPEED: f32 = 875.0 / 3.0;

/// Uncalibrated placeholder wall-jump horizontal push-off speed (uu/s),
/// applied outward along the wall's normal as an instantaneous velocity
/// change (like `JUMP_SPEED`, not a continuous force) on a fresh airborne
/// jump press while touching a wall — chosen only to produce a visibly
/// distinct push-off from the wall in tests, not derived from any measured
/// or documented Rocket League value. Unlike `JUMP_SPEED`, this port has no
/// public reference for a wall-jump-specific number to reuse. `pub` so
/// `world.rs`'s end-to-end tests can assert against it directly (including
/// distinguishing a wall jump from the differently-sized `DODGE_SPEED`),
/// the same way `JUMP_SPEED` already is.
///
/// `RB-PHYSICS-001-FR-067` looked for that missing reference directly:
/// fetching RocketSim's real `Car.cpp` found real Rocket League has no
/// separate wall-jump mechanic — or constant — at all. `_UpdateJump`
/// applies exactly one impulse, `GetUpDir() * mutatorConfig.jumpImmediateForce`
/// (the same real value this port's own `JUMP_SPEED` already matches),
/// gated only on `isOnGround`, itself defined purely by wheel-contact count
/// (`numWheelsInContact >= 3`) with no floor-vs-wall distinction at all; a
/// dedicated search of `RLConst.h` for any `WALL`-named constant found only
/// an unrelated Heatseeker-mode threshold. Since `RB-PHYSICS-001-FR-065`
/// already confirmed real Rocket League's cars ride on Bullet's own
/// raycast vehicle system (`btVehicleRL`), a car driving on a wall has its
/// own orientation continuously tipped to match that wall by ordinary
/// wheel/suspension contact forces, the same way a real car tilts to match
/// a ramp — so `GetUpDir()` (the car's own local up axis in world space)
/// already points along the wall's outward normal by the time a wall jump
/// fires, with no special-cased direction logic needed. Real Rocket
/// League's "wall jump" is thus the *identical* single grounded-jump
/// impulse, along whatever direction the car's own up axis currently
/// points — never a distinct horizontal-plus-vertical composite with its
/// own separate magnitude. This confirms, with the exact mechanism rather
/// than just the constant's absence, what `RB-PHYSICS-001-FR-031`'s
/// original audit only briefly noted as "a wall jump reusing the plain
/// jump impulse rather than its own faster speed."
///
/// Not adopted as a fix: this port's car has no wheels, raycasting, or
/// surface-tracking orientation system at all (the same architecture gap
/// `RB-PHYSICS-001-FR-065` found for steering) — its orientation doesn't
/// automatically tip to match a touched wall, so its own up axis stays
/// world-vertical throughout a wall touch. Applying only `JUMP_SPEED`
/// straight up on a wall touch, as the confirmed real mechanism would
/// suggest, would produce no push-off from the wall at all in this port,
/// defeating the entire point of a wall jump. This port's own two-component
/// composite (a separate horizontal push-off along the wall's normal, on
/// top of the same vertical `JUMP_SPEED`) remains a deliberate, necessary
/// substitute for the missing surface-tracking orientation mechanism, not
/// an unfilled calibration gap — `WALL_JUMP_HORIZONTAL_SPEED`'s own
/// magnitude is still an uncalibrated placeholder, but the two-component
/// shape itself is not a mistake to correct.
pub const WALL_JUMP_HORIZONTAL_SPEED: f32 = 550.0;

/// RocketSim's direction cancel: a flip whose `(forward, side)` parts are
/// both under this is a stall, with no impulse and no torque. Since
/// `RB-PHYSICS-001-FR-104` it no longer decides flip versus double jump;
/// `FLIP_INPUT_DEADZONE` does. The FR-075 note below took this cancel for
/// that decision, which made yaw against air roll a double jump.
///
/// `RB-PHYSICS-001-FR-075` found this port's own long-standing "not a
/// physics constant and not derived from any Rocket League value" framing
/// was stale: RocketSim's own confirmed `_UpdateDoubleJumpOrFlip`
/// cancellation check (`RB-PHYSICS-001-FR-072`/`FR-073`/`FR-074`'s own
/// fetches) is `if (abs(controls.yaw + controls.roll) < 0.1f &&
/// abs(controls.pitch) < 0.1f) { dodgeDir = {0,0,0}; }` — a dodge fires iff
/// `abs(yaw + roll) >= 0.1 || abs(pitch) >= 0.1`. Since `FR-073` already
/// folds yaw into this port's own `dodge_roll`/`wall_roll` (`roll + yaw`),
/// this port's own trigger — `dodge_pitch.abs() > DODGE_DEADZONE ||
/// dodge_roll.abs() > DODGE_DEADZONE` — is the *same* boolean expression as
/// the real one once `DODGE_DEADZONE` equals the real threshold, up to an
/// immaterial strict-vs-non-strict inequality at the exact boundary value
/// (a floating-point edge case with no practical effect). `0.1` is exactly
/// that real threshold: this constant already matches real Rocket League,
/// it just wasn't confirmed as such until this fetch. See
/// `normalize_dodge_direction`'s own doc comment for the related, already-
/// adopted normalization/snap findings this same cancellation check feeds
/// into.
pub(super) const DODGE_DEADZONE: f32 = 0.1;

/// Real confirmed threshold (`RB-PHYSICS-001-FR-074`): RocketSim's own
/// `_UpdateDoubleJumpOrFlip` zeroes any component of the *normalized*
/// `dodgeDir` whose absolute value is below `0.1`, applied after
/// `dodgeDir.safeNormalized()` — `if (abs(dodgeDir.x()) < 0.1f)
/// dodgeDir.x() = 0; if (abs(dodgeDir.y()) < 0.1f) dodgeDir.y() = 0;` —
/// snapping a near-axis-aligned diagonal stick input (e.g. 88 degrees
/// instead of a clean 90) to a pure single-axis dodge instead of leaving a
/// tiny, likely-unintentional perpendicular component from imprecise stick
/// centering. Numerically identical to `DODGE_DEADZONE` above, but a
/// distinct real constant serving a different purpose (a pre-normalization
/// raw-stick trigger threshold vs. a post-normalization direction-snap
/// threshold) — kept as its own name rather than reusing `DODGE_DEADZONE`,
/// so the two can diverge if either is ever recalibrated independently.
/// See `normalize_dodge_direction`'s own doc comment for the adoption
/// reasoning.
pub(super) const DODGE_DIRECTION_SNAP_THRESHOLD: f32 = 0.1;

/// Dodge horizontal impulse speed (uu/s), applied along `forward_axis`
/// and/or `right_axis` by the normalized dodge direction as an
/// instantaneous velocity change: RocketSim's `FLIP_INITIAL_VEL_SCALE`.
/// Long an uncalibrated `1400`; `RB-PHYSICS-001-FR-082` adopted the real
/// value after the owner's capture recorded a ~621 uu/s dodge where
/// RocketSim's formula with `500` predicts ~628. `pub` so `world.rs`'s
/// end-to-end tests can assert against it. Backward and side dodges at
/// speed scale above this via `dodge_speed_scale`.
pub const DODGE_SPEED: f32 = 500.0;

/// Confirmed real ratio: a *backward* pitch-dodge (one opposing the car's
/// own current forward-velocity direction, per `dodge_pitch_is_backward`)
/// grows up to this multiple of `DODGE_SPEED` as current speed rises
/// toward `MAX_CAR_SPEED` — RocketSim's own `Car.cpp`
/// (`_UpdateDoubleJumpOrFlip`) confirmed exact against `RLConst.h`:
/// `FLIP_BACKWARD_IMPULSE_MAX_SPEED_SCALE = 2.5f`. A forward pitch-dodge's
/// own real scale is exactly `1.0` (`FLIP_FORWARD_IMPULSE_MAX_SPEED_SCALE`)
/// — unchanged from `DODGE_SPEED`'s own base value — so there's no
/// separate forward-scale constant here. Only this *ratio* is adopted;
/// RocketSim's own real base magnitude (`FLIP_INITIAL_VEL_SCALE = 500.f`,
/// which the forward case corresponds to one-to-one) is deliberately not
/// substituted for `DODGE_SPEED` itself — see this constant's own Non-goals
/// in `RB-PHYSICS-001-FR-059`'s own Requirements entry for why.
pub(super) const DODGE_BACKWARD_SPEED_SCALE: f32 = 2.5;

/// Extra forward/backward scale on a backward dodge: RocketSim's
/// `FLIP_BACKWARD_IMPULSE_SCALE_X = 16 / 15`.
pub(super) const DODGE_BACKWARD_SCALE_X: f32 = 16.0 / 15.0;

/// Confirmed real ratio: a side (`roll`) dodge grows up to this multiple of
/// `DODGE_SPEED` as current speed rises toward `MAX_CAR_SPEED`, regardless
/// of left/right direction — RocketSim's own confirmed
/// `FLIP_SIDE_IMPULSE_MAX_SPEED_SCALE = 1.9f`. See `DODGE_BACKWARD_SPEED_SCALE`'s
/// own doc comment for the same "ratio adopted, base magnitude not"
/// caveat.
pub(super) const DODGE_SIDE_SPEED_SCALE: f32 = 1.9;

/// Below this current forward speed (uu/s), `dodge_pitch_is_backward`
/// falls back to stick direction alone rather than comparing it against a
/// near-zero, noisy current-velocity direction — RocketSim's own
/// confirmed threshold, `abs(forwardSpeed_UU) < 100.0f`.
pub(super) const DODGE_BACKWARD_CLASSIFICATION_SPEED_THRESHOLD: f32 = 100.0;

/// A dodge's real per-axis magnitude scale as a function of the car's
/// current forward speed — confirmed against RocketSim's own `Car.cpp`
/// during `RB-PHYSICS-001-FR-059`'s audit: `1.0` (no change) at a standing
/// start, rising linearly to `scale_at_max_speed` by `MAX_CAR_SPEED`, then
/// held flat beyond it (`forward_speed` can in principle exceed
/// `MAX_CAR_SPEED` transiently, e.g. after a boosted speed-flip chain;
/// RocketSim's own real ratio isn't itself clamped in the source, but this
/// port clamps it here rather than let an already-uncalibrated dodge
/// magnitude grow unbounded past the one confirmed reference point).
pub(super) fn dodge_speed_scale(forward_speed: f32, scale_at_max_speed: f32) -> f32 {
    let ratio = (forward_speed.abs() / MAX_CAR_SPEED).min(1.0);
    1.0 + (scale_at_max_speed - 1.0) * ratio
}

/// Whether a pitch-dodge counts as "backward" for `DODGE_BACKWARD_SPEED_SCALE`
/// purposes: opposing the car's own current forward-velocity direction: at
/// or above `DODGE_BACKWARD_CLASSIFICATION_SPEED_THRESHOLD`, that means
/// `dodge_pitch` and `forward_speed` disagree in sign (dodging forward
/// while already moving backward counts as "backward" too, the same as
/// the more common backward-dodge-while-moving-forward case — both oppose
/// current motion); below it, classification falls back to `dodge_pitch`'s
/// own sign alone, since comparing against a near-zero velocity direction
/// would be noise. Confirmed against RocketSim's own `Car.cpp`
/// (`shouldDodgeBackwards`): `dodge_pitch` is the dodge's forward
/// component, RocketSim's `dodgeDir.x` (`-controls.pitch`, see
/// `dodge_stick`).
pub(super) fn dodge_pitch_is_backward(dodge_pitch: f32, forward_speed: f32) -> bool {
    if forward_speed.abs() < DODGE_BACKWARD_CLASSIFICATION_SPEED_THRESHOLD {
        dodge_pitch < 0.0
    } else {
        (dodge_pitch >= 0.0) != (forward_speed >= 0.0)
    }
}

/// Normalizes a dodge's combined `(pitch, roll)` stick direction to unit
/// length, returning `(0.0, 0.0)` if both are exactly zero. `DODGE_SPEED`
/// is scaled by this normalized pair instead of the raw stick values, so a diagonal dodge (both axes held) has the same
/// total magnitude as an axis-aligned one.
///
/// `RB-PHYSICS-001-FR-059`'s own Non-goals had already found and flagged
/// this exact gap — this port previously summed each axis' own
/// full-strength contribution independently, so a diagonal dodge came out
/// `sqrt(2)`-ish times faster than an axis-aligned one, "a separate,
/// independent behavioral question this requirement doesn't take on."
/// `RB-PHYSICS-001-FR-072` fetched RocketSim's own `Car.cpp`
/// (`_UpdateDoubleJumpOrFlip`) and confirmed the real mechanism: `dodgeDir
/// = btVector3(-controls.pitch, controls.yaw + controls.roll,
/// 0).safeNormalized()`, applied before any further per-axis
/// forward/backward/side speed scaling
/// (`dodge_speed_scale`/`dodge_pitch_is_backward`, `FR-059`'s own already-
/// adopted finding). Unlike that per-direction *speed* ratio's own real
/// absolute magnitude (independently uncalibrated, per `FR-031`'s "false
/// precision" reasoning), normalization is a pure geometric operation this
/// port's own model represents exactly — it transfers cleanly regardless
/// of `DODGE_SPEED`'s own uncalibrated base value, the same way
/// `FR-058`/`FR-059`/`FR-068`'s own adopted ratios do.
///
/// Its `pitch` argument is the dodge's *forward* component, already
/// negated from the stick by `dodge_stick` (RocketSim's `-controls.pitch`,
/// `RB-PHYSICS-001-FR-082`); before FR-082 the raw stick pitch was used, so
/// a stick-forward dodge went backward.
///
/// Real yaw input's own contribution to `dodgeDir` (`controls.yaw +
/// controls.roll`) *is* folded in, though not by this function itself:
/// since `RB-PHYSICS-001-FR-073`, both call sites in `apply_driven_forces`
/// pass `roll + yaw` (each individually clamped to `[-1.0, 1.0]` first,
/// matching how `apply_driven_forces` already clamps pitch/yaw/roll
/// separately for air control) as this function's `roll` argument — this
/// port already reads `input.yaw` in this same function for air control, so
/// no new input plumbing was needed, just combining an already-available
/// value the same way the reference does. `RB-PHYSICS-001-FR-059`'s own
/// Non-goals had flagged this exact gap ("this port's dodge direction is
/// pitch/roll only").
///
/// Since `RB-PHYSICS-001-FR-074`, the returned pair also matches
/// RocketSim's own post-normalization small-component zeroing: after
/// normalizing, any component whose magnitude falls below
/// `DODGE_DIRECTION_SNAP_THRESHOLD` (real value `0.1`, confirmed identical
/// to `dodgeDir.x()`/`dodgeDir.y()`'s own zeroing in `_UpdateDoubleJumpOrFlip`)
/// is snapped to exactly zero — a near-axis-aligned diagonal stick input
/// no longer leaves a tiny, physically negligible but real perpendicular
/// dodge component. `RB-PHYSICS-001-FR-073`'s own Non-goals had flagged
/// this as a "separate architectural difference," but it isn't one: like
/// normalization itself, it's a pure post-processing step on the already-
/// normalized pair this function already computes, needing no new
/// machinery — the same "pure operation, no new architecture" transfer
/// this function's own earlier finding already used.
pub(super) fn normalize_dodge_direction(pitch: f32, roll: f32) -> (f32, f32) {
    let magnitude = (pitch * pitch + roll * roll).sqrt();
    if magnitude > 0.0 {
        let mut norm_pitch = pitch / magnitude;
        let mut norm_roll = roll / magnitude;
        if norm_pitch.abs() < DODGE_DIRECTION_SNAP_THRESHOLD {
            norm_pitch = 0.0;
        }
        if norm_roll.abs() < DODGE_DIRECTION_SNAP_THRESHOLD {
            norm_roll = 0.0;
        }
        (norm_pitch, norm_roll)
    } else {
        (0.0, 0.0)
    }
}

/// How long (s) a dodge's flip torque acts: RocketSim's `FLIP_TORQUE_TIME`.
pub(super) const FLIP_TORQUE_TIME: f32 = 0.65;

/// Flip angular acceleration (rad/s^2 per unit dodge direction at 120 Hz)
/// about the car's forward axis (from the side component) and right axis
/// (from the forward component): RocketSim's `FLIP_TORQUE_X` and
/// `FLIP_TORQUE_Y`. RocketSim applies `flipRelTorque * (X, Y)` through the
/// car's own inertia, so it is an angular acceleration, independent of the
/// car's mass distribution.
pub(super) const FLIP_TORQUE_SIDE: f32 = 260.0;
pub(super) const FLIP_TORQUE_FORWARD: f32 = 224.0;

/// After a flip's torque ends, air-control pitch stays locked this much
/// longer (s): RocketSim's `FLIP_PITCHLOCK_EXTRA_TIME`.
pub(super) const FLIP_PITCHLOCK_EXTRA_TIME: f32 = 0.3;

/// Vertical damping during a flip: from `FLIP_Z_DAMP_START` until the
/// torque ends, each 120 Hz tick scales vz by `1 - FLIP_Z_DAMP_120` while
/// the car falls, or unconditionally before `FLIP_Z_DAMP_END` (RocketSim).
/// This is what stalls a flipping car's fall.
pub(super) const FLIP_Z_DAMP_120: f32 = 0.35;
pub(super) const FLIP_Z_DAMP_START: f32 = 0.15;
pub(super) const FLIP_Z_DAMP_END: f32 = 0.21;

/// RocketSim's physics tick (s); flip torque and damping are defined per
/// tick at this rate and scaled to other step sizes.
const TICK_120: f32 = 1.0 / 120.0;

/// Tolerance (s) when comparing the flip clock with its limits, far below a
/// tick: summed step times land a hair either side of a limit (78 ticks of
/// `1/120` sum to `0.6499999`, a capture's recorded steps slightly over), so
/// a clock reaching a limit within it counts as reaching it
/// (`RB-PHYSICS-001-FR-094`).
const FLIP_CLOCK_TOLERANCE: f32 = 1e-4;

/// Whether the flip clock `time` is still before `limit`.
fn before(time: f32, limit: f32) -> bool {
    time < limit - FLIP_CLOCK_TOLERANCE
}

/// A dodge's flip since it was pressed, until landing: RocketSim's
/// `flipTime` and `flipRelTorque` (`RB-PHYSICS-001-FR-083`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlipState {
    /// Seconds since the dodge was pressed.
    pub time: f32,
    /// The dodge's normalized direction, forward and side components
    /// (RocketSim's `dodgeDir` before small components are snapped). Both
    /// zero for a stall, which applies no torque.
    pub direction: (f32, f32),
}

impl FlipState {
    /// Whether the flip torque still acts.
    pub fn is_flipping(&self) -> bool {
        before(self.time, FLIP_TORQUE_TIME)
    }
}

/// The scale on air control's pitch torque under a dodge (RocketSim's
/// `_UpdateAirTorque`): `0` while the flip spins the car at `FLIP_TORQUE_*`
/// and through `FLIP_PITCHLOCK_EXTRA_TIME` after it, `1` otherwise.
pub(super) fn flip_pitch_scale(flip: Option<FlipState>) -> f32 {
    match flip {
        Some(flip) if before(flip.time, FLIP_TORQUE_TIME + FLIP_PITCHLOCK_EXTRA_TIME) => 0.0,
        _ => 1.0,
    }
}

/// Applies a flip's torque for one airborne tick, after air control has
/// read the spin for its damping. Holding pitch against the flip's forward
/// direction scales the flip's pitch torque down by the stick amount: the
/// flip cancel.
///
/// Air control itself (roll and yaw torque, and all three dampings) stays on
/// through the flip (`RB-PHYSICS-001-FR-093`). RocketSim turns it off except
/// for the cancel, but the owner's capture shows the 4.317 s flip carrying
/// the held roll stick's torque and the dampings: its car-frame spin keeps
/// the roll/pitch ratio 1.35 while roll -1 is held and settles at 1.12 once
/// released, matching flip torque plus air control (1.35, 1.10), not the
/// flip torque alone (1.16).
///
/// Bullet accumulates the flip torque and integrates it after the damping
/// is computed from the step's starting spin, so the damping never sees
/// this tick's flip spin (`RB-PHYSICS-001-FR-097`).
pub(super) fn apply_flip_torque(
    car: &mut RigidBody,
    input: &ControllerInput,
    flip: Option<FlipState>,
) {
    let Some(flip) = flip.filter(FlipState::is_flipping) else {
        return;
    };
    let (mut forward_part, side_part) = flip.direction;
    // RocketSim compares the pitch stick with flipRelTorque.y, which is
    // the forward component: same sign means stick against the flip.
    let pitch = input.pitch.unwrap_or(0.0).clamp(-1.0, 1.0);
    let cancelling = forward_part != 0.0 && pitch != 0.0 && forward_part.signum() == pitch.signum();
    if cancelling {
        forward_part *= 1.0 - pitch.abs();
    }
    // flipRelTorque is divided by RocketSim's tick-time scale, so each tick
    // adds the same angular velocity whatever the step size. A stall
    // (no direction) adds none.
    let spin = right_axis(car) * (forward_part * FLIP_TORQUE_FORWARD)
        - forward_axis(car) * (side_part * FLIP_TORQUE_SIDE);
    car.angular_velocity += spin * TICK_120;
}

/// Advances an airborne car's flip clock by `dt` and applies the flip's
/// vertical damping (RocketSim's `_UpdateDoubleJumpOrFlip` tail).
pub(super) fn advance_flip(car: &mut RigidBody, flip: &mut Option<FlipState>, dt: f32) {
    let Some(flip) = flip else {
        return;
    };
    let was_flipping = flip.is_flipping();
    flip.time += dt;
    let past_torque = flip.time > FLIP_TORQUE_TIME + FLIP_CLOCK_TOLERANCE;
    if !was_flipping || past_torque || before(flip.time, FLIP_Z_DAMP_START) {
        return;
    }
    if car.linear_velocity.z < 0.0 || before(flip.time, FLIP_Z_DAMP_END) {
        car.linear_velocity.z *= (1.0 - FLIP_Z_DAMP_120).powf(dt / TICK_120);
    }
}

/// Maximum duration (seconds) that continuing to hold `jump` after a fresh
/// ground-jump press keeps adding extra upward acceleration
/// (`JUMP_HOLD_ACCELERATION`). Confirmed, not just guessed, during
/// `RB-PHYSICS-001-FR-031`'s audit: this port's pre-existing `0.2` already
/// matches both RocketSim's `RLConst.h` (`JUMP_MAX_TIME = 0.2f`) and
/// RLUtilities' `Jump::max_duration = 0.2f`. Real Rocket League also has a
/// `JUMP_MIN_TIME` (0.025s) of mandatory hold, see that constant's doc
/// comment.
pub(super) const JUMP_HOLD_MAX_DURATION: f32 = 0.2;

/// Continuous upward acceleration (uu/s^2) applied every step `jump` is
/// held and `JUMP_HOLD_MAX_DURATION` hasn't yet elapsed since the ground
/// jump's own fresh press, on top of that press's fixed `JUMP_SPEED`
/// impulse. Refined from an earlier `1400.0` approximation to the precise
/// value during `RB-PHYSICS-001-FR-031`'s audit: RocketSim's `RLConst.h`
/// defines `JUMP_ACCEL = 4375.f/3.f`, matched independently by RLUtilities'
/// `Jump::acceleration = 1458.3333f`. Applied at full strength from the
/// press tick on (`RB-PHYSICS-001-FR-091`), see `JUMP_MIN_TIME`.
pub(super) const JUMP_HOLD_ACCELERATION: f32 = 4375.0 / 3.0;

/// Seconds after a ground-jump press during which `JUMP_HOLD_ACCELERATION`
/// keeps applying regardless of whether `jump` is still held — a mandatory
/// minimum hold real Rocket League applies even to an instantaneous tap.
/// Confirmed against RocketSim's `RLConst.h` (`JUMP_MIN_TIME = 0.025f`) and
/// `Car.cpp`'s `_UpdateJump` gate (`jumpTime < JUMP_MIN_TIME || (jumpPressed
/// && jumpTime < JUMP_MAX_TIME)`) during `RB-PHYSICS-001-FR-064`.
///
/// RocketSim also scales the force by `JUMP_PRE_MIN_ACCEL_SCALE = 0.62`
/// inside this window, a value its own source marks as a TODO. This port
/// does not (`RB-PHYSICS-001-FR-091`): after the owner's capture's 4.142 s
/// press, the recorded car gains exactly `JUMP_HOLD_ACCELERATION` less
/// gravity less the sticky force (4.0 uu/s per tick) while its wheels
/// touch, then less gravity alone (6.7) once they leave; the scaled force
/// would lose 0.6 per tick.
pub(super) const JUMP_MIN_TIME: f32 = 0.025;

/// Variable jump height: applies the continuous hold acceleration using
/// whatever `jump_hold_time_remaining` the *previous* call left behind, so
/// it must run before this step's own ground-jump press can re-arm it (see
/// `ground_jump`).
pub(super) fn apply_jump_hold(
    car: &mut RigidBody,
    jump_held_now: bool,
    jump_hold_time_remaining: &mut f32,
    dt: f32,
) {
    // A fresh ground-jump press applies its own tick of this force in
    // `ground_jump`. RB-PHYSICS-001-FR-064: real Rocket League's own
    // `_UpdateJump` keeps applying it for the first `JUMP_MIN_TIME` seconds
    // since the press regardless of whether `jump` is still held — derived
    // here as `JUMP_HOLD_MAX_DURATION - *jump_hold_time_remaining` rather
    // than tracked as a second, separate elapsed-time field, since at rest
    // (`jump_hold_time_remaining == 0.0`) that derivation already reads as
    // `JUMP_HOLD_MAX_DURATION`, comfortably past `JUMP_MIN_TIME`, so a car
    // that never pressed jump never spuriously enters this branch. Only
    // once that mandatory window has passed does releasing `jump` end the
    // window immediately, even if time was left.
    let in_mandatory_pre_min_window =
        JUMP_HOLD_MAX_DURATION - *jump_hold_time_remaining < JUMP_MIN_TIME;
    if in_mandatory_pre_min_window || (jump_held_now && *jump_hold_time_remaining > 0.0) {
        push_jump_hold(car, jump_hold_time_remaining, dt);
    } else {
        *jump_hold_time_remaining = 0.0;
    }
}

/// One tick of `JUMP_HOLD_ACCELERATION` along the car's up axis, spending
/// `dt` of the hold window.
fn push_jump_hold(car: &mut RigidBody, jump_hold_time_remaining: &mut f32, dt: f32) {
    car.apply_central_force(up_axis(car) * (JUMP_HOLD_ACCELERATION * car.mass()));
    *jump_hold_time_remaining = (*jump_hold_time_remaining - dt).max(0.0);
}

/// Ground jump on a fresh press: a flat `JUMP_SPEED` velocity change, then
/// arms the hold window and applies its first tick of hold force, as
/// RocketSim's `_UpdateJump` does on the press tick
/// (`RB-PHYSICS-001-FR-091`).
pub(super) fn ground_jump(car: &mut RigidBody, jump_hold_time_remaining: &mut f32, dt: f32) {
    // An instantaneous velocity change, not a continuous force —
    // apply_impulse divides by mass internally, so scaling by
    // car.mass() here cancels that out and yields a flat
    // JUMP_SPEED velocity change regardless of the car's mass.
    // RB-PHYSICS-001-FR-101: along the car's up axis, as RocketSim's
    // `_UpdateJump` (`GetUpDir() * JUMP_IMMEDIATE_FORCE`).
    car.apply_impulse(up_axis(car) * (JUMP_SPEED * car.mass()), Vec3::ZERO);
    // This call's own apply_jump_hold already ran against the *previous*
    // value (0, since no ground jump was in flight yet), so the press tick's
    // hold force comes from here.
    *jump_hold_time_remaining = JUMP_HOLD_MAX_DURATION;
    push_jump_hold(car, jump_hold_time_remaining, dt);
}

/// `CAR_AUTOFLIP_IMPULSE` (uu/s): the pop off a surface the car lies
/// upside down on when jump is pressed (`RB-PHYSICS-001-FR-123`).
pub const AUTO_FLIP_SPEED: f32 = 200.0;
/// `CAR_AUTOFLIP_TORQUE` (rad/s^2) about the car's forward axis while the
/// auto-flip runs.
const AUTO_FLIP_TORQUE: f32 = 50.0;
/// `CAR_AUTOFLIP_TIME` (s): the flip runs this long times `|roll| / pi`.
const AUTO_FLIP_TIME: f32 = 0.4;
/// `CAR_AUTOFLIP_NORMZ_THRESH`: the surface must face up at least this much.
const AUTO_FLIP_NORMAL_Z_THRESHOLD: f32 = std::f32::consts::FRAC_1_SQRT_2;
/// `CAR_AUTOFLIP_ROLL_THRESH` (rad): the car must be rolled at least this far.
const AUTO_FLIP_ROLL_THRESHOLD: f32 = 2.8;

/// An auto-flip in progress (`RB-PHYSICS-001-FR-123`): how long its torque
/// still runs and which way about the forward axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AutoFlip {
    pub remaining: f32,
    pub torque_scale: f32,
}

/// RocketSim's roll from `Angle::FromRotMat`: minus Bullet's
/// `getEulerYPR` roll, `atan2(right.z, up.z)`.
fn roll(car: &RigidBody) -> f32 {
    (-right_axis(car).z).atan2(up_axis(car).z)
}

/// RocketSim's `_UpdateAutoFlip` (`RB-PHYSICS-001-FR-123`): a fresh jump
/// press while the car's body touched an upward-facing surface last tick
/// (`world_contact_normal`, its `worldContact`) and the car is rolled past
/// `AUTO_FLIP_ROLL_THRESHOLD` pops it `AUTO_FLIP_SPEED` away from its own
/// up axis and starts a roll torque toward upright, which then runs
/// `AUTO_FLIP_TIME * |roll| / pi` seconds from this tick on. The wheels
/// don't touch, so nothing else answers that press.
pub(super) fn auto_flip(
    car: &mut RigidBody,
    jump_pressed: bool,
    world_contact_normal: Option<Vec3>,
    flip: &mut Option<AutoFlip>,
    dt: f32,
) {
    let on_upward_surface =
        world_contact_normal.is_some_and(|n| n.z > AUTO_FLIP_NORMAL_Z_THRESHOLD);
    if jump_pressed && on_upward_surface {
        let roll = roll(car);
        if roll.abs() > AUTO_FLIP_ROLL_THRESHOLD {
            *flip = Some(AutoFlip {
                remaining: AUTO_FLIP_TIME * (roll.abs() / std::f32::consts::PI),
                torque_scale: roll.signum(),
            });
            car.apply_impulse(-up_axis(car) * (AUTO_FLIP_SPEED * car.mass()), Vec3::ZERO);
        }
    }
    let Some(active) = flip else {
        return;
    };
    if active.remaining <= 0.0 {
        *flip = None;
        return;
    }
    car.angular_velocity += forward_axis(car) * (AUTO_FLIP_TORQUE * active.torque_scale * dt);
    active.remaining -= dt;
}

/// Clamped dodge direction `(forward, side)` from the stick, the same way
/// for a ground dodge and a wall-jump dodge: RocketSim's
/// `dodgeDir = (-controls.pitch, controls.yaw + controls.roll)`. Rocket
/// League's pitch is negative with the stick pushed forward (nose down), so
/// a forward dodge is `-pitch` (`RB-PHYSICS-001-FR-082`).
fn dodge_stick(input: &ControllerInput) -> (f32, f32) {
    let forward = -input.pitch.unwrap_or(0.0).clamp(-1.0, 1.0);
    let side =
        input.roll.unwrap_or(0.0).clamp(-1.0, 1.0) + input.yaw.unwrap_or(0.0).clamp(-1.0, 1.0);
    (forward, side)
}

/// RocketSim's `CarConfig::dodgeDeadzone`: a jump press in the air flips,
/// rather than double-jumps, once `|yaw| + |pitch| + |roll|` reaches this
/// (`Car::_UpdateDoubleJumpOrFlip`; `RB-PHYSICS-001-FR-104`).
pub(super) const FLIP_INPUT_DEADZONE: f32 = 0.5;

/// The flip a jump press makes, if the stick asks for one
/// (`RB-PHYSICS-001-FR-104`): `None` for a double jump. Whether to flip
/// comes from the stick's total deflection (`FLIP_INPUT_DEADZONE`), and
/// only then its direction, `dodge_stick`. A direction under
/// `DODGE_DEADZONE` on both parts is a stall: `(0.0, 0.0)`, a flip with no
/// impulse and no torque. Yaw against air roll (`test2.jsonl` 15.617 s:
/// yaw +1, roll -1) is the usual stall input.
///
/// With a side part and pitch centred, the forward part comes from the
/// throttle (`RB-PHYSICS-001-FR-103`, ADR-0023). In the owner's keyboard
/// captures, a yaw-only press with throttle held dodges diagonally
/// (forward 354, side 354 scaled: `test2.jsonl` 6.058 s and 12.55 s), and
/// one without throttle dodges purely sideways. Rocket League dodges from
/// its own `DodgeForward` input, which the captures do not record; for
/// them it follows the throttle. The throttle never starts a flip on its
/// own, and no capture shows it during a stall, so a stall stays one.
pub(super) fn dodge_direction(input: &ControllerInput) -> Option<(f32, f32)> {
    let axis = |value: Option<f32>| value.unwrap_or(0.0).clamp(-1.0, 1.0).abs();
    let deflection = axis(input.pitch) + axis(input.yaw) + axis(input.roll);
    if deflection < FLIP_INPUT_DEADZONE {
        return None;
    }
    let (forward, side) = dodge_stick(input);
    if forward.abs() < DODGE_DEADZONE && side.abs() < DODGE_DEADZONE {
        return Some((0.0, 0.0));
    }
    let forward = if forward.abs() < DODGE_DEADZONE {
        input.throttle.clamp(-1.0, 1.0)
    } else {
        forward
    };
    Some((forward, side))
}

/// Applies a directional dodge on top of `base_impulse` (a velocity change,
/// scaled by mass here): translate along `forward_axis` and spin about
/// `right_axis` from the forward component, translate along `right_axis`
/// and spin about `forward_axis` from the side component, with RocketSim's
/// signs (`flipRelTorque = (-dodgeDir.y, dodgeDir.x)`): a forward dodge
/// noses down, a dodge to the right rolls the right side down. Leaves a
/// cancelable flip behind.
fn apply_dodge(
    car: &mut RigidBody,
    forward: Vec3,
    (pitch, roll): (f32, f32),
    base_impulse: Vec3,
    flip: &mut Option<FlipState>,
) {
    let forward_speed = car.linear_velocity.dot(&forward);
    let (norm_pitch, norm_roll) = normalize_dodge_direction(pitch, roll);
    // RocketSim pushes along the car's heading flattened to the ground
    // plane (`forwardDir2D`, `rightDir2D`), so a tilted car's dodge stays
    // horizontal. A car pointing straight up or down has no heading and
    // gets no horizontal push.
    let forward_2d = Vec3::new(forward.x, forward.y, 0.0)
        .normalize()
        .unwrap_or(Vec3::ZERO);
    let right_2d = Vec3::new(-forward_2d.y, forward_2d.x, 0.0);
    let mut dodge_impulse = base_impulse;
    if pitch.abs() > DODGE_DEADZONE {
        let scale = if dodge_pitch_is_backward(pitch, forward_speed) {
            dodge_speed_scale(forward_speed, DODGE_BACKWARD_SPEED_SCALE) * DODGE_BACKWARD_SCALE_X
        } else {
            1.0
        };
        dodge_impulse += forward_2d * (norm_pitch * DODGE_SPEED * scale);
    }
    if roll.abs() > DODGE_DEADZONE {
        let scale = dodge_speed_scale(forward_speed, DODGE_SIDE_SPEED_SCALE);
        dodge_impulse += right_2d * (norm_roll * DODGE_SPEED * scale);
    }
    car.apply_impulse(dodge_impulse * car.mass(), Vec3::ZERO);
    // The spin comes from `apply_flip_torque`, starting this same tick.
    let length = (pitch * pitch + roll * roll).sqrt();
    let direction = if length > 0.0 {
        (pitch / length, roll / length)
    } else {
        (0.0, 0.0)
    };
    *flip = Some(FlipState {
        time: 0.0,
        direction,
    });
}

/// `DOUBLEJUMP_MAX_DELAY` (`RLConst.h`, `RB-PHYSICS-001-FR-136`): the second
/// jump or dodge must come within this many seconds of the first jump's hold
/// ending.
pub(super) const DOUBLE_JUMP_MAX_DELAY: f32 = 1.25;

/// `JUMP_MIN_TIME + JUMP_RESET_TIME_PAD` (0.025 + 1/40 s): a grounded car
/// keeps its jump record this long, so a minimum-length jump that has not
/// yet left the ground is not forgotten.
const JUMP_RESET_WINDOW: f32 = 0.025 + 1.0 / 40.0;

/// `Car::_UpdateJump`'s and `_UpdateDoubleJumpOrFlip`'s timers:
/// whether this airtime began with a jump, how long since the jump press,
/// and how long since its hold ended.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct JumpClock {
    has_jumped: bool,
    jump_time: f32,
    air_time_since_jump: f32,
}

impl JumpClock {
    /// One tick: `jumping` while the jump hold runs. Landing forgets the
    /// jump (after the reset window); airborne after a finished jump the
    /// since-jump timer runs, otherwise it stays zero, as RocketSim's.
    pub(super) fn update(&mut self, jumping: bool, on_ground: bool, dt: f32) {
        if on_ground && !jumping {
            if !(self.has_jumped && self.jump_time < JUMP_RESET_WINDOW) {
                self.has_jumped = false;
                self.jump_time = 0.0;
            }
            self.air_time_since_jump = 0.0;
        }
        if jumping {
            self.has_jumped = true;
        }
        if self.has_jumped {
            self.jump_time += dt;
        }
        if !on_ground {
            self.air_time_since_jump = if self.has_jumped && !jumping {
                self.air_time_since_jump + dt
            } else {
                0.0
            };
        }
    }

    /// Whether this airtime began with a jump whose record is still held
    /// (a grounded car keeps it for `JUMP_RESET_WINDOW`).
    pub(super) fn has_jumped(&self) -> bool {
        self.has_jumped
    }

    /// Whether the second jump or dodge is still inside its window.
    pub(super) fn window_open(&self) -> bool {
        self.air_time_since_jump < DOUBLE_JUMP_MAX_DELAY
    }

    /// Touching a wall refills the window with the second jump.
    pub(super) fn refill(&mut self) {
        self.air_time_since_jump = 0.0;
    }
}

/// A fresh jump press while airborne. Priority: wall jump (plain, or a
/// wall-jump dodge) if touching a wall, else a double jump (plain, or a
/// dodge) if one is still available. See `super::apply_driven_forces` for
/// each flag's rules.
pub(super) fn airborne_jump_press(
    car: &mut RigidBody,
    input: &ControllerInput,
    forward: Vec3,
    wall_normal: Option<Vec3>,
    double_jump_available: &mut bool,
    flip: &mut Option<FlipState>,
) {
    if let Some(wall_normal) = wall_normal {
        // Wall jump takes priority over the double jump on this press: push
        // off outward along the wall's normal, plus the same upward
        // JUMP_SPEED every jump variant uses.
        let push = wall_normal * WALL_JUMP_HORIZONTAL_SPEED + Vec3::new(0.0, 0.0, JUMP_SPEED);
        if let Some(stick) = dodge_direction(input) {
            // Wall-jump dodge: unlike the plain wall jump below, this
            // *does* consume double_jump_available — the same resource a
            // ground dodge spends — a deliberate simplification (see the
            // parent module doc comment).
            apply_dodge(car, forward, stick, push, flip);
            *double_jump_available = false;
        } else {
            // Plain wall jump: doesn't consume double_jump_available
            // (already restored unconditionally by the caller, just from
            // touching the wall) — matching real Rocket League's "any
            // surface contact refills your second jump" rule.
            car.apply_impulse(push * car.mass(), Vec3::ZERO);
        }
    } else if *double_jump_available {
        if let Some(stick) = dodge_direction(input) {
            // Dodge: a directional flip instead of a plain vertical double
            // jump. Purely horizontal, with no vertical JUMP_SPEED
            // component — see the parent module doc comment.
            apply_dodge(car, forward, stick, Vec3::ZERO, flip);
        } else {
            // Same fixed-magnitude impulse as the ground jump — reusing
            // JUMP_SPEED rather than a second, separately-calibrated
            // constant, since this port has no public reference for a
            // distinct double-jump speed either.
            // Along the car's up axis, as RocketSim's double jump.
            car.apply_impulse(up_axis(car) * (JUMP_SPEED * car.mass()), Vec3::ZERO);
        }
        *double_jump_available = false;
    }
}
