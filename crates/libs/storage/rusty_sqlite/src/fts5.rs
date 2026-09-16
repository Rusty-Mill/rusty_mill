use rusqlite::Connection as RawConnection;

use crate::error::Result;

/// A built-in FTS5 tokenizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fts5Tokenizer {
    Unicode61,
    Ascii,
    Porter,
    Trigram,
}

impl Fts5Tokenizer {
    fn as_sql(self) -> &'static str {
        match self {
            Self::Unicode61 => "unicode61",
            Self::Ascii => "ascii",
            Self::Porter => "porter",
            Self::Trigram => "trigram",
        }
    }
}

fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn quote_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// A typed builder for `CREATE VIRTUAL TABLE ... USING fts5(...)` statements.
///
/// `rusqlite` exposes FTS5 only as raw SQL; this builder gives the common
/// options (columns, tokenizer, prefix indexes, external content tables) a
/// typed, composable API instead of hand-assembled strings.
///
/// ```
/// use rusty_sqlite::{Connection, Fts5TableBuilder, Fts5Tokenizer};
///
/// let conn = Connection::open_in_memory().unwrap();
/// Fts5TableBuilder::new("notes_fts")
///     .column("title")
///     .column("body")
///     .tokenizer(Fts5Tokenizer::Porter)
///     .prefix(2)
///     .prefix(3)
///     .create(conn.as_raw())
///     .unwrap();
/// ```
pub struct Fts5TableBuilder {
    name: String,
    columns: Vec<(String, bool)>,
    tokenizer: Option<Fts5Tokenizer>,
    prefixes: Vec<u32>,
    content_table: Option<String>,
    content_rowid: Option<String>,
}

impl Fts5TableBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            columns: Vec::new(),
            tokenizer: None,
            prefixes: Vec::new(),
            content_table: None,
            content_rowid: None,
        }
    }

    /// Adds an indexed column.
    pub fn column(mut self, name: impl Into<String>) -> Self {
        self.columns.push((name.into(), false));
        self
    }

    /// Adds a column that is stored but excluded from the full-text index
    /// (FTS5's `UNINDEXED` column option).
    pub fn unindexed_column(mut self, name: impl Into<String>) -> Self {
        self.columns.push((name.into(), true));
        self
    }

    /// Sets the tokenizer (`tokenize = '...'`). Defaults to FTS5's own
    /// default (`unicode61`) when unset.
    pub fn tokenizer(mut self, tokenizer: Fts5Tokenizer) -> Self {
        self.tokenizer = Some(tokenizer);
        self
    }

    /// Adds a prefix index for `n`-character prefixes (`prefix = 'n'`). Can
    /// be called more than once to index multiple prefix lengths.
    pub fn prefix(mut self, n: u32) -> Self {
        self.prefixes.push(n);
        self
    }

    /// Configures this table as a "contentless"/external-content table
    /// backed by `table`, keyed by `rowid_column` (`content = '...'`,
    /// `content_rowid = '...'`).
    pub fn external_content(
        mut self,
        table: impl Into<String>,
        rowid_column: impl Into<String>,
    ) -> Self {
        self.content_table = Some(table.into());
        self.content_rowid = Some(rowid_column.into());
        self
    }

    /// Renders the `CREATE VIRTUAL TABLE` statement without executing it.
    pub fn build_sql(&self) -> String {
        let mut options: Vec<String> = self
            .columns
            .iter()
            .map(|(name, unindexed)| {
                if *unindexed {
                    format!("{} UNINDEXED", quote_ident(name))
                } else {
                    quote_ident(name)
                }
            })
            .collect();

        if let Some(tokenizer) = self.tokenizer {
            options.push(format!("tokenize = {}", quote_literal(tokenizer.as_sql())));
        }
        for prefix in &self.prefixes {
            options.push(format!("prefix = {}", quote_literal(&prefix.to_string())));
        }
        if let Some(table) = &self.content_table {
            options.push(format!("content = {}", quote_literal(table)));
        }
        if let Some(rowid) = &self.content_rowid {
            options.push(format!("content_rowid = {}", quote_literal(rowid)));
        }

        format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS {} USING fts5({})",
            quote_ident(&self.name),
            options.join(", ")
        )
    }

    /// Executes the rendered `CREATE VIRTUAL TABLE` statement.
    pub fn create(&self, conn: &RawConnection) -> Result<()> {
        conn.execute_batch(&self.build_sql())?;
        Ok(())
    }
}
