---
category: Added
changelog: "`rusty-mcp` gains a `serve` feature: `AuthorizedHandler` and `LimitedHandler` put bearer-token auth and load shedding in front of a blocking `rusty_serve` server, without tower."
---
## 2026-10-15 - rusty-mcp: `serve` adapters for the auth and limits cores (sovereignty item 7)

- **Why:** the sans-IO cores (item 5) were only reachable through the tower layers. A blocking `rusty_serve` server could not use them, so it had no resource-server auth or load shedding without pulling in tower and axum.
- **Added (public API, opt-in feature `serve`, adds an optional `rusty_serve` dependency):** `rusty_mcp::serve::LimitedHandler` (at most N requests inside the wrapped handler; the rest get `503` with `Retry-After`, never queued) and `rusty_mcp::serve::AuthorizedHandler` (`401`/`400`/`403` with the same `WWW-Authenticate` challenges as `RequireAuthLayer`; the inner function receives the `VerifiedToken`). Both are `rusty_serve::SharedHandler`s; nest them with limits outermost so a shed request never costs a token validation.
- **Changed:** `auth::decision` is now `pub(crate)` so the adapter can use it. Nothing else is public that was not before.
- **Not changed:** the tower layers and the cores; no default feature, dependency or behaviour changes.
- **Verified:** unit tests for the small `std` executor and the response mapping; four tests against a real `rusty_serve` server (a valid token reaches the handler with its claims; each refusal's status and challenge; a second request over the limit is shed while the first is inside, then the permit comes back; a shed request is never validated). Each of these mutations failed a test: the limit ignored, the permit released early, claims unchecked, challenge dropped, `Retry-After` dropped, the header never read, the validator skipped.
- **Known limitations:** no timeout (a blocking handler cannot be cancelled; `rusty_serve::Limits` bounds idle time). The validator is polled on the connection's thread with no reactor, so it works for a validator that answers from memory (such as `StaticTokenValidator`) but not for the JWKS-backed `JwtValidator`, whose fetch needs a `tokio` runtime; use the tower layers for that. A validator that never completes holds its connection thread. `SharedHandler` only (not the locked `Handler`). The metrics core has no adapter here.
