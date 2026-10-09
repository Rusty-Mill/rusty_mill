//! Match flow (`RB-PHYSICS-001-FR-160`, ADR-0082): the phases a Soccar match moves through
//! around a goal, with the lengths and the goal line measured on the game
//! (`tools/rb_tape_bot/src/bin/rb_match_log.rs`, RB-RESEARCH-O024).
//!
//! ```text
//! Active --(ball past the goal line)--> GoalScored 3.0 s --> Replay 9.0 s
//!        --> Countdown 4.0 s --> Kickoff --(first touch)--> Active
//! ```
//!
//! With a clock (`Flow::with_clock`) the match also ends: the clock runs only in `Active` (a
//! 300 s match is 36000 ticks of it), and at zero play goes on until the ball is low; then a tie
//! starts overtime (a countdown with no replay, the clock counting up, the next goal wins) and a
//! lead ends the match. The very first countdown of a match is 850 ticks from the start of the match (an intro).
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
/// Ticks from the start of a match to its first kickoff (7.1 s, countdown and intro): the five logs
/// ended it at frames 846 to 853.
pub const FIRST_COUNTDOWN_TICKS: u32 = 850;
/// Ticks of a five-minute match's clock.
pub const FIVE_MINUTES: i64 = 36_000;
/// With the clock at zero, play ends when the ball's centre is this low (uu): it has met the
/// floor (the game ended matches at 92.2 and 97.3, the latter on the curve).
pub const MATCH_END_BALL_HEIGHT: f32 = 97.5;
/// The ball scores when its centre is past this |y|: the goal line (5120) and the ball's reach.
/// The game scored at 5215.55 and not at 5214.02 (`log2.jsonl`, 16 goals).
pub const GOAL_LINE_Y: f32 = 5215.0;
/// A kickoff nobody touches goes live anyway after this many ticks (5.0 s): three of 48 kickoffs of
/// the logs ended at 600 or 601 frames with the ball still on the spot.
pub const KICKOFF_TIMEOUT_TICKS: u32 = 600;
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
    /// The match is over; nothing moves.
    Ended,
}

/// The match as an observer sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchState {
    pub phase: Phase,
    /// Ticks spent in the phase so far.
    pub ticks_in_phase: u32,
    /// Goals: blue (team 0), orange (team 1).
    pub score: [u32; 2],
    /// Ticks of play left (negative once the clock has run out); `None` for an unlimited match.
    /// In overtime it is minus the ticks played since the clock ran out.
    pub clock_ticks: Option<i64>,
    /// Whether the match is in overtime.
    pub overtime: bool,
}

impl MatchState {
    /// The clock in seconds as the game shows it (`game_time_remaining`).
    pub fn seconds_remaining(&self) -> Option<f32> {
        self.clock_ticks.map(|ticks| ticks as f32 / 120.0)
    }
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
    /// The clock ran out level and the ball came down: overtime begins with a countdown.
    OvertimeStarted,
    /// The match is over.
    MatchEnded,
}

/// The phase machine.
#[derive(Debug, Clone, Copy)]
pub struct Flow {
    state: MatchState,
    countdown_ticks: u32,
    replay_ticks: u32,
}

impl Flow {
    /// A machine in `phase` with no goals.
    pub fn new(phase: Phase) -> Flow {
        Flow {
            state: MatchState {
                phase,
                ticks_in_phase: 0,
                score: [0, 0],
                clock_ticks: None,
                overtime: false,
            },
            countdown_ticks: COUNTDOWN_TICKS,
            replay_ticks: REPLAY_TICKS,
        }
    }

    /// The same machine with a match clock of `ticks` of play.
    pub fn with_clock(mut self, ticks: i64) -> Flow {
        self.state.clock_ticks = Some(ticks);
        self
    }

    /// The same machine with replays `ticks` long. The game's were 1076 to 1084 in 31 of 41 goals
    /// and 1103 to 1560 (up to 13 s) in the rest, for reasons the packets do not show.
    pub fn with_replay(mut self, ticks: u32) -> Flow {
        self.replay_ticks = ticks;
        self
    }

    /// The same machine with its next countdown `ticks` long (the first of a match is
    /// [`FIRST_COUNTDOWN_TICKS`]); later countdowns are [`COUNTDOWN_TICKS`] again.
    pub fn with_countdown(mut self, ticks: u32) -> Flow {
        self.countdown_ticks = ticks;
        self
    }

    /// The same score, started over in `phase`.
    pub fn restarted(&self, phase: Phase) -> Flow {
        Flow {
            state: MatchState {
                phase,
                ticks_in_phase: 0,
                ..self.state
            },
            countdown_ticks: self.countdown_ticks,
            replay_ticks: self.replay_ticks,
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
                // The clock runs in play only: down to zero and on below it, or up in overtime.
                if let Some(clock) = &mut self.state.clock_ticks {
                    *clock -= 1;
                }
                if ball.y.abs() > GOAL_LINE_Y {
                    // Blue attacks +y, orange -y.
                    let team = usize::from(ball.y < 0.0);
                    self.state.score[team] += 1;
                    self.enter(Phase::GoalScored);
                    return Transition::Goal { team };
                }
                let time_up =
                    self.state.clock_ticks.is_some_and(|c| c <= 0) && !self.state.overtime;
                if time_up && ball.z <= MATCH_END_BALL_HEIGHT {
                    if self.state.score[0] == self.state.score[1] {
                        self.state.overtime = true;
                        self.enter(Phase::Countdown);
                        return Transition::OvertimeStarted;
                    }
                    self.enter(Phase::Ended);
                    return Transition::MatchEnded;
                }
            }
            Phase::GoalScored if self.state.ticks_in_phase >= GOAL_SCORED_TICKS => {
                self.enter(Phase::Replay);
                return Transition::ReplayStarted;
            }
            Phase::Replay if self.state.ticks_in_phase >= self.replay_ticks => {
                // A goal in overtime ends the match; one after the clock ran out ends it or, if it
                // levelled the score, starts overtime.
                let expired = self.state.clock_ticks.is_some_and(|c| c <= 0);
                if self.state.overtime || (expired && self.state.score[0] != self.state.score[1]) {
                    self.enter(Phase::Ended);
                    return Transition::MatchEnded;
                }
                if expired {
                    self.state.overtime = true;
                    self.enter(Phase::Countdown);
                    return Transition::OvertimeStarted;
                }
                self.enter(Phase::Countdown);
                return Transition::CountdownStarted;
            }
            Phase::Countdown if self.state.ticks_in_phase >= self.countdown_ticks => {
                self.countdown_ticks = COUNTDOWN_TICKS;
                self.enter(Phase::Kickoff);
                return Transition::KickoffStarted;
            }
            Phase::Kickoff => {
                let moved = ball.x.hypot(ball.y) > TOUCH_DISTANCE
                    || ball_velocity.x.hypot(ball_velocity.y) > TOUCH_SPEED;
                if moved || self.state.ticks_in_phase >= KICKOFF_TIMEOUT_TICKS {
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
        // The kickoff waits for a touch, but only so long.
        assert_eq!(run(&mut flow, KICKOFF_TIMEOUT_TICKS - 1, centre), vec![]);
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

    fn goal_ball() -> Vec3 {
        Vec3::new(0.0, 5300.0, 100.0)
    }

    #[test]
    fn the_clock_runs_in_play_only() {
        let mut flow = Flow::new(Phase::Active).with_clock(FIVE_MINUTES);
        let high = Vec3::new(0.0, 0.0, 400.0);
        run(&mut flow, 120, high);
        assert_eq!(flow.state().seconds_remaining(), Some(299.0));
        // A goal stops it through the goal, the replay and the countdown.
        flow.after_step(goal_ball(), Vec3::ZERO);
        let stopped = flow.state().clock_ticks;
        run(
            &mut flow,
            GOAL_SCORED_TICKS + REPLAY_TICKS + COUNTDOWN_TICKS,
            high,
        );
        assert_eq!(flow.state().phase, Phase::Kickoff);
        assert_eq!(
            flow.state().clock_ticks,
            Some(stopped.unwrap_or(0) - 1).map(|c| c + 1)
        );
    }

    #[test]
    fn a_lead_at_zero_ends_the_match_once_the_ball_is_low() {
        let mut flow = Flow::new(Phase::Active).with_clock(3);
        flow.after_step(goal_ball(), Vec3::ZERO);
        let mut flow = flow.restarted(Phase::Active).with_clock(3);
        let high = Vec3::new(0.0, 0.0, 400.0);
        // Three ticks of clock, then play goes on below zero while the ball is up.
        assert_eq!(run(&mut flow, 50, high), vec![]);
        assert!(flow.state().clock_ticks.is_some_and(|c| c < 0));
        assert_eq!(
            flow.after_step(Vec3::new(0.0, 0.0, 92.2), Vec3::ZERO),
            Transition::MatchEnded
        );
        assert_eq!(flow.state().phase, Phase::Ended);
    }

    #[test]
    fn a_tie_at_zero_goes_to_overtime_and_the_next_goal_ends_the_match() {
        let mut flow = Flow::new(Phase::Active).with_clock(2);
        let high = Vec3::new(0.0, 0.0, 400.0);
        run(&mut flow, 10, high);
        // Level: no replay, straight to a countdown with the overtime clock running on.
        assert_eq!(
            flow.after_step(Vec3::new(0.0, 0.0, 92.2), Vec3::ZERO),
            Transition::OvertimeStarted
        );
        let state = flow.state();
        assert_eq!((state.phase, state.overtime), (Phase::Countdown, true));
        let clock = state.clock_ticks;
        let centre = Vec3::new(0.0, 0.0, 92.75);
        run(&mut flow, COUNTDOWN_TICKS, centre);
        assert_eq!(flow.state().phase, Phase::Kickoff);
        assert_eq!(flow.state().clock_ticks, clock, "the clock waits for play");
        flow.after_step(Vec3::new(1.0, 5.0, 95.0), Vec3::new(10.0, 50.0, 0.0));
        assert_eq!(flow.state().phase, Phase::Active);
        flow.after_step(goal_ball(), Vec3::ZERO);
        assert_eq!(flow.state().phase, Phase::GoalScored);
        assert_eq!(
            run(&mut flow, GOAL_SCORED_TICKS + REPLAY_TICKS, goal_ball()),
            vec![Transition::ReplayStarted, Transition::MatchEnded]
        );
        assert_eq!(flow.state().phase, Phase::Ended);
        assert_eq!(flow.state().score, [1, 0]);
    }

    #[test]
    fn the_first_countdown_of_a_match_is_longer() {
        let mut flow = Flow::new(Phase::Countdown).with_countdown(FIRST_COUNTDOWN_TICKS);
        let centre = Vec3::new(0.0, 0.0, 92.75);
        assert_eq!(run(&mut flow, FIRST_COUNTDOWN_TICKS - 1, centre), vec![]);
        assert_eq!(run(&mut flow, 1, centre), vec![Transition::KickoffStarted]);
        // The next one is the usual length again.
        let mut next = flow.restarted(Phase::Countdown);
        assert_eq!(
            run(&mut next, COUNTDOWN_TICKS, centre),
            vec![Transition::KickoffStarted]
        );
    }

    #[test]
    fn an_untouched_kickoff_goes_live_after_five_seconds() {
        let mut flow = Flow::new(Phase::Kickoff);
        let centre = Vec3::new(0.0, 0.0, 92.75);
        assert_eq!(run(&mut flow, KICKOFF_TIMEOUT_TICKS - 1, centre), vec![]);
        assert_eq!(run(&mut flow, 1, centre), vec![Transition::PlayStarted]);
    }
}
