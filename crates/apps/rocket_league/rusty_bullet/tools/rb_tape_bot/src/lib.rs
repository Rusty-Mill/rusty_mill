//! Shared pieces of the tape bots: the scenario's start state and a tape
//! input as RLBot messages. See docs/research/BOT-CAPTURE-PLAN.md.

use rb_scenario::{BallStart, CarStart, Input, Scenario};
use rlbot::flat::{
    ControllerState, DesiredBallState, DesiredCarState, DesiredGameState, DesiredPhysics, Float,
    RotatorPartial, Vector3Partial,
};

pub fn float(val: f32) -> Option<Float> {
    Some(Float { val })
}

pub fn vector(v: [f32; 3]) -> Option<Box<Vector3Partial>> {
    Some(Box::new(Vector3Partial {
        x: float(v[0]),
        y: float(v[1]),
        z: float(v[2]),
    }))
}

pub fn rotator(r: [f32; 3]) -> Option<Box<RotatorPartial>> {
    Some(Box::new(RotatorPartial {
        pitch: float(r[0]),
        yaw: float(r[1]),
        roll: float(r[2]),
    }))
}

pub fn car_state(car: &CarStart) -> DesiredCarState {
    DesiredCarState {
        physics: Some(Box::new(DesiredPhysics {
            location: car.location.and_then(vector),
            rotation: car.rotation.and_then(rotator),
            velocity: car.velocity.and_then(vector),
            angular_velocity: car.angular_velocity.and_then(vector),
        })),
        boost_amount: car.boost.and_then(float),
    }
}

pub fn ball_state(ball: &BallStart) -> DesiredBallState {
    DesiredBallState {
        physics: Box::new(DesiredPhysics {
            location: ball.location.and_then(vector),
            rotation: None,
            velocity: ball.velocity.and_then(vector),
            angular_velocity: ball.angular_velocity.and_then(vector),
        }),
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
