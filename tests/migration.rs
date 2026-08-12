use rusty_sqlite::{Connection, Migrations};

fn sample_migrations() -> Migrations {
    Migrations::new()
        .add(
            1,
            "create notes",
            "CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT NOT NULL);",
        )
        .add(
            2,
            "add created_at",
            "ALTER TABLE notes ADD COLUMN created_at TEXT;",
        )
}

#[test]
fn applies_pending_migrations_in_order() {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.migrate(&sample_migrations()).unwrap();

    conn.as_raw()
        .execute(
            "INSERT INTO notes (body, created_at) VALUES ('hi', '2026-01-01')",
            [],
        )
        .unwrap();

    let version: i64 = conn
        .as_raw()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 2);
}

#[test]
fn rerunning_migrations_is_a_no_op() {
    let mut conn = Connection::open_in_memory().unwrap();
    let migrations = sample_migrations();
    conn.migrate(&migrations).unwrap();
    // Running again must not try to re-apply "CREATE TABLE notes" and fail.
    conn.migrate(&migrations).unwrap();
}

#[test]
fn only_applies_newly_added_steps() {
    let mut conn = Connection::open_in_memory().unwrap();
    let first = Migrations::new().add(
        1,
        "create notes",
        "CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT NOT NULL);",
    );
    conn.migrate(&first).unwrap();

    conn.migrate(&sample_migrations()).unwrap();

    let version: i64 = conn
        .as_raw()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 2);
}

#[test]
fn out_of_order_migrations_are_rejected() {
    let mut conn = Connection::open_in_memory().unwrap();
    let migrations = Migrations::new()
        .add(2, "second", "SELECT 1;")
        .add(1, "first", "SELECT 1;");

    let err = conn.migrate(&migrations).unwrap_err();
    assert!(matches!(
        err,
        rusty_sqlite::Error::OutOfOrderMigration {
            version: 1,
            previous: 2
        }
    ));
}

#[test]
fn schema_newer_than_known_migrations_is_rejected() {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.as_raw()
        .pragma_update(None, "user_version", 5)
        .unwrap();

    let err = conn.migrate(&sample_migrations()).unwrap_err();
    assert!(matches!(
        err,
        rusty_sqlite::Error::SchemaTooNew {
            found: 5,
            latest: 2,
            ..
        }
    ));
}

#[test]
fn failed_step_rolls_back_and_does_not_advance_version() {
    let mut conn = Connection::open_in_memory().unwrap();
    let migrations = Migrations::new().add(1, "broken", "THIS IS NOT VALID SQL;");

    let err = conn.migrate(&migrations).unwrap_err();
    assert!(matches!(
        err,
        rusty_sqlite::Error::Migration { version: 1, .. }
    ));

    let version: i64 = conn
        .as_raw()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 0);
}
