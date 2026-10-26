---
category: Changed
changelog: rusty_lsp gains a default-on `runtime` feature owning tokio; rusty_term_core's `l13` feature no longer links tokio.
---
## 2026-10-23 - rusty_lsp: optional runtime, runtime-free `l13`

- **Changed:** rusty_lsp's tokio-backed modules (`server`, `client`, `service`, `transport`, `documents`, `cancel`, `testing`) sit behind a new `runtime` feature, on by default. `tcp`, `tracing` and `rusty-tokio` imply it. With default features off the crate is just `error`, `jsonrpc`, `lsp` and `text` (#850).
- **Changed:** the workspace entry for rusty_lsp turns default features off, so `rusty_term_l13` (which uses only the data types) and with it `rusty_term_core --features l13` no longer pull in tokio.
- **Known limitations:** a new in-workspace consumer of rusty_lsp's server or client must ask for `features = ["runtime"]`.
