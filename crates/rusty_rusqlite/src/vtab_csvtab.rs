//! `CsvTab` (issue #97) — exposes a CSV file as a read-only table, the
//! same spirit as real `rusqlite::vtab::csvtab`. An example/utility
//! module, not core `vtab` functionality (see `src/vtab.rs`'s doc
//! comment for that boundary).
//!
//! **Scope deviation from real `rusqlite`'s `csvtab`, stated plainly:**
//! this is a minimal, literal comma-split reader — no RFC4180 quoted-
//! field support (a field containing `,` or a newline can't be
//! represented), no custom delimiter, no `schema=` argument for
//! typed columns (every column is `TEXT`). Real `csvtab` handles all
//! of that via the `csv` crate. Adding a CSV-parsing dependency for a
//! low-priority example module (optional even in real `rusqlite`,
//! per issue #97) isn't worth it; if a genuine need for quoted fields
//! shows up, that's the point to reconsider.

use crate::dml_select::Expr;
use crate::error::{Error, Result};
use crate::value::Value;
use crate::vtab::{dequote, Context, CreateVTab, VTab, VTabCursor};

/// Reads `path`'s first line as comma-separated column names and every
/// subsequent line as a comma-separated row of `TEXT` values. A row
/// with fewer fields than there are columns reports `NULL` for the
/// missing trailing ones; a row with more fields than there are
/// columns silently ignores the extras — same "ragged CSV" tolerance
/// real `csvtab` has.
pub struct CsvTab {
    columns: Vec<String>,
    rows: Vec<Vec<String>>,
}

impl VTab for CsvTab {
    type Cursor = CsvCursor;

    fn column_names(&self) -> Vec<String> {
        self.columns.clone()
    }

    fn open(&self) -> Result<CsvCursor> {
        Ok(CsvCursor {
            rows: self.rows.clone(),
            pos: 0,
        })
    }
}

impl CreateVTab for CsvTab {
    /// `args`: exactly one, the file path (optionally quoted, same as
    /// any other module argument — see [`dequote`]).
    fn connect(args: &[String]) -> Result<Self> {
        let [path] = args else {
            return Err(Error::UnrecognizedStatement(
                "csv needs exactly 1 arg: a file path".to_string(),
            ));
        };
        let path = dequote(path.trim());
        let contents = std::fs::read_to_string(&path).map_err(|e| Error::Io(e.to_string()))?;

        let mut lines = contents.lines();
        let header = lines.next().ok_or_else(|| {
            Error::UnrecognizedStatement(format!("{path}: empty file, expected a header row"))
        })?;
        let columns: Vec<String> = header.split(',').map(|s| s.trim().to_string()).collect();
        let rows: Vec<Vec<String>> = lines
            .map(|line| line.split(',').map(|s| s.trim().to_string()).collect())
            .collect();
        Ok(CsvTab { columns, rows })
    }
}

pub struct CsvCursor {
    rows: Vec<Vec<String>>,
    pos: usize,
}

impl VTabCursor for CsvCursor {
    fn filter(&mut self, _filter: Option<&Expr>) -> Result<()> {
        Ok(())
    }

    fn next(&mut self) -> Result<()> {
        self.pos += 1;
        Ok(())
    }

    fn eof(&self) -> bool {
        self.pos >= self.rows.len()
    }

    fn column(&self, ctx: &mut Context, i: usize) -> Result<()> {
        match self.rows[self.pos].get(i) {
            Some(field) => ctx.set_result(&Value::Text(field.clone())),
            None => ctx.set_result(&Value::Null),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::Connection;
    use std::io::Write;

    struct TempCsv {
        path: std::path::PathBuf,
    }

    impl TempCsv {
        fn write(name: &str, contents: &str) -> TempCsv {
            let mut path = std::env::temp_dir();
            path.push(format!("rusty_rusqlite_csvtab_test_{name}.csv"));
            let mut file = std::fs::File::create(&path).unwrap();
            file.write_all(contents.as_bytes()).unwrap();
            TempCsv { path }
        }
    }

    impl Drop for TempCsv {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    #[test]
    fn csvtab_reads_header_and_rows() {
        let csv = TempCsv::write("basic", "name,age\nalice,30\nbob,25\n");
        let mut conn = Connection::open_in_memory().unwrap();
        conn.register_module::<CsvTab>("csv").unwrap();
        conn.execute(&format!(
            "CREATE VIRTUAL TABLE people USING csv('{}')",
            csv.path.display()
        ))
        .unwrap();

        let names: Vec<String> = conn
            .query_map("SELECT name FROM people", |row| row.get(0))
            .unwrap();
        assert_eq!(names, vec!["alice", "bob"]);

        let ages: Vec<String> = conn
            .query_map("SELECT age FROM people", |row| row.get(0))
            .unwrap();
        assert_eq!(ages, vec!["30", "25"]);
    }

    #[test]
    fn csvtab_reports_null_for_a_short_row() {
        let csv = TempCsv::write("ragged", "a,b,c\n1,2\n");
        let mut conn = Connection::open_in_memory().unwrap();
        conn.register_module::<CsvTab>("csv").unwrap();
        conn.execute(&format!(
            "CREATE VIRTUAL TABLE t USING csv('{}')",
            csv.path.display()
        ))
        .unwrap();

        let rows: Vec<Option<String>> =
            conn.query_map("SELECT c FROM t", |row| row.get(0)).unwrap();
        assert_eq!(rows, vec![None]);
    }

    #[test]
    fn csvtab_missing_file_is_an_io_error() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.register_module::<CsvTab>("csv").unwrap();
        assert!(matches!(
            conn.execute("CREATE VIRTUAL TABLE t USING csv('/no/such/file.csv')"),
            Err(Error::Io(_))
        ));
    }

    #[test]
    fn csvtab_empty_file_is_an_error() {
        let csv = TempCsv::write("empty", "");
        let mut conn = Connection::open_in_memory().unwrap();
        conn.register_module::<CsvTab>("csv").unwrap();
        assert!(conn
            .execute(&format!(
                "CREATE VIRTUAL TABLE t USING csv('{}')",
                csv.path.display()
            ))
            .is_err());
    }
}
