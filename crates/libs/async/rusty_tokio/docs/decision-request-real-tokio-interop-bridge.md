# Decision request: a first-class bridge into real-tokio-only dependencies

Status: **proposed** (not yet decided — written up per this crate's own
convention of a `docs/decision-request-*.md` doc before implementation
starts, mirroring `decision-request-windows-process-signal-ipc.md`; no
other governance model exists here beyond ordinary issue-driven review).
Date: 2026-09-02

## Context

`rusty_meshed` (the `baileyrd/meshed` → Rust migration landing in this
workspace, PR #131) is built entirely on `rusty_tokio`: its Kafka client,
consumer/producer bases, and every `#[rusty_tokio::test]` in
`rusty-meshed-domains`/`rusty-meshed-sdk`/`rusty-meshed-registry`/
`rusty-meshed-observability`/`rusty-meshed-schema-registry`. Several of
those crates also call `rusty_request` for HTTP (the schema registry
client, the platform registry client, a governance contract-gate check).

CI's full-workspace test job runs `cargo nextest run --all-features`
(`.github/workflows/ci.yml`). `rusty_request` is itself a workspace
member with an optional `tokio` feature
(`#[cfg(feature = "tokio")]`/`#[cfg(not(feature = "tokio"))]` switches its
internals between real `tokio::{time, task::spawn_blocking}` and
`rusty_tokio`'s own). `--all-features` unconditionally turns that feature
on for `rusty_request` itself, and Cargo's feature unification means
**every** consumer in the same build graph gets the same compiled
instance — confirmed via `cargo metadata`: `rusty_request`'s resolved
`features` list is `["tokio"]` in a full-workspace build, regardless of
what any individual consumer's own `Cargo.toml` requests.

The concrete failure (PR #131, `test (ubuntu-latest)`, reproduced
identically on all 3 nextest retries — not a flake):

```
thread '...' panicked at crates/rusty_request/src/client.rs:609:30:
there is no reactor running, must be called from the context of a Tokio 1.x runtime
```

Every `rusty-meshed-*` test that calls into `rusty_request` (via
`RegistryClient`/`SchemaRegistryEnforcer`/`SchemaRegistryClient`) runs
under `#[rusty_tokio::test]`. Once `rusty_request` is feature-unified
onto real tokio, its `time::timeout(...)` call needs a real tokio
runtime's timer registered on the polling thread — and a `rusty_tokio`
scheduler never registers one. The 3 tests that showed as failed are
almost certainly a fraction of the true count; nextest cancelled the run
after them (`--no-fail-fast` isn't set), so most later tests in the
alphabetical/scheduling order never got a chance to run.

**This is not new to this PR.** Three other workspace crates already
hit the same fork and resolved it by fully committing to real tokio
instead: `rusty_proxmox`/`rusty_opnsense` (forced onto real tokio by the
`rmcp`/wider MCP ecosystem, per `rusty_proxmox/Cargo.toml`'s own
comment) and `rusty_search`'s backend crates (forced by `wiremock`, a
real-tokio-only HTTP mocking crate, in their dev-dependencies). Both
depend on `rusty_request` with `features = ["tokio"]` explicitly and use
`#[tokio::test]`/`tokio::runtime::Runtime` throughout — a clean,
consistent choice *for those crates*, achievable because nothing else in
them needs `rusty_tokio` specifically.

`rusty-meshed-*` can't make the same choice: its Kafka client and
consumer/producer bases are `rusty_tokio`-native (duplex streams,
`TcpListener`, `join!`/`try_join!`/`select!`, `spawn`), and switching
that foundation to real tokio is a different, much larger undertaking
unrelated to this bug — the actual need here is narrower: *occasionally*
call into a dependency that happens to have been feature-unified onto
real tokio, from code that is otherwise entirely `rusty_tokio`-scheduled.

## The gap

`rusty_tokio` has no first-class, documented way to do that. The only
existing bridge anywhere in this workspace is `rusty_request`'s own
`src/tokio_compat.rs` — and that's narrower than what's needed here: it
bridges *I/O trait definitions* (`rusty_tls`/`rusty_http`'s
`rusty_tokio::io::AsyncRead`/`AsyncWrite` impls, made pollable under a
real tokio reactor for `rusty_request`'s own connector) so a real-tokio
caller can drive `rusty_tokio`-based I/O. It does not solve — and isn't
trying to solve — the reverse direction: a `rusty_tokio`-scheduled task
that needs to *drive a future which itself contains real-tokio
primitives* (`time::timeout`, `spawn_blocking`) to completion.

`OutboxRelay::start` (`rusty-meshed-sdk::outbox`) already hand-rolls the
closest thing to a real fix for a related but different problem: it
spawns a background `std::thread`, builds its own single-threaded real
runtime there, and `runtime.block_on(...)`s a loop on it — but that's a
dedicated background thread for a long-running loop, not something
reusable for an ad-hoc `.await`-shaped call from inside otherwise
`rusty_tokio`-scheduled async code.

Concretely, `rusty_tokio` is missing:

1. **A documented, first-class way to run a real-tokio-only future to
   completion from within `rusty_tokio`-scheduled code**, without the
   caller hand-rolling a `Runtime` and reasoning about deadlock risk
   themselves. Confirmed safe in principle for this crate's own
   scheduler: `rusty_tokio::test`/`main` "has exactly one runtime flavor
   (multi-threaded)" per its own macro docs, so blocking one worker
   thread on a nested real-tokio `block_on` doesn't stall concurrently
   scheduled `rusty_tokio` tasks (e.g. a fake HTTP/Kafka server spawned
   in the same test) — but nothing states this guarantee for callers to
   rely on, and every consumer has to independently verify it.
2. **A named, documented failure mode**: nothing today tells a
   `rusty_tokio`-based crate that adding a dependency on a workspace
   member with its own real-tokio opt-in feature (like `rusty_request`)
   is a latent trap under this repo's own `--all-features` CI
   convention — it silently compiles and passes locally (a scoped
   `cargo test -p <crate>` never unifies the feature on), and only
   breaks in the full-workspace CI sweep, at test time, with a panic
   whose message doesn't point at the actual cause.

## Options considered

**A. Hand-roll a `OnceLock<tokio::runtime::Runtime>` + `.block_on(...)`
bridge locally in each affected `rusty-meshed-*` crate.** Works
(verified safe against deadlock per the multi-threaded-scheduler
argument above), but requires adding `tokio` as a new direct dependency
to 4 crates (`rusty-meshed-schema-registry`, `rusty-meshed-sdk`,
`rusty-meshed-registry`, `rusty-meshed-observability`) and duplicating
the same ~10-line helper in each. Every *future* `rusty_tokio`-based
crate that adds a `rusty_request`-style dependency hits this exact wall
again and has to independently re-derive that it's safe.

**B. A per-thread `Runtime::enter()` guard instead of `block_on`.**
Rejected: `Runtime::enter()`'s guard is thread-local, but `rusty_tokio`'s
scheduler is work-stealing — a task can resume on a different OS thread
after an `.await` point than the one it started on, so a guard entered
once at test start isn't reliably still active when the inner real-tokio
future is actually polled. Would need the guard established on *every*
`rusty_tokio` worker thread, which only `rusty_tokio`'s own runtime setup
could arrange.

**C. Make `rusty_tokio` API-complete enough that dependents never need
real tokio at all.** Infeasible for the two existing forcing cases:
`rmcp`/the MCP Rust SDK and `wiremock` are external crates hard-wired to
real tokio's own `AsyncRead`/`AsyncWrite`/reactor/timer types at their
public API boundary — no amount of `rusty_tokio` feature growth changes
what a third-party crate demands of its caller.

**D. Promote option A's pattern into `rusty_tokio` itself as a
documented, reusable primitive** — the recommended option. Concretely:
a small bridge utility (e.g. `rusty_tokio::compat::block_on_real_tokio`),
backed by a lazily-initialized shared multi-threaded real
`tokio::runtime::Runtime`, gated behind an optional feature (e.g.
`rusty_tokio = { features = ["real-tokio-bridge"] }`) so the real `tokio`
dependency stays opt-in rather than unconditional. `rusty_tokio` is
already a dependency of every affected crate, so this closes the gap
without any of them adding a *new* dependency of their own. The
`--all-features` hazard itself gets documented once, in `rusty_tokio`'s
own README, as a named failure mode with this bridge as the answer,
instead of every future consumer rediscovering it from a bare panic
message.

## Recommendation

Option D. Scope for a first pass:

- `rusty_tokio::compat::block_on_real_tokio<F: Future>(fut: F) -> F::Output`
  (naming open to bikeshedding), feature-gated, backed by a
  `std::sync::OnceLock<tokio::runtime::Runtime>` built with
  `rt-multi-thread`.
- README section documenting the `--all-features`-plus-workspace-member
  feature-unification hazard by name, with this bridge as the
  recommended fix, cross-linked from `rusty_request`'s own "Running on
  real tokio instead" section.
- No change to `rusty_tokio`'s own scheduler/reactor/timer internals —
  this is purely an additive, optional escape hatch for dependency
  interop, consistent with the crate's stated purpose ("not to replace
  tokio").

Not attempted here: `rusty-meshed-*`'s own call sites are **not** wired
to use this yet — that's the follow-up once this decision is made,
tracked as its own PR against PR #131's CI failure.

## Consequences

- Adds an optional real `tokio` dependency to `rusty_tokio` itself
  (feature-gated, so crates that never need it pay nothing).
- Every future `rusty_tokio`-based crate in this workspace gets a
  documented, tested answer to "I need to call something that turned out
  to be feature-unified onto real tokio" instead of independently
  hitting this same panic and re-deriving the fix.
- Doesn't reduce the number of dependency edges — `rusty-meshed-*` still
  ends up depending (transitively, through `rusty_tokio`'s new feature)
  on real `tokio` for this narrow purpose. That's already true today via
  `rusty_request`'s existing "tokio" feature once unified; this just
  gives consumers a supported way to use it instead of a hand-rolled one.
