//! How the next turn holder is chosen. Recorded once per task, in `task_opened`.

use rusty_serde::{Deserialize, Serialize};

/// The scheduling policy of a task. One variant today; the event-driven
/// variants arrive behind this enum, after the shadow comparison is reviewed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SchedulingPolicy {
    /// The default holder of the current state takes the turn (v0.5 behaviour).
    /// Also what a log written before this field existed replays as.
    #[default]
    TurnBased,
}

impl SchedulingPolicy {
    /// The most roles `pick_next` may return, so the most turns live at once.
    /// One for every policy today: concurrent turns are a separate design.
    pub const fn max_live(self) -> usize {
        match self {
            SchedulingPolicy::TurnBased => 1,
        }
    }
}
