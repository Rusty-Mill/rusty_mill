# Architecture

## Overview
`rusty_sqlite` is a thin wrapper crate over [`rusqlite`](https://docs.rs/rusqlite),
not a full application. It exists to give every `rusty_*` consumer that embeds
SQLite a shared connection lifecycle, a typed FTS5 schema builder, and a migration
runner, instead of each one reimplementing pragma setup and hand-written
`CREATE VIRTUAL TABLE` SQL independently. It is not a query builder or ORM, and it
does not hide `rusqlite` — the raw connection is always reachable via
`Connection::as_raw`/`as_raw_mut`/`into_raw`.

## Boundaries
As a library (not a service), "ports and adapters" here means: don't let
`rusqlite`-specific details leak into the parts of the API a consumer is meant to
depend on for the long term, and keep each concern in its own module.

| Port | Adapter(s) | Notes |
| ---- | ---------- | ----- |
| Connection lifecycle | `connection::Connection` wrapping `rusqlite::Connection` | Owns pragma defaults (WAL, foreign keys, busy timeout); consumers reach the raw `rusqlite` connection explicitly via `as_raw`, not implicitly. |
| Schema versioning | `migration::Migrations` | Tracks version via `PRAGMA user_version`; no separate migrations table. Swappable in principle, not yet abstracted behind a trait since there's only one implementation. |
| FTS5 schema | `fts5::Fts5TableBuilder` | Only virtual-table DDL generation — it does not manage query execution or ranking, that stays the caller's / `rusty_search`'s concern. |
| Multi-threaded access | `pool::Pool` (feature `pool`, hand-rolled: `Mutex`+`Condvar` over `rusqlite::Connection`, `std` only) | Optional and off by default; single-connection use doesn't pay for the extra API surface. Originally backed by `r2d2`/`r2d2_sqlite`; hand-rolled in [#5](https://github.com/baileyrd/rusty_sqlite/issues/5) since the pool this crate actually needs (one connection type, no generic `ManageConnection` abstraction) is materially smaller than what those crates solve. |

## Structure
Modular monolith at the crate level: one crate, one `lib.rs`, one module per
concern (`connection`, `migration`, `fts5`, `pool`), re-exported flat from the
crate root. No internal service boundary exists or is anticipated — a wrapper
crate this size has no forcing function (independent scaling, a team/language
boundary, hard fault isolation) that would justify splitting it into multiple
crates before there's a second real consumer's requirements to design against.

## Data flow
A typical consumer: `Connection::open`/`open_in_memory` → apply pragmas →
`Connection::migrate(&Migrations)` to bring schema up to date →
`Fts5TableBuilder::create` (if using full-text search) → application code drives
`rusqlite` directly via `Connection::as_raw` for everything this crate doesn't
wrap (plain queries, transactions beyond migrations, etc.).

## Key decisions
See [docs/adr/](./docs/adr/) for the record of individual decisions and their
tradeoffs. No ADRs have been written yet beyond the seed template — the crate's
initial shape was scoped directly in
[baileyrd/rusty_sqlite#1](https://github.com/baileyrd/rusty_sqlite/issues/1) and
built in [#2](https://github.com/baileyrd/rusty_sqlite/pull/2) rather than through
a formal ADR; the first real ADR should capture the next non-obvious design
decision (e.g. if/when `sqlite-vec` support or a `SchemaBackend` abstraction is
added).

## Non-goals
- Not a query builder or ORM — `rusqlite`'s API is used directly for anything
  beyond connection setup, FTS5 schema, and migrations.
- Not (yet) a `sqlite-vec`/`vec0` wrapper — tracked separately by
  `Rusty-Mill/rusty_knowledge#18`; out of scope until that's picked up here.
- Does not decide whether `rusty_search`'s planned FTS5 backend
  (`baileyrd/rusty_search#14`) builds on top of this crate — that's a decision for
  `rusty_search`, not something this crate's architecture presumes.
