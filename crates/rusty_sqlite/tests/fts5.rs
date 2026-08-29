use rusty_sqlite::{Connection, Fts5TableBuilder, Fts5Tokenizer};

#[test]
fn builds_and_queries_a_basic_fts5_table() {
    let conn = Connection::open_in_memory().unwrap();
    Fts5TableBuilder::new("notes_fts")
        .column("title")
        .column("body")
        .tokenizer(Fts5Tokenizer::Porter)
        .prefix(2)
        .create(conn.as_raw())
        .unwrap();

    conn.as_raw()
        .execute(
            "INSERT INTO notes_fts (title, body) VALUES ('Hello World', 'a note about rust sqlite wrappers')",
            [],
        )
        .unwrap();

    let title: String = conn
        .as_raw()
        .query_row(
            "SELECT title FROM notes_fts WHERE notes_fts MATCH 'wrappers'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(title, "Hello World");
}

#[test]
fn unindexed_columns_are_stored_but_not_searchable() {
    let conn = Connection::open_in_memory().unwrap();
    Fts5TableBuilder::new("docs_fts")
        .column("body")
        .unindexed_column("doc_id")
        .create(conn.as_raw())
        .unwrap();

    conn.as_raw()
        .execute(
            "INSERT INTO docs_fts (body, doc_id) VALUES ('searchable text', 'not-searched')",
            [],
        )
        .unwrap();

    let count: i64 = conn
        .as_raw()
        .query_row(
            "SELECT count(*) FROM docs_fts WHERE docs_fts MATCH 'searched'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0, "UNINDEXED column contents must not be searchable");

    let count: i64 = conn
        .as_raw()
        .query_row(
            "SELECT count(*) FROM docs_fts WHERE docs_fts MATCH 'searchable'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn external_content_table_option_is_rendered() {
    let builder = Fts5TableBuilder::new("notes_fts")
        .column("body")
        .external_content("notes", "id");
    let sql = builder.build_sql();
    assert!(sql.contains("content = 'notes'"));
    assert!(sql.contains("content_rowid = 'id'"));
}

#[test]
fn identifiers_with_quotes_are_escaped() {
    let builder = Fts5TableBuilder::new("weird\"table").column("weird\"col");
    let sql = builder.build_sql();
    assert!(sql.contains("\"weird\"\"table\""));
    assert!(sql.contains("\"weird\"\"col\""));
}
