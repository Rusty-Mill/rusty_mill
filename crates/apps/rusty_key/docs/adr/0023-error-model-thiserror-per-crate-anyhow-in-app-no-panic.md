# ADR-0023: Error model — `thiserror` per library crate, `anyhow` in `app`, no-panic rule

- Status: Accepted
- Date: 2026-05-27
- Tags: error-model, standards, lints

## Context

The spec lacks a unified error model. Library crates need typed, composable
errors that callers can match on, while the top-level binary can use an
ergonomic catch-all. ADR-0007 also established a no-panic rule for the harness,
but nothing enforces it (consolidated plan §D).

## Decision

Use one `thiserror` enum per library crate, composed across crates with
`#[from]`; use `anyhow` only in the `app` crate. Convert `PolicyError` from a
struct to an enum so policy rejections carry structured attribution. Back the
no-panic rule (ADR-0007) with the `unwrap_used` and `panic` clippy lints. The
full taxonomy and `#[from]` composition live in `docs/dev/error-handling.md`;
the lint configuration lives in `docs/dev/coding-standards.md`.

## Consequences

- Library callers match on typed error variants; only `app` collapses errors to
  `anyhow::Error`.
- `unwrap()` / `panic!()` in library code becomes a lint failure, enforcing the
  recover-don't-crash discipline at CI time.
- Each crate owns its error surface; cross-crate errors compose without leaking
  one crate's variants into another's public API uncontrolled.
