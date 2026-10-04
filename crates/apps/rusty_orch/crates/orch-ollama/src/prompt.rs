//! Ollama's prompt: the shared core plus the shared format spec, unchanged.

use orch_core::board::Board;
use orch_core::goal::StopRule;
use orch_core::task::Task;

/// Render the prompt for `task` with the output-format spec for its role
/// under `stop` (ADR-0011).
pub fn render(task: &Task, board: &Board, stop: StopRule) -> String {
    let footer = orch_cli::format_spec(task.spec().role, stop);
    orch_cli::render(task, board, &footer)
}
