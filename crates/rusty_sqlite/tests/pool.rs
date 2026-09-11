#![cfg(feature = "pool")]

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use rusty_sqlite::{build_pool, build_pool_with_timeout};

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

#[test]
fn a_released_connection_is_reused_rather_than_reopened() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pool.db");
    // max_size 1: if the pool ever tried to open a second connection
    // instead of reusing the released one, this would deadlock/time out.
    let pool = build_pool_with_timeout(&path, 1, Duration::from_millis(500)).unwrap();

    for _ in 0..5 {
        let conn = pool.get().expect("released connection should be reusable");
        conn.execute_batch("CREATE TABLE IF NOT EXISTS t (id INTEGER PRIMARY KEY);")
            .unwrap();
        conn.execute("INSERT INTO t DEFAULT VALUES", []).unwrap();
        drop(conn);
    }
}

#[test]
fn exhausted_pool_blocks_until_a_connection_is_released() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pool.db");
    let pool = build_pool_with_timeout(&path, 1, Duration::from_secs(5)).unwrap();

    let held = pool.get().unwrap();

    let (ready_tx, ready_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    let pool_clone = pool.clone();
    let waiter = thread::spawn(move || {
        ready_tx.send(()).unwrap();
        let got = pool_clone.get();
        result_tx.send(got.is_ok()).unwrap();
    });

    ready_rx.recv().unwrap();
    // Give the waiter a moment to actually reach pool.get() and block.
    thread::sleep(Duration::from_millis(100));
    assert!(
        result_rx.try_recv().is_err(),
        "waiter should still be blocked while the only connection is held"
    );

    drop(held);
    assert!(
        result_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        "waiter should acquire the connection once it's released"
    );
    waiter.join().unwrap();
}

#[test]
fn acquire_times_out_when_the_pool_stays_exhausted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pool.db");
    let pool = build_pool_with_timeout(&path, 1, Duration::from_millis(100)).unwrap();

    let _held = pool.get().unwrap();
    let err = pool.get().unwrap_err();
    assert!(matches!(err, rusty_sqlite::Error::PoolTimeout));
}

#[test]
fn a_dropped_lease_with_an_open_transaction_is_rolled_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pool.db");
    let pool = build_pool_with_timeout(&path, 1, Duration::from_secs(5)).unwrap();

    {
        let conn = pool.get().unwrap();
        conn.execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT);")
            .unwrap();
    }

    {
        let conn = pool.get().unwrap();
        conn.execute_batch("BEGIN;").unwrap();
        conn.execute("INSERT INTO t (v) VALUES ('uncommitted')", [])
            .unwrap();
        // Dropped here without COMMIT -- the pool must roll this back
        // before the connection is handed to the next borrower.
    }

    let conn = pool.get().unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM t", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        count, 0,
        "uncommitted write from a dropped lease must not be visible"
    );

    // The recycled connection must be usable, i.e. not left mid-transaction.
    conn.execute_batch("BEGIN;").unwrap();
    conn.execute("INSERT INTO t (v) VALUES ('committed')", [])
        .unwrap();
    conn.execute_batch("COMMIT;").unwrap();

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM t", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn zero_max_size_is_rejected_immediately() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pool.db");
    match build_pool_with_timeout(&path, 0, Duration::from_secs(30)) {
        Ok(_) => panic!("expected InvalidPoolSize error, got Ok"),
        Err(rusty_sqlite::Error::InvalidPoolSize) => {}
        Err(other) => panic!("expected InvalidPoolSize, got {other:?}"),
    }
}
