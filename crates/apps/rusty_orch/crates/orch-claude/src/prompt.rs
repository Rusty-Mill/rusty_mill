//! Claude's prompt: the shared core and format spec, plus what Claude Code
//! can do that a bare model cannot: read the repository itself.

use orch_core::board::Board;
use orch_core::goal::StopRule;
use orch_core::task::Task;

/// Render the prompt for `task`. Path and commit refs stay refs: Claude
/// reads them from the working directory with its read-only tools. Only the
/// referenced board entries are inlined, as for every adapter. `stop` picks
/// the kinds the card may write (ADR-0011).
pub fn render(task: &Task, board: &Board, stop: StopRule) -> String {
    let mut footer = orch_cli::format_spec(task.spec().role, stop);
    footer.push_str(
        "- You may read files under the working directory to ground your entries; \
         cite what you read as path: refs.\n\
         - Your tools are read-only and there is no shell or network: do not try to \
         modify files or reach the network.\n",
    );
    orch_cli::render(task, board, &footer)
}
