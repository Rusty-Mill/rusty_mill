//! Codex's prompt: the shared core and format spec, plus what Codex can do
//! that a bare model cannot: read the repository itself.

use orch_core::board::Board;
use orch_core::task::Task;

/// Render the prompt for `task`. Path and commit refs stay refs: Codex reads
/// them from the working directory under the read-only sandbox. Only the
/// referenced board entries are inlined, as for every adapter.
pub fn render(task: &Task, board: &Board) -> String {
    let mut footer = orch_cli::format_spec(task.spec().role);
    footer.push_str(
        "- You may read files under the working directory to ground your entries; \
         cite what you read as path: refs.\n\
         - The sandbox is read-only and offline: do not modify files or use the network.\n",
    );
    orch_cli::render(task, board, &footer)
}
