---
category: Changed
changelog: `rusty-mcp`'s bearer-token authorization decisions now live in a sans-IO core (`auth::decision`); `RequireAuthLayer` is a thin adapter that runs the validator and builds the response.
---
## 2026-10-14 - rusty-mcp: sans-IO auth core (sovereignty item 5, `auth` part)

- **Why:** the layer mixed the decisions (which header is acceptable, is the token meant for this resource, which scopes are missing, what each refusal says) with `tower` and `axum`. The decisions now stand alone, so a later `rusty_serve` adapter and the axum removal need not touch them.
- **Added:** crate-private `auth::decision`: `bearer_token` (header to token), `from_validation` (validator result to rejection), `check_claims` (audience, then scopes), `Rejection`, and `Refusal` (status, optional `WWW-Authenticate` value, JSON body). No `await`, response type, `tower`, `axum` or `tokio`.
- **Changed:** `RequireAuthLayer` keeps its public API and behaviour. It extracts the header, awaits the validator (the one effectful step) and maps a `Refusal` to an axum response. The pure tests moved to sync tests beside the core; the layer keeps three adapter tests.
- **Boundary:** the core uses `rusty_http::StatusCode` and takes the header as a neutral `AuthorizationHeader` (absent, not text, text); `RequireAuthLayer` converts the `http` types at the edge. `rusty-mcp` gains a `rusty_http` dependency (no new crates in `Cargo.lock`).
- **Not changed:** status codes, challenges, bodies, the audience-before-scope order, the log lines. Nothing is public that was not before.
- **Verified:** `rusty-mcp` tests (all features) pass, including the unchanged `authorization` and `jwt_validator` suites. Each of these mutations failed a test: audience check skipped, scope check skipped, case-sensitive scheme, expired reported as unavailable, a challenge on the unavailable response, scope checked before audience, the challenge header dropped in the adapter, the refusal status ignored in the adapter.
- **Known limitations:** metrics recording still needs its core (item 5 continues). The validator trait stays async because validation is I/O.
