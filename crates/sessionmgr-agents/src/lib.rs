//! Per-agent-CLI adapters: `launch_args` and tier-3 `needs_input`
//! pattern matching.
//!
//! PLAN.md's "explicitly the highest-uncertainty, highest-effort part of
//! this project." Two adapters exist -- [`claude_code::ClaudeCode`] and
//! [`codex::Codex`] -- both built from real, measured CLI output rather
//! than assumed shapes; see `docs/phase-3-report.md`. Gemini CLI is not
//! here yet: it has a confirmed hook mechanism (`gemini hooks`) but no
//! adapter, because nothing about it has been verified against a real,
//! running session -- this machine has no Gemini credentials. Adding it
//! is a new file plus one `AgentKind` variant, not a redesign.
//!
//! Each adapter's pattern set lives in its own file, per PLAN.md, so a
//! CLI's next release breaking its prompt format is a one-file fix.

pub mod claude_code;
pub mod codex;
pub mod pattern_watch;

pub use pattern_watch::ScreenWatcher;
use sessionmgr_core::ports::AgentAdapterPort;
use sessionmgr_core::AgentKind;
/// The adapter for `kind`.
///
/// `Send + Sync`, not just `AgentAdapterPort`: the daemon's worker
/// shares this across a PTY-reader OS thread and async tasks behind an
/// `Arc`, and both adapters here are zero-sized with no interior state,
/// so the bound costs nothing real.
pub fn adapter_for(kind: AgentKind) -> Box<dyn AgentAdapterPort + Send + Sync> {
    match kind {
        AgentKind::ClaudeCode => Box::new(claude_code::ClaudeCode),
        AgentKind::Codex => Box::new(codex::Codex),
    }
}
