//! Audit logging / change tracking for `Session`: an opt-in, append-only
//! record of every write a session flushes, kept in an ordinary table
//! (`_rusty_db_audit_log` by default) inside the same transaction as the
//! write itself — so an audit entry only ever exists for a change that
//! genuinely took effect; if the transaction rolls back, the audit entry
//! for anything flushed into it rolls back right along with it.
//!
//! This records the rendered SQL statement and its bound parameters
//! (formatted to text) for each write, not a structured before/after
//! diff of column values — a lightweight write-ahead trail (which
//! statement ran, on which table, when), not a full row-history/diffing
//! system.

use crate::value::Value;

/// The kind of write an `AuditEntry` records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditOperation {
    Insert,
    Update,
    Delete,
}

impl AuditOperation {
    pub fn as_str(&self) -> &'static str {
        match self {
            AuditOperation::Insert => "INSERT",
            AuditOperation::Update => "UPDATE",
            AuditOperation::Delete => "DELETE",
        }
    }

    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s {
            "INSERT" => Some(AuditOperation::Insert),
            "UPDATE" => Some(AuditOperation::Update),
            "DELETE" => Some(AuditOperation::Delete),
            _ => None,
        }
    }
}

/// One row of the audit log: a single write a `Session` flushed.
#[derive(Debug, Clone, PartialEq)]
pub struct AuditEntry {
    pub table: String,
    pub operation: AuditOperation,
    /// The exact SQL statement that was executed for this write.
    pub sql: String,
    /// The statement's bound parameters, formatted to text (via
    /// `Value`'s own `Display`) and comma-separated — lossy, but enough
    /// to inspect what a write did without needing to parse the
    /// statement text apart. A column named in the entity's
    /// `Mapped::REDACTED_COLUMNS` renders as `REDACTED_PLACEHOLDER`
    /// (`"[REDACTED]"`) here instead of its real value; every other
    /// column renders verbatim, in plaintext.
    pub params_text: String,
}

/// The fixed placeholder `params_to_text` substitutes for any parameter
/// bound to a column named in `redacted_columns` — see
/// `Mapped::REDACTED_COLUMNS`.
pub const REDACTED_PLACEHOLDER: &str = "[REDACTED]";

/// Renders `params` to the audit log's `params_text`, in order — the same
/// lossy, comma-separated `Value::Display` text this always produced,
/// except a parameter whose column (per `param_columns`, position-for-
/// position with `params`) is named in `redacted_columns` renders as
/// `REDACTED_PLACEHOLDER` instead of its real value. `param_columns`
/// shorter than `params` (the default `ToSql::param_columns` — "no column
/// information available") or a `None` at a given position both leave
/// that parameter unredacted, exactly matching pre-redaction behavior.
pub(crate) fn params_to_text(
    params: &[Value],
    param_columns: &[Option<String>],
    redacted_columns: &[&str],
) -> String {
    params
        .iter()
        .enumerate()
        .map(|(i, value)| {
            let is_redacted = param_columns
                .get(i)
                .and_then(|c| c.as_deref())
                .is_some_and(|column| redacted_columns.contains(&column));
            if is_redacted {
                REDACTED_PLACEHOLDER.to_string()
            } else {
                value.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn table_ddl(quoted_table: &str) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS {quoted_table} (\
            table_name TEXT NOT NULL, \
            operation TEXT NOT NULL, \
            sql_text TEXT NOT NULL, \
            params_text TEXT NOT NULL, \
            recorded_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP\
        )"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_to_text_redacts_only_the_marked_column() {
        let params = vec![
            Value::Text("ada".to_string()),
            Value::Text("hunter2".to_string()),
        ];
        let param_columns = vec![Some("name".to_string()), Some("password_hash".to_string())];
        let redacted_columns = ["password_hash"];

        let text = params_to_text(&params, &param_columns, &redacted_columns);

        assert_eq!(text, "\"ada\", [REDACTED]");
        assert!(text.contains("ada"));
        assert!(!text.contains("hunter2"));
    }

    #[test]
    fn params_to_text_renders_everything_verbatim_with_no_redacted_columns() {
        let params = vec![
            Value::Text("ada".to_string()),
            Value::Text("hunter2".to_string()),
        ];
        let param_columns = vec![Some("name".to_string()), Some("password_hash".to_string())];

        let text = params_to_text(&params, &param_columns, &[]);

        assert_eq!(text, "\"ada\", \"hunter2\"");
    }

    #[test]
    fn params_to_text_leaves_params_unredacted_when_column_info_is_unavailable() {
        // The default `ToSql::param_columns` returns an empty `Vec` — a
        // shorter-than-`params` (or entirely empty) `param_columns` must
        // never redact anything, since there's no column to match against
        // `redacted_columns` in the first place.
        let params = vec![Value::Text("hunter2".to_string())];

        let text = params_to_text(&params, &[], &["password_hash"]);

        assert_eq!(text, "\"hunter2\"");
    }
}
