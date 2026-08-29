# Dependency audit

Sovereignty audit of `rusty_sqlite`'s direct, non-dev dependencies against the
`baileyrd/*` platform repos (the RustyMill org itself was unreachable from
the auditing session — see notes on the affected rows). One row per direct
dependency that survived the floor-dependency exclusion below.

| Dependency | Purpose | Classification | Internal candidate | Size | Recommended action | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| `rusqlite` | SQLite bindings | excluded (floor) | — | — | — | This is the dependency `rusty_sqlite` exists to wrap — `ARCHITECTURE.md` states outright it doesn't hide `rusqlite`. Same category as an already-decided floor dependency; not an audit candidate. |
| `thiserror` | Derive macro for the 5-variant `Error` enum | hand-roll candidate → **approved, hand-rolled** | `rusty_err` — considered, not usable | S | Hand-rolled `Display`/`std::error::Error` impls for the `Error` enum | `rusty_err`'s `Cargo.toml`/keywords claim "proc-macro... derive library," but its actual `src/lib.rs` (52 lines) has no derive macro — just a hand-written `Error`/`Context` trait pair, `#![no_std]`, with its own `Error` trait rather than `std::error::Error`. Disqualified by a source read, not usable as-is. |
| `r2d2` | Generic sync connection pooling (optional `pool` feature) | keep external → **approved, hand-rolled anyway** | none found | S | Hand-rolled a `Mutex`+`Condvar`-backed pool scoped to `rusqlite::Connection` | No internal coverage exists — `rusty_db` pools connections via `sqlx`'s *async* pools, an incompatible execution model for this crate's sync design. Hand-rolling a pool scoped to exactly one connection type (rather than reimplementing r2d2's generic `ManageConnection` abstraction) is materially smaller than the general problem r2d2 solves. |
| `r2d2_sqlite` | `r2d2` adapter for `rusqlite` (optional `pool` feature) | keep external → **approved, hand-rolled anyway** | — | S | Removed along with `r2d2` (folded into the same hand-rolled pool) | Was only the `rusqlite`-specific glue on top of `r2d2`; no longer needed once the pool itself is hand-rolled directly against `rusqlite::Connection`. |

## Outcome

All three non-floor dependencies were hand-rolled per explicit user sign-off
(overriding this audit's own `keep external` recommendation for
`r2d2`/`r2d2_sqlite` — logged here as the decision actually made, not the
one recommended). `rusqlite` remains, as the floor dependency this crate
exists to wrap.

See:
- [#4](https://github.com/baileyrd/rusty_sqlite/issues/4) — hand-roll `thiserror` replacement
- [#5](https://github.com/baileyrd/rusty_sqlite/issues/5) — hand-roll `r2d2`/`r2d2_sqlite` replacement
