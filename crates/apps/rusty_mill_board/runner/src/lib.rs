//! Mill Board runner: an optional process that works cards through stages.
//!
//! - [`config`]: the stage table (column, role, command, destinations).
//! - [`ports`]: the board and the workspace, as traits.
//! - [`runner`]: the stage loop, pure over those ports.
//! - [`http_board`], [`git_work`]: the real adapters (board API; git
//!   worktrees with sandboxed commands).

pub mod config;
pub mod git_work;
pub mod http_board;
pub mod ports;
pub mod runner;
