# Architecture

## Overview

A pure-Rust, embedded, single-process SQL database engine: SQL tokenizer/
parser, query planner, execution engine, and on-disk B-tree/page storage,
reimplemented from scratch — no `libsqlite3-sys` or other C dependency. An
ergonomic API layer on top targets behavioral parity with `rusqlite`'s
public surface (`Connection`, `Statement`, `Row`, `Transaction`, etc.) so
existing `rusqlite` call sites can migrate with minimal changes.

Not (yet, and not by default): multi-process concurrent access via a shared
on-disk file (SQLite's file-level locking/WAL story), full SQL-92/that
SQLite itself supports on day one, or wire-protocol network access — those
are scope decisions for later gap-analysis rounds, not ruled out permanently.

## Boundaries

Ports-and-adapters, per `ATLAS-LAYER-0001`/`ATLAS-BOUND-0001`
(`Atlas_Engineering_Standards_Library`, ATLAS-100): the engine's domain
logic (parser, planner, VM, B-tree) stays free of I/O; a storage-backend
port abstracts the page source so an in-memory backend and a file-backed
backend can both satisfy it without the engine depending on either
concretely.

No adapters exist yet — this table intentionally has no rows until real
code lands; see `gap-analysis.md` and open `parity-gap` issues for what's
being built toward.

| Port | Adapter(s) | Notes |
| ---- | ---------- | ----- |
|      |            |       |

## Structure

Modular monolith per `ATLAS-DEP-0010` (Atlas ecosystem default) — one crate
(or a small Cargo workspace once internal boundaries justify splitting:
e.g. `engine` vs. the `rusqlite`-compatible API layer), not separate
services. Splitting into a workspace happens when a concrete internal
boundary (parser vs. storage vs. API-compat layer) demonstrably needs
independent versioning or testing in isolation — not speculatively.

## Data flow

TBD — no query execution path exists yet. Will be documented once the
tokenizer → parser → planner → VM → storage pipeline has a first working
slice (tracked as the earliest `parity-gap` issues).

## Key decisions
See [docs/adr/](./docs/adr/) for the record of individual decisions and their tradeoffs.

## Non-goals

- Depending on the C SQLite library (`libsqlite3-sys`) or any other C code
  to implement core engine behavior — the point of this repo is sovereignty
  from that dependency, per the platform's sovereignty-loop stance.
- Matching SQLite's C API/ABI — only the `rusqlite` Rust API surface is the
  parity target, not SQLite's C-level interface.
- Full SQLite feature-for-feature parity (extensions, FTS, R-Tree, etc.) as
  a day-one requirement — these are gap-analysis candidates, prioritized
  incrementally, not a blocking bar for early milestones.
