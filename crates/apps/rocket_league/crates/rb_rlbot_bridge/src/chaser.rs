//! `rb_env`'s scripted chaser as a [`Policy`](crate::Policy).

use rb_domain::{ControllerInput, PhysicsFrame};
use rb_env::chaser::chase;

/// The chaser for a car on `team` (0 blue, attacks +y; 1 orange, attacks -y): it drives at the
/// point 160 uu behind the ball as seen from the opposing goal and boosts when pointed at it.
/// A car that is not in the frame gets a neutral input. Not a good player: a baseline that
/// makes a match happen.
pub fn chaser(team: usize) -> impl FnMut(&PhysicsFrame, usize) -> ControllerInput {
    move |frame, car| {
        frame
            .cars
            .get(car)
            .map_or_else(ControllerInput::default, |c| chase(c, &frame.ball, team))
    }
}
