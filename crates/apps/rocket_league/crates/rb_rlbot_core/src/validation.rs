//! Numeric checks shared by simulated and live match adapters.
use rb_rlbot_wire::{ControllerState, DesiredGameState, DesiredPhysics};
use std::io;

/// Rejects the entire request before any mutation or outbound command, including unsupported
/// fields: accepting only finite siblings would silently apply a different request.
pub fn validate_desired(state: &DesiredGameState) -> io::Result<()> {
    let finite = |v: Option<f32>| v.is_none_or(f32::is_finite);
    let physics = |p: &DesiredPhysics| {
        [p.location, p.velocity, p.angular_velocity]
            .iter()
            .all(|v| v.is_none_or(|v| [v.x, v.y, v.z].into_iter().all(finite)))
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
            "nonfinite desired state",
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
}
