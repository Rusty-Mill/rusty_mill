//! `rb_match_log --tie-up`: shots at goal until the score is level, each confirmed by a later
//! score observation (a shot that does not score is retried, a multi-goal deficit takes several).

/// Observations (of play inside the tie-up window) a shot gets to change the score before it is
/// taken for a miss and fired again.
pub const SHOT_PATIENCE: u32 = 120;

/// What the caller does after an observation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    /// The score is level: nothing more to do.
    Done,
    /// Put the ball in a goal: `+1.0` the orange goal (the blue side trails), `-1.0` the blue.
    Shoot(f32),
    /// A shot is out; look again later.
    Wait,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TieUp {
    /// The score and the observations since the shot in flight, if any.
    shot: Option<([u32; 2], u32)>,
}

impl TieUp {
    pub fn observe(&mut self, blue: u32, orange: u32) -> Action {
        if blue == orange {
            self.shot = None;
            return Action::Done;
        }
        if let Some((from, waited)) = &mut self.shot {
            // The shot scored if the score moved; either way a miss is given up on after a while.
            if *from == [blue, orange] && *waited < SHOT_PATIENCE {
                *waited += 1;
                return Action::Wait;
            }
        }
        self.shot = Some(([blue, orange], 0));
        Action::Shoot(if blue < orange { 1.0 } else { -1.0 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_level_score_is_done_without_a_shot() {
        assert_eq!(TieUp::default().observe(2, 2), Action::Done);
    }

    #[test]
    fn a_trailing_side_shoots_once_and_waits_for_the_score_to_move() {
        let mut tie = TieUp::default();
        assert_eq!(tie.observe(0, 1), Action::Shoot(1.0));
        assert_eq!(tie.observe(0, 1), Action::Wait);
        // Scored: the next observation shows 1-1, which is what confirms it.
        assert_eq!(tie.observe(1, 1), Action::Done);
        assert_eq!(TieUp::default().observe(3, 1), Action::Shoot(-1.0));
    }

    #[test]
    fn a_multi_goal_deficit_is_not_complete_after_the_first_goal() {
        let mut tie = TieUp::default();
        assert_eq!(tie.observe(0, 2), Action::Shoot(1.0));
        assert_eq!(tie.observe(0, 2), Action::Wait);
        // 1-2 is not level: shoot again.
        assert_eq!(tie.observe(1, 2), Action::Shoot(1.0));
        assert_eq!(tie.observe(2, 2), Action::Done);
    }

    #[test]
    fn a_shot_that_does_not_score_is_fired_again() {
        let mut tie = TieUp::default();
        assert_eq!(tie.observe(0, 1), Action::Shoot(1.0));
        for _ in 0..SHOT_PATIENCE {
            assert_eq!(tie.observe(0, 1), Action::Wait);
        }
        assert_eq!(tie.observe(0, 1), Action::Shoot(1.0));
    }
}
