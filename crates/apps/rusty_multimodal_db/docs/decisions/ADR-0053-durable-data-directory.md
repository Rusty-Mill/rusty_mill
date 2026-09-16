# ADR-0053: A durable data directory for `memory_server` — `SERVER_DATA_DIR`, open-or-create per table

- Status: **Accepted as designed** (promoted from Proposed on
  2026-09-07 — the owner's "accept as designed", option (a): one
  environment variable selecting a durable directory, open-or-create
  per table, empty on first start; (b) a positional argument, (c)
  durable by default at a fixed path, (d) seeding the samples into the
  durable directory, and (e) decline declined. Recorded in "Acceptance
  and implementation" below.) Proposed and implemented on one branch,
  the `ADR-0046`–`ADR-0052` cadence.
- Date: 2026-09-07
- Deciders: baileyrd
- Related: `docs/design/SERVER-DATA-DIR-DESIGN.md` (the full design),
  `ADR-0048` (`memory_server`), `ADR-0046`–`ADR-0052` (the runtime
  writes a restart must keep), `ADR-0025` (a path-valued variable's
  precedent).
- Supersedes/Superseded by: none. No wire change, no format change.

## Context

Every runtime write since `ADR-0046` survives a portable reopen, and
every test says so by calling `open_*_production_stack_portable`
in-process. The consumer's binary, `memory_server`, never reopens: it
seeds the sample dataset into a per-process scratch directory on every
start. An integration spike against `rusty_remind_me` found this on
its first restart. A backend that forgets on restart is not one.

## Decision

Add `open_or_create_memory_production_stack` and
`open_or_create_entity_production_stack` — open from the files alone
when the slot file exists, create empty when not — and one environment
variable on `memory_server`, `SERVER_DATA_DIR`: set, both tables live
there durably, empty on first start, reopened on every later one;
unset, today's scratch-and-samples behaviour unchanged. The startup
line says which. No seeding into a durable directory, no repair of a
partial one, no default journal placement, no change to the other
binaries in this round.

## Consequences

- Positive: the consumer can point its adapter at a process that keeps
  what it was told across restarts — the first time this crate's
  served surface has been durable end to end.
- Positive: additive and reversible; no format, wire, or client change.
- Named, not hidden: the other three binaries still scratch; a partial
  directory is a startup error the operator resolves; two processes on
  one directory are as unprotected as before.
- Named, not hidden: the binary is proven by hand for this round, not
  by an integration test; the test harness is an open question.

## Considered options

**(a) Accept as designed** — one variable, open-or-create per table,
empty on first start. **(b) A positional argument** — a parse
convention this crate avoids. **(c) Durable by default at a fixed
path** — changes every existing invocation. **(d) Seed the samples
into the durable directory** — fake data in a real backend. **(e)
Decline.**

## Acceptance and implementation

- 2026-09-07: proposed and implemented on the same branch as
  `SERVER-001` v0.43.0 / FR-053 — `src/generic/memory.rs`,
  `src/generic/entity.rs`, `src/bin/memory_server.rs`; tests: `memory`
  +1, `entity` +1; criterion 2 run by hand over a real socket (write,
  kill, restart, read back) and recorded in `docs/PROJECT-STATUS.md`.
  (PR #210.)
- 2026-09-07: accepted as designed (option (a); (b)–(e) declined). No
  change to the implementation. The other binaries and a binary-level
  test harness stay open questions.
