---
category: Fixed
changelog: rusty_tokio's JoinError is Sync, as tokio's is.
---
## 2026-11-03 - rusty_tokio JoinError is Sync

- **Fixed:** `JoinError` was `Send` but not `Sync`, because it held the panic payload
  (`Box<dyn Any + Send>`) directly, so it could not go into `Sync` error types such as
  `anyhow::Error` or `Box<dyn Error + Send + Sync>` (card #404). The payload now sits in a
  `Mutex` that is never locked through `&self`; `into_panic`/`try_into_panic` move it out.
  Additive: no signature changes.
