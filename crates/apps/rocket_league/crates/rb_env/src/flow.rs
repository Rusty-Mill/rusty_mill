//! Match flow (`RB-PHYSICS-001-FR-160`, ADR-0082): the phases a Soccar match moves through
//! around a goal, with the lengths and the goal line measured on the game
//! (`tools/rb_tape_bot/src/bin/rb_match_log.rs`, RB-RESEARCH-O024).
//!
//! ```text
//! Active --(ball past the goal line)--> GoalScored 3.0 s --> Replay 9.0 s
//!        --> Countdown 4.0 s --> Kickoff --(first touch)--> Active
//! ```
//!
//! This is pure bookkeeping: it looks at the ball and says when the phase changes. What a phase
//! does to the world (a frozen replay, a reset at the countdown, a held ball) is `Env`'s job.

use rb_domain::Vec3;

/// Ticks a goal is shown as scored while play carries on (3.0 s).
pub const GOAL_SCORED_TICKS: u32 = 360;
/// Ticks of the goal replay, during which the world does not move (9.0 s).
pub const REPLAY_TICKS: u32 = 1080;
/// Ticks of the kickoff countdown, cars and ball placed and inputs ignored (4.0 s).
pub const COUNTDOWN_TICKS: u32 = 480;
/// The ball scores when its centre is past this |y|: the goal line (5120) and the ball's reach.
/// The game scored at 5215.55 and not at 5214.02 (`log2.jsonl`, 16 goals).
pub const GOAL_LINE_Y: f32 = 5215.0;
/// A ball further than this (uu) from the centre spot in the plane has been touched.
pub const TOUCH_DISTANCE: f32 = 0.05;
/// A ball faster than this (uu/s) in the plane has been touched.
pub const TOUCH_SPEED: f32 = 0.5;

/// Where a match is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Kickoff countdown: everything placed, nothing driven.
    Countdown,
    /// Countdown over, ball not yet touched.
    Kickoff,
    /// The ball is in play.
    Active,
    /// A goal has just been scored; play carries on for [`GOAL_SCORED_TICKS`].
    GoalScored,
    /// The goal replay; the world is frozen for [`REPLAY_TICKS`].
    Replay,
}

/// The match as an observer sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchState {
    pub phase: Phase,
    /// Ticks spent in the phase so far.
    pub ticks_in_phase: u32,
    /// Goals: blue (team 0), orange (team 1).
    pub score: [u32; 2],
}

/// What changed on a tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    None,
    /// The ball crossed a goal line; the team scored.
    Goal {
        team: usize,
    },
    ReplayStarted,
    /// The replay is over: reset the field.
    CountdownStarted,
    KickoffStarted,
    /// The ball was touched.
    PlayStarted,
}

/// The phase machine.
#[derive(Debug, Clone, Copy)]
pub struct Flow {
    state: MatchState,
}

impl Flow {
    /// A machine in `phase` with no goals.
    pub fn new(phase: Phase) -> Flow {
        Flow {
            state: MatchState {
                phase,
                ticks_in_phase: 0,
                score: [0, 0],
            },
        }
    }

    /// The same score, started over in `phase`.
    pub fn restarted(&self, phase: Phase) -> Flow {
        Flow {
            state: MatchState {
                phase,
                ticks_in_phase: 0,
                score: self.state.score,
            },
        }
    }

    pub fn state(&self) -> MatchState {
        self.state
    }

    fn enter(&mut self, phase: Phase) {
        self.state.phase = phase;
        self.state.ticks_in_phase = 0;
    }

    /// Advances one tick given where the ball is after it. A phase's `ticks_in_phase` counts
    /// the ticks since it began; the tick that ends one is the first of the next.
    pub fn after_step(&mut self, ball: Vec3, ball_velocity: Vec3) -> Transition {
        self.state.ticks_in_phase += 1;
        match self.state.phase {
            Phase::Active => {
                if ball.y.abs() > GOAL_LINE_Y {
                    // Blue attacks +y, orange -y.
                    let team = usize::from(ball.y < 0.0);
                    self.state.score[team] += 1;
                    self.enter(Phase::GoalScored);
                    return Transition::Goal { team };
                }
            }
            Phase::GoalScored if self.state.ticks_in_phase >= GOAL_SCORED_TICKS => {
                self.enter(Phase::Replay);
                return Transition::ReplayStarted;
            }
            Phase::Replay if self.state.ticks_in_phase >= REPLAY_TICKS => {
                self.enter(Phase::Countdown);
                return Transition::CountdownStarted;
            }
            Phase::Countdown if self.state.ticks_in_phase >= COUNTDOWN_TICKS => {
                self.enter(Phase::Kickoff);
                return Transition::KickoffStarted;
            }
            Phase::Kickoff => {
                let moved = ball.x.hypot(ball.y) > TOUCH_DISTANCE
                    || ball_velocity.x.hypot(ball_velocity.y) > TOUCH_SPEED;
                if moved {
                    self.enter(Phase::Active);
                    return Transition::PlayStarted;
                }
            }
            _ => {}
        }
        Transition::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(flow: &mut Flow, ticks: u32, ball: Vec3) -> Vec<Transition> {
        (0..ticks)
            .map(|_| flow.after_step(ball, Vec3::ZERO))
            .filter(|t| *t != Transition::None)
            .collect()
    }

    #[test]
    fn a_ball_just_inside_the_goal_line_is_not_a_goal_and_one_past_it_is() {
        let mut flow = Flow::new(Phase::Active);
        assert_eq!(
            flow.after_step(Vec3::new(0.0, 5214.02, 390.0), Vec3::ZERO),
            Transition::None
        );
        assert_eq!(
            flow.after_step(Vec3::new(0.0, 5215.55, 390.0), Vec3::ZERO),
            Transition::Goal { team: 0 }
        );
        assert_eq!(flow.state().score, [1, 0]);
    }

    #[test]
    fn the_negative_goal_scores_for_orange() {
        let mut flow = Flow::new(Phase::Active);
        assert_eq!(
            flow.after_step(Vec3::new(100.0, -5300.0, 100.0), Vec3::ZERO),
            Transition::Goal { team: 1 }
        );
        assert_eq!(flow.state().score, [0, 1]);
    }

    #[test]
    fn a_goal_runs_through_the_phases_for_the_measured_times() {
        let mut flow = Flow::new(Phase::Active);
        flow.after_step(Vec3::new(0.0, 5300.0, 100.0), Vec3::ZERO);
        let away = Vec3::new(0.0, 5300.0, 100.0);
        assert_eq!(run(&mut flow, GOAL_SCORED_TICKS - 1, away), vec![]);
        assert_eq!(flow.state().phase, Phase::GoalScored);
        assert_eq!(run(&mut flow, 1, away), vec![Transition::ReplayStarted]);
        assert_eq!(
            run(&mut flow, REPLAY_TICKS, away),
            vec![Transition::CountdownStarted]
        );
        let centre = Vec3::new(0.0, 0.0, 92.75);
        assert_eq!(run(&mut flow, COUNTDOWN_TICKS - 1, centre), vec![]);
        assert_eq!(flow.state().phase, Phase::Countdown);
        assert_eq!(run(&mut flow, 1, centre), vec![Transition::KickoffStarted]);
        // The kickoff waits for a touch, however long.
        assert_eq!(run(&mut flow, 5_000, centre), vec![]);
        assert_eq!(flow.state().phase, Phase::Kickoff);
        let touched = flow.after_step(
            Vec3::new(-0.69, -7.45, 95.36),
            Vec3::new(-326.0, -1275.0, 375.0),
        );
        assert_eq!(touched, Transition::PlayStarted);
        assert_eq!(flow.state().phase, Phase::Active);
        assert_eq!(flow.state().score, [1, 0]);
    }

    #[test]
    fn a_ball_on_the_floor_inside_the_goal_mouth_is_not_scored_before_the_line() {
        let mut flow = Flow::new(Phase::Active);
        assert_eq!(run(&mut flow, 100, Vec3::new(0.0, 5100.0, 93.0)), vec![]);
    }
}
