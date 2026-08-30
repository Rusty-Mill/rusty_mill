//! Deferred signal events — the D6 core (RFC v2 §5.6; extraction map
//! D6), ported from rush's `trap.rs` mechanism: the OS-level handler does
//! exactly one atomic store, and ordinary code consumes the pending event
//! at safe points. Nothing here fires callbacks, allocates in handlers,
//! or interrupts anything — that discipline is the whole extraction.
//!
//! The slot is **single-entry**: a burst of events before the next
//! [`SignalSource::take`] coalesces to the latest one, exactly like the
//! donor's `PENDING_SIGNAL`. Consumers that must not miss terminations
//! check `take()` at their safe points (each loop iteration, each
//! `wait_any` timeout tick).
//!
//! Windows has no signals; its backend maps console control events
//! (Ctrl-C, Ctrl-Break, console close) onto these portable identities —
//! divergence 003.

use crate::error::Result;

/// Portable signal identities (deliberately a small mechanism-level set;
/// the full unix signal zoo is policy territory and stays with
/// consumers).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalEvent {
    /// Ctrl-C: `SIGINT` / `CTRL_C_EVENT`.
    Interrupt,
    /// Termination request: `SIGTERM` / `CTRL_BREAK_EVENT` (divergence
    /// 003 — Windows has no SIGTERM analog; Ctrl-Break is the nearest
    /// deliverable identity).
    Terminate,
    /// Session teardown: `SIGHUP` / `CTRL_CLOSE_EVENT`.
    Hangup,
}

/// A deferred, single-slot signal source. Object-safe.
pub trait SignalSource {
    /// Install the process-wide handler for `events`. Idempotent;
    /// process-global by nature (signal disposition is per-process state
    /// on every OS — this is a mechanism fact, not a design choice).
    fn install(&self, events: &[SignalEvent]) -> Result<()>;

    /// Consume the pending event, if any. Never blocks; a burst since
    /// the last call coalesces to the most recent event.
    fn take(&self) -> Option<SignalEvent>;
}
