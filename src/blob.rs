//! Incremental `BLOB` I/O (Part B gap row "Connection + blob module:
//! blob_open, incremental BLOB I/O"), opened via [`crate::Connection::blob_open`].
//!
//! **Design deviation, stated plainly:** real SQLite (and `rusqlite::Blob`)
//! addresses a blob by `rowid`. This crate's storage has no rowid concept
//! yet (see issue #14's `last_insert_rowid` discussion), so [`Blob`] is
//! instead addressed by `row_index` — the row's plain position within
//! `Table::rows` at the time [`crate::Connection::blob_open`] was called.
//! That position is stable only as long as no earlier row is removed —
//! weaker than a real rowid's stability — worth revisiting once this
//! crate has a real rowid concept. `Blob` also doesn't implement
//! `std::io::{Read, Write, Seek}` like `rusqlite::Blob` does; its
//! `read_at`/`write_at` cover the same random-access use case directly,
//! without pulling in `std::io::Error` conversion machinery this crate's
//! simpler [`crate::Error`] type doesn't map onto.

use crate::connection::Connection;
use crate::error::{Error, Result};
use crate::fromsql::FromSqlError;
use crate::value::{Type, Value};

/// A handle for incremental reads (and, unless opened read-only, writes)
/// into a single `BLOB` column value.
pub struct Blob<'conn> {
    conn: &'conn mut Connection,
    table: String,
    row_index: usize,
    column_index: usize,
    read_only: bool,
}

impl<'conn> Blob<'conn> {
    pub(crate) fn open(
        conn: &'conn mut Connection,
        table: &str,
        column: &str,
        row_index: usize,
        read_only: bool,
    ) -> Result<Blob<'conn>> {
        let column_index = {
            let t = conn.db().table(table)?;
            let column_index = t
                .column_names
                .iter()
                .position(|c| c == column)
                .ok_or_else(|| Error::UnknownColumn(column.to_string()))?;
            let row = t.rows.get(row_index).ok_or(Error::IndexOutOfBounds {
                index: row_index,
                len: t.rows.len(),
            })?;
            match &row[column_index] {
                Value::Blob(_) => {}
                other => {
                    return Err(Error::FromSql(FromSqlError::InvalidType {
                        expected: Type::Blob,
                        actual: other.value_type(),
                    }))
                }
            }
            column_index
        };

        Ok(Blob {
            conn,
            table: table.to_string(),
            row_index,
            column_index,
            read_only,
        })
    }

    fn bytes(&self) -> &[u8] {
        match self
            .conn
            .db()
            .table(&self.table)
            .expect("table validated at Blob::open")
            .rows
            .get(self.row_index)
            .expect("row validated at Blob::open")
            .get(self.column_index)
            .expect("column validated at Blob::open")
        {
            Value::Blob(b) => b,
            _ => unreachable!("validated as Value::Blob at Blob::open"),
        }
    }

    /// Returns the blob's current length in bytes.
    pub fn len(&self) -> usize {
        self.bytes().len()
    }

    /// Returns whether the blob is empty.
    pub fn is_empty(&self) -> bool {
        self.bytes().is_empty()
    }

    /// Returns whether this handle was opened read-only.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Reads the blob's full current content.
    pub fn read_all(&self) -> Vec<u8> {
        self.bytes().to_vec()
    }

    /// Reads `buf.len()` bytes starting at `offset` into `buf`. Errors if
    /// `[offset, offset + buf.len())` doesn't fit within the blob's
    /// current length.
    pub fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        let bytes = self.bytes();
        let len = bytes.len();
        let end = match offset.checked_add(buf.len()) {
            Some(end) if end <= len => end,
            _ => return Err(Error::IndexOutOfBounds { index: offset, len }),
        };
        buf.copy_from_slice(&bytes[offset..end]);
        Ok(())
    }

    /// Overwrites `data.len()` bytes starting at `offset`. Like real
    /// SQLite's `sqlite3_blob_write` (and `rusqlite::Blob`'s `Write` impl),
    /// this **cannot resize** the blob: `[offset, offset + data.len())`
    /// must already fit within the blob's current length. Errors with
    /// [`Error::ReadOnlyBlob`] if this handle was opened read-only.
    pub fn write_at(&mut self, offset: usize, data: &[u8]) -> Result<()> {
        if self.read_only {
            return Err(Error::ReadOnlyBlob);
        }
        let len = self.bytes().len();
        let end = match offset.checked_add(data.len()) {
            Some(end) if end <= len => end,
            _ => return Err(Error::IndexOutOfBounds { index: offset, len }),
        };
        let cell = self
            .conn
            .db_mut()
            .cell_mut(&self.table, self.row_index, self.column_index)
            .expect("table/row/column validated at Blob::open");
        match cell {
            Value::Blob(b) => b[offset..end].copy_from_slice(data),
            _ => unreachable!("validated as Value::Blob at Blob::open"),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (id INTEGER, data BLOB)")
            .unwrap();
        conn
    }

    #[test]
    fn opens_existing_blob_and_reads_full_content() {
        let mut conn = setup();
        conn.db_mut()
            .insert_row("t", vec![Value::Integer(1), Value::Blob(vec![1, 2, 3, 4])])
            .unwrap();

        let blob = conn.blob_open("t", "data", 0, true).unwrap();
        assert_eq!(blob.len(), 4);
        assert!(!blob.is_empty());
        assert_eq!(blob.read_all(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn read_at_reads_a_byte_range() {
        let mut conn = setup();
        conn.db_mut()
            .insert_row(
                "t",
                vec![Value::Integer(1), Value::Blob(vec![10, 20, 30, 40])],
            )
            .unwrap();

        let blob = conn.blob_open("t", "data", 0, true).unwrap();
        let mut buf = [0u8; 2];
        blob.read_at(1, &mut buf).unwrap();
        assert_eq!(buf, [20, 30]);
    }

    #[test]
    fn read_at_past_the_end_is_an_error() {
        let mut conn = setup();
        conn.db_mut()
            .insert_row("t", vec![Value::Integer(1), Value::Blob(vec![1, 2, 3])])
            .unwrap();

        let blob = conn.blob_open("t", "data", 0, true).unwrap();
        let mut buf = [0u8; 2];
        assert_eq!(
            blob.read_at(2, &mut buf),
            Err(Error::IndexOutOfBounds { index: 2, len: 3 })
        );
    }

    #[test]
    fn write_at_overwrites_in_place() {
        let mut conn = setup();
        conn.db_mut()
            .insert_row("t", vec![Value::Integer(1), Value::Blob(vec![1, 2, 3, 4])])
            .unwrap();

        let mut blob = conn.blob_open("t", "data", 0, false).unwrap();
        blob.write_at(1, &[99, 98]).unwrap();
        assert_eq!(blob.read_all(), vec![1, 99, 98, 4]);
    }

    #[test]
    fn write_at_cannot_resize_the_blob() {
        let mut conn = setup();
        conn.db_mut()
            .insert_row("t", vec![Value::Integer(1), Value::Blob(vec![1, 2, 3])])
            .unwrap();

        let mut blob = conn.blob_open("t", "data", 0, false).unwrap();
        assert_eq!(
            blob.write_at(2, &[9, 9]),
            Err(Error::IndexOutOfBounds { index: 2, len: 3 })
        );
    }

    #[test]
    fn write_at_on_read_only_handle_is_an_error() {
        let mut conn = setup();
        conn.db_mut()
            .insert_row("t", vec![Value::Integer(1), Value::Blob(vec![1, 2, 3])])
            .unwrap();

        let mut blob = conn.blob_open("t", "data", 0, true).unwrap();
        assert_eq!(blob.write_at(0, &[9]), Err(Error::ReadOnlyBlob));
    }

    #[test]
    fn open_on_non_blob_column_is_an_error() {
        let mut conn = setup();
        conn.db_mut()
            .insert_row("t", vec![Value::Integer(1), Value::Null])
            .unwrap();

        assert!(matches!(
            conn.blob_open("t", "data", 0, true),
            Err(Error::FromSql(FromSqlError::InvalidType { .. }))
        ));
    }

    #[test]
    fn open_on_missing_column_is_an_error() {
        let mut conn = setup();
        conn.db_mut()
            .insert_row("t", vec![Value::Integer(1), Value::Null])
            .unwrap();

        assert!(matches!(
            conn.blob_open("t", "missing", 0, true),
            Err(Error::UnknownColumn(_))
        ));
    }

    #[test]
    fn open_on_out_of_range_row_is_an_error() {
        let mut conn = setup();
        assert!(matches!(
            conn.blob_open("t", "data", 0, true),
            Err(Error::IndexOutOfBounds { index: 0, len: 0 })
        ));
    }
}
