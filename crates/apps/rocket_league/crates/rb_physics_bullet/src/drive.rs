//! Driven car input: couples `rb_domain::ControllerInput` into forces and
//! torques on a car `RigidBody`. Ground-driving (throttle, steering),
//! boost, handbrake/drift, a single ground jump, and air control in this
//! increment. Throttle, steering, handbrake, and jump are gated on the car
//! actually touching the ground (a free-floating box has no wheels to
//! grip, lock, or push off of, so airborne input does nothing for any of
//! them here); boost is not — it's a rocket, not an engine, so it works
//! identically grounded or airborne, the same way it does in real Rocket
//! League. Air control is the mirror image: gated on the car *not*
//! touching the ground (real air control needs no wheels at all — it's
//! pure torque, so it would be redundant with steering while grounded).
//!
//! Handbrake is modeled as a temporary ground-friction reduction rather
//! than a separate lateral-slip system: this port has no per-wheel tire
//! model (the car is one rigid box), so there's no distinct "rear grip"
//! to lose the way a real car's handbrake works. Instead, while `handbrake`
//! is held and the car is grounded, `RigidBody.friction` — the same
//! material property the ground-contact solver already reads for Coulomb
//! friction — is temporarily reduced, letting the box's existing momentum
//! carry it into a slide instead of gripping the ground and turning
//! cleanly. Releasing handbrake restores the car's original friction. This
//! reuses machinery the solver already has rather than inventing a second
//! grip model.
//!
//! Jump is a single, fixed-height vertical impulse fired on the *rising
//! edge* of `ControllerInput.jump` (a fresh press, not merely "held") while
//! grounded — holding the button through the resulting airborne period
//! doesn't re-fire it, and releasing then re-pressing while still airborne
//! doesn't fire it either (this increment has no double jump to grant).
//! Edge detection needs one bit of state to remember "was jump held as of
//! last step," carried by the caller (`PhysicsWorld::car_jump_held`) and
//! passed in as `jump_held`, the same pattern `boost_amount` already uses
//! for a resource that must persist across calls.
//!
//! Pitch, yaw, and roll each apply torque about one of the car's three
//! local axes (right, up, forward respectively), scaled directly by the
//! analog `Option<f32>` value — `None` (an input source that can't recover
//! an analog value, e.g. replay-derived input, see `rb_domain`) is treated
//! as zero, same as a centered stick. Unlike ground steering, air control
//! isn't speed-scaled: a car can spin freely from a standing start in the
//! air, since there's no wheel grip to require momentum for. All three
//! axes share one `AIR_CONTROL_TORQUE` constant — a real simplification,
//! since Rocket League's actual pitch/yaw/roll rates differ from each
//! other; this port doesn't model that difference.
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
//! roll) plus an instantaneous `DODGE_ANGULAR_SPEED` spin about the
//! perpendicular axis (`right_axis` for pitch, `forward_axis` for roll) —
//! the same axis/sign conventions air control's own pitch/roll torque
//! already uses, so a forward dodge looks like a fast version of a forward
//! air-control pitch. Both pitch and roll can contribute at once (a
//! diagonal dodge), simply summed — a documented simplification, since real
//! Rocket League normalizes the stick direction so a diagonal dodge isn't
//! faster than an axis-aligned one, and this port doesn't. A dodge is
//! purely horizontal (no vertical component, unlike the plain double
//! jump) — real Rocket League's dodge impulse does have a small upward
//! component too, not modeled here. Below `DODGE_DEADZONE` on both axes,
//! the plain vertical double jump fires exactly as before dodge existed.
//! Either way, the press still spends the one `double_jump_available` per
//! airborne period — a dodge and a plain double jump share the same
//! resource, matching real Rocket League. A dodge also leaves a per-car
//! `dodge_flip_active` flag set, spent by **flip-cancel** below; a plain
//! double jump explicitly clears it instead (there's no flip to cancel).
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
//! double jump's once-per-airborne-period limit. Wall jump doesn't check
//! `pitch`/`roll` for a dodge — it always fires the fixed outward-plus-
//! upward impulse, never a directional flip; real Rocket League's wall
//! jump can itself be dodged off of, but that's deliberately out of scope
//! here (see Not implemented).
//!
//! The ground jump has variable height: continuing to hold `jump` after the
//! fresh press that fires it adds a continuous `JUMP_HOLD_ACCELERATION`
//! upward force, for up to `JUMP_HOLD_MAX_DURATION` seconds, on top of the
//! fixed `JUMP_SPEED` impulse — releasing early (or the window simply
//! running out) stops the extra acceleration immediately, matching real
//! Rocket League's held-vs-tapped jump height difference. This is scoped to
//! the ground jump alone: the double jump, a dodge, and the wall jump are
//! still each a single fixed instantaneous impulse, completely unaffected
//! by how long jump is held, since firing any of them requires releasing
//! jump first (a fresh press), which itself unconditionally ends the ground
//! jump's hold window (see `apply_driven_forces`'s own doc comment for the
//! exact ordering). Tracked per car via `jump_hold_time_remaining`, the same
//! kind of caller-owned persisted state `jump_held`/`double_jump_available`
//! already are.
//!
//! A dodge's spin can be canceled early — **flip-cancel** — by pressing
//! jump again before landing or wall contact: a fresh press while airborne,
//! not touching a wall, `double_jump_available` already spent (so this
//! isn't a wall jump or another double jump/dodge), and `dodge_flip_active`
//! still set, zeroes `RigidBody.angular_velocity` outright and clears
//! `dodge_flip_active` — stopping the flip immediately, matching real
//! Rocket League. It doesn't touch linear velocity (the dodge's own
//! translation is unaffected) and doesn't consume or restore
//! `double_jump_available` (already spent by the dodge that set the flag).
//! This port has no timed flip animation to interrupt (a dodge is one
//! instantaneous angular-velocity kick, not a sustained torque over a fixed
//! duration — see above), so "mid-flip" here means "any time before
//! landing or a wall touch re-arms the double jump," a documented
//! simplification of real Rocket League's actual flip-duration window.
//! Wall jump keeps priority over flip-cancel on a fresh press while
//! touching a wall, unchanged. A plain double jump explicitly clears
//! `dodge_flip_active` rather than leaving it alone, so a stale flag from
//! an earlier dodge (long since landed from) can't make a *later*,
//! unrelated plain double jump's next press incorrectly fire a flip-cancel.
//!
//! **Not implemented** (tracked in `RB-PHYSICS-001`, not silently
//! dropped): a dodge variant for the wall jump (see above), and any
//! auto-orientation assistance on landing after a dodge. A car with no
//! input set (or all-neutral `ControllerInput::default()`) behaves exactly
//! as a free rigid box always has — this module only ever adds
//! force/torque/impulse or adjusts the existing friction property, never
//! removes physics outright.
//!
//! This is not a Bullet3 port (Bullet has no concept of "a car's engine")
//! — it's this project's own model of Rocket League's driving mechanics,
//! since the real numbers are not public. `MAX_CAR_SPEED`, `MAX_BOOST`,
//! `BOOST_ACCELERATION`, and `JUMP_SPEED` are commonly-cited
//! community-reverse-engineered approximations (the same body of public
//! research `PhysicsWorld::new`'s gravity constant comes from);
//! `THROTTLE_ACCELERATION` and `BOOST_CONSUMPTION_RATE` are simplified
//! constants standing in for Rocket League's real speed-dependent throttle
//! curve and boost-drain behavior; `STEER_TORQUE`,
//! `HANDBRAKE_FRICTION_MULTIPLIER`, `AIR_CONTROL_TORQUE`,
//! `WALL_JUMP_HORIZONTAL_SPEED`, `DODGE_SPEED`, and `DODGE_ANGULAR_SPEED`
//! are uncalibrated placeholders chosen only to produce a visibly
//! responsive turn/slide/spin/push-off/flip for this car's mass/inertia in
//! tests. None of these are independently confirmed by this project — see
//! `RB-PHYSICS-001-FR-005`.

use crate::body::RigidBody;
use rb_domain::{ControllerInput, Vec3};

/// Commonly-cited approximate max ground speed a car's engine alone can
/// reach (uu/s) — Rocket League's unboosted top speed. Also used here as
/// boost's own speed cap (real Rocket League's actual top speed, boosted
/// or not, is the same number) — a simplification, since throttle and
/// boost don't share one real top speed, but this port doesn't yet model
/// two separate ceilings.
pub const MAX_CAR_SPEED: f32 = 2300.0;

/// Simplified constant throttle acceleration (uu/s^2). Rocket League's
/// real throttle curve tapers off nonlinearly as speed rises toward
/// `MAX_CAR_SPEED`; this port uses one constant instead, a real
/// simplification (not a taper), pending calibration against recorded
/// data.
const THROTTLE_ACCELERATION: f32 = 1600.0;

/// Uncalibrated placeholder steering torque magnitude (about the car's
/// local up axis, at full `steer` input and at/above `MAX_CAR_SPEED`) —
/// chosen only so a full-lock turn is visibly responsive for this car's
/// mass/inertia in tests, not derived from any measured or documented
/// Rocket League value.
const STEER_TORQUE: f32 = 1_500_000.0;

/// Commonly-cited approximate boost acceleration (uu/s^2), a flat
/// constant (unlike throttle, boost doesn't taper with speed in real
/// Rocket League either).
const BOOST_ACCELERATION: f32 = 991.667;

/// Commonly-cited full boost tank size, in the same units `ControllerInput`
/// and `CarState::boost_amount` use.
pub const MAX_BOOST: f32 = 100.0;

/// Simplified constant boost drain rate (units/s) while `boost` is held —
/// a full tank lasting ~3 seconds nonstop is the commonly-cited number
/// this approximates; real Rocket League's actual drain behavior around
/// zero-throttle/wavedash edge cases isn't modeled.
const BOOST_CONSUMPTION_RATE: f32 = 33.3;

/// Uncalibrated placeholder: while grounded and `handbrake` is held, the
/// car's `RigidBody.friction` is multiplied by this factor before the
/// ground-contact solver runs, sharply reducing grip so existing momentum
/// carries the car into a slide instead of a clean turn. Chosen only to
/// produce a visibly reduced (not zero) grip in tests, not derived from any
/// measured or documented Rocket League value — this port has no per-wheel
/// tire model to calibrate a real rear-grip-loss number against.
const HANDBRAKE_FRICTION_MULTIPLIER: f32 = 0.1;

/// Commonly-cited approximate jump impulse speed (uu/s), applied as an
/// instantaneous vertical velocity change (not a continuous force) on a
/// fresh grounded jump press — a flat speed regardless of the car's mass,
/// matching how the real jump impulse doesn't scale with car mass either.
/// Also reused as the double jump's impulse magnitude (see the module doc
/// comment) — `pub` so `world.rs`'s end-to-end tests can assert against it
/// directly, the same way `MAX_CAR_SPEED`/`MAX_BOOST` already are.
pub const JUMP_SPEED: f32 = 292.0;

/// Uncalibrated placeholder air-control torque magnitude, shared by pitch,
/// yaw, and roll (about the car's local right, up, and forward axes
/// respectively) at full analog input — chosen only so a full-stick
/// rotation is visibly responsive for this car's mass/inertia in tests,
/// not derived from any measured or documented Rocket League value. Real
/// Rocket League's pitch/yaw/roll rates differ from each other; this port
/// doesn't model that difference.
const AIR_CONTROL_TORQUE: f32 = 1_000_000.0;

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
pub const WALL_JUMP_HORIZONTAL_SPEED: f32 = 550.0;

/// Arbitrary deadzone for `pitch`/`roll` input at the moment of a double
/// jump's fresh press: below this magnitude on both axes, the press is
/// treated as "no directional intent" and fires a plain vertical double
/// jump; at or above it on either axis, it fires a dodge instead. Not a
/// physics constant and not derived from any Rocket League value — purely
/// an input-processing threshold, chosen only to ignore tiny analog stick
/// drift.
const DODGE_DEADZONE: f32 = 0.1;

/// Uncalibrated placeholder dodge horizontal impulse speed (uu/s), applied
/// along `forward_axis` (scaled by `pitch`) and/or `right_axis` (scaled by
/// `roll`) as an instantaneous velocity change (like `JUMP_SPEED`, not a
/// continuous force) — chosen only to produce a visibly fast, distinct
/// dodge in tests, not derived from any measured or documented Rocket
/// League value. `pub` so `world.rs`'s end-to-end tests can assert against
/// it directly, the same way `JUMP_SPEED` already is.
pub const DODGE_SPEED: f32 = 1400.0;

/// Uncalibrated placeholder dodge spin speed (rad/s), added directly to
/// `RigidBody.angular_velocity` as an instantaneous change (mirroring how
/// `apply_impulse` directly changes `linear_velocity`, rather than
/// `apply_torque`'s continuous accumulation, since a dodge's flip is a
/// single instantaneous kick, not a sustained torque) — chosen only to
/// produce a visibly fast flip in tests, not derived from any measured or
/// documented Rocket League value.
const DODGE_ANGULAR_SPEED: f32 = 5.5;

/// Uncalibrated placeholder maximum duration (seconds) that continuing to
/// hold `jump` after a fresh ground-jump press keeps adding extra upward
/// acceleration (`JUMP_HOLD_ACCELERATION`) — this port has no public
/// reference for real Rocket League's actual hold-window length the way
/// `JUMP_SPEED` does, so this is chosen only to make a fully held jump
/// visibly taller than a tapped one in tests, not derived from any
/// measured or documented Rocket League value.
const JUMP_HOLD_MAX_DURATION: f32 = 0.2;

/// Uncalibrated placeholder continuous upward acceleration (uu/s^2)
/// applied every step `jump` is held and `JUMP_HOLD_MAX_DURATION` hasn't
/// yet elapsed since the ground jump's own fresh press, on top of that
/// press's fixed `JUMP_SPEED` impulse — chosen only to produce a clearly
/// taller held jump than a tapped one for this car's mass in tests, not
/// derived from any measured or documented Rocket League value.
const JUMP_HOLD_ACCELERATION: f32 = 1400.0;

fn forward_axis(car: &RigidBody) -> Vec3 {
    car.orientation.rotate(&Vec3::new(1.0, 0.0, 0.0))
}

fn up_axis(car: &RigidBody) -> Vec3 {
    car.orientation.rotate(&Vec3::new(0.0, 0.0, 1.0))
}

/// The car's local "right" axis (local +Y) — completes the right-handed
/// (forward, right, up) basis `up_axis × forward_axis` gives. Used only for
/// pitch torque (nose up/down about this axis); throttle/steer/boost/
/// handbrake/jump never need it.
fn right_axis(car: &RigidBody) -> Vec3 {
    car.orientation.rotate(&Vec3::new(0.0, 1.0, 0.0))
}

/// Applies throttle, steering, boost, handbrake, jump, double jump, wall
/// jump, and air control as forces/torques/impulses (or, for handbrake, a
/// temporary friction adjustment) on `car`. Throttle, steering, handbrake,
/// and the ground jump are a no-op unless `on_ground`; air control, double
/// jump, and wall jump are the reverse — a no-op unless *not* `on_ground`;
/// boost isn't gated on ground contact at all, but is a no-op once
/// `*boost_amount` reaches zero. `base_friction` is the car's own nominal
/// (non-handbraking) friction — handbrake temporarily reduces
/// `car.friction` below it while held and grounded, and restores it
/// otherwise, so callers don't need a separate restore step. `jump_held` is
/// the car's `input.jump` value as of the *previous* call — every jump
/// variant fires only on a rising edge (`input.jump && !*jump_held`), so a
/// continued press doesn't re-fire every step; it's updated to `input.jump`
/// on every call, including while airborne, so a fresh press is still
/// required for a double or wall jump even if the button was never
/// released after the ground jump. `double_jump_available` is whether the
/// car still has a double jump (plain or dodge) to spend this airborne
/// period — landing (`on_ground`) or merely touching a wall
/// (`wall_normal.is_some()`, no jump press required) both unconditionally
/// set it back to `true`; only an airborne fresh press that fires the
/// double jump or a dodge (not a wall jump) sets it to `false`, until the
/// next landing or wall touch. `wall_normal` is the outward normal of the
/// wall the car is currently touching, if any (computed by the caller the
/// same way `on_ground` is — see `PhysicsWorld`); a fresh press while
/// airborne and `wall_normal.is_some()` fires a wall jump instead of
/// consulting `double_jump_available` at all — wall jump never dodges,
/// regardless of `input.pitch`/`input.roll`. `jump_hold_time_remaining` is
/// how much longer, in seconds, continuing to hold `jump` should keep
/// adding extra upward acceleration to a ground jump — checked and
/// decremented *before* this call's own `on_ground`/`jump_pressed` handling
/// below, using whatever value the *previous* call left it at, so a fresh
/// ground-jump press's own step only ever fires the plain `JUMP_SPEED`
/// impulse; that same press then re-arms `jump_hold_time_remaining` to
/// `JUMP_HOLD_MAX_DURATION` for subsequent calls to consume. Releasing
/// `jump` immediately zeroes it (stopping the extra acceleration right
/// away), and it's otherwise untouched by the double jump, a dodge, or the
/// wall jump — see the module doc comment. `dodge_flip_active` is whether
/// the car's most recent double-jump-or-dodge press was a dodge whose spin
/// hasn't been canceled or superseded yet: the dodge branch sets it `true`,
/// the plain-double-jump branch explicitly sets it `false` (so a stale
/// `true` from an earlier, already-landed-from dodge can't leak into a
/// later unrelated double jump), and a further fresh press while airborne,
/// not touching a wall, with `double_jump_available` already spent and
/// `dodge_flip_active` still `true` cancels the flip — see the module doc
/// comment's flip-cancel paragraph. Call once per step, before
/// `integrate::integrate_velocities`, alongside `apply_gravity`.
#[allow(clippy::too_many_arguments)]
pub fn apply_driven_forces(
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
    dt: f32,
) {
    let forward = forward_axis(car);
    let jump_pressed = input.jump && !*jump_held;
    *jump_held = input.jump;

    // Variable jump height: apply the continuous hold acceleration using
    // whatever jump_hold_time_remaining the *previous* call left behind,
    // before this call's own on_ground/jump_pressed handling below can
    // re-arm it — so a fresh ground-jump press's own step here never gets
    // the extra force, only continued holding into later calls does.
    // Releasing jump ends the window immediately, even if time was left.
    if input.jump && *jump_hold_time_remaining > 0.0 {
        car.apply_central_force(Vec3::new(0.0, 0.0, JUMP_HOLD_ACCELERATION * car.mass()));
        *jump_hold_time_remaining = (*jump_hold_time_remaining - dt).max(0.0);
    } else {
        *jump_hold_time_remaining = 0.0;
    }

    if on_ground {
        // Landing (or simply resting) always restores the double jump,
        // regardless of this step's input.
        *double_jump_available = true;

        let forward_speed = car.linear_velocity.dot(&forward);
        let throttle = input.throttle.clamp(-1.0, 1.0);
        if throttle != 0.0 && throttle.signum() * forward_speed < MAX_CAR_SPEED {
            car.apply_central_force(forward * (throttle * THROTTLE_ACCELERATION * car.mass()));
        }

        let steer = input.steer.clamp(-1.0, 1.0);
        if steer != 0.0 {
            // A stationary car can't carve a turn — scale the available
            // torque by how fast it's already going, up to MAX_CAR_SPEED.
            let speed_factor = (car.linear_velocity.length() / MAX_CAR_SPEED).min(1.0);
            if speed_factor > 0.0 {
                car.apply_torque(up_axis(car) * (steer * STEER_TORQUE * speed_factor));
            }
        }

        car.friction = if input.handbrake {
            base_friction * HANDBRAKE_FRICTION_MULTIPLIER
        } else {
            base_friction
        };

        if jump_pressed {
            // An instantaneous velocity change, not a continuous force —
            // apply_impulse divides by mass internally, so scaling by
            // car.mass() here cancels that out and yields a flat
            // JUMP_SPEED velocity change regardless of the car's mass.
            car.apply_impulse(Vec3::new(0.0, 0.0, JUMP_SPEED * car.mass()), Vec3::ZERO);
            // Arms the hold window for continued holding on subsequent
            // calls — this call's own jump_hold_time_remaining check above
            // already ran against the *previous* value (0, since no ground
            // jump was in flight yet), so only the fixed impulse above
            // fires this step.
            *jump_hold_time_remaining = JUMP_HOLD_MAX_DURATION;
        }
    } else {
        if wall_normal.is_some() {
            // Touching a wall restores the double jump unconditionally —
            // the same "any surface contact refills your second jump"
            // rule landing uses — regardless of whether jump is pressed
            // this step.
            *double_jump_available = true;
        }

        // Air control: pitch/yaw/roll torque about the car's local
        // right/up/forward axes. Unlike ground steering, not scaled by
        // speed — a car can spin from a standing start in the air, since
        // there's no wheel grip to require momentum for.
        let pitch = input.pitch.unwrap_or(0.0).clamp(-1.0, 1.0);
        if pitch != 0.0 {
            car.apply_torque(right_axis(car) * (pitch * AIR_CONTROL_TORQUE));
        }

        let yaw = input.yaw.unwrap_or(0.0).clamp(-1.0, 1.0);
        if yaw != 0.0 {
            car.apply_torque(up_axis(car) * (yaw * AIR_CONTROL_TORQUE));
        }

        let roll = input.roll.unwrap_or(0.0).clamp(-1.0, 1.0);
        if roll != 0.0 {
            car.apply_torque(forward * (roll * AIR_CONTROL_TORQUE));
        }

        if jump_pressed {
            if let Some(wall_normal) = wall_normal {
                // Wall jump takes priority over the double jump on this
                // press: push off outward along the wall's normal, plus
                // the same upward JUMP_SPEED every jump variant uses.
                // Doesn't consume double_jump_available (already restored
                // unconditionally above, just from touching the wall) —
                // matching real Rocket League's "any surface contact
                // refills your second jump" rule, so a wall jump doesn't
                // cost a player their double jump.
                car.apply_impulse(
                    (wall_normal * WALL_JUMP_HORIZONTAL_SPEED + Vec3::new(0.0, 0.0, JUMP_SPEED))
                        * car.mass(),
                    Vec3::ZERO,
                );
            } else if *double_jump_available {
                let dodge_pitch = input.pitch.unwrap_or(0.0).clamp(-1.0, 1.0);
                let dodge_roll = input.roll.unwrap_or(0.0).clamp(-1.0, 1.0);
                if dodge_pitch.abs() > DODGE_DEADZONE || dodge_roll.abs() > DODGE_DEADZONE {
                    // Dodge: a directional flip instead of a plain vertical
                    // double jump, reusing the same axis/sign conventions
                    // air control's own pitch/roll torque already uses —
                    // forward/back from pitch (translate along
                    // forward_axis, spin about right_axis), left/right
                    // from roll (translate along right_axis, spin about
                    // forward_axis). Purely horizontal, with no vertical
                    // JUMP_SPEED component — see the module doc comment.
                    let mut dodge_impulse = Vec3::ZERO;
                    let mut dodge_spin = Vec3::ZERO;
                    if dodge_pitch.abs() > DODGE_DEADZONE {
                        dodge_impulse += forward * (dodge_pitch * DODGE_SPEED);
                        dodge_spin += right_axis(car) * (dodge_pitch * DODGE_ANGULAR_SPEED);
                    }
                    if dodge_roll.abs() > DODGE_DEADZONE {
                        dodge_impulse += right_axis(car) * (dodge_roll * DODGE_SPEED);
                        dodge_spin += forward * (dodge_roll * DODGE_ANGULAR_SPEED);
                    }
                    car.apply_impulse(dodge_impulse * car.mass(), Vec3::ZERO);
                    // A single instantaneous spin kick, not a continuous
                    // torque — mirrors how apply_impulse directly changes
                    // linear_velocity, since RigidBody has no equivalent
                    // "angular impulse" helper (and none is warranted for
                    // this one call site).
                    car.angular_velocity += dodge_spin;
                    // Leaves a cancelable flip behind for flip-cancel below
                    // to spend on a later press.
                    *dodge_flip_active = true;
                } else {
                    // Same fixed-magnitude impulse as the ground jump — reusing
                    // JUMP_SPEED rather than a second, separately-calibrated
                    // constant, since this port has no public reference for a
                    // distinct double-jump speed either.
                    car.apply_impulse(Vec3::new(0.0, 0.0, JUMP_SPEED * car.mass()), Vec3::ZERO);
                    // No flip to cancel — and explicitly clearing this
                    // (rather than leaving it alone) prevents a stale `true`
                    // from an earlier, already-landed-from dodge from
                    // leaking into this unrelated plain double jump.
                    *dodge_flip_active = false;
                }
                *double_jump_available = false;
            } else if *dodge_flip_active {
                // Flip-cancel: a further fresh press with no double jump
                // left and an uncanceled dodge flip still active stops the
                // spin outright, without touching the dodge's own
                // translation or double_jump_available (already spent by
                // the dodge that set this flag) — see the module doc
                // comment for why "mid-flip" means "any time before landing
                // or a wall touch" in this port.
                car.angular_velocity = Vec3::ZERO;
                *dodge_flip_active = false;
            }
        }
    }

    if input.boost && *boost_amount > 0.0 {
        let forward_speed = car.linear_velocity.dot(&forward);
        if forward_speed < MAX_CAR_SPEED {
            car.apply_central_force(forward * (BOOST_ACCELERATION * car.mass()));
        }
        // Held boost drains the tank even when the force above didn't
        // apply (e.g. already at MAX_CAR_SPEED, or pushing into a wall) —
        // matching real Rocket League, where holding boost costs fuel
        // regardless of whether it's doing anything.
        *boost_amount = (*boost_amount - BOOST_CONSUMPTION_RATE * dt).max(0.0);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::integrate;

    const DEFAULT_TEST_FRICTION: f32 = 0.5;

    fn car() -> RigidBody {
        RigidBody::car_box(Vec3::new(60.0, 30.0, 18.0), 180.0, Vec3::ZERO)
    }

    fn step_with_input(
        car: &mut RigidBody,
        input: &ControllerInput,
        on_ground: bool,
        boost_amount: &mut f32,
        dt: f32,
    ) {
        let mut jump_held = false;
        step_with_input_and_jump_state(car, input, on_ground, boost_amount, &mut jump_held, dt);
    }

    fn step_with_input_and_jump_state(
        car: &mut RigidBody,
        input: &ControllerInput,
        on_ground: bool,
        boost_amount: &mut f32,
        jump_held: &mut bool,
        dt: f32,
    ) {
        let mut double_jump_available = true;
        step_with_input_and_double_jump_state(
            car,
            input,
            on_ground,
            boost_amount,
            jump_held,
            &mut double_jump_available,
            dt,
        );
    }

    fn step_with_input_and_double_jump_state(
        car: &mut RigidBody,
        input: &ControllerInput,
        on_ground: bool,
        boost_amount: &mut f32,
        jump_held: &mut bool,
        double_jump_available: &mut bool,
        dt: f32,
    ) {
        step_with_input_and_wall(
            car,
            input,
            on_ground,
            None,
            boost_amount,
            jump_held,
            double_jump_available,
            dt,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn step_with_input_and_wall(
        car: &mut RigidBody,
        input: &ControllerInput,
        on_ground: bool,
        wall_normal: Option<Vec3>,
        boost_amount: &mut f32,
        jump_held: &mut bool,
        double_jump_available: &mut bool,
        dt: f32,
    ) {
        let mut jump_hold_time_remaining = 0.0;
        step_with_input_and_hold(
            car,
            input,
            on_ground,
            wall_normal,
            boost_amount,
            jump_held,
            double_jump_available,
            &mut jump_hold_time_remaining,
            dt,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn step_with_input_and_hold(
        car: &mut RigidBody,
        input: &ControllerInput,
        on_ground: bool,
        wall_normal: Option<Vec3>,
        boost_amount: &mut f32,
        jump_held: &mut bool,
        double_jump_available: &mut bool,
        jump_hold_time_remaining: &mut f32,
        dt: f32,
    ) {
        let mut dodge_flip_active = false;
        step_with_input_and_dodge_flip(
            car,
            input,
            on_ground,
            wall_normal,
            boost_amount,
            jump_held,
            double_jump_available,
            jump_hold_time_remaining,
            &mut dodge_flip_active,
            dt,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn step_with_input_and_dodge_flip(
        car: &mut RigidBody,
        input: &ControllerInput,
        on_ground: bool,
        wall_normal: Option<Vec3>,
        boost_amount: &mut f32,
        jump_held: &mut bool,
        double_jump_available: &mut bool,
        jump_hold_time_remaining: &mut f32,
        dodge_flip_active: &mut bool,
        dt: f32,
    ) {
        car.clear_forces();
        apply_driven_forces(
            car,
            input,
            on_ground,
            wall_normal,
            boost_amount,
            jump_held,
            double_jump_available,
            jump_hold_time_remaining,
            dodge_flip_active,
            DEFAULT_TEST_FRICTION,
            dt,
        );
        integrate::integrate_velocities(car, dt);
    }

    fn full_throttle() -> ControllerInput {
        ControllerInput {
            throttle: 1.0,
            ..Default::default()
        }
    }

    fn full_steer() -> ControllerInput {
        ControllerInput {
            steer: 1.0,
            ..Default::default()
        }
    }

    fn full_boost() -> ControllerInput {
        ControllerInput {
            boost: true,
            ..Default::default()
        }
    }

    fn full_handbrake() -> ControllerInput {
        ControllerInput {
            handbrake: true,
            ..Default::default()
        }
    }

    fn full_jump() -> ControllerInput {
        ControllerInput {
            jump: true,
            ..Default::default()
        }
    }

    fn full_pitch() -> ControllerInput {
        ControllerInput {
            pitch: Some(1.0),
            ..Default::default()
        }
    }

    fn full_yaw() -> ControllerInput {
        ControllerInput {
            yaw: Some(1.0),
            ..Default::default()
        }
    }

    fn full_roll() -> ControllerInput {
        ControllerInput {
            roll: Some(1.0),
            ..Default::default()
        }
    }

    #[test]
    fn neutral_input_applies_no_force_or_torque() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        step_with_input(
            &mut c,
            &ControllerInput::default(),
            true,
            &mut boost,
            1.0 / 60.0,
        );
        assert_eq!(c.linear_velocity, Vec3::ZERO);
        assert_eq!(c.angular_velocity, Vec3::ZERO);
        assert_eq!(boost, MAX_BOOST, "unused boost shouldn't drain");
        assert_eq!(
            c.friction, DEFAULT_TEST_FRICTION,
            "no handbrake input shouldn't touch friction"
        );
    }

    #[test]
    fn throttle_accelerates_a_grounded_car_forward() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        for _ in 0..60 {
            step_with_input(&mut c, &full_throttle(), true, &mut boost, 1.0 / 60.0);
        }
        assert!(
            c.linear_velocity.x > 0.0,
            "expected forward speed to increase, got {}",
            c.linear_velocity.x
        );
        assert!((c.linear_velocity.y).abs() < 1e-4);
        assert!((c.linear_velocity.z).abs() < 1e-4);
    }

    #[test]
    fn throttle_has_no_effect_while_airborne() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        for _ in 0..60 {
            step_with_input(&mut c, &full_throttle(), false, &mut boost, 1.0 / 60.0);
        }
        assert_eq!(
            c.linear_velocity.x, 0.0,
            "airborne throttle shouldn't add forward speed"
        );
    }

    #[test]
    fn throttle_stops_accelerating_at_max_speed() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        c.linear_velocity = Vec3::new(MAX_CAR_SPEED, 0.0, 0.0);
        step_with_input(&mut c, &full_throttle(), true, &mut boost, 1.0 / 60.0);
        assert!(
            (c.linear_velocity.x - MAX_CAR_SPEED).abs() < 1e-4,
            "expected throttle to stop pushing past MAX_CAR_SPEED, got {}",
            c.linear_velocity.x
        );
    }

    #[test]
    fn reverse_throttle_accelerates_backward() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        let input = ControllerInput {
            throttle: -1.0,
            ..Default::default()
        };
        for _ in 0..60 {
            step_with_input(&mut c, &input, true, &mut boost, 1.0 / 60.0);
        }
        assert!(
            c.linear_velocity.x < 0.0,
            "expected reverse throttle to push backward, got {}",
            c.linear_velocity.x
        );
    }

    #[test]
    fn steer_has_no_effect_on_a_stationary_car() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        step_with_input(&mut c, &full_steer(), true, &mut boost, 1.0 / 60.0);
        assert_eq!(
            c.angular_velocity,
            Vec3::ZERO,
            "a parked car shouldn't be able to turn in place"
        );
    }

    #[test]
    fn steer_yaws_a_moving_car() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        c.linear_velocity = Vec3::new(1000.0, 0.0, 0.0);
        step_with_input(&mut c, &full_steer(), true, &mut boost, 1.0 / 60.0);
        assert!(
            c.angular_velocity.z.abs() > 0.0,
            "expected a moving car to yaw under full steer, got {:?}",
            c.angular_velocity
        );
    }

    #[test]
    fn opposite_steer_yaws_the_opposite_way() {
        let mut left = car();
        let mut left_boost = MAX_BOOST;
        left.linear_velocity = Vec3::new(1000.0, 0.0, 0.0);
        step_with_input(&mut left, &full_steer(), true, &mut left_boost, 1.0 / 60.0);

        let mut right = car();
        let mut right_boost = MAX_BOOST;
        right.linear_velocity = Vec3::new(1000.0, 0.0, 0.0);
        let opposite = ControllerInput {
            steer: -1.0,
            ..Default::default()
        };
        step_with_input(&mut right, &opposite, true, &mut right_boost, 1.0 / 60.0);

        assert!(left.angular_velocity.z * right.angular_velocity.z < 0.0);
    }

    #[test]
    fn boost_accelerates_a_car_regardless_of_ground_contact() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        for _ in 0..60 {
            step_with_input(&mut c, &full_boost(), false, &mut boost, 1.0 / 60.0);
        }
        assert!(
            c.linear_velocity.x > 0.0,
            "expected boost to accelerate an airborne car, got {}",
            c.linear_velocity.x
        );
    }

    #[test]
    fn boost_drains_the_tank_over_time_and_clamps_at_zero() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        for _ in 0..600 {
            step_with_input(&mut c, &full_boost(), false, &mut boost, 1.0 / 60.0);
        }
        assert_eq!(boost, 0.0, "expected a full tank to run out within 10s");
    }

    #[test]
    fn boost_has_no_effect_when_the_tank_is_empty() {
        let mut c = car();
        let mut boost = 0.0;
        step_with_input(&mut c, &full_boost(), false, &mut boost, 1.0 / 60.0);
        assert_eq!(c.linear_velocity, Vec3::ZERO);
        assert_eq!(boost, 0.0);
    }

    #[test]
    fn boost_still_drains_at_max_speed_even_though_it_stops_accelerating() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        c.linear_velocity = Vec3::new(MAX_CAR_SPEED, 0.0, 0.0);
        step_with_input(&mut c, &full_boost(), false, &mut boost, 1.0 / 60.0);
        assert!(
            (c.linear_velocity.x - MAX_CAR_SPEED).abs() < 1e-4,
            "expected boost to stop pushing past MAX_CAR_SPEED, got {}",
            c.linear_velocity.x
        );
        assert!(
            boost < MAX_BOOST,
            "expected held boost to still drain even at max speed"
        );
    }

    #[test]
    fn handbrake_reduces_friction_while_grounded() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        step_with_input(&mut c, &full_handbrake(), true, &mut boost, 1.0 / 60.0);
        assert_eq!(
            c.friction,
            DEFAULT_TEST_FRICTION * HANDBRAKE_FRICTION_MULTIPLIER,
            "expected handbrake to reduce friction below its base value"
        );
    }

    #[test]
    fn handbrake_has_no_effect_on_friction_while_airborne() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        step_with_input(&mut c, &full_handbrake(), false, &mut boost, 1.0 / 60.0);
        assert_eq!(
            c.friction, DEFAULT_TEST_FRICTION,
            "airborne handbrake shouldn't touch friction — no wheels to lock"
        );
    }

    #[test]
    fn releasing_handbrake_restores_friction() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        step_with_input(&mut c, &full_handbrake(), true, &mut boost, 1.0 / 60.0);
        assert!(
            c.friction < DEFAULT_TEST_FRICTION,
            "handbrake should have engaged"
        );
        step_with_input(
            &mut c,
            &ControllerInput::default(),
            true,
            &mut boost,
            1.0 / 60.0,
        );
        assert_eq!(
            c.friction, DEFAULT_TEST_FRICTION,
            "releasing handbrake should restore the car's base friction"
        );
    }

    #[test]
    fn jump_gives_a_grounded_car_upward_velocity() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        step_with_input(&mut c, &full_jump(), true, &mut boost, 1.0 / 60.0);
        assert!(
            (c.linear_velocity.z - JUMP_SPEED).abs() < 1.0,
            "expected roughly JUMP_SPEED upward velocity, got {}",
            c.linear_velocity.z
        );
    }

    #[test]
    fn holding_jump_does_not_refire_every_step() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        step_with_input_and_jump_state(
            &mut c,
            &full_jump(),
            true,
            &mut boost,
            &mut jump_held,
            1.0 / 60.0,
        );
        let velocity_after_first_press = c.linear_velocity.z;
        // Still held, still (nominally) grounded — a second call with the
        // same jump_held state must not add a second impulse.
        step_with_input_and_jump_state(
            &mut c,
            &full_jump(),
            true,
            &mut boost,
            &mut jump_held,
            1.0 / 60.0,
        );
        assert!(
            c.linear_velocity.z <= velocity_after_first_press + 1.0,
            "expected holding jump to not re-fire a second impulse, \
             velocity after first press={velocity_after_first_press}, after second={}",
            c.linear_velocity.z
        );
    }

    #[test]
    fn releasing_and_repressing_jump_fires_again() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        step_with_input_and_jump_state(
            &mut c,
            &full_jump(),
            true,
            &mut boost,
            &mut jump_held,
            1.0 / 60.0,
        );
        let velocity_after_first_press = c.linear_velocity.z;
        // Release, then press again — this must fire a second impulse.
        step_with_input_and_jump_state(
            &mut c,
            &ControllerInput::default(),
            true,
            &mut boost,
            &mut jump_held,
            1.0 / 60.0,
        );
        step_with_input_and_jump_state(
            &mut c,
            &full_jump(),
            true,
            &mut boost,
            &mut jump_held,
            1.0 / 60.0,
        );
        assert!(
            c.linear_velocity.z > velocity_after_first_press,
            "expected releasing then re-pressing jump to fire again, \
             velocity after first press={velocity_after_first_press}, after second press={}",
            c.linear_velocity.z
        );
    }

    #[test]
    fn double_jump_gives_an_airborne_car_upward_velocity_when_available() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        // step_with_input's throwaway jump_held/double_jump_available both
        // start fresh (unheld, available), so a single airborne jump press
        // here is exactly the "double jump available" case.
        step_with_input(&mut c, &full_jump(), false, &mut boost, 1.0 / 60.0);
        assert!(
            (c.linear_velocity.z - JUMP_SPEED).abs() < 1.0,
            "expected roughly JUMP_SPEED upward velocity from an available double jump, got {}",
            c.linear_velocity.z
        );
    }

    #[test]
    fn double_jump_has_no_effect_when_unavailable() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        let mut double_jump_available = false;
        step_with_input_and_double_jump_state(
            &mut c,
            &full_jump(),
            false,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            1.0 / 60.0,
        );
        assert_eq!(
            c.linear_velocity.z, 0.0,
            "expected an unavailable double jump to add no upward velocity"
        );
    }

    #[test]
    fn double_jump_is_consumed_after_use_and_does_not_refire() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        let mut double_jump_available = true;
        step_with_input_and_double_jump_state(
            &mut c,
            &full_jump(),
            false,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            1.0 / 60.0,
        );
        let velocity_after_double_jump = c.linear_velocity.z;
        assert!(
            !double_jump_available,
            "expected using the double jump to consume it"
        );

        // Release, then press again while still airborne — must not fire a
        // second impulse now that the double jump is spent.
        step_with_input_and_double_jump_state(
            &mut c,
            &ControllerInput::default(),
            false,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            1.0 / 60.0,
        );
        step_with_input_and_double_jump_state(
            &mut c,
            &full_jump(),
            false,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            1.0 / 60.0,
        );
        assert!(
            (c.linear_velocity.z - velocity_after_double_jump).abs() < 1.0,
            "expected a spent double jump to not refire, velocity after first use=\
             {velocity_after_double_jump}, after second press={}",
            c.linear_velocity.z
        );
    }

    #[test]
    fn landing_restores_double_jump_availability() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        let mut double_jump_available = false;
        step_with_input_and_double_jump_state(
            &mut c,
            &ControllerInput::default(),
            true,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            1.0 / 60.0,
        );
        assert!(
            double_jump_available,
            "expected touching the ground to restore the double jump"
        );
    }

    #[test]
    fn wall_jump_pushes_an_airborne_car_outward_and_upward() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        let mut double_jump_available = true;
        let wall_normal = Vec3::new(1.0, 0.0, 0.0);
        step_with_input_and_wall(
            &mut c,
            &full_jump(),
            false,
            Some(wall_normal),
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            1.0 / 60.0,
        );
        assert!(
            (c.linear_velocity.x - WALL_JUMP_HORIZONTAL_SPEED).abs() < 1.0,
            "expected roughly WALL_JUMP_HORIZONTAL_SPEED outward velocity along the wall normal, \
             got {}",
            c.linear_velocity.x
        );
        assert!(
            (c.linear_velocity.z - JUMP_SPEED).abs() < 1.0,
            "expected roughly JUMP_SPEED upward velocity, got {}",
            c.linear_velocity.z
        );
    }

    #[test]
    fn wall_jump_has_no_effect_while_grounded() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        let mut double_jump_available = true;
        step_with_input_and_wall(
            &mut c,
            &full_jump(),
            true,
            Some(Vec3::new(1.0, 0.0, 0.0)),
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            1.0 / 60.0,
        );
        assert!(
            (c.linear_velocity.z - JUMP_SPEED).abs() < 1.0,
            "expected the ordinary ground jump, not a wall-jump push-off, got {:?}",
            c.linear_velocity
        );
        assert_eq!(
            c.linear_velocity.x, 0.0,
            "grounded jump shouldn't apply any wall push-off even if wall_normal is Some"
        );
    }

    #[test]
    fn wall_jump_takes_priority_over_double_jump_without_consuming_it() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        let mut double_jump_available = true;
        step_with_input_and_wall(
            &mut c,
            &full_jump(),
            false,
            Some(Vec3::new(1.0, 0.0, 0.0)),
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            1.0 / 60.0,
        );
        assert!(
            c.linear_velocity.x > 0.0,
            "expected a wall jump (outward velocity), not a plain double jump, got {:?}",
            c.linear_velocity
        );
        assert!(
            double_jump_available,
            "expected a wall jump to leave the double jump available (not consume it)"
        );
    }

    #[test]
    fn wall_contact_restores_double_jump_availability() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        let mut double_jump_available = false;
        step_with_input_and_wall(
            &mut c,
            &ControllerInput::default(),
            false,
            Some(Vec3::new(1.0, 0.0, 0.0)),
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            1.0 / 60.0,
        );
        assert!(
            double_jump_available,
            "expected touching a wall to restore the double jump, matching real Rocket \
             League's any-surface-contact-refills-your-second-jump rule"
        );
    }

    #[test]
    fn dodge_gives_forward_velocity_and_spin_when_pitched_in_the_air() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        let mut double_jump_available = true;
        let input = ControllerInput {
            jump: true,
            pitch: Some(1.0),
            ..Default::default()
        };
        step_with_input_and_double_jump_state(
            &mut c,
            &input,
            false,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            1.0 / 60.0,
        );
        assert!(
            (c.linear_velocity.x - DODGE_SPEED).abs() < 1.0,
            "expected roughly DODGE_SPEED forward velocity, got {}",
            c.linear_velocity.x
        );
        // A small additional contribution from air control's own
        // continuous pitch torque (applied unconditionally, same as
        // ever) is expected and tolerated here.
        assert!(
            (c.angular_velocity.y - DODGE_ANGULAR_SPEED).abs() < 1.0,
            "expected roughly DODGE_ANGULAR_SPEED spin about the right axis, got {}",
            c.angular_velocity.y
        );
    }

    #[test]
    fn dodge_gives_lateral_velocity_and_spin_when_rolled_in_the_air() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        let mut double_jump_available = true;
        let input = ControllerInput {
            jump: true,
            roll: Some(1.0),
            ..Default::default()
        };
        step_with_input_and_double_jump_state(
            &mut c,
            &input,
            false,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            1.0 / 60.0,
        );
        assert!(
            (c.linear_velocity.y - DODGE_SPEED).abs() < 1.0,
            "expected roughly DODGE_SPEED lateral velocity, got {}",
            c.linear_velocity.y
        );
        assert!(
            (c.angular_velocity.x - DODGE_ANGULAR_SPEED).abs() < 1.0,
            "expected roughly DODGE_ANGULAR_SPEED spin about the forward axis, got {}",
            c.angular_velocity.x
        );
    }

    #[test]
    fn small_stick_deflection_below_deadzone_still_gives_a_plain_double_jump() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        let mut double_jump_available = true;
        let input = ControllerInput {
            jump: true,
            pitch: Some(0.05), // below DODGE_DEADZONE (0.1)
            ..Default::default()
        };
        step_with_input_and_double_jump_state(
            &mut c,
            &input,
            false,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            1.0 / 60.0,
        );
        assert_eq!(
            c.linear_velocity.x, 0.0,
            "expected no dodge push-off from a below-deadzone stick deflection"
        );
        assert!(
            (c.linear_velocity.z - JUMP_SPEED).abs() < 1.0,
            "expected a plain double jump instead, got {}",
            c.linear_velocity.z
        );
    }

    #[test]
    fn dodge_consumes_the_double_jump_same_as_a_plain_one() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        let mut double_jump_available = true;
        let input = ControllerInput {
            jump: true,
            pitch: Some(1.0),
            ..Default::default()
        };
        step_with_input_and_double_jump_state(
            &mut c,
            &input,
            false,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            1.0 / 60.0,
        );
        assert!(
            !double_jump_available,
            "expected a dodge to spend the double jump, same as a plain one"
        );
    }

    #[test]
    fn opposite_pitch_dodges_the_opposite_direction() {
        let mut left = car();
        let mut left_boost = MAX_BOOST;
        let mut left_jump_held = false;
        let mut left_double_jump_available = true;
        let forward_input = ControllerInput {
            jump: true,
            pitch: Some(1.0),
            ..Default::default()
        };
        step_with_input_and_double_jump_state(
            &mut left,
            &forward_input,
            false,
            &mut left_boost,
            &mut left_jump_held,
            &mut left_double_jump_available,
            1.0 / 60.0,
        );

        let mut right = car();
        let mut right_boost = MAX_BOOST;
        let mut right_jump_held = false;
        let mut right_double_jump_available = true;
        let backward_input = ControllerInput {
            jump: true,
            pitch: Some(-1.0),
            ..Default::default()
        };
        step_with_input_and_double_jump_state(
            &mut right,
            &backward_input,
            false,
            &mut right_boost,
            &mut right_jump_held,
            &mut right_double_jump_available,
            1.0 / 60.0,
        );

        assert!(left.linear_velocity.x * right.linear_velocity.x < 0.0);
        assert!(left.angular_velocity.y * right.angular_velocity.y < 0.0);
    }

    #[test]
    fn a_diagonal_dodge_combines_pitch_and_roll() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        let mut double_jump_available = true;
        let input = ControllerInput {
            jump: true,
            pitch: Some(1.0),
            roll: Some(1.0),
            ..Default::default()
        };
        step_with_input_and_double_jump_state(
            &mut c,
            &input,
            false,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            1.0 / 60.0,
        );
        assert!(
            (c.linear_velocity.x - DODGE_SPEED).abs() < 1.0,
            "expected the forward component of a diagonal dodge, got {}",
            c.linear_velocity.x
        );
        assert!(
            (c.linear_velocity.y - DODGE_SPEED).abs() < 1.0,
            "expected the lateral component of a diagonal dodge, got {}",
            c.linear_velocity.y
        );
    }

    #[test]
    fn dodge_has_no_effect_while_grounded() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        let mut double_jump_available = true;
        let input = ControllerInput {
            jump: true,
            pitch: Some(1.0),
            ..Default::default()
        };
        step_with_input_and_double_jump_state(
            &mut c,
            &input,
            true,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            1.0 / 60.0,
        );
        assert_eq!(
            c.linear_velocity.x, 0.0,
            "expected no dodge push-off from a grounded jump, regardless of stick input"
        );
        assert!(
            (c.linear_velocity.z - JUMP_SPEED).abs() < 1.0,
            "expected the ordinary ground jump instead, got {}",
            c.linear_velocity.z
        );
    }

    #[test]
    fn wall_jump_fires_instead_of_a_dodge_when_touching_a_wall() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        let mut double_jump_available = true;
        let input = ControllerInput {
            jump: true,
            pitch: Some(1.0),
            ..Default::default()
        };
        step_with_input_and_wall(
            &mut c,
            &input,
            false,
            Some(Vec3::new(1.0, 0.0, 0.0)),
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            1.0 / 60.0,
        );
        // A dodge would give ~DODGE_SPEED (1400) horizontal velocity and no
        // vertical component; a wall jump gives ~WALL_JUMP_HORIZONTAL_SPEED
        // (550) horizontal plus ~JUMP_SPEED vertical — distinct enough to
        // tell the two apart.
        assert!(
            (c.linear_velocity.x - WALL_JUMP_HORIZONTAL_SPEED).abs() < 1.0,
            "expected a wall jump's push-off, not a dodge's, got {}",
            c.linear_velocity.x
        );
        assert!(
            (c.linear_velocity.z - JUMP_SPEED).abs() < 1.0,
            "expected the wall jump's upward component, got {}",
            c.linear_velocity.z
        );
    }

    #[test]
    fn air_control_has_no_effect_while_grounded() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        let input = ControllerInput {
            pitch: Some(1.0),
            yaw: Some(1.0),
            roll: Some(1.0),
            ..Default::default()
        };
        step_with_input(&mut c, &input, true, &mut boost, 1.0 / 60.0);
        assert_eq!(
            c.angular_velocity,
            Vec3::ZERO,
            "grounded air control shouldn't spin the car — steering already owns yaw on the ground"
        );
    }

    #[test]
    fn a_stationary_airborne_car_can_pitch_yaw_and_roll() {
        // Unlike ground steering, air control isn't speed-scaled — a car
        // with zero velocity should still spin freely.
        let mut c = car();
        let mut boost = MAX_BOOST;
        step_with_input(&mut c, &full_pitch(), false, &mut boost, 1.0 / 60.0);
        assert!(
            c.angular_velocity.y.abs() > 0.0,
            "expected pitch to produce angular velocity about the local right (Y) axis, got {:?}",
            c.angular_velocity
        );

        let mut c = car();
        step_with_input(&mut c, &full_yaw(), false, &mut boost, 1.0 / 60.0);
        assert!(
            c.angular_velocity.z.abs() > 0.0,
            "expected yaw to produce angular velocity about the local up (Z) axis, got {:?}",
            c.angular_velocity
        );

        let mut c = car();
        step_with_input(&mut c, &full_roll(), false, &mut boost, 1.0 / 60.0);
        assert!(
            c.angular_velocity.x.abs() > 0.0,
            "expected roll to produce angular velocity about the local forward (X) axis, got {:?}",
            c.angular_velocity
        );
    }

    #[test]
    fn no_analog_value_is_treated_as_neutral_air_control() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        step_with_input(
            &mut c,
            &ControllerInput::default(),
            false,
            &mut boost,
            1.0 / 60.0,
        );
        assert_eq!(
            c.angular_velocity,
            Vec3::ZERO,
            "pitch/yaw/roll all None (e.g. replay-derived input) should behave like neutral input"
        );
    }

    #[test]
    fn opposite_yaw_spins_the_opposite_way_in_the_air() {
        let mut left = car();
        let mut left_boost = MAX_BOOST;
        step_with_input(&mut left, &full_yaw(), false, &mut left_boost, 1.0 / 60.0);

        let mut right = car();
        let mut right_boost = MAX_BOOST;
        let opposite = ControllerInput {
            yaw: Some(-1.0),
            ..Default::default()
        };
        step_with_input(&mut right, &opposite, false, &mut right_boost, 1.0 / 60.0);

        assert!(left.angular_velocity.z * right.angular_velocity.z < 0.0);
    }

    #[test]
    fn holding_jump_after_a_ground_jump_adds_more_upward_velocity_than_a_tap() {
        let dt = 1.0 / 120.0;

        let mut tapped = car();
        let mut tapped_boost = MAX_BOOST;
        let mut tapped_jump_held = false;
        let mut tapped_double_jump_available = true;
        let mut tapped_hold_remaining = 0.0;
        step_with_input_and_hold(
            &mut tapped,
            &full_jump(),
            true,
            None,
            &mut tapped_boost,
            &mut tapped_jump_held,
            &mut tapped_double_jump_available,
            &mut tapped_hold_remaining,
            dt,
        );
        for _ in 0..12 {
            step_with_input_and_hold(
                &mut tapped,
                &ControllerInput::default(),
                true,
                None,
                &mut tapped_boost,
                &mut tapped_jump_held,
                &mut tapped_double_jump_available,
                &mut tapped_hold_remaining,
                dt,
            );
        }

        let mut held = car();
        let mut held_boost = MAX_BOOST;
        let mut held_jump_held = false;
        let mut held_double_jump_available = true;
        let mut held_hold_remaining = 0.0;
        for _ in 0..13 {
            step_with_input_and_hold(
                &mut held,
                &full_jump(),
                true,
                None,
                &mut held_boost,
                &mut held_jump_held,
                &mut held_double_jump_available,
                &mut held_hold_remaining,
                dt,
            );
        }

        assert!(
            held.linear_velocity.z > tapped.linear_velocity.z + 1.0,
            "expected holding jump to accrue more upward velocity than tapping it, \
             tapped={}, held={}",
            tapped.linear_velocity.z,
            held.linear_velocity.z
        );
    }

    #[test]
    fn releasing_jump_early_stops_the_extra_acceleration_from_a_held_ground_jump() {
        let dt = 1.0 / 120.0;
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        let mut double_jump_available = true;
        let mut hold_remaining = 0.0;

        // Press, hold for a few steps (well short of JUMP_HOLD_MAX_DURATION),
        // then release.
        for _ in 0..5 {
            step_with_input_and_hold(
                &mut c,
                &full_jump(),
                true,
                None,
                &mut boost,
                &mut jump_held,
                &mut double_jump_available,
                &mut hold_remaining,
                dt,
            );
        }
        assert!(
            hold_remaining > 0.0,
            "expected the hold window to still have time left at this point"
        );
        step_with_input_and_hold(
            &mut c,
            &ControllerInput::default(),
            true,
            None,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            &mut hold_remaining,
            dt,
        );
        assert_eq!(
            hold_remaining, 0.0,
            "expected releasing jump to immediately end the hold window"
        );
        let velocity_at_release = c.linear_velocity.z;

        // Continue with jump released for several more steps — no further
        // gain expected even though the hold window had time remaining.
        for _ in 0..10 {
            step_with_input_and_hold(
                &mut c,
                &ControllerInput::default(),
                true,
                None,
                &mut boost,
                &mut jump_held,
                &mut double_jump_available,
                &mut hold_remaining,
                dt,
            );
        }
        assert!(
            (c.linear_velocity.z - velocity_at_release).abs() < 1e-3,
            "expected no further upward velocity gain after releasing jump early, \
             velocity at release={velocity_at_release}, after={}",
            c.linear_velocity.z
        );
    }

    #[test]
    fn held_jump_stops_gaining_extra_velocity_once_the_hold_window_expires() {
        let dt = 1.0 / 120.0;
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        let mut double_jump_available = true;
        let mut hold_remaining = 0.0;

        // Press, then hold well past JUMP_HOLD_MAX_DURATION (0.2s = 24
        // steps at this dt).
        for _ in 0..30 {
            step_with_input_and_hold(
                &mut c,
                &full_jump(),
                true,
                None,
                &mut boost,
                &mut jump_held,
                &mut double_jump_available,
                &mut hold_remaining,
                dt,
            );
        }
        let velocity_at_30_steps = c.linear_velocity.z;
        assert_eq!(
            hold_remaining, 0.0,
            "expected the hold window to have fully expired by now"
        );

        // Continue holding for several more steps — no further gain
        // expected, since the window has already run out.
        for _ in 0..10 {
            step_with_input_and_hold(
                &mut c,
                &full_jump(),
                true,
                None,
                &mut boost,
                &mut jump_held,
                &mut double_jump_available,
                &mut hold_remaining,
                dt,
            );
        }
        assert!(
            (c.linear_velocity.z - velocity_at_30_steps).abs() < 1e-3,
            "expected no further upward velocity gain once the hold window has expired, \
             velocity at 30 steps={velocity_at_30_steps}, after 10 more={}",
            c.linear_velocity.z
        );
    }

    #[test]
    fn double_jump_after_a_held_ground_jump_is_not_boosted_by_the_hold_window() {
        // Regression guard: variable jump height is scoped to the ground
        // jump alone — holding jump through the ground jump's whole hold
        // window must not leak any extra acceleration into a later double
        // jump.
        let dt = 1.0 / 120.0;
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        let mut double_jump_available = true;
        let mut hold_remaining = 0.0;

        for _ in 0..24 {
            step_with_input_and_hold(
                &mut c,
                &full_jump(),
                true,
                None,
                &mut boost,
                &mut jump_held,
                &mut double_jump_available,
                &mut hold_remaining,
                dt,
            );
        }
        let velocity_after_held_ground_jump = c.linear_velocity.z;

        // Release, then press again while airborne — a plain double jump.
        step_with_input_and_hold(
            &mut c,
            &ControllerInput::default(),
            false,
            None,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            &mut hold_remaining,
            dt,
        );
        step_with_input_and_hold(
            &mut c,
            &full_jump(),
            false,
            None,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            &mut hold_remaining,
            dt,
        );

        assert!(
            (c.linear_velocity.z - (velocity_after_held_ground_jump + JUMP_SPEED)).abs() < 1.0,
            "expected the double jump to add exactly one more JUMP_SPEED kick, not an extra \
             variable-height boost left over from holding the ground jump, after held ground \
             jump={velocity_after_held_ground_jump}, after double jump={}",
            c.linear_velocity.z
        );
    }

    #[test]
    fn a_second_jump_press_cancels_a_dodges_spin() {
        let dt = 1.0 / 60.0;
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        let mut double_jump_available = true;
        let mut hold_remaining = 0.0;
        let mut dodge_flip_active = false;

        let dodge_input = ControllerInput {
            jump: true,
            pitch: Some(1.0),
            ..Default::default()
        };
        step_with_input_and_dodge_flip(
            &mut c,
            &dodge_input,
            false,
            None,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            &mut hold_remaining,
            &mut dodge_flip_active,
            dt,
        );
        assert!(
            c.angular_velocity.length() > 0.0,
            "expected the dodge to leave the car spinning, got {:?}",
            c.angular_velocity
        );
        assert!(
            dodge_flip_active,
            "expected the dodge to leave a cancelable flip active"
        );

        // Release, then press again — no directional intent needed for a
        // flip-cancel, unlike a fresh dodge.
        step_with_input_and_dodge_flip(
            &mut c,
            &ControllerInput::default(),
            false,
            None,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            &mut hold_remaining,
            &mut dodge_flip_active,
            dt,
        );
        step_with_input_and_dodge_flip(
            &mut c,
            &full_jump(),
            false,
            None,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            &mut hold_remaining,
            &mut dodge_flip_active,
            dt,
        );

        assert_eq!(
            c.angular_velocity,
            Vec3::ZERO,
            "expected the second jump press to cancel the dodge's spin outright"
        );
        assert!(
            !dodge_flip_active,
            "expected flip-cancel to spend the cancelable-flip flag"
        );
    }

    #[test]
    fn flip_cancel_does_not_touch_linear_velocity_or_the_double_jump_resource() {
        let dt = 1.0 / 60.0;
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        let mut double_jump_available = true;
        let mut hold_remaining = 0.0;
        let mut dodge_flip_active = false;

        let dodge_input = ControllerInput {
            jump: true,
            pitch: Some(1.0),
            ..Default::default()
        };
        step_with_input_and_dodge_flip(
            &mut c,
            &dodge_input,
            false,
            None,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            &mut hold_remaining,
            &mut dodge_flip_active,
            dt,
        );
        let linear_velocity_after_dodge = c.linear_velocity;
        assert!(
            !double_jump_available,
            "expected the dodge to have already spent the double jump"
        );

        step_with_input_and_dodge_flip(
            &mut c,
            &ControllerInput::default(),
            false,
            None,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            &mut hold_remaining,
            &mut dodge_flip_active,
            dt,
        );
        step_with_input_and_dodge_flip(
            &mut c,
            &full_jump(),
            false,
            None,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            &mut hold_remaining,
            &mut dodge_flip_active,
            dt,
        );

        assert_eq!(
            c.linear_velocity, linear_velocity_after_dodge,
            "expected flip-cancel to leave the dodge's own translation untouched"
        );
        assert!(
            !double_jump_available,
            "expected flip-cancel to neither consume nor restore the double jump"
        );
    }

    #[test]
    fn a_plain_double_jump_clears_a_stale_dodge_flip_flag_from_an_earlier_dodge() {
        // Regression guard: a dodge sets dodge_flip_active, and if nothing
        // ever explicitly cleared it, a much later, completely unrelated
        // plain double jump (after landing from the dodge and taking off
        // again) would incorrectly let a further press fire a flip-cancel
        // that stops nothing real.
        let dt = 1.0 / 60.0;
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        let mut double_jump_available = true;
        let mut hold_remaining = 0.0;
        let mut dodge_flip_active = false;

        let dodge_input = ControllerInput {
            jump: true,
            pitch: Some(1.0),
            ..Default::default()
        };
        step_with_input_and_dodge_flip(
            &mut c,
            &dodge_input,
            false,
            None,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            &mut hold_remaining,
            &mut dodge_flip_active,
            dt,
        );
        assert!(dodge_flip_active, "expected the dodge to set the flag");

        // Land (restores double_jump_available), then take off again and
        // fire a plain double jump (no stick input) — this must clear the
        // stale flag from the earlier dodge.
        step_with_input_and_dodge_flip(
            &mut c,
            &ControllerInput::default(),
            true,
            None,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            &mut hold_remaining,
            &mut dodge_flip_active,
            dt,
        );
        step_with_input_and_dodge_flip(
            &mut c,
            &full_jump(),
            false,
            None,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            &mut hold_remaining,
            &mut dodge_flip_active,
            dt,
        );
        assert!(
            !dodge_flip_active,
            "expected a plain double jump to clear any stale dodge_flip_active"
        );
        let angular_velocity_after_plain_double_jump = c.angular_velocity;

        // Release, then press again — must NOT fire a flip-cancel, since
        // there's no real flip active anymore.
        step_with_input_and_dodge_flip(
            &mut c,
            &ControllerInput::default(),
            false,
            None,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            &mut hold_remaining,
            &mut dodge_flip_active,
            dt,
        );
        step_with_input_and_dodge_flip(
            &mut c,
            &full_jump(),
            false,
            None,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            &mut hold_remaining,
            &mut dodge_flip_active,
            dt,
        );
        assert_eq!(
            c.angular_velocity, angular_velocity_after_plain_double_jump,
            "expected no spurious flip-cancel after an unrelated plain double jump"
        );
    }

    #[test]
    fn wall_jump_still_takes_priority_over_flip_cancel_when_touching_a_wall() {
        let dt = 1.0 / 60.0;
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        let mut double_jump_available = true;
        let mut hold_remaining = 0.0;
        let mut dodge_flip_active = false;

        let dodge_input = ControllerInput {
            jump: true,
            pitch: Some(1.0),
            ..Default::default()
        };
        step_with_input_and_dodge_flip(
            &mut c,
            &dodge_input,
            false,
            None,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            &mut hold_remaining,
            &mut dodge_flip_active,
            dt,
        );
        assert!(dodge_flip_active);

        // Release, then press again while touching a wall — must fire a
        // wall jump, not a flip-cancel.
        step_with_input_and_dodge_flip(
            &mut c,
            &ControllerInput::default(),
            false,
            None,
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            &mut hold_remaining,
            &mut dodge_flip_active,
            dt,
        );
        step_with_input_and_dodge_flip(
            &mut c,
            &full_jump(),
            false,
            Some(Vec3::new(1.0, 0.0, 0.0)),
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            &mut hold_remaining,
            &mut dodge_flip_active,
            dt,
        );

        assert!(
            c.linear_velocity.x > 0.0,
            "expected the wall jump's outward push-off, got {:?}",
            c.linear_velocity
        );
        assert!(
            c.angular_velocity.length() > 0.0,
            "expected the wall jump to leave the dodge's spin untouched (not flip-canceled), \
             got {:?}",
            c.angular_velocity
        );
        assert!(
            dodge_flip_active,
            "expected the wall jump to leave the cancelable flip flag untouched"
        );
    }
}
