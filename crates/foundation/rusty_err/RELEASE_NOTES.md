# Release Notes

One entry per merged PR against `main`, reverse chronological, each linking
to its PR. No version tags yet (pre-1.0).

---

## PR #5 — Make BoxError Send + Sync
**2026-08-12** · [#5](https://github.com/baileyrd/rusty_err/pull/5)

- **Fixed:** `BoxError` wasn't `Send`/`Sync` unconditionally, which broke
  any error type embedding one when used across an `#[async_trait]`
  boundary (those default to `Send` futures). Root cause: `AnyError`, the
  private trait `BoxError` boxes errors behind, had no `Send`/`Sync` bound.
  Added `Send + Sync` as supertrait bounds on `AnyError`, and required them
  on `BoxError::new`'s/`From<E>`'s `E: Error + 'static` bound — matching
  `anyhow::Error`'s own construction-time guarantee. The base `Error` trait
  itself stays unbounded, so non-thread-safe types can still implement it;
  they just can't be boxed by `BoxError`.
- 2 new regression tests: `box_error_is_send_and_sync` mirrors the issue's
  compile probe directly, `search_error_is_send` asserts a
  `#[derive(Error)]` enum with a `BoxError` field is itself `Send`.
- Verified locally: `cargo test --all-features`, `cargo fmt --all --
  --check`, and `cargo clippy --all-targets --all-features -- -D warnings`
  all pass.
- Fixes [#4](https://github.com/baileyrd/rusty_err/issues/4).

## PR #3 — Apply standard repo-config governance file set
**2026-08-12** · [#3](https://github.com/baileyrd/rusty_err/pull/3)

- **Added:** the standard repo-config governance file set — README,
  CONTRIBUTING, CODE_OF_CONDUCT, SECURITY, CHANGELOG, RELEASE_NOTES,
  ARCHITECTURE, and an ADR seed under `docs/adr/`.
- **Added:** `.github/PULL_REQUEST_TEMPLATE/`, `.github/ISSUE_TEMPLATE/`, and
  a `ci-rust.yml` workflow (fmt --check, clippy -D warnings, test), once a
  local skill-installation gap that was blocking them got fixed separately.
- **Fixed:** `rusty_std` was a `path = "../rusty_std"` dependency, which only
  resolves when a sibling checkout happens to already exist — it fails
  outright on a fresh CI checkout of just this repo (verified directly).
  Switched to a pinned `git` dependency, matching the exact pattern
  `rusty_std` itself already uses for `rusty_libc`/`rusty_win32`. Without
  this, `ci-rust.yml` would have been permanently red.
- Verified locally end-to-end: `cargo fmt --all -- --check`, `cargo clippy
  --all-targets --all-features -- -D warnings`, and `cargo test
  --all-features` all pass; a fresh clone (no sibling `rusty_std` present)
  builds successfully.

## PR #2 — Add #[derive(Error)] macro and BoxError
**2026-08-12** · [#2](https://github.com/baileyrd/rusty_err/pull/2)

- **Added:** `rusty_err_derive`, a `#[derive(Error)]` proc-macro matching
  `thiserror`'s `#[error("...")]`/`#[from]` shape for enums, re-exported as
  `rusty_err::Error`.
- **Added:** `BoxError`, a boxed, type-erased sovereign error (an
  `anyhow::Error` analog) preserving `Display`/`Debug`/`source()`/downcast
  instead of `Context`'s immediate stringification.
- **Added:** a blanket `impl<E: core::error::Error> Error for E` bridging
  the wider ecosystem's errors (`serde_json::Error`, `rusqlite::Error`, ...)
  into the sovereign `Error` trait, so they work as `#[from]`/`#[source]`
  fields and `BoxError` payloads with no hand-written glue.
- **Known limitation:** the bridge is one hop deep — it doesn't recurse into
  a bridged error's own `core::error::Error::source()` chain, since that
  would require unsafe trait-object-to-trait-object coercion.
- 13 new unit/integration tests, all passing. Verified `no_std`-clean on a
  bare-metal target (`thumbv7em-none-eabihf`), not just the hosted default.
