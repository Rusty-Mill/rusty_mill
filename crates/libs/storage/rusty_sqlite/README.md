# rusty_sqlite

A thin, ergonomic wrapper over [`rusqlite`](https://docs.rs/rusqlite) for embedding SQLite as an application's persistence layer.

This crate does not try to replace `rusqlite` — it re-exports it — and instead fills three gaps that come up in every consumer that embeds SQLite directly:

- **Cross-platform connections by construction.** `rusqlite`'s `bundled` feature is on unconditionally, so there's no system SQLite dependency. [`Connection::open`]/[`Connection::open_in_memory`] also apply sane default pragmas: WAL journaling, foreign key enforcement, and a busy timeout.
- **Typed FTS5 schema building.** `rusqlite` only ever exposes FTS5 as hand-written `CREATE VIRTUAL TABLE ... USING fts5(...)` SQL. [`Fts5TableBuilder`] gives the common options — columns, `UNINDEXED` columns, tokenizer, prefix indexes, external content tables — a typed, composable API.
- **Migration lifecycle management.** [`Migrations`] tracks schema version via `PRAGMA user_version` and applies pending steps in registration order, each in its own transaction, so it's safe to call on every application startup.

Enable the `pool` feature for a small built-in, `std`-only connection pool for multi-threaded applications.

## Example

```rust
use rusty_sqlite::{Connection, Fts5TableBuilder, Fts5Tokenizer, Migrations};

let mut conn = Connection::open_in_memory()?;

let migrations = Migrations::new()
    .add(1, "create notes", "CREATE TABLE notes (id INTEGER PRIMARY KEY, title TEXT, body TEXT);");
conn.migrate(&migrations)?;

Fts5TableBuilder::new("notes_fts")
    .column("title")
    .column("body")
    .tokenizer(Fts5Tokenizer::Porter)
    .external_content("notes", "id")
    .create(conn.as_raw())?;
# Ok::<(), rusty_sqlite::Error>(())
```

## Status

Early: the API surface covers the connection lifecycle, FTS5 schema building, and versioned migrations. `sqlite-vec` (`vec0`) virtual-table support is not yet implemented.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
