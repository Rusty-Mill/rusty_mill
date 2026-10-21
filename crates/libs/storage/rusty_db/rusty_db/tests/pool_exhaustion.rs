#![cfg(all(feature = "sqlite", feature = "derive"))]

//! Exercises `PoolConfig`: constraining a pool's `max_connections` (and
//! `acquire_timeout`) so exhaustion can be triggered deterministically,
//! rather than needing to guess how many concurrent operations it'd take
//! to exhaust a default-sized pool.

use std::time::Duration;

use rusty_db::prelude::*;
use tokio::task::{spawn_local, LocalSet};
use tokio::time::timeout;

#[derive(Debug, Clone, PartialEq, Mapped)]
#[table(name = "users")]
struct User {
    #[table(primary_key)]
    id: i64,
    name: String,
}

/// A file-backed database (not `:memory:`, whose pool is forced to a
/// single connection regardless of `max_connections`) with an explicit,
/// small `PoolConfig`.
async fn file_engine_with(name: &str, config: PoolConfig) -> rusty_db::Result<Engine> {
    let path = std::env::temp_dir().join(format!(
        "rusty_db_pool_exhaustion_{name}_{}.sqlite3",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let engine = SqliteDriver::engine_with(&url, config).await?;
    engine
        .connect()
        .await?
        .execute(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
            &[],
        )
        .await?;
    Ok(engine)
}

#[tokio::test]
async fn acquiring_beyond_max_connections_blocks_then_succeeds_after_release(
) -> rusty_db::Result<()> {
    let engine = file_engine_with("blocks_then_succeeds", PoolConfig::new(1)).await?;

    // The pool's only connection.
    let held = engine.connect().await?;

    // With it checked out, a second acquire attempt must not succeed — it
    // should just wait. Prove that by timing it out quickly rather than
    // assuming any particular scheduling order.
    let quick_attempt = timeout(Duration::from_millis(200), engine.connect()).await;
    assert!(
        quick_attempt.is_err(),
        "expected connect() to still be waiting for the held connection"
    );

    drop(held); // release it back to the pool

    // Now that the connection is free again, the same kind of acquire
    // should succeed well within a generous timeout.
    let after_release = timeout(Duration::from_secs(5), engine.connect()).await;
    assert!(
        after_release.is_ok(),
        "connect() should have succeeded once the held connection was released"
    );
    after_release.unwrap()?;

    Ok(())
}

#[tokio::test]
async fn pool_respects_a_higher_max_connections_before_blocking() -> rusty_db::Result<()> {
    let engine = file_engine_with("respects_max_two", PoolConfig::new(2)).await?;

    let first = engine.connect().await?;
    let second = timeout(Duration::from_secs(2), engine.connect())
        .await
        .expect("acquiring the 2nd of 2 allowed connections should not block");
    let second = second?;

    // A 3rd concurrent connection now exceeds max_connections=2.
    let third_attempt = timeout(Duration::from_millis(200), engine.connect()).await;
    assert!(
        third_attempt.is_err(),
        "expected the pool to be exhausted at max_connections=2"
    );

    drop(first);
    let third_after_release = timeout(Duration::from_secs(5), engine.connect()).await;
    assert!(third_after_release.is_ok());
    third_after_release.unwrap()?;

    drop(second);
    Ok(())
}

#[tokio::test]
async fn acquire_timeout_errors_instead_of_hanging_forever() -> rusty_db::Result<()> {
    // The acquire timeout also bounds opening the pool's first connection, so it is not set so low
    // that a loaded runner's file, WAL and schema setup can use it up before the test starts (that
    // failed at ~0.2 s on Windows CI as "pool timed out" before the assertion was reached). This test
    // needs no file and no table: one in-memory connection, held, is enough to exhaust the pool.
    const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(1);
    let config = PoolConfig::new(1).with_acquire_timeout(ACQUIRE_TIMEOUT);
    let engine = SqliteDriver::engine_with("sqlite::memory:", config)
        .await
        .unwrap_or_else(|e| panic!("stage 1, creating the pool: {e}"));

    let held = engine.connect().await.unwrap_or_else(|e| {
        panic!("stage 2, the first checkout (the pool's only connection): {e}")
    });

    // No external timeout wrapper on the call under test: this relies entirely on the pool's own
    // configured acquire_timeout to give up. The outer timeout is only a safety net, so a broken
    // acquire_timeout fails this test instead of hanging the whole suite.
    let start = std::time::Instant::now();
    let outcome = timeout(Duration::from_secs(10), engine.connect()).await;
    let elapsed = start.elapsed();

    let result = outcome.expect("engine.connect() hung well past its configured acquire_timeout");
    let error = match result {
        Ok(_) => panic!(
            "stage 3, the exhausted acquire: expected the pool's own timeout, got a connection"
        ),
        Err(error) => error.to_string(),
    };
    assert!(
        error.contains("timed out"),
        "stage 3, the exhausted acquire: expected the pool's timeout error, got: {error}"
    );
    assert!(
        elapsed >= ACQUIRE_TIMEOUT / 2 && elapsed < ACQUIRE_TIMEOUT * 5,
        "acquire_timeout should fire after about {ACQUIRE_TIMEOUT:?}, took {elapsed:?}"
    );

    drop(held);
    Ok(())
}

#[tokio::test]
async fn sessions_serialize_correctly_when_the_pool_is_exhausted() -> rusty_db::Result<()> {
    const COUNT: i64 = 5;
    let engine = file_engine_with("sessions_serialize", PoolConfig::new(1)).await?;
    let local = LocalSet::new();

    local
        .run_until(async {
            let mut handles = Vec::with_capacity(COUNT as usize);
            for id in 0..COUNT {
                let engine = engine.clone();
                handles.push(spawn_local(async move {
                    let mut session = engine.session();
                    session.add(&User {
                        id,
                        name: format!("user-{id}"),
                    });
                    session.commit().await
                }));
            }
            for handle in handles {
                handle.await.unwrap()?;
            }
            rusty_db::Result::Ok(())
        })
        .await?;

    // Every session eventually got its turn on the single connection, none
    // were lost or corrupted by contending for it.
    let rows: Vec<User> = engine.fetch_all_as(&Select::from(&User::table())).await?;
    assert_eq!(rows.len(), COUNT as usize);

    Ok(())
}
