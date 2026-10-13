//! How the next turn holder is chosen. Recorded once per task, in `task_opened`.

use rusty_serde::{Deserialize, Serialize};

/// The scheduling policy of a task. One variant today; the event-driven
/// variants arrive behind this enum, after the shadow comparison is reviewed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SchedulingPolicy {
    /// The roles that serve the current state take turns in `turn_order`, the
    /// default holder first (v0.5 behaviour).
    /// Also what a log written before this field existed replays as.
    #[default]
    TurnBased,
}
