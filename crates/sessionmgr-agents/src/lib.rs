//! Per-agent-CLI adapters: `launch_args` and tier-3 `needs_input`
//! pattern matching.
//!
//! PLAN.md's "explicitly the highest-uncertainty, highest-effort part of
//! this project." Three adapters exist -- [`claude_code::ClaudeCode`],
//! [`codex::Codex`], and [`gemini::GeminiCli`]. The first two are built
//! from real, measured CLI output; see `docs/phase-3-report.md`. The
//! third is built from `gemini-cli`'s own shipped source and hooks
//! reference docs rather than a live-captured session -- this machine
//! has no Gemini credentials, so `gemini` refuses to start at all before
//! reaching an interactive screen or firing a hook. See
//! [`gemini`]'s own module docs for exactly what that does and does not
//! change about its adapter's confidence.
//!
//! Each adapter's pattern set lives in its own file, per PLAN.md, so a
//! CLI's next release breaking its prompt format is a one-file fix.

pub mod claude_code;
pub mod codex;
pub mod gemini;
pub mod pattern_watch;

pub use pattern_watch::ScreenWatcher;
use sessionmgr_core::ports::AgentAdapterPort;
use sessionmgr_core::AgentKind;
/// The adapter for `kind`.
///
/// `Send + Sync`, not just `AgentAdapterPort`: the daemon's worker
/// shares this across a PTY-reader OS thread and async tasks behind an
/// `Arc`, and every adapter here is zero-sized with no interior state,
/// so the bound costs nothing real.
pub fn adapter_for(kind: AgentKind) -> Box<dyn AgentAdapterPort + Send + Sync> {
    match kind {
        AgentKind::ClaudeCode => Box::new(claude_code::ClaudeCode),
        AgentKind::Codex => Box::new(codex::Codex),
        AgentKind::Gemini => Box::new(gemini::GeminiCli),
    }
}
