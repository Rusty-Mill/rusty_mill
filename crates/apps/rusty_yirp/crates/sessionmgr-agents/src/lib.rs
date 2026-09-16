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
pub mod handoff;
pub mod pattern_watch;

pub use handoff::render_handoff;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `AgentAdapterPort::supports_fork` and `fork_args` answer two
    /// different questions asked at two different times -- see
    /// `supports_fork`'s own docs for why a state-dependent adapter
    /// (Gemini CLI, whose `fork_args` can legitimately return `None` for
    /// a specific call even though its mechanism works) breaks the
    /// strict equality this test used to assert in both directions.
    ///
    /// The direction that still always holds, for every adapter present
    /// and future: an adapter that says it does **not** support Fork at
    /// all must never produce a command line regardless of what it is
    /// given -- run once here across every adapter this crate ships
    /// rather than duplicated per adapter's own test module, so a new
    /// adapter can't silently return `Some` from `fork_args` while also
    /// answering `false` from `supports_fork`.
    #[test]
    fn an_adapter_that_does_not_support_fork_never_produces_fork_args() {
        let source = sessionmgr_core::ports::ForkSource {
            native_session_id: Some("source"),
            workspace_cwd: std::path::Path::new("/workspace"),
        };
        for kind in [AgentKind::ClaudeCode, AgentKind::Codex, AgentKind::Gemini] {
            let adapter = adapter_for(kind);
            if !adapter.supports_fork() {
                assert!(
                    adapter.fork_args(source, "new", &[]).is_none(),
                    "{kind:?}: supports_fork() is false but fork_args(..) still produced a command"
                );
            }
        }
    }
}
