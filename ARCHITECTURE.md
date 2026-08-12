# Architecture

## Overview
`rusty_time` is a leaf `#![no_std]` + `alloc` library crate: three value types
(`Date`, `Time`, `DateTime`), an RFC 3339 parser, and an ISO-8601 formatter. It has
no I/O, no runtime, and no background tasks — every public function is a pure
transformation from input bytes/fields to a value or an `Err`.

## Boundaries
Ports-and-adapters doesn't apply structurally here — there's no I/O to keep out of
the domain layer, so there's nothing to adapt. The only boundary that matters is the
public API surface (`Date`, `Time`, `DateTime` and their methods in `src/lib.rs`)
versus internal helpers (`parse_digits`, `expect_byte`, `days_in_month`,
`is_leap_year`, `Date::days_since_epoch`), which stay private and can change shape
without a semver bump.

## Structure
Single-crate, single-file (`src/lib.rs`). That's appropriate at the current size —
split into modules (`date.rs`, `time.rs`, `parse.rs`, ...) only once the file becomes
hard to navigate, not preemptively.

One structural quirk worth flagging: `Cargo.toml` declares `rusty_std` as a
`path = "../rusty_std"` dependency, even though nothing in `src/lib.rs` currently
uses it. That means:
- A standalone `git clone` of `rusty_time` won't `cargo build` — `rusty_std` must be
  checked out as a sibling directory first (see README's Getting Started).
- CI checks out both repos as siblings under `$GITHUB_WORKSPACE` to reproduce that
  layout (see `.github/workflows/ci-rust.yml`).

If `rusty_std` stays unused, consider dropping the dependency; if it's needed soon,
say what for in the PR that starts using it.

## Data flow
`DateTime::parse(&str)` → validates and slices RFC 3339 fields → builds a `Date` and
`Time` (each independently validated, e.g. day-of-month against the actual
month/year) → wraps them with a UTC offset into a `DateTime`. `DateTime::timestamp()`
reverses part of that: civil-calendar day count (Howard Hinnant's `days_from_civil`)
+ time-of-day − offset = Unix seconds.

## Key decisions
See [docs/adr/](./docs/adr/) for the record of individual decisions and their
tradeoffs.

## Non-goals
- No timezone database (IANA tz) support — only fixed numeric UTC offsets, matching
  RFC 3339's scope. A named-timezone crate would be a different, heavier tool.
- No leap-second representation (`:60` seconds are rejected, not normalized).
- No calendar arithmetic beyond RFC 3339 parsing/formatting (no "add N days",
  duration types, etc.) unless a concrete consumer needs it — see issue tracker
  before adding.
