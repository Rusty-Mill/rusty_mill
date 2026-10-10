//! Shared pieces of the tape bots: the scenario's start state and a tape
//! input as RLBot messages. See docs/research/BOT-CAPTURE-PLAN.md.

pub mod tie_up;

use rb_rlbot_wire::{
    ControllerState, DesiredBallState, DesiredCarState, DesiredGameState, DesiredPhysics,
    PartialRotator, PartialVec3,
};
use rb_scenario::{BallStart, CarStart, Input, Scenario};

pub fn vector(v: [f32; 3]) -> PartialVec3 {
    PartialVec3 {
        x: Some(v[0]),
        y: Some(v[1]),
        z: Some(v[2]),
    }
}

pub fn rotator(r: [f32; 3]) -> PartialRotator {
    PartialRotator {
        pitch: Some(r[0]),
        yaw: Some(r[1]),
        roll: Some(r[2]),
    }
}

pub fn car_state(car: &CarStart) -> DesiredCarState {
    DesiredCarState {
        physics: Some(DesiredPhysics {
            location: car.location.map(vector),
            rotation: car.rotation.map(rotator),
            velocity: car.velocity.map(vector),
            angular_velocity: car.angular_velocity.map(vector),
        }),
        boost_amount: car.boost,
    }
}

pub fn ball_state(ball: &BallStart) -> DesiredBallState {
    DesiredBallState {
        physics: DesiredPhysics {
            location: ball.location.map(vector),
            rotation: None,
            velocity: ball.velocity.map(vector),
            angular_velocity: ball.angular_velocity.map(vector),
        },
    }
}

pub fn controller(input: Input) -> ControllerState {
    ControllerState {
        throttle: input.throttle,
        steer: input.steer,
        pitch: input.pitch,
        yaw: input.yaw,
        roll: input.roll,
        jump: input.jump,
        boost: input.boost,
        handbrake: input.handbrake,
        ..Default::default()
    }
}

/// The desired game state that puts every car of `scenario` (in car order)
/// and the ball at their start.
pub fn start_state(scenario: &Scenario) -> DesiredGameState {
    let cars = std::iter::once(&scenario.car)
        .chain(scenario.others.iter().map(|other| &other.car))
        .map(car_state)
        .collect();
    DesiredGameState {
        ball_states: scenario.ball.iter().map(ball_state).collect(),
        car_states: cars,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_input_field_lands_in_its_own_controller_field() {
        let input = Input {
            throttle: 0.1,
            steer: 0.2,
            pitch: 0.3,
            yaw: 0.4,
            roll: 0.5,
            jump: true,
            boost: true,
            handbrake: true,
        };
        let c = controller(input);
        assert_eq!(
            [c.throttle, c.steer, c.pitch, c.yaw, c.roll],
            [0.1, 0.2, 0.3, 0.4, 0.5]
        );
        assert!(c.jump && c.boost && c.handbrake && !c.use_item);
        let off = controller(Input::default());
        assert!(!off.jump && !off.boost && !off.handbrake);
    }

    #[test]
    fn the_start_state_sets_only_what_the_scenario_gives() {
        let scenario = Scenario::from_json(
            r#"{"name":"s","car":{"location":[1,2,3],"boost":40},"steps":[{"ticks":1}]}"#,
        )
        .unwrap();
        let state = start_state(&scenario);
        assert!(state.ball_states.is_empty(), "no ball in the scenario");
        let car = &state.car_states[0];
        let physics = car.physics.as_ref().unwrap();
        assert_eq!(physics.location, Some(vector([1.0, 2.0, 3.0])));
        assert_eq!(physics.velocity, None, "unset stays unset");
        assert_eq!(physics.rotation, None);
        assert_eq!(car.boost_amount, Some(40.0));
    }

    #[test]
    fn vectors_and_rotators_keep_their_axis_order() {
        let v = vector([1.0, 2.0, 3.0]);
        assert_eq!((v.x, v.y, v.z), (Some(1.0), Some(2.0), Some(3.0)));
        let r = rotator([0.1, 0.2, 0.3]);
        assert_eq!((r.pitch, r.yaw, r.roll), (Some(0.1), Some(0.2), Some(0.3)));
    }
}
