use rusty_sqlite::Connection;
use sqlx::{Connection as _, Executor};

/// Both drivers must link the fixed bundled engine and share WAL data.
#[tokio::test]
async fn sqlx_and_rusqlite_share_patched_bundled_sqlite() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shared.db");
    let rusqlite = Connection::open(&path).unwrap();
    assert!(rusty_sqlite::rusqlite::version_number() >= 3_054_000);
    rusqlite
        .as_raw()
        .execute_batch(
            "CREATE TABLE shared (value TEXT NOT NULL); INSERT INTO shared VALUES ('rusqlite');",
        )
        .unwrap();

    let options = sqlx::sqlite::SqliteConnectOptions::new().filename(&path);
    let mut sqlx = sqlx::SqliteConnection::connect_with(&options)
        .await
        .unwrap();
    let sqlx_version: String = sqlx::query_scalar("SELECT sqlite_version()")
        .fetch_one(&mut sqlx)
        .await
        .unwrap();
    assert_eq!(sqlx_version, rusty_sqlite::rusqlite::version());
    let value: String = sqlx::query_scalar("SELECT value FROM shared")
        .fetch_one(&mut sqlx)
        .await
        .unwrap();
    assert_eq!(value, "rusqlite");
    sqlx.execute("INSERT INTO shared VALUES ('sqlx')")
        .await
        .unwrap();
    assert!(sqlx
        .execute("INSERT INTO shared VALUES (NULL)")
        .await
        .is_err());
    let count: i64 = rusqlite
        .as_raw()
        .query_row("SELECT COUNT(*) FROM shared", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 2);
    sqlx.close().await.unwrap();
}
