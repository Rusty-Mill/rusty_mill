use super::air::*;
use super::boost::*;
use super::ground::*;
use super::jump::*;
use super::*;
use crate::integrate;

fn car() -> RigidBody {
    RigidBody::standard_car(Vec3::ZERO)
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
    let mut flip = None;
    step_with_input_and_dodge_flip(
        car,
        input,
        on_ground,
        wall_normal,
        boost_amount,
        jump_held,
        double_jump_available,
        jump_hold_time_remaining,
        &mut flip,
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
    flip: &mut Option<FlipState>,
    dt: f32,
) {
    // The helper chain above threads each field separately so older tests
    // keep their narrow signatures; pack them into one DriveState for the
    // call and unpack afterward.
    let mut state = DriveState {
        boost_amount: *boost_amount,
        jump_held: *jump_held,
        double_jump_available: *double_jump_available,
        jump_hold_time_remaining: *jump_hold_time_remaining,
        flip: *flip,
        handbrake_amount: 0.0,
    };
    car.clear_forces();
    apply_driven_forces(car, input, on_ground, wall_normal, &mut state, dt);
    *boost_amount = state.boost_amount;
    *jump_held = state.jump_held;
    *double_jump_available = state.double_jump_available;
    *jump_hold_time_remaining = state.jump_hold_time_remaining;
    *flip = state.flip;
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
        pitch: Some(-1.0),
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

    // RB-PHYSICS-001-FR-081: grounded boost also forces full throttle (as
    // in RocketSim), so take the engine's share out to compare boost alone.
    let grounded_boost_only = grounded.linear_velocity.x - THROTTLE_ACCELERATION / 60.0;
    assert!(
        airborne.linear_velocity.x > grounded_boost_only,
        "expected airborne boost ({}) to accelerate faster than grounded boost ({})",
        airborne.linear_velocity.x,
        grounded_boost_only
    );
    let ratio = airborne.linear_velocity.x / grounded_boost_only;
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

const TICK: f32 = 1.0 / 120.0;

/// Angular velocity after a dodge press tick at 120 Hz: the flip torque
/// acts on the press tick itself, with air control already off
/// (RB-PHYSICS-001-FR-085).
fn flip_spin_after_press_tick(input: &ControllerInput) -> Vec3 {
    let mut c = car();
    let mut state = DriveState::new();
    c.clear_forces();
    apply_driven_forces(&mut c, input, false, None, &mut state, TICK);
    integrate::integrate_velocities(&mut c, TICK);
    c.angular_velocity
}

/// One grounded `apply_ground_control` tick on a car facing +X (right is
/// +Y), with `throttle` as the effective throttle.
fn ground_tick(velocity: Vec3, throttle: f32, handbrake_amount: f32) -> Vec3 {
    let mut c = car();
    c.linear_velocity = velocity;
    let input = ControllerInput {
        throttle,
        handbrake: handbrake_amount > 0.0,
        ..Default::default()
    };
    apply_ground_control(
        &mut c,
        &input,
        throttle,
        Vec3::new(1.0, 0.0, 0.0),
        handbrake_amount,
        TICK,
    );
    c.linear_velocity
}

fn assert_close(got: f32, expected: f32, what: &str) {
    assert!(
        (got - expected).abs() < 1e-3,
        "{what}: got {got}, expected {expected}"
    );
}

#[test]
fn slip_ratio_is_zero_at_the_lateral_threshold_and_one_when_purely_sideways() {
    assert_eq!(slip_ratio(1000.0, SLIP_LATERAL_SPEED_THRESHOLD), 0.0);
    assert_eq!(slip_ratio(0.0, -300.0), 1.0);
    assert_close(slip_ratio(-300.0, 100.0), 0.25, "mixed slip");
}

#[test]
fn pedals_follow_rocketsims_throttle_and_brake_rules() {
    assert_eq!(pedals(1.0, 1000.0, false), (1.0, 0.0), "driving");
    assert_eq!(
        pedals(0.0, 1000.0, false),
        (0.0, COASTING_BRAKE_FACTOR),
        "coasting"
    );
    assert_eq!(pedals(0.0, 20.0, false), (0.0, 1.0), "slow coast stops");
    assert_eq!(pedals(-1.0, 1000.0, false), (0.0, 1.0), "reverse brakes");
    assert_eq!(
        pedals(-1.0, 20.0, false),
        (-1.0, 0.0),
        "slow reverse drives"
    );
    assert_eq!(pedals(0.0, 1000.0, true), (0.0, 0.0), "handbrake: no brake");
    assert_eq!(
        pedals(-1.0, 1000.0, true),
        (-1.0, 0.0),
        "handbrake: no brake"
    );
}

#[test]
fn tire_grip_follows_the_slip_curve_and_blends_in_the_handbrake() {
    assert_eq!(tire_grip(0.0, 0.0), (1.0, 1.0));
    assert_close(tire_grip(1.0, 0.0).0, 0.2, "full slide lateral");
    let (lateral, longitudinal) = tire_grip(0.0, 1.0);
    assert_close(lateral, HANDBRAKE_LAT_FRICTION_FACTOR, "handbrake lateral");
    assert_close(longitudinal, 0.5, "handbrake longitudinal");
    assert_close(tire_grip(0.0, 0.5).0, 0.55, "half handbrake lateral");
}

#[test]
fn handbrake_ramps_up_at_the_rise_rate_and_down_at_the_fall_rate() {
    assert_close(ramp_handbrake(0.0, true, TICK), 5.0 * TICK, "rise");
    assert_eq!(ramp_handbrake(0.99, true, TICK), 1.0, "clamped at full");
    assert_close(ramp_handbrake(1.0, false, TICK), 1.0 - 2.0 * TICK, "fall");
    assert_eq!(ramp_handbrake(0.0, false, TICK), 0.0, "clamped at zero");
}

#[test]
fn lateral_grip_rate_is_below_the_point_mass_bound_and_orientation_independent() {
    let level = car();
    let rate = lateral_grip_rate(&level, Vec3::new(0.0, 1.0, 0.0));
    // Four wheels at 0.2 * 60 each if every push went through the centre
    // of mass; spin at the contact points takes some of it.
    assert!(rate > 0.0 && rate < 48.0, "rate {rate}");

    let mut turned = car();
    // 1 rad yaw about +Z.
    turned.orientation = rb_domain::Quat::new(0.0, 0.0, 0.5_f32.sin(), 0.5_f32.cos());
    turned.update_inertia_tensor();
    let right = turned.orientation.rotate(&Vec3::new(0.0, 1.0, 0.0));
    assert_close(lateral_grip_rate(&turned, right), rate, "turned car");
}

#[test]
fn rolling_tires_cancel_lateral_velocity_at_the_grip_rate() {
    // 4 uu/s sideways is at the slip threshold: full grip.
    let rate = lateral_grip_rate(&car(), Vec3::new(0.0, 1.0, 0.0));
    let v = ground_tick(Vec3::new(1000.0, 4.0, 0.0), 1.0, 0.0);
    assert_close(v.y, 4.0 * (1.0 - rate * TICK), "lateral speed");
}

#[test]
fn a_full_slide_keeps_more_lateral_speed_than_a_rolling_car() {
    let rate = lateral_grip_rate(&car(), Vec3::new(0.0, 1.0, 0.0));
    let v = ground_tick(Vec3::new(0.0, 500.0, 0.0), 1.0, 0.0);
    assert_close(v.y, 500.0 * (1.0 - 0.2 * rate * TICK), "lateral speed");
}

#[test]
fn a_coasting_car_decelerates_at_the_coasting_rate_both_ways() {
    let coast = BRAKE_DECELERATION * COASTING_BRAKE_FACTOR * TICK;
    let forward = ground_tick(Vec3::new(1000.0, 0.0, 0.0), 0.0, 0.0);
    assert_close(forward.x, 1000.0 - coast, "forward coast");
    let reverse = ground_tick(Vec3::new(-1000.0, 0.0, 0.0), 0.0, 0.0);
    assert_close(reverse.x, -1000.0 + coast, "reverse coast");
}

#[test]
fn opposing_throttle_brakes_at_full_brake_deceleration() {
    let v = ground_tick(Vec3::new(1000.0, 0.0, 0.0), -1.0, 0.0);
    assert_close(v.x, 1000.0 - BRAKE_DECELERATION * TICK, "braking");
}

#[test]
fn a_slow_coasting_car_brakes_to_a_stop_without_reversing() {
    let v = ground_tick(Vec3::new(20.0, 0.0, 0.0), 0.0, 0.0);
    assert_eq!(v.x, 0.0);
}

#[test]
fn a_handbraking_car_does_not_coast_brake() {
    let v = ground_tick(Vec3::new(1000.0, 0.0, 0.0), 0.0, 1.0);
    assert_eq!(v.x, 1000.0);
}

#[test]
fn releasing_steer_stops_a_grounded_cars_turn() {
    // Straight wheels with grip end the yaw a released turn left behind.
    let mut c = car();
    c.linear_velocity = Vec3::new(1000.0, 0.0, 0.0);
    c.angular_velocity = Vec3::new(0.0, 0.0, 2.0);
    let mut boost = MAX_BOOST;
    step_with_input(&mut c, &full_throttle(), true, &mut boost, TICK);
    assert!(
        c.angular_velocity.z.abs() < 1e-4,
        "yaw {}",
        c.angular_velocity.z
    );
}

#[test]
fn steer_yaw_rate_blends_halfway_at_half_handbrake() {
    let normal = steer_yaw_rate(1000.0, 1.0, 0.0);
    let powerslide = steer_yaw_rate(1000.0, 1.0, 1.0);
    let half = steer_yaw_rate(1000.0, 1.0, 0.5);
    assert!(
        (half - normal).abs() < (powerslide - normal).abs(),
        "half {half} should sit between {normal} and {powerslide}"
    );
    assert!((half - powerslide).abs() < (powerslide - normal).abs());
}

#[test]
fn tire_grip_does_not_act_on_an_airborne_car() {
    let mut c = car();
    c.linear_velocity = Vec3::new(0.0, 500.0, 0.0);
    let mut boost = MAX_BOOST;
    step_with_input(&mut c, &full_handbrake(), false, &mut boost, 1.0 / 120.0);
    assert_eq!(c.linear_velocity.y, 500.0);
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
    assert!(
        (c.linear_velocity.x - DODGE_SPEED).abs() < 1.0,
        "expected roughly DODGE_SPEED forward velocity, got {}",
        c.linear_velocity.x
    );
    // The flip spins the car nose down from the press tick (RB-PHYSICS-001-FR-085).
    // The flip noses the car down on the press tick.
    let spin = flip_spin_after_press_tick(&input);
    assert_close(spin.y, FLIP_TORQUE_FORWARD / 120.0, "pitch spin");
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
    // A dodge to the right rolls the right side down on the press tick.
    let spin = flip_spin_after_press_tick(&input);
    assert_close(spin.x, -FLIP_TORQUE_SIDE / 120.0, "roll spin");
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
        pitch: Some(-1.0),
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
        pitch: Some(1.0),
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
        pitch: Some(-1.0),
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
    // RB-PHYSICS-001-FR-072: a diagonal dodge's combined (pitch, roll)
    // direction is normalized to unit length before scaling by
    // DODGE_SPEED, matching RocketSim's own confirmed real
    // `dodgeDir.safeNormalized()` step — so each axis gets
    // DODGE_SPEED / sqrt(2), not a full DODGE_SPEED each (which would
    // make a diagonal dodge sqrt(2) times faster than an
    // axis-aligned one).
    let expected = DODGE_SPEED / std::f32::consts::SQRT_2;
    assert!(
        (c.linear_velocity.x - expected).abs() < 1.0,
        "expected the forward component of a diagonal dodge, got {}",
        c.linear_velocity.x
    );
    assert!(
        (c.linear_velocity.y - expected).abs() < 1.0,
        "expected the lateral component of a diagonal dodge, got {}",
        c.linear_velocity.y
    );
    let total_magnitude = (c.linear_velocity.x.powi(2) + c.linear_velocity.y.powi(2)).sqrt();
    assert!(
        (total_magnitude - DODGE_SPEED).abs() < 1.0,
        "expected a diagonal dodge's total magnitude to match an \
         axis-aligned one's DODGE_SPEED, got {total_magnitude}"
    );
}

#[test]
fn a_yaw_only_press_fires_a_sideways_dodge_like_roll() {
    // RB-PHYSICS-001-FR-073: real Rocket League's dodgeDir.y combines
    // yaw + roll, so a pure yaw stick nudge (no roll held) fires the
    // same sideways dodge a roll-only press would.
    let mut c = car();
    let mut boost = MAX_BOOST;
    let mut jump_held = false;
    let mut double_jump_available = true;
    let input = ControllerInput {
        jump: true,
        yaw: Some(1.0),
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
        "expected roughly DODGE_SPEED lateral velocity from yaw alone, got {}",
        c.linear_velocity.y
    );
    // Yaw folds into the dodge's side direction: the same roll spin.
    let spin = flip_spin_after_press_tick(&input);
    assert_close(spin.x, -FLIP_TORQUE_SIDE / 120.0, "roll spin");
}

#[test]
fn yaw_and_roll_combine_in_the_dodge_direction() {
    // RB-PHYSICS-001-FR-073: yaw and roll both feed the same combined
    // roll-axis stick value (roll + yaw) before normalization, so equal
    // opposite yaw and roll cancel out to no sideways dodge at all.
    let mut c = car();
    let mut boost = MAX_BOOST;
    let mut jump_held = false;
    let mut double_jump_available = true;
    let input = ControllerInput {
        jump: true,
        roll: Some(1.0),
        yaw: Some(-1.0),
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
        c.linear_velocity.y, 0.0,
        "expected equal-and-opposite roll and yaw to cancel out, got {}",
        c.linear_velocity.y
    );
    assert!(
        (c.linear_velocity.z - JUMP_SPEED).abs() < 1.0,
        "expected a plain double jump instead, since combined roll+yaw \
         and pitch are both below DODGE_DEADZONE, got {}",
        c.linear_velocity.z
    );
}

#[test]
fn normalize_dodge_direction_preserves_a_single_axis() {
    // A pure axis-aligned dodge (the other axis exactly zero) is
    // unaffected: normalizing (1.0, 0.0) or (0.0, 1.0) yields the same
    // unit value back.
    assert_eq!(normalize_dodge_direction(1.0, 0.0), (1.0, 0.0));
    assert_eq!(normalize_dodge_direction(0.0, 1.0), (0.0, 1.0));
    assert_eq!(normalize_dodge_direction(-1.0, 0.0), (-1.0, 0.0));
}

#[test]
fn normalize_dodge_direction_normalizes_a_diagonal_to_unit_length() {
    let (pitch, roll) = normalize_dodge_direction(1.0, 1.0);
    let magnitude = (pitch * pitch + roll * roll).sqrt();
    assert!(
        (magnitude - 1.0).abs() < 1e-6,
        "expected a diagonal direction to normalize to unit length, got {magnitude}"
    );
    assert!(
        (pitch - roll).abs() < 1e-6,
        "expected an equal 45-degree split"
    );
}

#[test]
fn normalize_dodge_direction_snaps_a_near_axis_aligned_input_to_a_pure_axis() {
    // RB-PHYSICS-001-FR-074: RocketSim's own post-normalization
    // small-component zeroing snaps a near-axis-aligned diagonal (e.g.
    // a stick nudged almost, but not quite, purely forward) to a pure
    // single-axis dodge instead of leaving a tiny perpendicular
    // component. pitch=1.0, roll=0.05 normalizes to roll ~= 0.05,
    // well below DODGE_DIRECTION_SNAP_THRESHOLD (0.1).
    let (pitch, roll) = normalize_dodge_direction(1.0, 0.05);
    assert_eq!(
        roll, 0.0,
        "expected the tiny roll component to snap to zero"
    );
    assert!(
        pitch > 0.9,
        "expected the pitch component to stay close to its full unit magnitude, got {pitch}"
    );
}

#[test]
fn normalize_dodge_direction_does_not_snap_a_clearly_diagonal_input() {
    // A genuinely diagonal input (both axes well above the snap
    // threshold once normalized) is unaffected by the snap.
    let (pitch, roll) = normalize_dodge_direction(1.0, 0.5);
    assert!(
        pitch.abs() >= DODGE_DIRECTION_SNAP_THRESHOLD
            && roll.abs() >= DODGE_DIRECTION_SNAP_THRESHOLD,
        "expected neither component to snap to zero, got ({pitch}, {roll})"
    );
    assert!(roll > 0.0, "expected the roll component to stay nonzero");
}

#[test]
fn normalize_dodge_direction_is_zero_for_zero_input() {
    assert_eq!(normalize_dodge_direction(0.0, 0.0), (0.0, 0.0));
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
        (delta - (-DODGE_SPEED * DODGE_BACKWARD_SPEED_SCALE * DODGE_BACKWARD_SCALE_X)).abs() < 1.0,
        "expected a backward dodge at max speed to scale to DODGE_SPEED \
         * DODGE_BACKWARD_SPEED_SCALE * DODGE_BACKWARD_SCALE_X, got delta {}",
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
        pitch: Some(-1.0),
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
        pitch: Some(-1.0),
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
        pitch: Some(-1.0),
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
    // same as ever) is expected and tolerated here — only a flip's own
    // spin must be absent.
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
        pitch: Some(-1.0),
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
        pitch: Some(1.0),
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
        pitch: Some(-1.0),
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
    // RB-PHYSICS-001-FR-072: the dodge's own (pitch, roll) direction is
    // normalized before scaling — see
    // `a_diagonal_dodge_combines_pitch_and_roll`'s own comment — so
    // each dodge component is DODGE_SPEED / sqrt(2), on top of the
    // unaffected wall push-off.
    let expected_dodge_component = DODGE_SPEED / std::f32::consts::SQRT_2;
    assert!(
        (c.linear_velocity.x - (WALL_JUMP_HORIZONTAL_SPEED + expected_dodge_component)).abs() < 1.0,
        "expected the wall push-off plus the forward dodge component, got {}",
        c.linear_velocity.x
    );
    assert!(
        (c.linear_velocity.y - expected_dodge_component).abs() < 1.0,
        "expected the lateral dodge component, got {}",
        c.linear_velocity.y
    );
}

#[test]
fn a_yaw_only_press_fires_a_sideways_wall_jump_dodge_like_roll() {
    // RB-PHYSICS-001-FR-073: the wall-jump-dodge path folds yaw into
    // the same combined roll-axis stick value as the ground dodge.
    let mut c = car();
    let mut boost = MAX_BOOST;
    let mut jump_held = false;
    let mut double_jump_available = true;
    let input = ControllerInput {
        jump: true,
        yaw: Some(1.0),
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
        (c.linear_velocity.y - DODGE_SPEED).abs() < 1.0,
        "expected roughly DODGE_SPEED lateral dodge velocity from yaw alone, got {}",
        c.linear_velocity.y
    );
}

#[test]
fn air_control_has_no_effect_while_grounded() {
    let mut c = car();
    let mut boost = MAX_BOOST;
    let input = ControllerInput {
        pitch: Some(-1.0),
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
        "expected no air control while grounded, got {:?}",
        c.angular_velocity
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

#[test]
fn drive_state_new_starts_full_boost_released_with_double_jump_available() {
    let state = DriveState::new();
    assert_eq!(
        state,
        DriveState {
            boost_amount: MAX_BOOST,
            jump_held: false,
            double_jump_available: true,
            jump_hold_time_remaining: 0.0,
            flip: None,
            handbrake_amount: 0.0,
        }
    );
}

#[test]
fn apply_driven_forces_updates_every_drive_state_field_it_owns() {
    // One grounded step with jump + boost pressed: jump_held latches, the
    // hold window arms, and boost drains.
    let mut car = car();
    let mut state = DriveState::new();
    state.double_jump_available = false;
    let input = ControllerInput {
        jump: true,
        boost: true,
        ..Default::default()
    };
    let dt = 1.0 / 120.0;
    apply_driven_forces(&mut car, &input, true, None, &mut state, dt);
    assert!(state.jump_held);
    assert!(state.double_jump_available);
    assert_eq!(state.jump_hold_time_remaining, JUMP_HOLD_MAX_DURATION);
    assert!(state.boost_amount < MAX_BOOST);
}

#[test]
fn steer_yaw_rate_follows_the_bicycle_model_on_the_real_steer_curve() {
    // 500 uu/s sits exactly on a curve point (0.31930 rad).
    let expected = 500.0 * 0.31930_f32.tan() / WHEELBASE;
    let got = steer_yaw_rate(500.0, 1.0, 0.0);
    assert!(
        (got - expected).abs() < 1e-4,
        "got {got}, expected {expected}"
    );
    assert!(
        (expected - 1.95).abs() < 0.01,
        "sanity: ~1.95 rad/s, got {expected}"
    );
}

#[test]
fn steer_yaw_rate_is_zero_at_a_standstill_and_flips_in_reverse() {
    assert_eq!(steer_yaw_rate(0.0, 1.0, 0.0), 0.0);
    let forward = steer_yaw_rate(500.0, 1.0, 0.0);
    let reverse = steer_yaw_rate(-500.0, 1.0, 0.0);
    assert!(forward > 0.0);
    assert!(
        (reverse + forward).abs() < 1e-6,
        "reverse should mirror forward"
    );
}

#[test]
fn steer_yaw_rate_uses_the_powerslide_curve_while_handbraking() {
    // At 2500 uu/s the powerslide curve (0.12610 rad) steers more than the
    // normal one (interpolated between 1750 and 3000, ~0.0547 rad).
    let normal = steer_yaw_rate(2500.0, 1.0, 0.0);
    let powerslide = steer_yaw_rate(2500.0, 1.0, 1.0);
    let expected = 2500.0 * 0.12610_f32.tan() / WHEELBASE;
    assert!((powerslide - expected).abs() < 1e-3);
    assert!(powerslide > normal);
}

#[test]
fn curve_clamps_outside_its_range_and_interpolates_inside() {
    let points = [(0.0, 1.0), (10.0, 0.0)];
    assert_eq!(curve(&points, -5.0), 1.0);
    assert_eq!(curve(&points, 5.0), 0.5);
    assert_eq!(curve(&points, 50.0), 0.0);
    assert_eq!(curve(&[], 1.0), 0.0);
}

#[test]
fn steer_does_not_yaw_an_airborne_car() {
    let mut c = car();
    let mut boost = MAX_BOOST;
    c.linear_velocity = Vec3::new(1000.0, 0.0, 0.0);
    let input = ControllerInput {
        steer: 1.0,
        ..Default::default()
    };
    step_with_input(&mut c, &input, false, &mut boost, 1.0 / 60.0);
    // Airborne, steer alone does nothing (yaw comes from `input.yaw` air
    // control, which is unset here).
    assert!(
        c.angular_velocity.z.abs() < 1e-3,
        "expected no yaw from steer while airborne, got {:?}",
        c.angular_velocity
    );
}

#[test]
fn stick_forward_and_left_dodges_forward_and_left_with_rocketsims_impulse() {
    // RB-PHYSICS-001-FR-082, from the owner's capture at 4.317 s: pitch -1
    // (stick forward) and roll -1 at ~1200 uu/s dodged forward-left, not
    // backward. RocketSim: dodgeDir = (-pitch, yaw + roll) normalized, times
    // 500, side scaled by 1 + 0.9 * speed / 2300; forward unscaled.
    let mut c = car();
    c.linear_velocity = Vec3::new(1200.0, 0.0, 0.0);
    let mut boost = MAX_BOOST;
    let mut jump_held = false;
    let mut double_jump_available = true;
    let input = ControllerInput {
        jump: true,
        pitch: Some(-1.0),
        roll: Some(-1.0),
        ..Default::default()
    };
    step_with_input_and_double_jump_state(
        &mut c,
        &input,
        false,
        &mut boost,
        &mut jump_held,
        &mut double_jump_available,
        1.0 / 120.0,
    );
    let component = 500.0 * std::f32::consts::FRAC_1_SQRT_2;
    let side_scale = 1.0 + 0.9 * 1200.0 / MAX_CAR_SPEED;
    assert_close(c.linear_velocity.x - 1200.0, component, "forward delta");
    assert_close(c.linear_velocity.y, -component * side_scale, "side delta");
}

#[test]
fn stick_forward_noses_a_car_down_in_the_air() {
    // RocketSim pitch torque is about -right: pitch -1 (stick forward)
    // rotates the nose down, a positive spin about the car's +Y axis.
    let mut c = car();
    let mut boost = MAX_BOOST;
    let input = ControllerInput {
        pitch: Some(-1.0),
        ..Default::default()
    };
    step_with_input(&mut c, &input, false, &mut boost, 1.0 / 120.0);
    assert!(c.angular_velocity.y > 0.0, "spin {:?}", c.angular_velocity);
}

#[test]
fn a_tilted_cars_dodge_stays_horizontal() {
    // RocketSim dodges along the heading flattened to the ground plane:
    // a nose-up car's forward dodge adds no vertical speed (owner's capture,
    // 4.325 s: recorded vz held while a 3D push dropped it ~47 uu/s).
    let mut c = car();
    // Pitched nose-up 0.5 rad about +Y (negative rotation raises the nose).
    c.orientation = rb_domain::Quat::new(0.0, (-0.25_f32).sin(), 0.0, (-0.25_f32).cos());
    c.update_inertia_tensor();
    let mut boost = MAX_BOOST;
    let mut jump_held = false;
    let mut double_jump_available = true;
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
        1.0 / 120.0,
    );
    assert_close(c.linear_velocity.x, DODGE_SPEED, "horizontal dodge");
    // Only gravity (650 uu/s^2 over one tick) changes vz.
    assert!(
        c.linear_velocity.z.abs() < 10.0,
        "vz {}",
        c.linear_velocity.z
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

fn flip_at(time: f32, direction: (f32, f32)) -> Option<FlipState> {
    Some(FlipState { time, direction })
}

#[test]
fn a_flip_disables_air_control_and_spins_the_car_while_its_torque_lasts() {
    let mut c = car();
    let gate = apply_flip_torque(
        &mut c,
        &ControllerInput::default(),
        flip_at(0.1, (1.0, 0.0)),
    );
    assert!(!gate.enabled, "air control during a flip");
    assert_close(
        c.angular_velocity.y,
        FLIP_TORQUE_FORWARD / 120.0,
        "pitch spin",
    );
}

#[test]
fn a_flips_torque_ends_at_flip_torque_time_and_pitch_unlocks_after_the_extra_time() {
    let mut c = car();
    let locked = apply_flip_torque(
        &mut c,
        &ControllerInput::default(),
        flip_at(0.7, (1.0, 0.0)),
    );
    assert_eq!(c.angular_velocity, Vec3::ZERO, "no torque after 0.65 s");
    assert!(locked.enabled);
    assert_eq!(locked.pitch_scale, 0.0, "pitch locked until 0.95 s");
    let open = apply_flip_torque(
        &mut c,
        &ControllerInput::default(),
        flip_at(1.0, (1.0, 0.0)),
    );
    assert_eq!(open.pitch_scale, 1.0);
}

#[test]
fn holding_pitch_against_a_flip_cancels_its_pitch_spin_and_frees_air_control() {
    // A forward flip (forward component +1) is cancelled by pitch +1
    // (stick back), RocketSim's flip cancel.
    let mut c = car();
    let input = ControllerInput {
        pitch: Some(1.0),
        ..Default::default()
    };
    let gate = apply_flip_torque(&mut c, &input, flip_at(0.1, (1.0, 0.0)));
    assert!(gate.enabled, "cancel re-enables air control");
    assert_eq!(
        gate.pitch_scale, 0.0,
        "but pitch stays locked while flipping"
    );
    assert_eq!(
        c.angular_velocity.y, 0.0,
        "full cancel removes the pitch spin"
    );
}

#[test]
fn a_stall_applies_no_flip_torque() {
    let mut c = car();
    let gate = apply_flip_torque(
        &mut c,
        &ControllerInput::default(),
        flip_at(0.1, (0.0, 0.0)),
    );
    assert!(gate.enabled);
    assert_eq!(c.angular_velocity, Vec3::ZERO);
}

#[test]
fn a_flip_damps_vertical_speed_inside_its_window_only() {
    let damp = 1.0 - FLIP_Z_DAMP_120;
    // Falling at 0.2 s: damped.
    let mut c = car();
    c.linear_velocity.z = -100.0;
    let mut flip = flip_at(0.2, (1.0, 0.0));
    advance_flip(&mut c, &mut flip, TICK);
    assert_close(c.linear_velocity.z, -100.0 * damp, "falling, in window");
    // Rising at 0.18 s (before FLIP_Z_DAMP_END): damped too.
    c.linear_velocity.z = 100.0;
    let mut flip = flip_at(0.18, (1.0, 0.0));
    advance_flip(&mut c, &mut flip, TICK);
    assert_close(c.linear_velocity.z, 100.0 * damp, "rising, early window");
    // Rising at 0.3 s: untouched. Before 0.15 s: untouched.
    for start in [0.3, 0.05] {
        c.linear_velocity.z = 100.0;
        let mut flip = flip_at(start, (1.0, 0.0));
        advance_flip(&mut c, &mut flip, TICK);
        assert_eq!(c.linear_velocity.z, 100.0, "start {start}");
    }
}

#[test]
fn a_dodge_starts_a_flip_and_landing_clears_it() {
    let mut c = car();
    let mut state = DriveState::new();
    let dodge = ControllerInput {
        jump: true,
        pitch: Some(-1.0),
        ..Default::default()
    };
    apply_driven_forces(&mut c, &dodge, false, None, &mut state, TICK);
    assert_eq!(
        state.flip,
        Some(FlipState {
            time: TICK,
            direction: (1.0, 0.0),
        }),
        "a dodge starts a flip"
    );
    apply_driven_forces(
        &mut c,
        &ControllerInput::default(),
        true,
        None,
        &mut state,
        TICK,
    );
    assert_eq!(state.flip, None);
}

#[test]
fn full_stick_air_control_accelerates_each_axis_at_rocketsims_rate() {
    // From rest there is no damping: one tick adds torque * CAR_TORQUE_SCALE
    // * dt about RocketSim's axes (pitch -right, yaw up, roll -forward).
    let step = CAR_TORQUE_SCALE * TICK;
    for (input, expected) in [
        (
            ControllerInput {
                pitch: Some(1.0),
                ..Default::default()
            },
            Vec3::new(0.0, -AIR_CONTROL_TORQUE.x * step, 0.0),
        ),
        (
            ControllerInput {
                yaw: Some(1.0),
                ..Default::default()
            },
            Vec3::new(0.0, 0.0, AIR_CONTROL_TORQUE.y * step),
        ),
        (
            ControllerInput {
                roll: Some(1.0),
                ..Default::default()
            },
            Vec3::new(-AIR_CONTROL_TORQUE.z * step, 0.0, 0.0),
        ),
    ] {
        let mut c = car();
        apply_air_control(&mut c, &input, 1.0, TICK);
        assert!(
            (c.angular_velocity - expected).length() < 1e-5,
            "got {:?}, expected {expected:?}",
            c.angular_velocity
        );
    }
}

#[test]
fn air_control_damps_free_spin_and_held_pitch_turns_its_damping_off() {
    let step = CAR_TORQUE_SCALE * TICK;
    // Rolling at 1 rad/s with the stick centered: roll damping slows it.
    let mut c = car();
    c.angular_velocity = Vec3::new(1.0, 0.0, 0.0);
    apply_air_control(&mut c, &ControllerInput::default(), 1.0, TICK);
    assert_close(
        c.angular_velocity.x,
        1.0 - AIR_CONTROL_DAMPING.z * step,
        "roll damping",
    );
    // Pitching at 1 rad/s about -right (-Y) with full pitch held: the
    // stick's torque adds and its damping is off.
    let mut c = car();
    c.angular_velocity = Vec3::new(0.0, -1.0, 0.0);
    let input = ControllerInput {
        pitch: Some(1.0),
        ..Default::default()
    };
    apply_air_control(&mut c, &input, 1.0, TICK);
    assert_close(
        c.angular_velocity.y,
        -1.0 - AIR_CONTROL_TORQUE.x * step,
        "pitch held, no damping",
    );
}

#[test]
fn a_locked_pitch_gives_no_pitch_torque_but_full_pitch_damping() {
    let step = CAR_TORQUE_SCALE * TICK;
    let mut c = car();
    c.angular_velocity = Vec3::new(0.0, -1.0, 0.0);
    let input = ControllerInput {
        pitch: Some(1.0),
        ..Default::default()
    };
    apply_air_control(&mut c, &input, 0.0, TICK);
    assert_close(
        c.angular_velocity.y,
        -1.0 + AIR_CONTROL_DAMPING.x * step,
        "locked pitch",
    );
}

#[test]
fn throttle_pushes_an_airborne_car_forward_gently() {
    let mut c = car();
    let mut boost = MAX_BOOST;
    step_with_input(&mut c, &full_throttle(), false, &mut boost, TICK);
    assert_close(
        c.linear_velocity.x,
        THROTTLE_AIR_ACCELERATION * TICK,
        "air throttle",
    );
}
