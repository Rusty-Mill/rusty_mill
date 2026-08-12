#![cfg(feature = "pool")]

use rusty_sqlite::build_pool;

#[test]
fn pooled_connections_share_the_same_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pool.db");
    let pool = build_pool(&path, 4).unwrap();

    {
        let conn = pool.get().unwrap();
        conn.execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT);")
            .unwrap();
        conn.execute("INSERT INTO t (v) VALUES ('from first checkout')", [])
            .unwrap();
    }

    let conn = pool.get().unwrap();
    let value: String = conn
        .query_row("SELECT v FROM t WHERE id = 1", [], |row| row.get(0))
        .unwrap();
    assert_eq!(value, "from first checkout");
}
