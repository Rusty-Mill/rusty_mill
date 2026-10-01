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
//! Pitch, yaw, and roll each apply torque about one of the car's three
//! local axes (right, up, forward respectively), scaled directly by the
//! analog `Option<f32>` value — `None` (an input source that can't recover
//! an analog value, e.g. replay-derived input, see `rb_domain`) is treated
//! as zero, same as a centered stick. Unlike ground steering, air control
//! isn't speed-scaled: a car can spin freely from a standing start in the
//! air, since there's no wheel grip to require momentum for. Since
//! `RB-PHYSICS-001-FR-068`, the three axes no longer share one flat
//! `AIR_CONTROL_TORQUE` magnitude: yaw and roll are scaled from pitch's own
//! by `AIR_CONTROL_YAW_SCALE`/`AIR_CONTROL_ROLL_SCALE`, RocketSim's own
//! confirmed real per-axis ratios (`CAR_AIR_CONTROL_TORQUE = Vec(130, 95,
//! 400)` in pitch-yaw-roll order) — real Rocket League's pitch/yaw/roll
//! rates still differ in absolute magnitude too, not just this ratio, but
//! only the ratio transfers cleanly onto this port's own uncalibrated
//! pitch baseline (see `AIR_CONTROL_TORQUE`'s own doc comment). Real air
//! control also applies a per-axis angular-velocity damping torque this
//! port has no equivalent for at all — see `AIR_CONTROL_ROLL_SCALE`'s own
//! doc comment for `RB-PHYSICS-001-FR-071`'s full finding. Since
//! `RB-PHYSICS-001-FR-057`, sustained air control (or a dodge's own kick,
//! or the landing-orientation assist) can no longer spin a car arbitrarily
//! fast, though: `clamp_angular_speed` caps the result at
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
//! roll) plus an instantaneous `DODGE_ANGULAR_SPEED` spin about the
//! perpendicular axis (`right_axis` for pitch, `forward_axis` for roll),
//! with RocketSim's signs: stick forward (pitch -1) dodges forward and
//! noses down, like a fast air-control pitch (`RB-PHYSICS-001-FR-082`). Since `RB-PHYSICS-001-FR-073`, the roll axis's own
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
//! component too, not modeled here. Since `RB-PHYSICS-001-FR-069`, real
//! Rocket League's own dodge spin is confirmed to be a continuous torque
//! applied every step for `FLIP_TORQUE_TIME` (0.65s) with no decay, not
//! this port's own single instantaneous `DODGE_ANGULAR_SPEED` kick, and
//! the real torque also genuinely differs between pitch and roll — see
//! `DODGE_ANGULAR_SPEED`'s own doc comment for the full finding and why
//! it isn't adopted. Below `DODGE_DEADZONE` on both axes,
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
//! impulse and `DODGE_ANGULAR_SPEED` spin (identical axis/sign conventions
//! to the ground dodge), leaving a cancelable flip behind
//! (`dodge_flip_active`) just like a ground dodge does. Unlike the plain
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
//! whether `jump` is still held (scaled down by `JUMP_PRE_MIN_ACCEL_SCALE`)
//! — real Rocket League's own `_UpdateJump` has this same mandatory
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
//! A dodge's spin can be canceled early — **flip-cancel** — by pressing
//! jump again before landing or wall contact: a fresh press while airborne,
//! not touching a wall, `double_jump_available` already spent (so this
//! isn't a wall jump or another double jump/dodge), and `dodge_flip_active`
//! still set, zeroes `RigidBody.angular_velocity` outright and clears
//! `dodge_flip_active` — stopping the flip immediately. This applies
//! equally to a wall-jump dodge's spin, since it also consumes
//! `double_jump_available` and sets `dodge_flip_active` exactly like a
//! ground dodge does. It doesn't touch linear velocity (the dodge's own
//! translation is unaffected) and doesn't consume or restore
//! `double_jump_available` (already spent by the dodge that set the flag).
//! This port has no timed flip animation to interrupt (a dodge is one
//! instantaneous angular-velocity kick, not a sustained torque over a fixed
//! duration — see above), so "mid-flip" here means "any time before
//! landing or a wall touch re-arms the double jump," a documented
//! simplification of real Rocket League's actual flip-duration window.
//! `RB-PHYSICS-001-FR-070` fetched RocketSim's real `Car.cpp` and found real
//! Rocket League's own flip-cancel is a substantially different mechanism
//! from this jump-press trigger and outright zeroing: it's driven by
//! *holding pitch* (not pressing jump again) in the same direction as the
//! dodge's own pitch-torque component, continuously, every tick, for as
//! long as `isFlipping` holds — `pitchScale = 1 - abs(controls.pitch)`
//! scales down (not zeros, unless pitch is fully held) only the flip's
//! *pitch-axis* torque component, leaving any roll-axis component
//! untouched; a sideways (roll-only) dodge has no pitch-torque component at
//! all and so can't be canceled by pitch input in real Rocket League at
//! all, unlike this port's own jump-press cancel, which zeros every axis
//! uniformly regardless of dodge direction. Not adopted: this port's dodge
//! is a single flat angular-velocity kick with no per-axis torque split to
//! partially cancel (`RB-PHYSICS-001-FR-069` already confirmed this same
//! architecture gap for the dodge's own spin), and real flip-cancel's input
//! channel (a continuously-held stick) is a different trigger shape than
//! this port's discrete jump-press-again gate — reproducing it would need
//! the same continuous per-axis torque and elapsed-flip-time state
//! `RB-PHYSICS-001-FR-059`'s own Non-goals already flagged as out of scope.
//! Wall jump keeps priority over flip-cancel on a fresh press while
//! touching a wall, unchanged. A plain double jump explicitly clears
//! `dodge_flip_active` rather than leaving it alone, so a stale flag from
//! an earlier dodge (long since landed from) can't make a *later*,
//! unrelated plain double jump's next press incorrectly fire a flip-cancel.
//!
//! **Landing auto-orientation assistance**: while airborne, with no active
//! `pitch`/`roll` air control input this step and no fresh jump press this
//! step (so the assist never fights the player's own stick input, and
//! never interacts within one `apply_driven_forces` call with a
//! dodge/wall-jump-dodge/double-jump/flip-cancel's own direct velocity or
//! angular-velocity change), a gentle restoring torque nudges the car's
//! local up axis toward world up: `up_axis(car).cross(&world_up)` gives
//! both the correction axis and, since both are unit vectors, a magnitude
//! already proportional to the sine of the tilt angle — a level car earns
//! no correction, a heavily tilted one earns a stronger nudge. This isn't a
//! simplification of one specific real system: `RB-PHYSICS-001-FR-060`
//! fetched RocketSim's real `Car.cpp` and found real Rocket League has no
//! single mechanic matching "continuously nudge an airborne car upright
//! with no player input." It instead has two distinct, real, *grounded*,
//! input-gated systems — **auto-flip** (a turtle-recovery flip, firing only
//! on an actual jump press while touching a mostly-upright surface
//! (`CAR_AUTOFLIP_NORMZ_THRESH`) with roll already past a threshold
//! (`CAR_AUTOFLIP_ROLL_THRESH`), timed over `CAR_AUTOFLIP_TIME`) and
//! **auto-roll** (a continuous torque aligning the car to the ground's
//! surface normal, but only while throttle is held and at least one wheel
//! has contact) — neither of which is this port's own airborne, input-free
//! nudge. This port's `LANDING_AUTO_UPRIGHT_TORQUE` remains its own
//! invented placeholder for "eventually right yourself before landing," not
//! a documented simplification of either real system; implementing either
//! for real would mean adding new grounded, input-gated state machinery
//! this port doesn't have, out of scope here — see `RB-PHYSICS-001-FR-060`'s
//! own Non-goals. A car resting *exactly* upside-down (`up` antiparallel to
//! world up) is a singularity this simple scheme doesn't resolve (the cross
//! product is exactly zero, any perpendicular axis would do, but none is
//! chosen) — an unlikely exact case, not addressed here.
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
//! `JUMP_MIN_TIME`/`JUMP_PRE_MIN_ACCEL_SCALE` are commonly-cited,
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
//! placeholder torque: since `RB-PHYSICS-001-FR-080` it sets the yaw rate
//! from RocketSim's real steer-angle curves through a bicycle model over
//! the Octane wheelbase (`ground::steer_yaw_rate`, ADR-0011), resolving
//! the wrong-shape finding `RB-PHYSICS-001-FR-065` recorded.
//! The handbrake no longer uses a placeholder friction multiplier: since
//! `RB-PHYSICS-001-FR-081` it scales the tire model's per-axis grip by
//! RocketSim's real factors, resolving `RB-PHYSICS-001-FR-066`'s
//! wrong-shape finding. `LANDING_AUTO_UPRIGHT_TORQUE`
//! remains an uncalibrated placeholder chosen
//! only to produce a visibly responsive flip for
//! this car's mass/inertia in tests — in
//! particular it isn't a simplification of one real system at all, since
//! `RB-PHYSICS-001-FR-060` found real Rocket League's two closest systems
//! (auto-flip, auto-roll) are both grounded and input-gated, unlike this
//! port's own airborne, input-free nudge — see that module doc section's
//! own detail. `WALL_JUMP_HORIZONTAL_SPEED` remains an uncalibrated
//! placeholder too, but since `RB-PHYSICS-001-FR-067` real Rocket League is
//! confirmed to have no distinct wall-jump mechanic or constant to
//! calibrate against at all — see that requirement's own entry and
//! `WALL_JUMP_HORIZONTAL_SPEED`'s own doc comment for the full finding.
//! `AIR_CONTROL_TORQUE` itself (pitch's own magnitude) remains an
//! uncalibrated placeholder too, but since `RB-PHYSICS-001-FR-068` its own
//! per-axis *ratio* — unlike `WALL_JUMP_HORIZONTAL_SPEED`'s own
//! confirmed-but-not-adopted finding —
//! is confirmed and directly adopted: yaw and roll are scaled from pitch's
//! own by RocketSim's own confirmed real ratios, since real air control
//! turned out to be the same *kind* of direct per-axis torque mechanism
//! this port already models, unlike those other findings' own
//! architecture mismatches — see that requirement's own entry and
//! `AIR_CONTROL_YAW_SCALE`'s own doc comment for the full finding.
//! `DODGE_ANGULAR_SPEED` itself remains an uncalibrated placeholder too,
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

pub use boost::MAX_BOOST;

/// `ground::steer_yaw_rate`, exposed to sibling modules' tests only.
#[cfg(test)]
pub(crate) fn steer_yaw_rate_for_tests(
    forward_speed: f32,
    steer: f32,
    handbrake_amount: f32,
) -> f32 {
    ground::steer_yaw_rate(forward_speed, steer, handbrake_amount)
}
pub use jump::{DODGE_SPEED, JUMP_SPEED, WALL_JUMP_HORIZONTAL_SPEED};

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
/// Coincidentally equal to this port's own pre-existing
/// `DODGE_ANGULAR_SPEED` placeholder further below — chosen independently,
/// before this cap existed, only to look visibly fast in tests, not
/// derived from this same real value (see that constant's own doc
/// comment). The two serve different purposes (an instantaneous kick
/// magnitude vs. a continuous hard ceiling on the result); nothing here
/// depends on them staying numerically equal.
///
/// Only covers this port's own driven-forces sources (continuous air
/// control torque integrated this step, plus any single-step direct
/// `angular_velocity` write like a dodge's kick or the landing-orientation
/// assist) — a same-step contact-solver impulse (e.g. a hard collision
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
/// (forward, right, up) basis `up_axis × forward_axis` gives. Used only for
/// pitch torque (nose up/down about this axis); throttle/steer/boost/
/// handbrake/jump never need it.
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
    /// *before* this call's own ground-jump press can re-arm it, so a fresh
    /// press's own step only fires the plain `JUMP_SPEED` impulse; the press
    /// then re-arms it to `JUMP_HOLD_MAX_DURATION`. Since
    /// `RB-PHYSICS-001-FR-064`, releasing `jump` inside the first
    /// `JUMP_MIN_TIME` seconds doesn't zero it: that mandatory window keeps
    /// decrementing it at a `JUMP_PRE_MIN_ACCEL_SCALE`-scaled acceleration.
    /// Untouched by the double jump, a dodge, or the wall jump.
    pub jump_hold_time_remaining: f32,
    /// Whether the car's most recent double-jump-or-dodge press was a dodge
    /// whose spin hasn't been canceled or superseded yet. A dodge sets it,
    /// a plain double jump clears it (so a stale `true` can't leak into a
    /// later unrelated double jump), and a further fresh airborne press
    /// with no wall contact and no double jump left spends it to cancel the
    /// flip — see the module doc comment's flip-cancel paragraph.
    pub dodge_flip_active: bool,
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
            dodge_flip_active: false,
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
/// jump, and air control as forces/torques/impulses (or, for steering and
/// tire grip, direct velocity changes) on `car`, reading and updating `state`
/// (see each `DriveState` field for its rules). Throttle, steering,
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
    on_ground: bool,
    wall_normal: Option<Vec3>,
    state: &mut DriveState,
    dt: f32,
) {
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

    if on_ground {
        // Landing (or simply resting) always restores the double jump,
        // regardless of this step's input.
        state.double_jump_available = true;
        // RocketSim treats a boosting car as full throttle for its pedals.
        let throttle = if input.boost && state.boost_amount > 0.0 {
            1.0
        } else {
            input.throttle.clamp(-1.0, 1.0)
        };
        ground::apply_ground_control(car, input, throttle, forward, state.handbrake_amount, dt);
        if jump_pressed {
            jump::ground_jump(car, &mut state.jump_hold_time_remaining);
        }
    } else {
        if wall_normal.is_some() {
            // Touching a wall restores the double jump unconditionally —
            // the same "any surface contact refills your second jump"
            // rule landing uses — regardless of whether jump is pressed.
            state.double_jump_available = true;
        }
        air::apply_air_control(car, input, forward, jump_pressed);
        if jump_pressed {
            jump::airborne_jump_press(
                car,
                input,
                forward,
                wall_normal,
                &mut state.double_jump_available,
                &mut state.dodge_flip_active,
            );
        }
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

/// Scales `car.angular_velocity` back down to `MAX_CAR_ANGULAR_SPEED` if
/// its length exceeds it, preserving direction — a no-op otherwise. Call
/// once per step, right after `integrate::integrate_velocities`, so it
/// sees this step's fully-integrated angular velocity (this function's own
/// caller, and `apply_driven_forces`'s doc comment, cover why the ordering
/// matters: torque applied by `apply_driven_forces` isn't reflected in
/// `angular_velocity` until `integrate_velocities` runs).
pub fn clamp_angular_speed(car: &mut RigidBody) {
    let speed = car.angular_velocity.length();
    if speed > MAX_CAR_ANGULAR_SPEED {
        car.angular_velocity *= MAX_CAR_ANGULAR_SPEED / speed;
    }
}
