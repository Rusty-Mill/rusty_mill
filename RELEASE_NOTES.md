# Release Notes

One entry per merged PR against `main`, reverse chronological, each linking
to its PR. No version tags yet (pre-1.0).

---

## PR #3 — Apply standard repo-config governance file set
**2026-08-12** · [#3](https://github.com/baileyrd/rusty_err/pull/3)

- **Added:** the standard repo-config governance file set — README,
  CONTRIBUTING, CODE_OF_CONDUCT, SECURITY, CHANGELOG, RELEASE_NOTES,
  ARCHITECTURE, and an ADR seed under `docs/adr/`.
- **Known limitation:** this skill's installed template assets are missing
  `.github/PULL_REQUEST_TEMPLATE/`, `.github/ISSUE_TEMPLATE/`, and the
  stack-selected CI workflow (`ci-rust.yml`) — none of those were applied
  here, only the eight root/docs markdown files.

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
