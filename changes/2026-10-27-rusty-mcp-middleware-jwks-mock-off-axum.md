---
category: Changed
changelog: "`rusty_mcp_middleware`'s JWT tests serve their mock JWKS endpoint on `rusty_serve` instead of `axum`, dropping the crate's last `axum` dev-dependency (#875)."
---
## 2026-10-27 - rusty_mcp_middleware's JWKS mock leaves axum

- **Changed:** `tests/jwt_validator.rs` serves its JWKS (and the raw error and not-JSON replies) at `/jwks.json` from a blocking `rusty_serve` server on a thread of its own, 404 elsewhere, as the `axum` route did. `rusty_serve` was already a dev-dependency (for `tests/trace_context.rs`).
- **Dependencies:** `axum` leaves `rusty_mcp_middleware`'s `[dev-dependencies]`. Nothing is added.
- **Known limitations:** the tests still run on `#[tokio::test]` with `reqwest` and raw tokio sockets, because the validator's JWKS fetch runs on its caller's tokio runtime (#906).
