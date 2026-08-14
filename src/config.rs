//! `config`/`limits` module enums (Part B gap row "config/limits modules:
//! DbConfig, Limit enums"). Definitions only — nothing reads or enforces
//! these yet; that's `Connection`'s configuration-knobs issue.

/// A boolean-valued connection configuration option, set via
/// `Connection::db_config`/`set_db_config` (not yet implemented). See
/// <https://www.sqlite.org/c3ref/c_dbconfig_defensive.html>.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbConfig {
    /// Enable/disable foreign key constraint enforcement.
    EnableForeignKeys,
    /// Enable/disable triggers.
    EnableTriggers,
    /// Enable/disable the `defensive` flag (extra guards against
    /// corrupting the database via SQL, e.g. writing to `sqlite_dbpage`).
    Defensive,
}

/// A resource limit, set via `Connection::limit`/`set_limit` (not yet
/// implemented). See <https://www.sqlite.org/c3ref/c_limit_attached.html>.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}
