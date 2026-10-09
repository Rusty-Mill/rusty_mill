//! A scripted chaser: the simplest opponent that makes a match happen. It drives at the point
//! 160 uu behind the ball as seen from the opposing goal and boosts when pointed at it. Not a good
//! player; a baseline for tests and examples of whole matches.

use rb_domain::{BallState, CarState, ControllerInput, Quat, Vec3};

fn yaw(q: &Quat) -> f32 {
    (2.0 * (q.w * q.z + q.x * q.y)).atan2(1.0 - 2.0 * (q.y * q.y + q.z * q.z))
}

/// The chaser's input for `car` on `team` (0 blue attacks +y, 1 orange -y).
pub fn chase(car: &CarState, ball: &BallState, team: usize) -> ControllerInput {
    let goal_y = if team == 0 { 5120.0 } else { -5120.0 };
    let to_goal = Vec3::new(-ball.position.x, goal_y - ball.position.y, 0.0);
    let length = to_goal.length().max(1.0);
    let target = Vec3::new(
        ball.position.x - to_goal.x / length * 160.0,
        ball.position.y - to_goal.y / length * 160.0,
        0.0,
    );
    let (dx, dy) = (target.x - car.position.x, target.y - car.position.y);
    let mut error = dy.atan2(dx) - yaw(&car.rotation);
    while error > std::f32::consts::PI {
        error -= std::f32::consts::TAU;
    }
    while error < -std::f32::consts::PI {
        error += std::f32::consts::TAU;
    }
    ControllerInput {
        throttle: 1.0,
        steer: (error * 2.0).clamp(-1.0, 1.0),
        boost: error.abs() < 0.3,
        ..ControllerInput::default()
    }
}
