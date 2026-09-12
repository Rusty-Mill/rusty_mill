#![cfg(feature = "mysql")]

//! Regression test for a `BIGINT UNSIGNED` column value above `i64::MAX`:
//! `row_from_mysql` used to decode it via a plain `as i64` cast, silently
//! wrapping it to a negative number instead of surfacing an error. It must
//! now reject the value with a decode error instead of returning a
//! silently-wrong `Value::I64`.

use rusty_db::prelude::*;

/// Connects to a real MySQL/MariaDB server for this test. There's no way to
/// spin one up portably in every environment this test suite runs in, so
/// this is opt-in: point `MYSQL_TEST_URL` at a scratch database (its schema
/// is created and dropped by this test) or the test skips itself instead of
/// failing when no server is reachable.
async fn test_engine() -> Option<Engine> {
    let url = std::env::var("MYSQL_TEST_URL")
        .unwrap_or_else(|_| "mysql://rusty:rusty@127.0.0.1/rusty_db_test".to_string());
    match MySqlDriver::engine(&url).await {
        Ok(engine) => Some(engine),
        Err(err) => {
            eprintln!("skipping MySQL test: {err}");
            None
        }
    }
}

#[tokio::test]
async fn bigint_unsigned_value_above_i64_max_errors_instead_of_wrapping_negative(
) -> rusty_db::Result<()> {
    let Some(engine) = test_engine().await else {
        return Ok(());
    };
    engine
        .connect()
        .await?
        .execute("DROP TABLE IF EXISTS bigint_unsigned_overflow_mysql_t", &[])
        .await?;
    engine
        .connect()
        .await?
        .execute(
            "CREATE TABLE bigint_unsigned_overflow_mysql_t (\
                 id BIGINT PRIMARY KEY, big_value BIGINT UNSIGNED NOT NULL\
             )",
            &[],
        )
        .await?;

    // u64::MAX (18446744073709551615) is above i64::MAX (9223372036854775807):
    // decoding it via `as i64` used to silently wrap to -1 instead of erroring.
    engine
        .connect()
        .await?
        .execute_unprepared(
            "INSERT INTO bigint_unsigned_overflow_mysql_t (id, big_value) \
             VALUES (1, 18446744073709551615)",
        )
        .await?;

    let result = engine
        .connect()
        .await?
        .fetch_one(
            "SELECT big_value FROM bigint_unsigned_overflow_mysql_t WHERE id = 1",
            &[],
        )
        .await;
    assert!(
        result.is_err(),
        "expected a decode error for a BIGINT UNSIGNED value above i64::MAX, got {result:?}"
    );

    engine
        .connect()
        .await?
        .execute("DROP TABLE bigint_unsigned_overflow_mysql_t", &[])
        .await?;
    Ok(())
}
