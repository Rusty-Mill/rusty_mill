//! Numeric checks shared by simulated and live match adapters.
use rb_rlbot_wire::{ControllerState, DesiredGameState, DesiredPhysics, PartialVec3};
use std::io;

/// Largest `|axis|` of a requested location, in uu. The arena is about 8200 x 10300 x 2000, so
/// this leaves room for a ball placed outside it and nothing a physics engine cannot hold.
pub const MAX_LOCATION: f32 = 20_000.0;
/// Largest `|axis|` of a requested velocity, in uu/s (a ball is capped at 6000, a car at 2300).
pub const MAX_VELOCITY: f32 = 20_000.0;
/// Largest `|axis|` of a requested angular velocity, in rad/s (the game caps at about 6).
pub const MAX_ANGULAR_VELOCITY: f32 = 100.0;

/// Rejects the entire request before any mutation or outbound command, including unsupported
/// fields: accepting only finite siblings would silently apply a different request. Nonfinite
/// values and ones beyond the `MAX_*` bounds are refused (not clamped: a clamped request is a
/// different request), so a buggy bot cannot hand the game or the simulation a value it cannot
/// hold.
pub fn validate_desired(state: &DesiredGameState) -> io::Result<()> {
    let finite = |v: Option<f32>| v.is_none_or(f32::is_finite);
    let within = |v: Option<PartialVec3>, max: f32| {
        v.is_none_or(|v| {
            [v.x, v.y, v.z]
                .into_iter()
                .all(|a| finite(a) && a.is_none_or(|a| a.abs() <= max))
        })
    };
    let physics = |p: &DesiredPhysics| {
        within(p.location, MAX_LOCATION)
            && within(p.velocity, MAX_VELOCITY)
            && within(p.angular_velocity, MAX_ANGULAR_VELOCITY)
            && p.rotation
                .is_none_or(|r| [r.pitch, r.yaw, r.roll].into_iter().all(finite))
    };
    if state.ball_states.iter().all(|b| physics(&b.physics))
        && state
            .car_states
            .iter()
            .all(|c| c.physics.as_ref().is_none_or(physics) && finite(c.boost_amount))
        && state
            .match_info
            .is_none_or(|m| finite(m.world_gravity_z) && finite(m.game_speed))
    {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "nonfinite or out-of-range desired state",
        ))
    }
}

/// Clamps finite axes to their legal range and replaces nonfinite axes with neutral input.
pub fn sanitise_controller(state: &ControllerState) -> ControllerState {
    let axis = |v: f32| {
        if v.is_finite() {
            v.clamp(-1.0, 1.0)
        } else {
            0.0
        }
    };
    ControllerState {
        throttle: axis(state.throttle),
        steer: axis(state.steer),
        pitch: axis(state.pitch),
        yaw: axis(state.yaw),
        roll: axis(state.roll),
        ..*state
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rb_rlbot_wire::{DesiredBallState, DesiredCarState, DesiredMatchInfo, PartialVec3};
    #[test]
    fn validates_even_unsupported_fields_and_valid_siblings_do_not_hide_bad_values() {
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let physics = DesiredPhysics {
                location: Some(PartialVec3 {
                    x: Some(bad),
                    y: Some(3.0),
                    z: None,
                }),
                ..Default::default()
            };
            for state in [
                DesiredGameState {
                    ball_states: vec![DesiredBallState::default(), DesiredBallState { physics }],
                    ..Default::default()
                },
                DesiredGameState {
                    car_states: vec![DesiredCarState {
                        physics: Some(physics),
                        boost_amount: Some(45.0),
                    }],
                    ..Default::default()
                },
                DesiredGameState {
                    car_states: vec![DesiredCarState {
                        boost_amount: Some(bad),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                DesiredGameState {
                    match_info: Some(DesiredMatchInfo {
                        game_speed: Some(bad),
                        world_gravity_z: Some(-650.0),
                    }),
                    ..Default::default()
                },
            ] {
                assert_eq!(
                    validate_desired(&state).unwrap_err().kind(),
                    io::ErrorKind::InvalidInput
                );
            }
        }
        assert!(validate_desired(&DesiredGameState::default()).is_ok());
    }

    #[test]
    fn finite_but_extreme_values_are_refused_and_the_bounds_themselves_are_not() {
        let vec = |x: f32| {
            Some(PartialVec3 {
                x: Some(x),
                y: None,
                z: None,
            })
        };
        let ball = |physics: DesiredPhysics| DesiredGameState {
            ball_states: vec![DesiredBallState { physics }],
            ..Default::default()
        };
        let build = |which: usize, v: Option<PartialVec3>| {
            let mut p = DesiredPhysics::default();
            match which {
                0 => p.location = v,
                1 => p.velocity = v,
                _ => p.angular_velocity = v,
            }
            p
        };
        let cases = [
            ("location", 0, MAX_LOCATION),
            ("velocity", 1, MAX_VELOCITY),
            ("angular velocity", 2, MAX_ANGULAR_VELOCITY),
        ];
        for (what, which, max) in cases {
            assert!(
                validate_desired(&ball(build(which, vec(max)))).is_ok(),
                "{what} at the bound"
            );
            assert!(
                validate_desired(&ball(build(which, vec(-max)))).is_ok(),
                "{what} at -bound"
            );
            for beyond in [max * 1.01, -max * 1.01, 1.0e30, f32::MAX] {
                assert_eq!(
                    validate_desired(&ball(build(which, vec(beyond))))
                        .unwrap_err()
                        .kind(),
                    io::ErrorKind::InvalidInput,
                    "{what} {beyond}"
                );
            }
        }
    }
}
