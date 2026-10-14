---
category: Changed
changelog: `rusty-mcp`'s concurrency limit is now a sans-IO core (an atomic counter, no `tokio::sync::Semaphore`) with `LimitsLayer` as a thin tower adapter over it.
---
## 2026-10-14 - rusty-mcp: sans-IO limits core (sovereignty items 5 and 8, `limits` part)

- **Why:** the layers mixed the decision (how many in flight, what a refusal says) with the transport (tower, axum, tokio). The decision now stands alone, so a later `rusty_serve` adapter and the axum removal need not touch it.
- **Added:** crate-private `admission` module: `Admission` (shared atomic in-flight count; `try_admit` refuses rather than waits, one compare-and-swap step), `Permit` (frees its slot on drop) and `Rejection` (status, optional `Retry-After`, body for the `503` and `504`). It has no `tokio`, `axum` or `tower`.
- **Changed:** `LimitsLayer` keeps its public API and behaviour; it holds an `Admission`, owns the timeout (`tokio::time`) and maps a `Rejection` to an axum response. `limits` no longer uses `tokio::sync::Semaphore`.
- **Not changed:** the status codes, bodies and `Retry-After: 1`; zero still clamps to one; the timeout still bounds time to first response. Nothing is public that was not before.
- **Verified:** `rusty-mcp` tests (all features) pass, including the unchanged `limits` suite and a 16-thread test that the limit is never exceeded. Mutations each failed a test: no decrement on drop, an off-by-one limit, a non-atomic check-then-store, a refusal that takes a slot, and dropping `Retry-After` in the adapter.
- **Known limitations:** `auth` and metrics recording still need their cores (item 5 continues). `tokio` stays a dependency for the timeout and `auth`.
