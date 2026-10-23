---
category: Added
changelog: `docs/research/ASYNC-RUNTIME-INVENTORY.md` inventories `tokio`/`futures`/`futures-util`/`async-stream` use and lists the gaps to `rusty_tokio` (#686).
---
## 2026-10-22 - Async runtime inventory and gap list (#686)

- **Added:** `docs/research/ASYNC-RUNTIME-INVENTORY.md` lists the 86 workspace crates that depend on `tokio`, `futures`, `futures-util` or `async-stream`, the API each uses, 12 gaps against `rusty_tokio` with a close/keep decision, a tier per crate (37 can move now, 22 need gaps closed, 27 are held by tokio-bound third-party crates), and a pilot (`rk-constrain`).
- **Finding:** `rusty_stream` is a durable log, not a `futures`/`async-stream` replacement; no in-house `Stream` API exists yet (gap G7).
- **Known limitations:** read-only research; method-level API was not probed and is checked by compiling each crate when it migrates. Follow-up tickets are proposed, not filed.
