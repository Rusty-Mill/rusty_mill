# Release Notes

Per-PR record of what changed in `rusty_tokio` and why, newest first. This
repo has no version tags yet, so the unit of change is the merged PR.
`CHANGELOG.md` remains the semver-facing record of user-visible API changes;
this file carries the reasoning and the deliberate scope cuts behind them.

---

## Repo governance file set + dependency sovereignty audit
**2026-08-15** · PR pending

- **Added:** the standard governance file set — `CONTRIBUTING.md`,
  `SECURITY.md`, `CODE_OF_CONDUCT.md`, `ARCHITECTURE.md`, this file, an ADR
  seed under `docs/adr/`, and `.github/` PR + issue templates. The repo scored
  2/10 against the standard set before this (only `README.md` and
  `CHANGELOG.md`).
- **Added:** `.github/workflows/ci-rust.yml`. Adapted rather than taken stock:
  the standard workflow's `cargo test --workspace` silently skips every test
  target carrying `required-features`, which here means the `futures-io-compat`
  and `tracing` suites never run. Those now run explicitly. The io_uring and
  `thread-per-core` features are compile-checked but **not** run — hosted
  runners vary in whether their seccomp profile permits `io_uring_setup`, and a
  flaky gate is worse than an honest compile-only one. That is a real coverage
  gap, not a solved problem: nothing in CI exercises the io_uring paths.
- **Added:** `dependency-audit.md` — classification of all six external
  dependencies against internal (`rustils`, `rustils_async`, `rusty_sync`,
  `rusty_wire`) coverage. Conclusion: one genuine drop candidate
  (`syn`/`quote`/`proc-macro2`), three interop contracts where the external
  crate *is* the deliverable (`bytes`, `futures-io`, `tracing`), and two
  documented decisions left standing (`crossbeam-deque`, `io-uring`).
  `libc`/`windows-sys` excluded as the deliberate rustils RFC v2 floor.
- **Known limitation:** `ARCHITECTURE.md` notes that several load-bearing
  decisions (#8's `crossbeam-deque` adoption, #9's io_uring scope limit, the
  Windows socket-layer split) still live in issue threads and manifest
  comments rather than ADRs. The ADR directory is seeded with the template
  only; migrating those is outstanding.
- No behavior change — documentation, CI, and templates only.
