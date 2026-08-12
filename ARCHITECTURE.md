# Architecture

## Overview
`rusty_err` is a `#![no_std]` + `alloc` error-handling library for the Rusty
Mill ecosystem: a sovereign `Error` trait (an alternative to
`core::error::Error`/`std::error::Error` for code that wants to stay
decoupled from `std`), a `Context` extension trait, a `BoxError` type-erased
boxed error, and a `#[derive(Error)]` proc-macro matching `thiserror`'s
`#[error("...")]`/`#[from]` shape. It exists so `no_std` crates in the
ecosystem (and `std` crates that want the same ergonomics) aren't forced to
pull in `thiserror` + `anyhow`.

**Non-goals:** not a drop-in, byte-for-byte replacement for `thiserror` — the
derive macro currently supports enums only (no structs), and `BoxError`
deliberately doesn't implement the sovereign `Error` trait itself (see "Key
decisions"), so it can't be nested as a `#[from]`/`#[source]` field one level
down. It's meant as an outermost catch-all, matching how `anyhow::Error` is
actually used in practice.

## Boundaries
This is a leaf library, not a service — there's no I/O to put behind
ports-and-adapters. The boundary that actually matters here is proc-macro
code (needs `std`, `syn`/`quote`/`proc-macro2`) vs. the `no_std` runtime
surface consumers actually link against:

| Crate | Role | Notes |
| ----- | ---- | ----- |
| `rusty_err` (root) | Public `no_std` + `alloc` runtime surface: `Error` trait, `Context` trait, `BoxError`, the `core::error::Error` bridge impl | What every consumer actually links against |
| `rusty_err_derive` (`derive/`) | `#[derive(Error)]` proc-macro codegen | Compiles with full `std` (proc-macro crates always do) but only ever emits `no_std`-safe code; kept as a separate crate so `syn`/`quote`/`proc-macro2` never end up in the runtime dependency graph |

## Structure
A two-crate Cargo workspace (root `rusty_err` + `derive/`), not a modular
monolith split by domain — there's only one domain here (error handling).
`rusty_err` re-exports `rusty_err_derive::Error` so consumers see a single
`rusty_err::Error` name for both the trait and the derive macro (the same
pattern `serde`/`serde_derive` uses — the two live in different namespaces,
so there's no collision).

## Data flow
Not applicable — no requests/events. The relevant flow is compile-time: a
consumer's `#[derive(Error)]` enum is expanded by `rusty_err_derive` into
`Display`/`Error`/`From` impls that reference `rusty_err::Error` by absolute
path (`::rusty_err::Error`), so the consuming crate must depend on this
crate under its real name `rusty_err` (no dependency-renaming support yet).

## Key decisions
See [docs/adr/](./docs/adr/) for the record of individual decisions and their
tradeoffs. Two decisions worth calling out up front (not yet written up as
ADRs):
- `BoxError` does not implement the sovereign `Error` trait, to avoid
  conflicting with `core`'s reflexive `impl<T> From<T> for T` against
  `BoxError`'s own blanket `From<E: Error>` impl.
- `impl<E: core::error::Error> Error for E` bridges the wider ecosystem's
  errors into the sovereign trait, but only one hop deep — `source()` on a
  bridged type returns `None` rather than recursing into the wrapped error's
  own `core::error::Error::source()`, since `&dyn core::error::Error` can't
  be safely re-coerced into `&dyn Error` without unsafe code.

## Non-goals
- Not attempting `std::error::Error` interop beyond the one-hop bridge above.
- Not supporting struct input to `#[derive(Error)]` yet — enums only.
- Not renaming-dependency-safe: generated code assumes the dependency is
  named `rusty_err` in the consumer's `Cargo.toml`.
