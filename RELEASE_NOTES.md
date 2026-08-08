# Release Notes

One entry per pull request merged into `main`, newest first. There are no version tags — the
crate is not published, so a merged PR is the unit of change. Each entry links its PR and states
the reasoning, not only the diff.

Backfilled from #52 onward. Earlier PRs are recorded in their own descriptions.

---

## PR #87 — Add the standard governance file set
**2026-08-08** · [#87](https://github.com/baileyrd/rusty_acp/pull/87)

- **Added:** `CONTRIBUTING`, `CODE_OF_CONDUCT`, `SECURITY`, `CHANGELOG`, `RELEASE_NOTES`,
  `ARCHITECTURE`, an ADR seed, and PR/issue templates. The repo had a strong README, a strong
  `CLAUDE.md` and nothing else — the rules a contributor needs were real but lived only in one
  agent-facing file.
- **Changed:** `ARCHITECTURE.md` written against the real system rather than left as scaffold —
  the `Store` port and its four adapters, the append-then-publish ordering, and the four
  invariants the code depends on.
- The generated `ci-rust.yml` was deleted rather than kept. This repo's `ci.yml` already runs 17
  checks across three toolchains against live Redis and Postgres; a second, weaker workflow would
  have been noise.
- Known gap: `CHANGELOG.md` is seeded but empty, and `docs/adr/` holds only the template. The
  decisions worth recording are currently in merged PR bodies and module docs.

## PR #86 — Put trace context behind a `trace` feature
**2026-08-08** · [#86](https://github.com/baileyrd/rusty_acp/pull/86)

- **Changed:** trace context is opt-in rather than always-on. #85 shipped it ungated on the
  grounds that there was no dependency to gate; that was the wrong test. A feature gates what a
  build *does* as much as what it links, and this puts a header on every outbound request and adds
  public API.
- Gated at the edges, not threaded through: `src/trace.rs` compiles either way and only its `pub`
  re-export is gated, which keeps the launch spec and run signatures free of `cfg`.
- **Added:** `tests/trace_disabled.rs`, gated the opposite way, so exactly one of the two trace
  test files always runs. Verified it fails when the header is forced on with the feature off.
- CI grew a seventeenth check: `--no-default-features --features trace`.

## PR #85 — Carry a trace across the replica boundary
**2026-08-07** · [#85](https://github.com/baileyrd/rusty_acp/pull/85) · closes #78

- **Added:** W3C `traceparent` read on the way in, written on the way out, and `trace_id` recorded
  on both the request and run spans. Before this every span was a root, so the spans one client
  call produced across replicas were unrelated islands.
- **Added:** `acp.request` spans. The issue assumed these existed; they did not — the crate had
  only `acp.run` and the reaper's, so a request that never started a run produced no correlated
  output at all.
- Correlation is by shared **field**, not a span tree. An `async` run outlives the request that
  created it, so it cannot be a child of a span that has already closed.
- Deliberate scope cut: no `tracing-opentelemetry`. That also settles the store-crossing question
  the issue left open — linking a cancel on one replica to a run on another is a span *link*, a
  concept `tracing` has no vocabulary for, so `Notification` and the `Store` contract are
  untouched.

## PR #84 — Check the coverage claim against the specification
**2026-08-07** · [#84](https://github.com/baileyrd/rusty_acp/pull/84) · closes #80

- **Added:** the ACP v0.2.0 OpenAPI document vendored under `spec/`, and `tests/spec_coverage.rs`
  checking both directions — every declared operation is routed, and every route the spec does not
  declare is listed with its reason.
- The reverse direction is the useful half. Five extensions had accumulated across nine PRs; they
  are now data, and a sixth cannot be added silently.
- Vendored rather than fetched: a networked test fails when the network does, and would silently
  retarget the crate the day upstream edits the document.
- Scope cut: route coverage only. The 40 declared schemas are a separate, larger piece of work.

## PR #83 — Dereference a session's history eight at a time
**2026-08-07** · [#83](https://github.com/baileyrd/rusty_acp/pull/83) · closes #79

- **Changed:** `fetch_session_history` overlaps its fetches. Measured against a 24-turn stub at
  200ms per request: **4.86s → ~0.6s**, peak in flight 1 → 8.
- `buffered`, not `buffer_unordered` — the ordering that matters is of the answer, not the
  requests, and the two were the same thing only because the loop was serial.
- Fixed limit of 8, no builder knob: the constraint is the other end, since history URLs may point
  at several hosts.

## PR #82 — Refuse a run whose session is already full
**2026-08-07** · [#82](https://github.com/baileyrd/rusty_acp/pull/82) · closes #77

- **Added:** `max_session_bytes`, default 32 MiB. Nothing bounded how long one conversation grew —
  `max_sessions` bounds the count, a TTL and retention bound the age.
- A **gate at admission**, not a cap during the run, so the caller is told while it can still act.
- Known limitation, stated rather than hidden: a run already admitted still records its output, so
  a session can overshoot by up to one `max_run_output_bytes`. That buys one read per run instead
  of two.

## PR #81 — Fail a run whose output outgrows what can be served
**2026-08-07** · [#81](https://github.com/baileyrd/rusty_acp/pull/81) · closes #76

- **Added:** `max_run_output_bytes`, default 8 MiB. #60/#68 bounded the event log; the aggregate
  beside it was unbounded and is written on every transition and carried in every `run.*` event.
- **Fails** rather than truncating, unlike the log. `Run::output` is a plain list in the ACP schema
  with nowhere to mark a hole, and `run.*` events travel over SSE where there is no header to put
  a caveat in.
- **Fixed:** two builder doc comments were attached to the wrong methods — `max_run_event_bytes`
  documented session eviction and vice versa.

## PR #75 — Read a notified event by index, not by seeking past it
**2026-08-07** · [#75](https://github.com/baileyrd/rusty_acp/pull/75) · closes #74

- **Fixed:** `PostgresStore` could deliver the wrong event under the right index. It read a
  notified event by seeking, so a log trimmed past that index returned the earliest survivor
  labelled with the index asked for — and that index becomes the client's `Last-Event-ID`.
- Losing the event would have been fine; serving a wrong cursor confidently is the failure this
  crate keeps refusing.
- Verified against both shared backends, with the live event acting as a barrier so the test does
  not rest on a timeout.

## PR #73 — A conformance suite third-party Stores can be run against
**2026-08-07** · [#73](https://github.com/baileyrd/rusty_acp/pull/73) · closes #69

- **Added:** `store-testkit`, a 16-check suite a backend written outside this crate can run
  against itself, plus `tests/store_conformance.rs` running it against all three built-ins and
  against a deliberately broken store.
- Paid back inward immediately. Every store-level invariant had been checked for Postgres only;
  holding the other two to one contract found that **Redis truncated lease TTLs to whole seconds**,
  so a 1500ms lease expired after one and could lapse under a replica still renewing it.
- **Changed:** `Store::publish` now documents that an event must be appended before it is
  published — Postgres sends only the index, so violating it works on two backends and disappears
  on the third.

## PR #72 — Say where a run's event list starts
**2026-08-07** · [#72](https://github.com/baileyrd/rusty_acp/pull/72) · closes #67

- **Added:** an `Acp-Events-From` header on the JSON event list, and `RunEventLog::is_complete()`
  on the client, so a trimmed log is distinguishable from a short run.
- Sent on every response: `0` means whole, *absent* means the server predates this. Those are
  different answers.

## PR #71 — Bound a run's event log on the shared backends
**2026-08-07** · [#71](https://github.com/baileyrd/rusty_acp/pull/71) · closes #68

- **Changed:** Redis and Postgres now bound one run's log by bytes, as `InMemoryStore` already did.
- **Fixed:** Redis derived an event's index from what `RPUSH` returned, so trimming the front would
  have restarted it and handed two events the same `Last-Event-ID`. The index moved to a counter of
  its own.

## PR #70 — Stop measuring a terminal event by its whole run
**2026-08-07** · [#70](https://github.com/baileyrd/rusty_acp/pull/70) · closes #66

- **Fixed:** `Event::approximate_size` walked `run.output`, so a terminal event was measured as
  enormous and displaced the entire retained log. Measured: 60 parts → 7 retained → 1.

## PR #65 — Client-side metrics
**2026-08-07** · [#65](https://github.com/baileyrd/rusty_acp/pull/65) · closes #61

- **Added:** six client metrics behind the existing `metrics` facade — requests sent, retried,
  retries exhausted, and the reconnection counterparts.

## PR #64 — Bound a run's event log
**2026-08-06** · [#64](https://github.com/baileyrd/rusty_acp/pull/64) · closes #60

- **Added:** `max_run_event_bytes` on `InMemoryStore`, and a 410 on the resume path when a client
  asks for an event that has been dropped. A TTL bounds how long a log is kept, not how much.

## PR #63 — Limit request bodies
**2026-08-06** · [#63](https://github.com/baileyrd/rusty_acp/pull/63) · closes #59

- **Added:** `max_request_bytes`, default 8 MiB, layered on the whole router rather than on
  `POST /runs` alone — a limit that only guards the endpoint you thought of is not a limit.

## PR #62 — Contain an agent panic
**2026-08-06** · [#62](https://github.com/baileyrd/rusty_acp/pull/62) · closes #58

- **Fixed:** a panicking agent left its run non-terminal and its lease held until expiry. The agent
  now runs in a spawned task whose `JoinError` is turned into a failed run.
- **Fixed:** moving the agent into a spawned task broke span inheritance, caught by #16's own test;
  restored with `.instrument(Span::current())`.

## PR #57 — Re-export `reqwest`
**2026-08-06** · [#57](https://github.com/baileyrd/rusty_acp/pull/57) · closes #49

- **Added:** `pub use reqwest`, so a caller building an authenticated client cannot get a version
  mismatch between two types with the same name.

## PR #56 — Sweep untouched sessions in Postgres
**2026-08-06** · [#56](https://github.com/baileyrd/rusty_acp/pull/56) · closes #46

- **Added:** session sweeping, guarded by `NOT EXISTS` against non-terminal runs. `Swept` reports
  runs and sessions separately because a swept session is indistinguishable from one that never
  existed.

## PR #55 — Drain waits for parked runs
**2026-08-06** · [#55](https://github.com/baileyrd/rusty_acp/pull/55) · closes #54

- **Fixed:** `drain` returned while runs were still finishing — measured at 89 of 200 complete, a
  45% misreport. The in-flight slot now follows the run rather than the agent body.

## PR #53 — A graceful shutdown example
**2026-08-06** · [#53](https://github.com/baileyrd/rusty_acp/pull/53) · closes #48

- **Added:** `examples/graceful_shutdown.rs`, carrying five tests of its own so the ordering of the
  three shutdown steps is asserted against running replicas rather than described in a comment.

## PR #52 — Bound how long a run waits for an answer
**2026-08-06** · [#52](https://github.com/baileyrd/rusty_acp/pull/52) · closes #45

- **Added:** `await_timeout`, so a run parked awaiting a client answer is failed rather than
  holding a task, a store entry and a lease indefinitely.
