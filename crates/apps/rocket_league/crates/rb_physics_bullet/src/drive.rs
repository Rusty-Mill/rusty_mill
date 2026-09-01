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
//! grip model. Since `RB-PHYSICS-001-FR-066`, real Rocket League's own
//! handbrake friction reduction is confirmed genuinely anisotropic (a
//! separate, much milder reduction to forward/backward grip than to
//! sideways grip) rather than the single uniform multiplier this port
//! applies to both — see `HANDBRAKE_FRICTION_MULTIPLIER`'s own doc
//! comment for the full finding and why it isn't adopted.
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
//! other; this port doesn't model that difference. Since
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
//! perpendicular axis (`right_axis` for pitch, `forward_axis` for roll) —
//! the same axis/sign conventions air control's own pitch/roll torque
//! already uses, so a forward dodge looks like a fast version of a forward
//! air-control pitch. Both pitch and roll can contribute at once (a
//! diagonal dodge), simply summed — a documented simplification, since real
//! Rocket League normalizes the stick direction so a diagonal dodge isn't
//! faster than an axis-aligned one, and this port doesn't. Since
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
//! `dodge_flip_active` — stopping the flip immediately, matching real
//! Rocket League. This applies equally to a wall-jump dodge's spin, since
//! it also consumes `double_jump_available` and sets `dodge_flip_active`
//! exactly like a ground dodge does. It doesn't touch linear velocity (the
//! dodge's own translation is unaffected) and doesn't consume or restore
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
//! behaves exactly as a free rigid box always has — this module only ever
//! adds force/torque/impulse or adjusts the existing friction property,
//! never removes physics outright.
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
//! `THROTTLE_ACCELERATION` already has. `STEER_TORQUE` itself remains an
//! uncalibrated placeholder, but since `RB-PHYSICS-001-FR-065` its own
//! `speed_factor` scale-up is confirmed to have the wrong *shape*, not
//! merely an uncalibrated magnitude — see that requirement's own entry and
//! `STEER_TORQUE`'s own doc comment for the full finding.
//! `HANDBRAKE_FRICTION_MULTIPLIER` itself likewise remains an uncalibrated
//! placeholder, but since `RB-PHYSICS-001-FR-066` its own single uniform
//! reduction is confirmed to have the wrong *shape* too — real Rocket
//! League applies a genuinely anisotropic (direction-dependent) reduction,
//! not this port's one isotropic factor — see that requirement's own
//! entry and `HANDBRAKE_FRICTION_MULTIPLIER`'s own doc comment for the
//! full finding. `AIR_CONTROL_TORQUE` and
//! `DODGE_ANGULAR_SPEED`, and
//! `LANDING_AUTO_UPRIGHT_TORQUE` remain uncalibrated placeholders chosen
//! only to produce a visibly responsive turn/spin/flip for
//! this car's mass/inertia in tests — `LANDING_AUTO_UPRIGHT_TORQUE` in
//! particular isn't a simplification of one real system at all, since
//! `RB-PHYSICS-001-FR-060` found real Rocket League's two closest systems
//! (auto-flip, auto-roll) are both grounded and input-gated, unlike this
//! port's own airborne, input-free nudge — see that module doc section's
//! own detail. `WALL_JUMP_HORIZONTAL_SPEED` remains an uncalibrated
//! placeholder too, but since `RB-PHYSICS-001-FR-067` real Rocket League is
//! confirmed to have no distinct wall-jump mechanic or constant to
//! calibrate against at all — see that requirement's own entry and
//! `WALL_JUMP_HORIZONTAL_SPEED`'s own doc comment for the full finding.
//! `RB-PHYSICS-001-FR-031`'s audit
//! found real reference numbers for some of these (a dodge's real ~500
//! uu/s base impulse; a wall jump reusing the plain jump impulse rather
//! than its own faster speed, confirmed exact by `RB-PHYSICS-001-FR-067`;
//! real air-control torque/damping
//! coefficients), but none of them port directly: they're expressed as
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

use crate::body::RigidBody;
use rb_domain::{ControllerInput, Vec3};

/// Commonly-cited boosted top speed (uu/s), boost's own speed cap here —
/// confirmed against real Rocket League's own reverse-engineered constants
/// during `RB-PHYSICS-001-FR-031`'s audit (`CAR_MAX_SPEED = 2300.f` in the
/// RocketSim project's `RLConst.h`; matched independently by RLUtilities'
/// `Car::v_max` and the RLBot community wiki's "Useful Game Values" page).
/// Also used as the turning-torque scale-up reference in `speed_factor`
/// below — an arbitrary normalization choice, not a claim that a car's
/// actual turning grip caps out at boosted speed specifically.
/// `RB-PHYSICS-001-FR-065` found `speed_factor`'s own scale-up *direction*
/// against this reference doesn't match real Rocket League's own steering
/// model at all — see `STEER_TORQUE`'s own doc comment for the finding.
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

/// Peak throttle acceleration (uu/s^2), at a standing start — still an
/// uncalibrated placeholder pending calibration against recorded data
/// (unlike the taper shape it's scaled by, see `drive_speed_taper` below,
/// this magnitude itself has no confirmed real-world source). Since
/// `RB-PHYSICS-001-FR-058`, this is no longer applied flat: it's scaled by
/// `drive_speed_taper`'s own real curve as speed rises, tapering smoothly
/// to zero at `UNBOOSTED_MAX_CAR_SPEED` instead of applying at full
/// strength right up to a hard cutoff.
const THROTTLE_ACCELERATION: f32 = 1600.0;

/// Real Rocket League's own speed-dependent drive-force taper —
/// RocketSim's `DRIVE_SPEED_TORQUE_FACTOR_CURVE`, a 3-point piecewise-
/// linear curve confirmed exact against its own `RLConst.h` during
/// `RB-PHYSICS-001-FR-058`'s audit: full torque from a standing start
/// (`(0, 1.0)`), tapering linearly down to 10% by `1400` uu/s
/// (`(1400, 0.1)`), then a final, much steeper linear drop to exactly
/// zero at `UNBOOSTED_MAX_CAR_SPEED` (`(1410, 0.0)`). Fetching RocketSim's
/// own `Car.cpp` confirmed this curve is looked up by the car's *signed*
/// forward speed (`abs()`'d there, since real RocketSim's own throttle
/// gate isn't direction-aware) and multiplied directly against the drive
/// force applied to each wheel — a pure, unitless ratio, unlike
/// `THROTTLE_TORQUE_AMOUNT` (RocketSim's own name for this project's
/// `THROTTLE_ACCELERATION`), which is expressed in Bullet's own internal
/// mass/distance units and doesn't transfer to this port's own
/// differently-calibrated car body the same clean way — see
/// `RB-PHYSICS-001-FR-031`'s and `FR-057`'s own "false precision" findings
/// for absolute torque/force magnitudes. Only the curve's *shape* is
/// adopted here, not a new peak magnitude.
const DRIVE_SPEED_TAPER_BREAKPOINTS: [(f32, f32); 3] =
    [(0.0, 1.0), (1400.0, 0.1), (UNBOOSTED_MAX_CAR_SPEED, 0.0)];

/// Linearly interpolates `DRIVE_SPEED_TAPER_BREAKPOINTS` at
/// `signed_speed_in_throttle_direction` (the same
/// `throttle.signum() * forward_speed` quantity `apply_driven_forces`'s
/// own throttle gate already computes) — `1.0` (full acceleration) at or
/// below the first breakpoint, `0.0` at or beyond the last. Deliberately
/// evaluated against this port's own pre-existing *signed*,
/// direction-aware speed (clamped to non-negative here, since a negative
/// value means "not yet moving this way," which should read as a
/// standing start, not an out-of-range lookup) rather than switching to
/// real RocketSim's own direction-agnostic `abs(forward speed)` — that
/// would be a second, independent behavioral change (whether accelerating
/// against your own current motion tapers too) this requirement doesn't
/// take on; see its own Non-goals.
fn drive_speed_taper(signed_speed_in_throttle_direction: f32) -> f32 {
    let speed = signed_speed_in_throttle_direction.max(0.0);
    let points = DRIVE_SPEED_TAPER_BREAKPOINTS;
    if speed <= points[0].0 {
        return points[0].1;
    }
    for window in points.windows(2) {
        let (x0, y0) = window[0];
        let (x1, y1) = window[1];
        if speed <= x1 {
            return y0 + (y1 - y0) * (speed - x0) / (x1 - x0);
        }
    }
    points[points.len() - 1].1
}

/// Uncalibrated placeholder steering torque magnitude (about the car's
/// local up axis, at full `steer` input and at/above `MAX_CAR_SPEED`) —
/// chosen only so a full-lock turn is visibly responsive for this car's
/// mass/inertia in tests, not derived from any measured or documented
/// Rocket League value.
///
/// `RB-PHYSICS-001-FR-065` fetched RocketSim's real `Car.cpp` (`_UpdateWheels`,
/// matching `RB-PHYSICS-001-FR-058`/`FR-059`/`FR-064`'s own
/// real-implementation-file method) and found real Rocket League's
/// steering isn't a direct yaw-torque model at all: a wheel's *steer
/// angle* (not a torque) is set from a confirmed real
/// `STEER_ANGLE_FROM_SPEED_CURVE` (`RLConst.h`, radians), and that angled
/// wheel's lateral tire friction — computed per-wheel by `btVehicleRL`, a
/// custom extension of Bullet's own raycast vehicle system
/// (`btDefaultVehicleRaycaster`), through a further confirmed
/// `LAT_FRICTION_CURVE` slip-friction curve — is what actually turns the
/// car. This port has no wheels, raycasting, or tire-slip model at all
/// (the car is one rigid box), so this real mechanism can't be ported
/// without a substantially larger architecture change, the same category
/// `RB-PHYSICS-001-FR-063` already established for per-contact-pair-type
/// restitution/friction.
///
/// One finding is still directly actionable even without that larger
/// change: the confirmed real curve's own *shape* is the opposite of this
/// port's own `speed_factor` below. Real Rocket League's maximum steering
/// angle is highest at a standstill (`0.53356` rad ≈ 30.6° at 0 uu/s) and
/// decreases sharply as speed rises (down to `0.03454` rad ≈ 2° at 3000
/// uu/s) — a car can turn tightest from a stop, only gently at speed. This
/// port's own `speed_factor` does the opposite: zero torque at a
/// standstill, scaling *up* to full `STEER_TORQUE` at `MAX_CAR_SPEED`.
/// Not adopted as a fix: unlike `RB-PHYSICS-001-FR-058`'s throttle taper
/// or `FR-059`'s dodge scale (direct multipliers on a force/impulse this
/// port already applies the same way real Rocket League does), the real
/// curve maps speed to a *wheel angle*, which real Rocket League then
/// feeds through nonlinear tire-slip friction (dependent on wheelbase
/// geometry and friction curves this port doesn't model at all) to
/// produce the actual turning force — there's no principled way to carry
/// even the curve's normalized shape onto this port's own direct-torque
/// model. Reversing `speed_factor`'s direction without that transfer
/// function would substitute one unconfirmed guess for another, not adopt
/// a confirmed real value — the same reasoning that kept
/// `RB-PHYSICS-001-FR-057`'s `AIR_CONTROL_TORQUE` and `FR-059`'s
/// `DODGE_SPEED` base magnitude as placeholders despite a real reference
/// existing for each.
const STEER_TORQUE: f32 = 1_500_000.0;

/// Boost acceleration while grounded (uu/s^2) — unlike throttle, boost
/// doesn't taper with speed in real Rocket League, so this (like
/// `BOOST_ACCELERATION_AIR` below) is a flat constant, not a curve.
/// Confirmed exact against RocketSim's own `RLConst.h`
/// (`BOOST_ACCEL_GROUND = 2975.f / 3.f`, fetched directly during
/// `RB-PHYSICS-001-FR-056`) — written as the same fraction the reference
/// uses, matching `JUMP_SPEED`'s own precedent for an exact fractional
/// source value, rather than that fraction's earlier `991.667` decimal
/// approximation (the two are equal to float precision; this is a
/// clarity change, not a value change).
const BOOST_ACCELERATION_GROUND: f32 = 2975.0 / 3.0;

/// Boost acceleration while airborne (uu/s^2) — genuinely different from
/// `BOOST_ACCELERATION_GROUND`, not a rounding of the same number.
/// `RB-PHYSICS-001-FR-056` fetched RocketSim's own `RLConst.h` directly
/// and found `BOOST_ACCEL_AIR = 3175.f / 3.f`, distinctly higher than the
/// grounded value — a split this port's own earlier single flat
/// `BOOST_ACCELERATION` constant didn't model at all (every airborne
/// boost this crate ever applied used the *grounded* number, understating
/// real airborne boost strength by about 6.5%). `apply_driven_forces`
/// now selects between the two by `on_ground`, matching the reference
/// split exactly — a genuine behavioral fix, not just a doc correction,
/// found via the same "fetch primary source directly" method this
/// project already applies throughout (see `RB-PHYSICS-001-FR-031`'s own
/// audit and every reference-validation FR since).
const BOOST_ACCELERATION_AIR: f32 = 3175.0 / 3.0;

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
///
/// `RB-PHYSICS-001-FR-066` fetched RocketSim's real `Car.cpp`
/// (`_UpdateWheels`, matching `RB-PHYSICS-001-FR-058`/`FR-059`/`FR-064`/
/// `FR-065`'s own real-implementation-file method) and found real Rocket
/// League's own handbrake friction reduction is genuinely anisotropic, not
/// a single uniform multiplier: two separate confirmed real curves,
/// `HANDBRAKE_LAT_FRICTION_FACTOR_CURVE` (`0.1` at every speed — this
/// value's own coincidental exact match to this port's own `0.1` is
/// striking but not a confirmation, see below) and
/// `HANDBRAKE_LONG_FRICTION_FACTOR_CURVE` (`0.5` at a standstill, `0.9` at
/// and above 1 uu/s — effectively a near-constant, barely-reduced `0.9`
/// for any real driving speed), are applied to lateral and longitudinal
/// tire friction independently. Real Rocket League's handbrake drift
/// keeps a car's forward/backward grip almost intact (`x0.9`) while
/// cutting sideways grip to a tenth (`x0.1`) — this port's own single
/// isotropic `RigidBody.friction` scalar, read identically by both of
/// `solver::friction_directions`' own two tangent rows, has no way to
/// apply a different factor to each direction without threading a second,
/// direction-specific friction coefficient through every one of
/// `solver.rs`'s several row-limit-computation call sites
/// (`resolve_contacts`, `resolve_contacts_between`,
/// `resolve_static_manifolds`, `resolve_dynamic_manifolds`,
/// `resolve_manifolds`) — a substantially larger architecture change than
/// this finding alone justifies, the same category
/// `RB-PHYSICS-001-FR-063`/`FR-065` already established. `0.1`'s own
/// coincidental match to the real lateral-only factor is exactly that: a
/// coincidence, since this port's uniform `0.1` also (wrongly) crushes
/// longitudinal grip to a tenth, where real Rocket League keeps it near
/// `0.9` — this port's own handbrake understates real forward-momentum
/// retention during a drift. Not adopted as a fix; left for a future,
/// dedicated requirement.
const HANDBRAKE_FRICTION_MULTIPLIER: f32 = 0.1;

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
/// it directly, the same way `JUMP_SPEED` already is. This is the
/// standing-start (and forward-dodge) magnitude specifically — since
/// `RB-PHYSICS-001-FR-059`, a backward or side dodge made at speed scales
/// above this via `dodge_speed_scale`, matching RocketSim's own confirmed
/// per-direction speed dependence, even though this base value itself
/// remains unconfirmed.
pub const DODGE_SPEED: f32 = 1400.0;

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
const DODGE_BACKWARD_SPEED_SCALE: f32 = 2.5;

/// Confirmed real ratio: a side (`roll`) dodge grows up to this multiple of
/// `DODGE_SPEED` as current speed rises toward `MAX_CAR_SPEED`, regardless
/// of left/right direction — RocketSim's own confirmed
/// `FLIP_SIDE_IMPULSE_MAX_SPEED_SCALE = 1.9f`. See `DODGE_BACKWARD_SPEED_SCALE`'s
/// own doc comment for the same "ratio adopted, base magnitude not"
/// caveat.
const DODGE_SIDE_SPEED_SCALE: f32 = 1.9;

/// Below this current forward speed (uu/s), `dodge_pitch_is_backward`
/// falls back to stick direction alone rather than comparing it against a
/// near-zero, noisy current-velocity direction — RocketSim's own
/// confirmed threshold, `abs(forwardSpeed_UU) < 100.0f`.
const DODGE_BACKWARD_CLASSIFICATION_SPEED_THRESHOLD: f32 = 100.0;

/// A dodge's real per-axis magnitude scale as a function of the car's
/// current forward speed — confirmed against RocketSim's own `Car.cpp`
/// during `RB-PHYSICS-001-FR-059`'s audit: `1.0` (no change) at a standing
/// start, rising linearly to `scale_at_max_speed` by `MAX_CAR_SPEED`, then
/// held flat beyond it (`forward_speed` can in principle exceed
/// `MAX_CAR_SPEED` transiently, e.g. after a boosted speed-flip chain;
/// RocketSim's own real ratio isn't itself clamped in the source, but this
/// port clamps it here rather than let an already-uncalibrated dodge
/// magnitude grow unbounded past the one confirmed reference point).
fn dodge_speed_scale(forward_speed: f32, scale_at_max_speed: f32) -> f32 {
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
/// (`shouldDodgeBackwards`), re-derived in this port's own sign convention
/// (positive `dodge_pitch` means forward, matching `apply_driven_forces`'s
/// own `dodge_impulse += forward * (dodge_pitch * DODGE_SPEED)`) rather
/// than translated symbol-for-symbol from the reference's own stick-sign
/// convention.
fn dodge_pitch_is_backward(dodge_pitch: f32, forward_speed: f32) -> bool {
    if forward_speed.abs() < DODGE_BACKWARD_CLASSIFICATION_SPEED_THRESHOLD {
        dodge_pitch < 0.0
    } else {
        (dodge_pitch >= 0.0) != (forward_speed >= 0.0)
    }
}

/// Uncalibrated placeholder dodge spin speed (rad/s), added directly to
/// `RigidBody.angular_velocity` as an instantaneous change (mirroring how
/// `apply_impulse` directly changes `linear_velocity`, rather than
/// `apply_torque`'s continuous accumulation, since a dodge's flip is a
/// single instantaneous kick, not a sustained torque) — chosen only to
/// produce a visibly fast flip in tests, not derived from any measured or
/// documented Rocket League value. Numerically equal to
/// `MAX_CAR_ANGULAR_SPEED` above, confirmed only since
/// `RB-PHYSICS-001-FR-057` — a coincidence, not a shared derivation: this
/// constant predates that cap and was picked independently, so a dodge
/// kick landing exactly at the cap rather than comfortably under or over
/// it isn't a deliberate design choice either way.
const DODGE_ANGULAR_SPEED: f32 = 5.5;

/// Maximum duration (seconds) that continuing to hold `jump` after a fresh
/// ground-jump press keeps adding extra upward acceleration
/// (`JUMP_HOLD_ACCELERATION`). Confirmed, not just guessed, during
/// `RB-PHYSICS-001-FR-031`'s audit: this port's pre-existing `0.2` already
/// matches both RocketSim's `RLConst.h` (`JUMP_MAX_TIME = 0.2f`) and
/// RLUtilities' `Jump::max_duration = 0.2f`. Real Rocket League also has a
/// `JUMP_MIN_TIME` (0.025s) during which the hold acceleration is scaled
/// down (`JUMP_PRE_MIN_ACCEL_SCALE`) rather than applied at full strength
/// immediately — since `RB-PHYSICS-001-FR-064`, that two-phase ramp is
/// modeled too, see `JUMP_MIN_TIME`'s own doc comment.
const JUMP_HOLD_MAX_DURATION: f32 = 0.2;

/// Continuous upward acceleration (uu/s^2) applied every step `jump` is
/// held and `JUMP_HOLD_MAX_DURATION` hasn't yet elapsed since the ground
/// jump's own fresh press, on top of that press's fixed `JUMP_SPEED`
/// impulse. Refined from an earlier `1400.0` approximation to the precise
/// value during `RB-PHYSICS-001-FR-031`'s audit: RocketSim's `RLConst.h`
/// defines `JUMP_ACCEL = 4375.f/3.f`, matched independently by RLUtilities'
/// `Jump::acceleration = 1458.3333f`. Scaled down by `JUMP_PRE_MIN_ACCEL_SCALE`
/// during `JUMP_MIN_TIME`'s own mandatory window — see that constant's own
/// doc comment.
const JUMP_HOLD_ACCELERATION: f32 = 4375.0 / 3.0;

/// Seconds after a ground-jump press during which `JUMP_HOLD_ACCELERATION`
/// (scaled by `JUMP_PRE_MIN_ACCEL_SCALE`) keeps applying regardless of
/// whether `jump` is still held — a mandatory minimum hold real Rocket
/// League's own engine applies even to an instantaneous tap. Confirmed
/// exact against RocketSim's real `RLConst.h` (`JUMP_MIN_TIME = 0.025f`)
/// during `RB-PHYSICS-001-FR-064`; fetching the same reference's actual
/// `Car.cpp` (`_UpdateJump`) directly confirmed the exact mechanism —
/// `jumpTime < JUMP_MIN_TIME || (jumpPressed && jumpTime < JUMP_MAX_TIME)`
/// gates whether the force applies at all, with the pre-`JUMP_MIN_TIME`
/// branch scaling it down regardless of `jumpPressed` — not merely the
/// constant's existence. That same source's own inline comment (`// TODO:
/// Either move to RLConst or preferably don't use this system at all`)
/// flags this as a stopgap even its own authors consider provisional, not a
/// deliberate permanent design choice — adopted here anyway since it's
/// still the real, currently-shipping behavior, and both this and
/// `JUMP_PRE_MIN_ACCEL_SCALE` are a duration and a dimensionless ratio
/// respectively, not a torque or force calibrated against real Rocket
/// League's own specific car mass/inertia, so unlike most of `drive.rs`'s
/// own torque-shaped placeholders (see the module doc comment's own
/// "false precision" discussion) they transfer cleanly regardless of this
/// port's car body not matching that calibration.
const JUMP_MIN_TIME: f32 = 0.025;

/// Multiplier applied to `JUMP_HOLD_ACCELERATION` during `JUMP_MIN_TIME`'s
/// own mandatory window. Confirmed exact against RocketSim's real
/// `Car.cpp` (`_UpdateJump`, `constexpr float JUMP_PRE_MIN_ACCEL_SCALE =
/// 0.62f;`) during `RB-PHYSICS-001-FR-064` — a hard step-scale applied
/// all-or-nothing for the whole window (`totalJumpForce *=
/// JUMP_PRE_MIN_ACCEL_SCALE`), not an interpolation ramping from `0.62` up
/// to `1.0` as `JUMP_MIN_TIME` approaches.
const JUMP_PRE_MIN_ACCEL_SCALE: f32 = 0.62;

/// Uncalibrated placeholder landing-auto-orientation restoring-torque
/// magnitude — applied while airborne with no active `pitch`/`roll` air
/// control input, scaled by `up_axis(car).cross(&world_up)` (already
/// proportional to the sine of the car's tilt off level, since both
/// vectors are unit length, so a bigger tilt earns a stronger nudge and an
/// already-level car earns none). Chosen only to be a visibly gentler
/// correction than full active air control (`AIR_CONTROL_TORQUE`) for this
/// car's mass/inertia in tests — a full order of magnitude smaller — not
/// derived from any measured or documented Rocket League value; this port
/// has no public reference for the real assist's actual strength or
/// trigger condition either (see the module doc comment).
const LANDING_AUTO_UPRIGHT_TORQUE: f32 = 100_000.0;

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
/// `JUMP_HOLD_MAX_DURATION` for subsequent calls to consume. Since
/// `RB-PHYSICS-001-FR-064`, releasing `jump` doesn't always zero it right
/// away: `JUMP_MIN_TIME` seconds' own mandatory window (derived as
/// `JUMP_HOLD_MAX_DURATION - *jump_hold_time_remaining < JUMP_MIN_TIME`)
/// keeps decrementing it, at a `JUMP_PRE_MIN_ACCEL_SCALE`-scaled
/// acceleration, regardless of `input.jump` — only past that window does
/// releasing `jump` stop the extra acceleration immediately. It's otherwise
/// untouched by the double jump, a dodge, or the wall jump — see the module
/// doc comment. `dodge_flip_active` is whether
/// the car's most recent double-jump-or-dodge press was a dodge whose spin
/// hasn't been canceled or superseded yet: the dodge branch sets it `true`,
/// the plain-double-jump branch explicitly sets it `false` (so a stale
/// `true` from an earlier, already-landed-from dodge can't leak into a
/// later unrelated double jump), and a further fresh press while airborne,
/// not touching a wall, with `double_jump_available` already spent and
/// `dodge_flip_active` still `true` cancels the flip — see the module doc
/// comment's flip-cancel paragraph. Since `RB-PHYSICS-001-FR-037`, any
/// genuinely active `input` (see `input_is_active`) wakes `car`
/// unconditionally before anything else in this call runs, regardless of
/// whether `car` was already asleep or what velocity results this step —
/// see this crate's own `body::RigidBody::wake` doc comment for why a
/// velocity-only wake check isn't enough here. Call once per step, before
/// `integrate::integrate_velocities`, alongside `apply_gravity`; follow it
/// with `clamp_angular_speed` right *after* that same
/// `integrate_velocities` call, so `MAX_CAR_ANGULAR_SPEED` sees this step's
/// fully-integrated angular velocity, torque contributions included.
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
    // RB-PHYSICS-001-FR-037: any genuinely active input wakes the car
    // unconditionally, before that input's own force/impulse has a chance
    // to move it — a resultant-velocity-only wake check would zero right
    // back out a driving force whose one-frame delta is itself smaller
    // than `body::LINEAR_SLEEP_VELOCITY_THRESHOLD` (e.g. one frame of
    // throttle from a dead stop at a very small `dt`), permanently
    // stranding an asleep car that should be free to start moving. Uses
    // `unwrap_or(0.0)` for the analog channels so a recovered-but-literally-
    // zero `Some(0.0)` reads as neutral the same way an unrecovered `None`
    // does — see `input_is_active`'s own doc comment.
    if input_is_active(input) {
        car.wake();
    }

    let forward = forward_axis(car);
    let jump_pressed = input.jump && !*jump_held;
    *jump_held = input.jump;

    // Variable jump height: apply the continuous hold acceleration using
    // whatever jump_hold_time_remaining the *previous* call left behind,
    // before this call's own on_ground/jump_pressed handling below can
    // re-arm it — so a fresh ground-jump press's own step here never gets
    // the extra force, only continued holding (or the mandatory window
    // below) into later calls does. RB-PHYSICS-001-FR-064: real Rocket
    // League's own `_UpdateJump` keeps applying this force, scaled by
    // `JUMP_PRE_MIN_ACCEL_SCALE`, for the first `JUMP_MIN_TIME` seconds
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
    if in_mandatory_pre_min_window || (input.jump && *jump_hold_time_remaining > 0.0) {
        let mut hold_acceleration = JUMP_HOLD_ACCELERATION;
        if in_mandatory_pre_min_window {
            hold_acceleration *= JUMP_PRE_MIN_ACCEL_SCALE;
        }
        car.apply_central_force(Vec3::new(0.0, 0.0, hold_acceleration * car.mass()));
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
        if throttle != 0.0 {
            let taper = drive_speed_taper(throttle.signum() * forward_speed);
            if taper > 0.0 {
                car.apply_central_force(
                    forward * (throttle * THROTTLE_ACCELERATION * taper * car.mass()),
                );
            }
        }

        let steer = input.steer.clamp(-1.0, 1.0);
        if steer != 0.0 {
            // A stationary car can't carve a turn — scale the available
            // torque by how fast it's already going, up to MAX_CAR_SPEED.
            // RB-PHYSICS-001-FR-065: real Rocket League's own confirmed
            // steering curve has this backwards — maximum turning ability
            // is highest at a standstill and decreases with speed — but
            // that curve doesn't transfer onto this port's own
            // direct-torque model; see STEER_TORQUE's own doc comment.
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

        // Landing auto-orientation assistance: with no active pitch/roll
        // air control this step (so the assist never fights the player's
        // own input) and no fresh jump press this step (so it never
        // interacts, within the same integrate_velocities call, with a
        // dodge/wall-jump-dodge/double-jump/flip-cancel's own direct
        // velocity or angular-velocity change — those already dominate the
        // car's rotation for that instant anyway), gently nudge the car's
        // local up axis toward world up. `up.cross(&world_up)` gives both
        // the correction axis and, since both are unit vectors, a
        // magnitude already proportional to the sine of the tilt angle —
        // a level car (or one resting exactly upside-down, an unlikely
        // singularity this simple scheme doesn't resolve) gets no
        // correction, a heavily tilted one gets a stronger nudge. See the
        // module doc comment for why this applies continuously whenever
        // airborne rather than only near the ground.
        if pitch == 0.0 && roll == 0.0 && !jump_pressed {
            let world_up = Vec3::new(0.0, 0.0, 1.0);
            let correction_axis = up_axis(car).cross(&world_up);
            if correction_axis.length() > 0.0 {
                car.apply_torque(correction_axis * LANDING_AUTO_UPRIGHT_TORQUE);
            }
        }

        if jump_pressed {
            if let Some(wall_normal) = wall_normal {
                // Wall jump takes priority over the double jump on this
                // press: push off outward along the wall's normal, plus
                // the same upward JUMP_SPEED every jump variant uses.
                let wall_pitch = input.pitch.unwrap_or(0.0).clamp(-1.0, 1.0);
                let wall_roll = input.roll.unwrap_or(0.0).clamp(-1.0, 1.0);
                if wall_pitch.abs() > DODGE_DEADZONE || wall_roll.abs() > DODGE_DEADZONE {
                    // Wall-jump dodge: the same outward-plus-upward push
                    // combined with a directional DODGE_SPEED impulse and
                    // DODGE_ANGULAR_SPEED spin, reusing the ground dodge's
                    // own axis/sign conventions. Unlike the plain wall jump
                    // below, this *does* consume double_jump_available —
                    // the same resource a ground dodge spends — a
                    // deliberate simplification (see the module doc
                    // comment). Leaves a cancelable flip active
                    // (dodge_flip_active), same as a ground dodge.
                    let wall_jump_forward_speed = car.linear_velocity.dot(&forward);
                    let mut dodge_impulse =
                        wall_normal * WALL_JUMP_HORIZONTAL_SPEED + Vec3::new(0.0, 0.0, JUMP_SPEED);
                    let mut dodge_spin = Vec3::ZERO;
                    if wall_pitch.abs() > DODGE_DEADZONE {
                        let scale = if dodge_pitch_is_backward(wall_pitch, wall_jump_forward_speed)
                        {
                            dodge_speed_scale(wall_jump_forward_speed, DODGE_BACKWARD_SPEED_SCALE)
                        } else {
                            1.0
                        };
                        dodge_impulse += forward * (wall_pitch * DODGE_SPEED * scale);
                        dodge_spin += right_axis(car) * (wall_pitch * DODGE_ANGULAR_SPEED);
                    }
                    if wall_roll.abs() > DODGE_DEADZONE {
                        let scale =
                            dodge_speed_scale(wall_jump_forward_speed, DODGE_SIDE_SPEED_SCALE);
                        dodge_impulse += right_axis(car) * (wall_roll * DODGE_SPEED * scale);
                        dodge_spin += forward * (wall_roll * DODGE_ANGULAR_SPEED);
                    }
                    car.apply_impulse(dodge_impulse * car.mass(), Vec3::ZERO);
                    car.angular_velocity += dodge_spin;
                    *dodge_flip_active = true;
                    *double_jump_available = false;
                } else {
                    // Plain wall jump (unchanged): doesn't consume
                    // double_jump_available (already restored
                    // unconditionally above, just from touching the wall)
                    // — matching real Rocket League's "any surface contact
                    // refills your second jump" rule, so a plain wall jump
                    // doesn't cost a player their double jump.
                    car.apply_impulse(
                        (wall_normal * WALL_JUMP_HORIZONTAL_SPEED
                            + Vec3::new(0.0, 0.0, JUMP_SPEED))
                            * car.mass(),
                        Vec3::ZERO,
                    );
                }
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
                    let dodge_forward_speed = car.linear_velocity.dot(&forward);
                    let mut dodge_impulse = Vec3::ZERO;
                    let mut dodge_spin = Vec3::ZERO;
                    if dodge_pitch.abs() > DODGE_DEADZONE {
                        let scale = if dodge_pitch_is_backward(dodge_pitch, dodge_forward_speed) {
                            dodge_speed_scale(dodge_forward_speed, DODGE_BACKWARD_SPEED_SCALE)
                        } else {
                            1.0
                        };
                        dodge_impulse += forward * (dodge_pitch * DODGE_SPEED * scale);
                        dodge_spin += right_axis(car) * (dodge_pitch * DODGE_ANGULAR_SPEED);
                    }
                    if dodge_roll.abs() > DODGE_DEADZONE {
                        let scale = dodge_speed_scale(dodge_forward_speed, DODGE_SIDE_SPEED_SCALE);
                        dodge_impulse += right_axis(car) * (dodge_roll * DODGE_SPEED * scale);
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
            // RB-PHYSICS-001-FR-056: real Rocket League's own boost
            // acceleration is genuinely higher airborne than grounded —
            // `on_ground` gates which magnitude applies, not whether
            // boost applies at all (it always does, regardless of ground
            // contact, per this function's own doc comment above).
            let boost_acceleration = if on_ground {
                BOOST_ACCELERATION_GROUND
            } else {
                BOOST_ACCELERATION_AIR
            };
            car.apply_central_force(forward * (boost_acceleration * car.mass()));
        }
        // Held boost drains the tank even when the force above didn't
        // apply (e.g. already at MAX_CAR_SPEED, or pushing into a wall) —
        // matching real Rocket League, where holding boost costs fuel
        // regardless of whether it's doing anything.
        *boost_amount = (*boost_amount - BOOST_CONSUMPTION_RATE * dt).max(0.0);
    }
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
        clamp_angular_speed(car);
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
    fn throttle_stops_accelerating_at_unboosted_max_speed() {
        // RB-PHYSICS-001-FR-031: throttle's own cap is UNBOOSTED_MAX_CAR_SPEED
        // (1410 uu/s), not the boosted MAX_CAR_SPEED (2300) — see
        // UNBOOSTED_MAX_CAR_SPEED's own doc comment for why these are now
        // two separate constants.
        let mut c = car();
        let mut boost = MAX_BOOST;
        c.linear_velocity = Vec3::new(UNBOOSTED_MAX_CAR_SPEED, 0.0, 0.0);
        step_with_input(&mut c, &full_throttle(), true, &mut boost, 1.0 / 60.0);
        assert!(
            (c.linear_velocity.x - UNBOOSTED_MAX_CAR_SPEED).abs() < 1e-4,
            "expected throttle to stop pushing past UNBOOSTED_MAX_CAR_SPEED, got {}",
            c.linear_velocity.x
        );
    }

    #[test]
    fn throttle_alone_cannot_reach_the_boosted_top_speed() {
        // The real bug RB-PHYSICS-001-FR-031 fixed: before the audit,
        // throttle shared MAX_CAR_SPEED (2300, the *boosted* cap) as its
        // own ceiling, letting a car reach boosted top speed on throttle
        // alone. Held throttle for a generous 20 simulated seconds (no
        // drag to fight against) should now plateau at UNBOOSTED_MAX_CAR_SPEED,
        // well short of MAX_CAR_SPEED.
        let mut c = car();
        let mut boost = MAX_BOOST;
        let dt = 1.0 / 60.0;
        for _ in 0..(20.0 / dt) as u32 {
            step_with_input(&mut c, &full_throttle(), true, &mut boost, dt);
        }
        assert!(
            // Since RB-PHYSICS-001-FR-058, this is a genuine taper rather
            // than a hard per-step cutoff — acceleration tapers smoothly to
            // zero over the last 10 uu/s below UNBOOSTED_MAX_CAR_SPEED, so
            // overshoot here should in practice be far smaller than a full
            // THROTTLE_ACCELERATION*dt step; the generous +30.0 margin is
            // kept only to avoid a brittle exact-value assertion.
            c.linear_velocity.x <= UNBOOSTED_MAX_CAR_SPEED + 30.0,
            "expected throttle alone to cap out at UNBOOSTED_MAX_CAR_SPEED, got {}",
            c.linear_velocity.x
        );
        assert!(
            c.linear_velocity.x < MAX_CAR_SPEED - 1.0,
            "expected throttle alone to stay well short of the boosted MAX_CAR_SPEED, got {}",
            c.linear_velocity.x
        );
    }

    #[test]
    fn drive_speed_taper_matches_the_real_curve_breakpoints_exactly() {
        // RB-PHYSICS-001-FR-058: RocketSim's own DRIVE_SPEED_TORQUE_FACTOR_CURVE
        // is (0, 1.0), (1400, 0.1), (1410, 0.0) — confirmed exact against
        // its own RLConst.h.
        assert_eq!(drive_speed_taper(0.0), 1.0);
        assert!(
            (drive_speed_taper(1400.0) - 0.1).abs() < 1e-6,
            "expected the 1400 uu/s breakpoint to be 0.1, got {}",
            drive_speed_taper(1400.0)
        );
        assert_eq!(drive_speed_taper(UNBOOSTED_MAX_CAR_SPEED), 0.0);
        // Linear interpolation partway along each segment.
        assert!(
            (drive_speed_taper(700.0) - 0.55).abs() < 1e-4,
            "expected the midpoint of the first segment (0, 1.0)-(1400, 0.1) \
             to interpolate to 0.55, got {}",
            drive_speed_taper(700.0)
        );
        assert!(
            (drive_speed_taper(1405.0) - 0.05).abs() < 1e-4,
            "expected the midpoint of the final segment (1400, 0.1)-(1410, 0.0) \
             to interpolate to 0.05, got {}",
            drive_speed_taper(1405.0)
        );
        // Clamped outside the curve's own domain in both directions.
        assert_eq!(
            drive_speed_taper(-100.0),
            1.0,
            "expected a negative (not-yet-moving-this-way) input to clamp to full torque"
        );
        assert_eq!(
            drive_speed_taper(2000.0),
            0.0,
            "expected a speed past the curve's own domain to clamp to zero"
        );
    }

    #[test]
    fn throttle_acceleration_tapers_well_before_reaching_unboosted_max_speed() {
        // RB-PHYSICS-001-FR-058: before this requirement, throttle applied
        // THROTTLE_ACCELERATION at full strength right up to a hard cutoff
        // at UNBOOSTED_MAX_CAR_SPEED. Real Rocket League's own curve is
        // already down to 10% strength at 1400 uu/s (10 short of the cap)
        // — this test would fail (see a ~1600 uu/s^2-scale delta instead)
        // without the taper.
        let mut c = car();
        let mut boost = MAX_BOOST;
        let dt = 1.0 / 60.0;
        c.linear_velocity = Vec3::new(1400.0, 0.0, 0.0);
        step_with_input(&mut c, &full_throttle(), true, &mut boost, dt);
        let delta = c.linear_velocity.x - 1400.0;
        let expected_full_strength_delta = THROTTLE_ACCELERATION * dt;
        assert!(
            (delta - 0.1 * expected_full_strength_delta).abs() < 1e-3,
            "expected the delta at 1400 uu/s to be ~10% of a full-strength step \
             ({}), got {}",
            0.1 * expected_full_strength_delta,
            delta
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
    fn boost_accelerates_an_airborne_car_faster_than_a_grounded_one() {
        // RB-PHYSICS-001-FR-056: real Rocket League's own boost
        // acceleration is genuinely higher airborne than grounded
        // (RocketSim's own RLConst.h: BOOST_ACCEL_AIR = 3175/3 vs
        // BOOST_ACCEL_GROUND = 2975/3) -- this port's own earlier single
        // flat BOOST_ACCELERATION constant collapsed both into the
        // grounded number, understating airborne boost. One step's worth
        // of full boost from a dead stop produces a velocity delta of
        // exactly `boost_acceleration * dt` regardless of mass (force is
        // `boost_acceleration * mass`, so it cancels on integration),
        // making the exact ratio between the two directly checkable.
        let mut grounded = car();
        let mut grounded_boost = MAX_BOOST;
        step_with_input(
            &mut grounded,
            &full_boost(),
            true,
            &mut grounded_boost,
            1.0 / 60.0,
        );

        let mut airborne = car();
        let mut airborne_boost = MAX_BOOST;
        step_with_input(
            &mut airborne,
            &full_boost(),
            false,
            &mut airborne_boost,
            1.0 / 60.0,
        );

        assert!(
            airborne.linear_velocity.x > grounded.linear_velocity.x,
            "expected airborne boost ({}) to accelerate faster than grounded boost ({})",
            airborne.linear_velocity.x,
            grounded.linear_velocity.x
        );
        let ratio = airborne.linear_velocity.x / grounded.linear_velocity.x;
        let expected_ratio = BOOST_ACCELERATION_AIR / BOOST_ACCELERATION_GROUND;
        assert!(
            (ratio - expected_ratio).abs() < 1e-4,
            "expected the airborne/grounded ratio to match RocketSim's own \
             BOOST_ACCEL_AIR/BOOST_ACCEL_GROUND ratio ({expected_ratio}), got {ratio}"
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
    fn dodge_speed_scale_matches_the_real_curve() {
        // RB-PHYSICS-001-FR-059: RocketSim's own
        // FLIP_BACKWARD_IMPULSE_MAX_SPEED_SCALE (2.5) and
        // FLIP_SIDE_IMPULSE_MAX_SPEED_SCALE (1.9) confirmed exact against
        // its own RLConst.h.
        assert_eq!(dodge_speed_scale(0.0, 2.5), 1.0);
        assert_eq!(dodge_speed_scale(MAX_CAR_SPEED, 2.5), 2.5);
        assert!(
            (dodge_speed_scale(MAX_CAR_SPEED / 2.0, 2.5) - 1.75).abs() < 1e-4,
            "expected the midpoint to interpolate to 1.75, got {}",
            dodge_speed_scale(MAX_CAR_SPEED / 2.0, 2.5)
        );
        assert_eq!(
            dodge_speed_scale(MAX_CAR_SPEED * 2.0, 2.5),
            2.5,
            "expected speed past MAX_CAR_SPEED to clamp"
        );
        assert_eq!(
            dodge_speed_scale(-MAX_CAR_SPEED, 1.9),
            1.9,
            "expected the scale to use the speed's magnitude, not its sign"
        );
    }

    #[test]
    fn dodge_pitch_is_backward_matches_the_real_classification() {
        // Below DODGE_BACKWARD_CLASSIFICATION_SPEED_THRESHOLD, classification
        // falls back to stick direction alone.
        assert!(dodge_pitch_is_backward(-1.0, 0.0));
        assert!(!dodge_pitch_is_backward(1.0, 0.0));
        // At speed, classification compares dodge direction to current
        // motion: opposing directions count as "backward" (a real backward
        // dodge, or a forward dodge while already moving backward).
        assert!(dodge_pitch_is_backward(-1.0, MAX_CAR_SPEED));
        assert!(dodge_pitch_is_backward(1.0, -MAX_CAR_SPEED));
        assert!(!dodge_pitch_is_backward(1.0, MAX_CAR_SPEED));
        assert!(!dodge_pitch_is_backward(-1.0, -MAX_CAR_SPEED));
    }

    #[test]
    fn a_backward_dodge_scales_up_with_current_forward_speed() {
        // RB-PHYSICS-001-FR-059: a dodge opposing the car's own current
        // motion now scales up toward DODGE_BACKWARD_SPEED_SCALE as speed
        // rises, matching RocketSim's own confirmed
        // FLIP_BACKWARD_IMPULSE_MAX_SPEED_SCALE — this test would see a
        // plain DODGE_SPEED-sized delta instead, without the scale.
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        let mut double_jump_available = true;
        c.linear_velocity = Vec3::new(MAX_CAR_SPEED, 0.0, 0.0);
        let before = c.linear_velocity.x;
        let input = ControllerInput {
            jump: true,
            pitch: Some(-1.0),
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
        let delta = c.linear_velocity.x - before;
        assert!(
            (delta - (-DODGE_SPEED * DODGE_BACKWARD_SPEED_SCALE)).abs() < 1.0,
            "expected a backward dodge at max speed to scale to DODGE_SPEED \
             * DODGE_BACKWARD_SPEED_SCALE, got delta {}",
            delta
        );
    }

    #[test]
    fn a_forward_dodge_does_not_scale_with_current_forward_speed() {
        // The real forward-dodge scale is exactly 1.0 (RocketSim's own
        // FLIP_FORWARD_IMPULSE_MAX_SPEED_SCALE) — a forward dodge stays at
        // DODGE_SPEED regardless of current speed, unlike a backward or
        // side dodge.
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        let mut double_jump_available = true;
        c.linear_velocity = Vec3::new(MAX_CAR_SPEED, 0.0, 0.0);
        let before = c.linear_velocity.x;
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
        let delta = c.linear_velocity.x - before;
        assert!(
            (delta - DODGE_SPEED).abs() < 1.0,
            "expected a forward dodge to stay at plain DODGE_SPEED \
             regardless of current speed, got delta {}",
            delta
        );
    }

    #[test]
    fn a_side_dodge_scales_up_with_current_forward_speed() {
        // RB-PHYSICS-001-FR-059: a side (roll) dodge scales up toward
        // DODGE_SIDE_SPEED_SCALE as current forward speed rises, regardless
        // of direction, matching RocketSim's own confirmed
        // FLIP_SIDE_IMPULSE_MAX_SPEED_SCALE.
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        let mut double_jump_available = true;
        c.linear_velocity = Vec3::new(MAX_CAR_SPEED, 0.0, 0.0);
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
            (c.linear_velocity.y - DODGE_SPEED * DODGE_SIDE_SPEED_SCALE).abs() < 1.0,
            "expected a side dodge at max speed to scale to DODGE_SPEED * \
             DODGE_SIDE_SPEED_SCALE, got {}",
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
    fn a_wall_jump_dodges_outward_and_upward_with_a_flip_when_touching_a_wall_with_stick_input() {
        // Regression guard for the *reversed* premise: a wall jump used to
        // always ignore stick input; now directional stick input at or
        // above DODGE_DEADZONE fires a wall-jump dodge instead, combining
        // the wall's own push-off with a DODGE_SPEED horizontal component
        // and a visible spin.
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
        assert!(
            (c.linear_velocity.x - (WALL_JUMP_HORIZONTAL_SPEED + DODGE_SPEED)).abs() < 1.0,
            "expected the wall push-off plus the forward dodge component, got {}",
            c.linear_velocity.x
        );
        assert!(
            (c.linear_velocity.z - JUMP_SPEED).abs() < 1.0,
            "expected the wall jump's upward component, got {}",
            c.linear_velocity.z
        );
        assert!(
            c.angular_velocity.length() > 0.0,
            "expected the wall-jump dodge to give the car a visible flip, got {:?}",
            c.angular_velocity
        );
    }

    #[test]
    fn a_wall_jump_dodge_consumes_the_double_jump_unlike_a_plain_wall_jump() {
        let dt = 1.0 / 60.0;

        let mut dodging = car();
        let mut dodging_boost = MAX_BOOST;
        let mut dodging_jump_held = false;
        let mut dodging_double_jump_available = true;
        let dodge_input = ControllerInput {
            jump: true,
            pitch: Some(1.0),
            ..Default::default()
        };
        step_with_input_and_wall(
            &mut dodging,
            &dodge_input,
            false,
            Some(Vec3::new(1.0, 0.0, 0.0)),
            &mut dodging_boost,
            &mut dodging_jump_held,
            &mut dodging_double_jump_available,
            dt,
        );
        assert!(
            !dodging_double_jump_available,
            "expected a wall-jump dodge to consume the double jump, unlike a plain wall jump"
        );

        let mut plain = car();
        let mut plain_boost = MAX_BOOST;
        let mut plain_jump_held = false;
        let mut plain_double_jump_available = true;
        step_with_input_and_wall(
            &mut plain,
            &full_jump(),
            false,
            Some(Vec3::new(1.0, 0.0, 0.0)),
            &mut plain_boost,
            &mut plain_jump_held,
            &mut plain_double_jump_available,
            dt,
        );
        assert!(
            plain_double_jump_available,
            "expected a plain wall jump (no stick input) to leave the double jump available"
        );
    }

    #[test]
    fn a_wall_jump_dodges_spin_can_be_flip_cancelled() {
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
            Some(Vec3::new(1.0, 0.0, 0.0)),
            &mut boost,
            &mut jump_held,
            &mut double_jump_available,
            &mut hold_remaining,
            &mut dodge_flip_active,
            dt,
        );
        assert!(
            dodge_flip_active,
            "expected the wall-jump dodge to set the flag"
        );

        // Release, then press again while no longer touching a wall —
        // flip-cancel.
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
            "expected the second jump press to cancel the wall-jump dodge's spin outright"
        );
        assert!(!dodge_flip_active);
    }

    #[test]
    fn below_deadzone_stick_input_at_a_wall_still_gives_a_plain_wall_jump() {
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        let mut double_jump_available = true;
        let input = ControllerInput {
            jump: true,
            pitch: Some(0.05), // below DODGE_DEADZONE (0.1)
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
        assert!(
            (c.linear_velocity.x - WALL_JUMP_HORIZONTAL_SPEED).abs() < 1.0,
            "expected a plain wall jump from a below-deadzone stick deflection, got {}",
            c.linear_velocity.x
        );
        // A small additional contribution from air control's own
        // continuous pitch torque (applied unconditionally while airborne,
        // same as ever) is expected and tolerated here — only the flip's
        // own DODGE_ANGULAR_SPEED-scale kick must be absent.
        assert!(
            c.angular_velocity.length() < 1.0,
            "expected no dodge-scale flip from a below-deadzone stick deflection, got {:?}",
            c.angular_velocity
        );
        assert!(
            double_jump_available,
            "expected a plain wall jump to leave the double jump available"
        );
    }

    #[test]
    fn opposite_pitch_wall_jump_dodges_the_opposite_direction() {
        let wall_normal = Vec3::new(1.0, 0.0, 0.0);

        let mut left = car();
        let mut left_boost = MAX_BOOST;
        let mut left_jump_held = false;
        let mut left_double_jump_available = true;
        let forward_input = ControllerInput {
            jump: true,
            pitch: Some(1.0),
            ..Default::default()
        };
        step_with_input_and_wall(
            &mut left,
            &forward_input,
            false,
            Some(wall_normal),
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
        step_with_input_and_wall(
            &mut right,
            &backward_input,
            false,
            Some(wall_normal),
            &mut right_boost,
            &mut right_jump_held,
            &mut right_double_jump_available,
            1.0 / 60.0,
        );

        // Both still get the same fixed wall push-off along wall_normal;
        // only the dodge's own forward-axis contribution should differ in
        // sign, so compare velocity relative to the fixed WALL_JUMP_HORIZONTAL_SPEED
        // baseline rather than raw velocity.
        assert!(
            (left.linear_velocity.x - WALL_JUMP_HORIZONTAL_SPEED)
                > (right.linear_velocity.x - WALL_JUMP_HORIZONTAL_SPEED)
        );
        assert!(left.angular_velocity.y * right.angular_velocity.y < 0.0);
    }

    #[test]
    fn a_diagonal_wall_jump_dodge_combines_pitch_and_roll() {
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
        assert!(
            (c.linear_velocity.x - (WALL_JUMP_HORIZONTAL_SPEED + DODGE_SPEED)).abs() < 1.0,
            "expected the wall push-off plus the forward dodge component, got {}",
            c.linear_velocity.x
        );
        assert!(
            (c.linear_velocity.y - DODGE_SPEED).abs() < 1.0,
            "expected the lateral dodge component, got {}",
            c.linear_velocity.y
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
    fn jump_hold_acceleration_is_scaled_down_during_the_mandatory_pre_min_time_window() {
        // RB-PHYSICS-001-FR-064: real Rocket League's own `_UpdateJump`
        // scales the hold acceleration by `JUMP_PRE_MIN_ACCEL_SCALE` for the
        // first `JUMP_MIN_TIME` seconds after a ground-jump press, applied
        // regardless of whether `jump` is still held.
        let dt = 1.0 / 120.0;
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        let mut double_jump_available = true;
        let mut hold_remaining = 0.0;

        // Press: arms the window; only the fixed JUMP_SPEED impulse fires
        // this step.
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
        let velocity_after_press = c.linear_velocity.z;

        // The very next step is still well within JUMP_MIN_TIME (0.025s = 3
        // steps at this dt).
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

        let expected_gain = JUMP_HOLD_ACCELERATION * JUMP_PRE_MIN_ACCEL_SCALE * dt;
        assert!(
            (c.linear_velocity.z - (velocity_after_press + expected_gain)).abs() < 1e-2,
            "expected the mandatory pre-min-time window's own scaled acceleration, \
             got a gain of {}, expected {}",
            c.linear_velocity.z - velocity_after_press,
            expected_gain
        );
    }

    #[test]
    fn releasing_jump_within_the_mandatory_pre_min_time_window_does_not_immediately_stop_the_extra_acceleration(
    ) {
        // Unlike releasing jump after JUMP_MIN_TIME has already elapsed (see
        // releasing_jump_early_stops_the_extra_acceleration_from_a_held_ground_jump,
        // which releases well past it), releasing within the mandatory
        // window doesn't end it early — real Rocket League's own engine
        // keeps applying the scaled acceleration regardless of `jump`.
        let dt = 1.0 / 120.0;
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        let mut double_jump_available = true;
        let mut hold_remaining = 0.0;

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
        let velocity_after_press = c.linear_velocity.z;

        // Release immediately (a tap) — still within JUMP_MIN_TIME.
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

        let expected_gain = JUMP_HOLD_ACCELERATION * JUMP_PRE_MIN_ACCEL_SCALE * dt;
        assert!(
            (c.linear_velocity.z - (velocity_after_press + expected_gain)).abs() < 1e-2,
            "expected a tap to still gain the mandatory window's own scaled acceleration \
             despite releasing jump immediately, got a gain of {}, expected {}",
            c.linear_velocity.z - velocity_after_press,
            expected_gain
        );
        assert!(
            hold_remaining > 0.0,
            "expected the mandatory window to still have time left, not yet closed"
        );
    }

    #[test]
    fn mandatory_pre_min_time_window_closes_on_schedule_even_when_jump_is_never_held() {
        let dt = 1.0 / 120.0;
        let mut c = car();
        let mut boost = MAX_BOOST;
        let mut jump_held = false;
        let mut double_jump_available = true;
        let mut hold_remaining = 0.0;

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

        // Release immediately and stay released well past JUMP_MIN_TIME
        // (0.025s = 3 steps at this dt) — the mandatory window must still
        // close on its own schedule even though jump was never held past
        // the press.
        for _ in 0..6 {
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
        assert_eq!(
            hold_remaining, 0.0,
            "expected the mandatory window to have closed by now"
        );
        let velocity_after_window_closes = c.linear_velocity.z;

        for _ in 0..5 {
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
            (c.linear_velocity.z - velocity_after_window_closes).abs() < 1e-3,
            "expected no further upward velocity gain once the mandatory window has closed, \
             velocity at window close={velocity_after_window_closes}, after={}",
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

    /// A car tilted 90 degrees about its local forward axis — up_axis
    /// becomes (0, -1, 0) instead of world up (0, 0, 1). Drive.rs's test
    /// helpers only call `integrate::integrate_velocities`, never
    /// `integrate::integrate_transform`, so a car's `orientation` never
    /// actually changes step to step here — the only way to exercise the
    /// landing-assistance torque's dependence on orientation in isolation
    /// is to set it directly like this.
    fn tilted_car() -> RigidBody {
        let mut c = car();
        c.orientation = rb_domain::Quat::new(
            std::f32::consts::FRAC_1_SQRT_2,
            0.0,
            0.0,
            std::f32::consts::FRAC_1_SQRT_2,
        );
        c.update_inertia_tensor();
        c
    }

    #[test]
    fn a_tilted_airborne_car_gets_a_corrective_torque_from_landing_assistance() {
        let mut c = tilted_car();
        let mut boost = MAX_BOOST;
        step_with_input(
            &mut c,
            &ControllerInput::default(),
            false,
            &mut boost,
            1.0 / 60.0,
        );
        assert!(
            c.angular_velocity.length() > 0.0,
            "expected a tilted airborne car with neutral input to gain a corrective angular \
             velocity from landing-orientation assistance, got {:?}",
            c.angular_velocity
        );
    }

    #[test]
    fn an_already_upright_airborne_car_gets_no_corrective_torque() {
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
            "expected an already-level car to get no correction, got {:?}",
            c.angular_velocity
        );
    }

    #[test]
    fn landing_assistance_does_not_apply_while_grounded() {
        let mut c = tilted_car();
        let mut boost = MAX_BOOST;
        step_with_input(
            &mut c,
            &ControllerInput::default(),
            true,
            &mut boost,
            1.0 / 60.0,
        );
        assert_eq!(
            c.angular_velocity,
            Vec3::ZERO,
            "expected no landing-orientation assistance while grounded, got {:?}",
            c.angular_velocity
        );
    }

    #[test]
    fn landing_assistance_does_not_apply_while_actively_air_controlling() {
        let mut without_input = tilted_car();
        let mut without_input_boost = MAX_BOOST;
        step_with_input(
            &mut without_input,
            &ControllerInput::default(),
            false,
            &mut without_input_boost,
            1.0 / 60.0,
        );
        let assisted_angular_velocity = without_input.angular_velocity;
        assert!(assisted_angular_velocity.length() > 0.0);

        let mut with_pitch = tilted_car();
        let mut with_pitch_boost = MAX_BOOST;
        step_with_input(
            &mut with_pitch,
            &full_pitch(),
            false,
            &mut with_pitch_boost,
            1.0 / 60.0,
        );
        // Full pitch input drives its own AIR_CONTROL_TORQUE-scale
        // rotation about the right axis (y); the landing assist's own
        // much smaller contribution must not additionally appear (it
        // would only ever add to x/z here, since the assist's correction
        // axis for this tilt is purely along x — see tilted_car).
        assert_eq!(
            with_pitch.angular_velocity.x, 0.0,
            "expected active pitch input to suppress landing-orientation assistance entirely, \
             got {:?}",
            with_pitch.angular_velocity
        );
    }

    #[test]
    fn clamp_angular_speed_is_a_no_op_below_the_cap() {
        let mut c = car();
        c.angular_velocity = Vec3::new(1.0, 2.0, 0.0);
        clamp_angular_speed(&mut c);
        assert_eq!(
            c.angular_velocity,
            Vec3::new(1.0, 2.0, 0.0),
            "expected an already-under-cap angular velocity to pass through unchanged, got {:?}",
            c.angular_velocity
        );
    }

    #[test]
    fn clamp_angular_speed_scales_an_over_cap_velocity_down_to_the_cap_preserving_direction() {
        let mut c = car();
        c.angular_velocity = Vec3::new(0.0, 0.0, 20.0);
        clamp_angular_speed(&mut c);
        assert!(
            (c.angular_velocity.length() - MAX_CAR_ANGULAR_SPEED).abs() < 1e-4,
            "expected the clamp to scale magnitude down to exactly MAX_CAR_ANGULAR_SPEED, got \
             {:?}",
            c.angular_velocity
        );
        assert_eq!(
            c.angular_velocity.x, 0.0,
            "expected the clamp to preserve direction (x), got {:?}",
            c.angular_velocity
        );
        assert_eq!(
            c.angular_velocity.y, 0.0,
            "expected the clamp to preserve direction (y), got {:?}",
            c.angular_velocity
        );
        assert!(
            c.angular_velocity.z > 0.0,
            "expected the clamp to preserve direction (z), got {:?}",
            c.angular_velocity
        );
    }

    #[test]
    fn sustained_full_roll_input_never_exceeds_the_hard_angular_speed_cap() {
        // RB-PHYSICS-001-FR-057: before this cap existed, nothing bounded
        // the continuous AIR_CONTROL_TORQUE contribution air control adds
        // every step, so holding full roll input indefinitely spun a car
        // arbitrarily fast. Two real seconds of full roll at this car's own
        // mass/inertia gains far more than MAX_CAR_ANGULAR_SPEED (5.5
        // rad/s) worth of angular velocity if nothing clamps it, so this
        // test would fail without `clamp_angular_speed` in `step_with_input`'s
        // own step helper.
        let mut c = car();
        let mut boost = MAX_BOOST;
        let dt = 1.0 / 60.0;
        for _ in 0..(2.0 / dt) as u32 {
            step_with_input(&mut c, &full_roll(), false, &mut boost, dt);
        }
        assert!(
            c.angular_velocity.length() <= MAX_CAR_ANGULAR_SPEED + 1e-3,
            "expected sustained full roll input to cap out at MAX_CAR_ANGULAR_SPEED, got {:?} \
             (length {})",
            c.angular_velocity,
            c.angular_velocity.length()
        );
        assert!(
            c.angular_velocity.length() > MAX_CAR_ANGULAR_SPEED - 0.5,
            "expected sustained full roll input to actually reach the cap, not merely stay under \
             it, got {:?}",
            c.angular_velocity
        );
    }
}
