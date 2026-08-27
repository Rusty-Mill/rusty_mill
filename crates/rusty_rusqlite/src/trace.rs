//! `trace` module (Part B gap row "trace module: TraceEvent,
//! TraceEventCodes, ConnRef, StmtRef, config_log, log"), consumed by
//! [`crate::Connection::trace_v2`].
//!
//! **Partially superseded already:** [`crate::Connection::trace`]/
//! [`crate::Connection::profile`] (the older callback-based tracing API
//! real `rusqlite` still ships alongside this newer one) already cover
//! "observe SQL text and execution time" — `trace_v2` unifies both into
//! one callback keyed by [`TraceEventCodes`], matching real SQLite's own
//! `sqlite3_trace_v2`.
//!
//! **`config_log`/`log` are not implemented, on purpose:** `config_log`
//! hooks SQLite's internal C-level diagnostic log (`sqlite3_log`). This
//! is a from-scratch Rust engine with no `libsqlite3-sys` dependency and
//! no equivalent internal log stream — unlike `busy_timeout`/`db_config`
//! (stored-but-unenforced settings that *would* matter if this engine
//! grew lock contention or foreign-key checking to add), a `config_log`
//! callback here would never fire under any circumstance this codebase
//! could produce. Implementing it as inert scaffolding would misrepresent
//! it as more functional than it is, so it's left out rather than added
//! for API-shape completeness alone. `log` (SQLite's macro for writing
//! to that same log) has nothing to write to for the same reason.
//!
//! **`Row` isn't a `TraceEvent` variant:** real SQLite fires it once per
//! row as a statement steps incrementally. This engine's queries run to
//! completion in one call (see `ARCHITECTURE.md` — no virtual machine to
//! step), so there's no per-row moment to fire it at.

use crate::connection::Connection;
use std::time::Duration;

/// Which [`TraceEvent`] kinds a [`crate::Connection::trace_v2`] callback
/// wants to observe, mirroring SQLite's `SQLITE_TRACE_*` bitmask
/// constants. Combine with `|`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceEventCodes(u32);

impl TraceEventCodes {
    /// Fires [`TraceEvent::Stmt`] — before a statement runs.
    pub const STMT: TraceEventCodes = TraceEventCodes(1 << 0);
    /// Fires [`TraceEvent::Profile`] — after a statement finishes.
    pub const PROFILE: TraceEventCodes = TraceEventCodes(1 << 1);
    /// Fires [`TraceEvent::Close`] — when a connection closes. Real
    /// SQLite's `SQLITE_TRACE_CLOSE`.
    pub const CLOSE: TraceEventCodes = TraceEventCodes(1 << 2);
    /// Every event kind this crate can fire (`Row`, real SQLite's fourth
    /// bit, has no equivalent here — see this module's doc comment).
    pub const ALL: TraceEventCodes =
        TraceEventCodes(Self::STMT.0 | Self::PROFILE.0 | Self::CLOSE.0);

    /// Returns whether `self` includes every bit set in `other`.
    pub fn contains(&self, other: TraceEventCodes) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for TraceEventCodes {
    type Output = TraceEventCodes;
    fn bitor(self, rhs: Self) -> Self {
        TraceEventCodes(self.0 | rhs.0)
    }
}

/// The SQL text a [`TraceEvent`] fired for.
///
/// **Simplified from real `rusqlite::StmtRef`**, which wraps a raw
/// `sqlite3_stmt` C handle (letting a callback query things like
/// `expanded_sql()` on it). This engine has no such handle — `StmtRef`
/// exposes only the SQL text itself, which is what `sql()` would return
/// on the real type anyway for a parameter-free statement (the common
/// case for `Connection::execute`/`query_*`, which don't support
/// parameters at all — see `crate::Statement` for the type that does).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StmtRef<'a>(&'a str);

impl<'a> StmtRef<'a> {
    pub(crate) fn new(sql: &'a str) -> StmtRef<'a> {
        StmtRef(sql)
    }

    /// The statement's SQL text.
    pub fn sql(&self) -> &str {
        self.0
    }
}

/// The connection a [`TraceEvent::Close`] fired for.
///
/// **Simplified from real `rusqlite::ConnRef`** for the same reason as
/// [`StmtRef`] — no raw C handle to wrap, so this exposes read-only
/// [`Connection`] methods directly instead.
pub struct ConnRef<'a>(&'a Connection);

impl<'a> ConnRef<'a> {
    pub(crate) fn new(conn: &'a Connection) -> ConnRef<'a> {
        ConnRef(conn)
    }

    /// Returns whether the connection is still open — always `true` for
    /// a `Close` event, since it fires just before the connection
    /// actually closes.
    pub fn is_open(&self) -> bool {
        self.0.is_open()
    }
}

/// An event a [`crate::Connection::trace_v2`] callback observes.
pub enum TraceEvent<'a> {
    /// A statement is about to run. The second field is real SQLite's
    /// "expanded SQL" (parameter values substituted in) — always equal
    /// to the first for [`Connection::execute`]/`query_*`, which don't
    /// support parameters (see [`StmtRef`]'s doc comment).
    Stmt(StmtRef<'a>, &'a str),
    /// A statement finished running (successfully or not — timing is
    /// reported either way, matching real SQLite).
    Profile(StmtRef<'a>, Duration),
    /// A connection is about to close.
    Close(ConnRef<'a>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_event_codes_combine_and_contains() {
        let mask = TraceEventCodes::STMT | TraceEventCodes::PROFILE;
        assert!(mask.contains(TraceEventCodes::STMT));
        assert!(mask.contains(TraceEventCodes::PROFILE));
        assert!(!mask.contains(TraceEventCodes::CLOSE));
        assert!(TraceEventCodes::ALL.contains(mask));
    }

    #[test]
    fn stmt_ref_exposes_sql_text() {
        let stmt_ref = StmtRef::new("SELECT 1");
        assert_eq!(stmt_ref.sql(), "SELECT 1");
    }

    #[test]
    fn conn_ref_reflects_open_state() {
        let conn = Connection::open_in_memory().unwrap();
        let conn_ref = ConnRef::new(&conn);
        assert!(conn_ref.is_open());
    }
}
