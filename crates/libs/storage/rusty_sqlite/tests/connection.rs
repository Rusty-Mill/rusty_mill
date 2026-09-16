use std::time::Duration;

use rusty_sqlite::{Connection, OpenOptions};

#[test]
fn open_in_memory_applies_default_pragmas() {
    let conn = Connection::open_in_memory().unwrap();
    let foreign_keys: i64 = conn
        .as_raw()
        .pragma_query_value(None, "foreign_keys", |row| row.get(0))
        .unwrap();
    assert_eq!(foreign_keys, 1);
}

#[test]
fn open_on_disk_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");

    {
        let conn = Connection::open(&path).unwrap();
        conn.as_raw()
            .execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT);")
            .unwrap();
        conn.as_raw()
            .execute("INSERT INTO t (v) VALUES ('hello')", [])
            .unwrap();
    }

    let conn = Connection::open(&path).unwrap();
    let value: String = conn
        .as_raw()
        .query_row("SELECT v FROM t WHERE id = 1", [], |row| row.get(0))
        .unwrap();
    assert_eq!(value, "hello");
}

#[test]
fn custom_open_options_can_disable_foreign_keys() {
    let conn = Connection::open_with(
        ":memory:",
        OpenOptions {
            wal: false,
            foreign_keys: false,
            busy_timeout: Duration::from_millis(100),
        },
    )
    .unwrap();
    let foreign_keys: i64 = conn
        .as_raw()
        .pragma_query_value(None, "foreign_keys", |row| row.get(0))
        .unwrap();
    assert_eq!(foreign_keys, 0);
}
