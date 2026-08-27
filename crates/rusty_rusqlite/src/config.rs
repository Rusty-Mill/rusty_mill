//! `config`/`limits` module enums (Part B gap row "config/limits modules:
//! DbConfig, Limit enums"). Stored (but not yet enforced anywhere in the
//! engine) via `Connection::db_config`/`set_db_config`/`limit`/
//! `set_limit` — see that issue's PR for what "enforced" would mean for
//! each variant.

/// A boolean-valued connection configuration option, set via
/// [`crate::Connection::db_config`]/[`crate::Connection::set_db_config`].
/// See <https://www.sqlite.org/c3ref/c_dbconfig_defensive.html>.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DbConfig {
    /// Enable/disable foreign key constraint enforcement.
    EnableForeignKeys,
    /// Enable/disable triggers.
    EnableTriggers,
    /// Enable/disable the `defensive` flag (extra guards against
    /// corrupting the database via SQL, e.g. writing to `sqlite_dbpage`).
    Defensive,
}

/// A resource limit, set via [`crate::Connection::limit`]/
/// [`crate::Connection::set_limit`]. See
/// <https://www.sqlite.org/c3ref/c_limit_attached.html>.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Limit {
    /// Maximum length, in bytes, of any string or blob.
    Length,
    /// Maximum number of columns in a table definition or result set.
    Column,
    /// Maximum depth of an expression tree.
    ExprDepth,
    /// Maximum number of terms in a compound `SELECT`.
    CompoundSelect,
    /// Maximum number of instructions in the execution engine's compiled
    /// form of a SQL statement.
    VdbeOp,
    /// Maximum number of arguments to a SQL function.
    FunctionArg,
    /// Maximum number of attached databases.
    AttachedDb,
}

/// Flags controlling how a connection is opened, via
/// [`crate::Connection::open_with_flags`]/
/// [`crate::Connection::open_in_memory_with_flags`] (Part B gap row
/// "Connection: constructors").
///
/// **Design deviation, stated plainly:** a hand-rolled bitmask (no
/// `bitflags` dependency) mirroring `rusqlite::OpenFlags`'s constant
/// names for API-shape parity. Most bits are accepted but inert — this
/// engine has no shared-cache mode, no per-connection-vs-shared mutex
/// distinction (single-threaded, single-writer in-memory model), and
/// doesn't parse paths as `file:` URIs. Only [`OpenFlags::READ_ONLY`]
/// (enforced: mutating calls on a read-only connection error) and
/// [`OpenFlags::CREATE`] (enforced: without it, opening a nonexistent
/// path errors instead of silently starting an empty database) actually
/// change behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenFlags(u32);

impl OpenFlags {
    pub const READ_ONLY: OpenFlags = OpenFlags(0x0000_0001);
    pub const READ_WRITE: OpenFlags = OpenFlags(0x0000_0002);
    pub const CREATE: OpenFlags = OpenFlags(0x0000_0004);
    /// Accepted for shape parity; inert.
    pub const URI: OpenFlags = OpenFlags(0x0000_0040);
    /// Accepted for shape parity; inert.
    pub const NO_MUTEX: OpenFlags = OpenFlags(0x0000_8000);
    /// Accepted for shape parity; inert.
    pub const FULL_MUTEX: OpenFlags = OpenFlags(0x0001_0000);
    /// Accepted for shape parity; inert.
    pub const SHARED_CACHE: OpenFlags = OpenFlags(0x0002_0000);
    /// Accepted for shape parity; inert.
    pub const PRIVATE_CACHE: OpenFlags = OpenFlags(0x0004_0000);

    /// Whether every bit set in `other` is also set in `self`.
    pub fn contains(self, other: OpenFlags) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for OpenFlags {
    type Output = OpenFlags;
    fn bitor(self, rhs: OpenFlags) -> OpenFlags {
        OpenFlags(self.0 | rhs.0)
    }
}

impl Default for OpenFlags {
    /// `READ_WRITE | CREATE`, matching `rusqlite::OpenFlags::default()`.
    fn default() -> OpenFlags {
        OpenFlags::READ_WRITE | OpenFlags::CREATE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_config_variants_are_distinct() {
        assert_ne!(DbConfig::EnableForeignKeys, DbConfig::EnableTriggers);
        assert_eq!(DbConfig::Defensive, DbConfig::Defensive);
    }

    #[test]
    fn limit_variants_are_distinct() {
        assert_ne!(Limit::Length, Limit::Column);
        assert_eq!(Limit::AttachedDb, Limit::AttachedDb);
    }

    #[test]
    fn default_open_flags_are_read_write_create() {
        let flags = OpenFlags::default();
        assert!(flags.contains(OpenFlags::READ_WRITE));
        assert!(flags.contains(OpenFlags::CREATE));
        assert!(!flags.contains(OpenFlags::READ_ONLY));
    }

    #[test]
    fn bitor_combines_flags() {
        let flags = OpenFlags::READ_ONLY | OpenFlags::NO_MUTEX;
        assert!(flags.contains(OpenFlags::READ_ONLY));
        assert!(flags.contains(OpenFlags::NO_MUTEX));
        assert!(!flags.contains(OpenFlags::CREATE));
    }
}
