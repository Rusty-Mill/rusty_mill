---
category: Changed
changelog: rusty-db-core's BoxStream and StreamExt are now rusty_stream's; the sqlx drivers drop futures-util and async-stream.
---
## 2026-10-25 - rusty_db streams on rusty_stream

- **Changed:** `rusty_db_core::BoxStream` and `rusty_db_core::StreamExt` re-export `rusty_stream` instead of `futures-core`/`futures-util`. `Engine::fetch_stream`, `fetch_stream_as` and `Connection::fetch_stream` keep their signatures. Callers that use the re-exported `StreamExt` keep compiling; a caller that applies `futures::StreamExt` to the stream must switch to the re-export (Part of #909).
- **Changed:** `rusty-db-sqlite`, `rusty-db-postgres` and `rusty-db-mysql` build their row streams with `rusty_stream::try_stream!` and read sqlx's stream through `rusty_stream::Compat`.
- **Known limitations:** `with_timeout` and rusty-db's tests stay on tokio. sqlx (`runtime-tokio`) panics outside a tokio runtime, so both wait until sqlx is replaced (#706).
