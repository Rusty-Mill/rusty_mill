//! `Connection::serialize`/`deserialize` (Part B gap row "Connection +
//! serialize module: serialize/deserialize"). A hand-rolled binary
//! encoding of this crate's own in-memory table state.
//!
//! **Not byte-compatible with real SQLite's file format** — this crate
//! has no page/B-tree file format (see `ARCHITECTURE.md`'s non-goals), so
//! `rusqlite::Connection::serialize`'s output can't be deserialized here
//! and vice versa. This format exists purely to round-trip *this crate's*
//! `Database` state; adding a real dependency (a serde-based crate, for
//! instance) to do that would be a new-dependency decision needing
//! sign-off, so this is hand-rolled with only `std`.

use crate::ddl::ColumnDef;
use crate::error::{Error, Result};
use crate::storage::{Database, Table};
use crate::value::Value;

const MAGIC: &[u8; 4] = b"RQL1";

/// Serializes a [`Database`]'s full table state into this crate's own
/// binary format.
pub fn serialize(db: &Database) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    let tables = db.tables();
    write_u32(&mut out, tables.len() as u32);
    for (name, table) in tables {
        write_string(&mut out, name);
        write_table(&mut out, table);
    }
    out
}

/// Deserializes bytes produced by [`serialize`] back into a [`Database`].
pub fn deserialize(bytes: &[u8]) -> Result<Database> {
    let mut r = Reader { bytes, pos: 0 };
    r.expect_bytes(MAGIC)?;
    let table_count = r.read_u32()?;
    let mut db = Database::new();
    for _ in 0..table_count {
        let name = r.read_string()?;
        let table = r.read_table()?;
        db.insert_table_raw(name, table);
    }
    Ok(db)
}

fn write_table(out: &mut Vec<u8>, table: &Table) {
    write_u32(out, table.columns.len() as u32);
    for col in &table.columns {
        write_column_def(out, col);
    }
    write_u32(out, table.rows.len() as u32);
    for row in &table.rows {
        write_u32(out, row.len() as u32);
        for value in row {
            write_value(out, value);
        }
    }
}

fn write_column_def(out: &mut Vec<u8>, col: &ColumnDef) {
    write_string(out, &col.name);
    match &col.type_name {
        Some(t) => {
            out.push(1);
            write_string(out, t);
        }
        None => out.push(0),
    }
    out.push(col.not_null as u8);
    out.push(col.primary_key as u8);
}

fn write_value(out: &mut Vec<u8>, value: &Value) {
    match value {
        Value::Null => out.push(0),
        Value::Integer(i) => {
            out.push(1);
            out.extend_from_slice(&i.to_le_bytes());
        }
        Value::Real(f) => {
            out.push(2);
            out.extend_from_slice(&f.to_le_bytes());
        }
        Value::Text(s) => {
            out.push(3);
            write_string(out, s);
        }
        Value::Blob(b) => {
            out.push(4);
            write_u32(out, b.len() as u32);
            out.extend_from_slice(b);
        }
    }
}

fn write_string(out: &mut Vec<u8>, s: &str) {
    write_u32(out, s.len() as u32);
    out.extend_from_slice(s.as_bytes());
}

fn write_u32(out: &mut Vec<u8>, n: u32) {
    out.extend_from_slice(&n.to_le_bytes());
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or(Error::Deserialize)?;
        let slice = self.bytes.get(self.pos..end).ok_or(Error::Deserialize)?;
        self.pos = end;
        Ok(slice)
    }

    fn expect_bytes(&mut self, expected: &[u8]) -> Result<()> {
        if self.take(expected.len())? == expected {
            Ok(())
        } else {
            Err(Error::Deserialize)
        }
    }

    fn read_u32(&mut self) -> Result<u32> {
        let bytes: [u8; 4] = self.take(4)?.try_into().map_err(|_| Error::Deserialize)?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn read_i64(&mut self) -> Result<i64> {
        let bytes: [u8; 8] = self.take(8)?.try_into().map_err(|_| Error::Deserialize)?;
        Ok(i64::from_le_bytes(bytes))
    }

    fn read_f64(&mut self) -> Result<f64> {
        let bytes: [u8; 8] = self.take(8)?.try_into().map_err(|_| Error::Deserialize)?;
        Ok(f64::from_le_bytes(bytes))
    }

    fn read_bool(&mut self) -> Result<bool> {
        Ok(self.take(1)?[0] != 0)
    }

    fn read_string(&mut self) -> Result<String> {
        let len = self.read_u32()? as usize;
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| Error::Deserialize)
    }

    fn read_value(&mut self) -> Result<Value> {
        let tag = self.take(1)?[0];
        match tag {
            0 => Ok(Value::Null),
            1 => Ok(Value::Integer(self.read_i64()?)),
            2 => Ok(Value::Real(self.read_f64()?)),
            3 => Ok(Value::Text(self.read_string()?)),
            4 => {
                let len = self.read_u32()? as usize;
                Ok(Value::Blob(self.take(len)?.to_vec()))
            }
            _ => Err(Error::Deserialize),
        }
    }

    fn read_column_def(&mut self) -> Result<ColumnDef> {
        let name = self.read_string()?;
        let has_type = self.take(1)?[0] != 0;
        let type_name = if has_type {
            Some(self.read_string()?)
        } else {
            None
        };
        let not_null = self.read_bool()?;
        let primary_key = self.read_bool()?;
        Ok(ColumnDef {
            name,
            type_name,
            not_null,
            primary_key,
        })
    }

    fn read_table(&mut self) -> Result<Table> {
        let column_count = self.read_u32()?;
        let mut columns = Vec::with_capacity(column_count as usize);
        for _ in 0..column_count {
            columns.push(self.read_column_def()?);
        }
        let column_names = columns.iter().map(|c| c.name.clone()).collect();

        let row_count = self.read_u32()?;
        let mut rows = Vec::with_capacity(row_count as usize);
        for _ in 0..row_count {
            let value_count = self.read_u32()?;
            let mut row = Vec::with_capacity(value_count as usize);
            for _ in 0..value_count {
                row.push(self.read_value()?);
            }
            rows.push(row);
        }

        Ok(Table {
            column_names,
            columns,
            rows,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{execute_create_table, execute_insert};
    use crate::{parse_create_table, parse_insert, tokenize};

    fn sample_db() -> Database {
        let mut db = Database::new();
        let create = parse_create_table(
            &tokenize("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT NOT NULL)").unwrap(),
        )
        .unwrap();
        execute_create_table(&mut db, &create).unwrap();
        let insert =
            parse_insert(&tokenize("INSERT INTO t VALUES (1, 'a'), (2, 'b')").unwrap()).unwrap();
        execute_insert(&mut db, &insert).unwrap();
        db
    }

    #[test]
    fn round_trips_table_state() {
        let db = sample_db();
        let bytes = serialize(&db);
        let restored = deserialize(&bytes).unwrap();

        let original_table = db.table("t").unwrap();
        let restored_table = restored.table("t").unwrap();
        assert_eq!(original_table.column_names, restored_table.column_names);
        assert_eq!(original_table.rows, restored_table.rows);
        assert_eq!(original_table.columns, restored_table.columns);
    }

    #[test]
    fn round_trips_empty_database() {
        let db = Database::new();
        let bytes = serialize(&db);
        let restored = deserialize(&bytes).unwrap();
        assert!(restored.table("anything").is_err());
    }

    #[test]
    fn deserialize_rejects_bad_magic() {
        assert_eq!(deserialize(b"XXXX").unwrap_err(), Error::Deserialize);
    }

    #[test]
    fn deserialize_rejects_truncated_data() {
        let db = sample_db();
        let mut bytes = serialize(&db);
        bytes.truncate(bytes.len() - 3);
        assert_eq!(deserialize(&bytes).unwrap_err(), Error::Deserialize);
    }
}
