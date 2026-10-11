---
category: Changed
changelog: `rusty-mcp` fetches the JWKS with `rusty_request` instead of `reqwest` (no second runtime), and `JwtValidatorError::Http` is removed, so the crate moves to 0.8.0.
---
## 2026-10-11 - rusty-mcp 0.8.0: JWKS fetch on rusty_request (sovereignty list item 3)

- **Changed:** the `jwt` feature fetches the JWKS with `rusty_request` (feature `tokio`) instead of `reqwest`. The request runs on the caller's own `tokio` runtime, so no private runtime or thread is started. Settings carried over from the old client: the request timeout, at most 10 redirects, no cookie jar, no proxy from the environment. `rusty-mcp` no longer depends on `reqwest` outside its tests.
- **Changed (breaking, pre-1.0):** `JwtValidatorError::Http` is removed. `rusty_request::ClientBuilder::build` cannot fail, so nothing produces it any more; `JwtValidatorError` now has only `NoAlgorithms` (still `#[non_exhaustive]`). Version 0.7.0 to 0.8.0. The only in-workspace user wraps the error in a `Box` without matching on variants.
- **Behaviour:** error text for a failed fetch now comes from `rusty_request`, so the wording after `JWKS fetch failed:` differs; the `TokenError::Unavailable` classification and the `JWKS endpoint returned <status>` and `malformed JWKS` messages are unchanged.
- **Verified:** `rusty-mcp` and gateway crate tests, clippy `-D warnings`, `cargo-shear` and the workspace policy scripts (see the PR). The 8 existing validator tests pass unchanged on the new client. Four new tests cover a failing endpoint, a malformed JWKS, a malformed URI and a server that never answers; the timeout test and the failing-endpoint test each fail with their code broken (checked).
- **Known limitations:** HTTPS under real `tokio` is covered by `rusty_request`'s own tests, not by a test in this crate (the validator tests use plain `http://` on localhost). No before/after latency measurement was taken. An invalid `jwks_uri` is still reported on first use as `Unavailable`, not at build time.
