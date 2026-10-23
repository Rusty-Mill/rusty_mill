//! How the next turn holder is chosen. Recorded once per task, in `task_opened`.

use rusty_serde::{Deserialize, Serialize};

/// The scheduling policy of a task, chosen when it opens and never changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SchedulingPolicy {
    /// The roles that serve the current state take turns in `turn_order`, the
    /// default holder first (v0.5 behaviour).
    /// Also what a log written before this field existed replays as, which is
    /// why it stays this enum's `Default`; hosts open new one-turn tasks
    /// `EventDriven`.
    #[default]
    TurnBased,
    /// The next turn goes to the first role `wake_set` makes selectable, and to
    /// the `TurnBased` choice when none is. Still exactly one live turn, so the
    /// task must open with `max_live_turns == 1`. Chosen per task at open.
    EventDriven,
}
