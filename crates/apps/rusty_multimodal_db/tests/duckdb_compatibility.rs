//! Exercise the embedded DuckDB API used by the external database benchmark.
#![cfg(feature = "external-db-bench")]

#[test]
fn bundled_duckdb_supports_prepared_queries_and_transactions() -> duckdb::Result<()> {
    let mut connection = duckdb::Connection::open_in_memory()?;
    connection.execute_batch("CREATE TABLE dogs (id INTEGER PRIMARY KEY, age INTEGER)")?;
    let transaction = connection.transaction()?;
    transaction.execute("INSERT INTO dogs VALUES (?, ?)", [1, 7])?;
    transaction.execute("INSERT INTO dogs VALUES (?, ?)", [2, 3])?;
    transaction.commit()?;

    let mut lookup = connection.prepare("SELECT age FROM dogs WHERE id = ?")?;
    assert_eq!(lookup.query_row([1], |row| row.get::<_, i32>(0))?, 7);
    assert!(matches!(
        lookup.query_row([99], |row| row.get::<_, i32>(0)),
        Err(duckdb::Error::QueryReturnedNoRows)
    ));
    let total: i64 =
        connection.query_row("SELECT SUM(age)::BIGINT FROM dogs", [], |row| row.get(0))?;
    assert_eq!(total, 10);
    Ok(())
}
