# ADR-0099: `memory_server` Defaults — Idle Timeout, Connection Cap, and Synced Updates On

- Status: **Proposed and implemented on one branch; the owner chose
  it** (2026-09-21, "Defaults"). Option (b) of `ADR-0093` and of `ADR-0097`,
  taken together.
- Date: 2026-09-21
- Deciders: baileyrd
- Related: `ADR-0093`/`docs/design/SERVER-CONNECTION-LIMITS-DESIGN.md`
  (the limits, opt-in), `ADR-0097`/`docs/design/SERVER-SYNCED-UPDATES-DESIGN.md`
  (synced updates, opt-in; ~100 µs per update measured), `ADR-0032`
  (`ServeOptions::default()` reproduces the original server — still
  true; this round changes the binary's defaults, not the library's).
- Supersedes/Superseded by: amends `ADR-0093`'s and `ADR-0097`'s
  "opt-in, unset by default" for `memory_server` only. Additive:
  `bounded_env`/`switch_env`, two constants, the module docs. No wire
  or library change.

## Context

Three of the review's hardening rounds shipped opt-in, so a
deployment that set nothing was as unbounded and as write-back as
before them. The owner asked for the defaults to be on.

## Decision

Implement, in `memory_server` only: unset, `SERVER_IDLE_TIMEOUT_SECS`
is 300 and `SERVER_MAX_CONNECTIONS` is 1024 (`0` turns either off);
unset, `SERVER_SYNC_UPDATES` is on (`0` turns it off; only `0`/`1`
are accepted). `SERVER_MAX_QUERY_ROWS` stays off unless set: under a
cap a `Query` with no `limit` is refused, and every `SELECT` without
`LIMIT` compiles to one — a default that fails the consumer's plain
`SELECT` is not a default. A cap that clamps a missing `limit` instead
of refusing it is the fork for a later round. Proven against the
compiled binary through its ready banner, the one observable a test
can afford: the defaults, `0` turning each off, and a malformed value
refusing to start.

## Consequences

- Positive: a fresh `memory_server` is bounded and durable-on-ack
  without a single variable set.
- Negative / tradeoffs: an existing deployment that restarts on this
  build pays ~100 µs per acknowledged in-place update
  (`SERVER_SYNC_UPDATES=0` takes it back) and drops a connection idle
  for five minutes (`SERVER_IDLE_TIMEOUT_SECS=0` keeps it). Both are
  named in the banner at every start.
- Named, not hidden: the row cap is the one limit still off by
  default, for the reason above. `dog_server`/`entity_server`/
  `reminder_server` read none of these variables and are unchanged.

## Acceptance and implementation

- 2026-09-21: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.82.0 / `FR-094`.
  `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — `memory_server_defaults` 3 (new), 965 tests across 44 targets, 0 failed. Builder: Claude; independent Codex inspection owed.
