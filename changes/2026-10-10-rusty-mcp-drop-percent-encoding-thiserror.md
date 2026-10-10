---
category: Changed
changelog: **`rusty-mcp` drops `percent-encoding` and `thiserror`** (sovereignty plan item 1). Baggage values use `rusty_percent`; the four public error enums keep the same messages and `std::error::Error` impls, written by hand. Baggage values no longer escape `-`, `_`, `.` and `~`, which are unreserved and decode identically.
---
## 2026-10-10 - `rusty-mcp`: `percent-encoding` and `thiserror` replaced

- **Changed:** `trace::Baggage` encodes and decodes with `rusty_percent`. The encoded form is shorter for values containing `-_.~`; decoding is unchanged.
- **Changed:** `JwtValidatorError`, `AuthConfigError`, `TokenError` and `OtelError` implement `Display` and `std::error::Error` by hand (same messages, same `source()`), so `thiserror` is gone. `rusty_err`'s derive was not used: it implements its own `Error` trait, not `std::error::Error`, which would break `?` into `Box<dyn std::error::Error>` in the gateway.
