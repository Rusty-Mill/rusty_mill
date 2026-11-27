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

/// Who may answer a task's human gates (#447, D191). Recorded once, in `task_opened`, only when
/// not owned: an owned task (`None`) needs a board-signed receipt for its plan and merge
/// approvals, so absent is the strict case and old logs replay as owned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Gates {
    /// A scratch or evidence task: a labelled `--as` answer is enough. The host allows it only
    /// for repos on its scratch list, and never with claims mode (D190).
    #[rusty_serde(rename = "scratch")]
    Scratch,
}
