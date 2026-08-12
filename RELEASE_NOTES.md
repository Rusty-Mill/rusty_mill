# Release Notes

One entry per merged PR against `main`, reverse chronological, each linking to its
PR and (where one exists) to the doc that covers the change in full detail.

---

## PR #2 — Apply repo-config governance file set
**2026-08-12** · [#2](https://github.com/baileyrd/rustils_async/pull/2)

- **Added:** PR templates (feature/bug_fix/docs/chore), issue templates
  (bug_report/feature_request + contact-link config), CONTRIBUTING.md,
  CODE_OF_CONDUCT.md, SECURITY.md, CHANGELOG.md, ARCHITECTURE.md, an ADR seed
  template, and a `ci-rust.yml` workflow (fmt check, clippy `-D warnings`, `cargo
  test --workspace`) so the "PR + green CI, merge with a merge commit" workflow has
  a real gate to run against.
- **Known limitation, stated plainly:** the repo-config skill's own local template
  package was missing its entire `.github/` payload (PR templates, issue templates,
  CI workflow templates) when this ran — those pieces were hand-authored to match
  the skill's documented conventions rather than copied, so they haven't been
  cross-checked against whatever the skill's canonical templates actually say.
  Worth reconciling once the skill package is fixed upstream.
- SECURITY.md's contact is a real address (baileyrd@gmail.com), not a placeholder —
  confirmed with the repo owner during setup.

## PR #1 — Bootstrap rustils_async: native async process domain, no hidden runtime
**2026-08-12** · [#1](https://github.com/baileyrd/rustils_async/pull/1)

- **Added:** `reactor-core` (runtime-agnostic async-io primitives per Rusty-Mill
  AKB ADR-0160), `platform-async` (async `Spawner`/`Child` traits for the
  `process` domain, reusing rustils' own types via a pinned git dependency),
  `platform-async-mock`, `platform-async-linux` (a real pidfd + epoll reactor,
  explicit and `Drop`-cleaned-up, no hidden global runtime), `threading` (scoped
  thread spawn with a decoded join outcome, `Mutex`/`RwLock` with an explicit
  poisoning policy), and `coreutils-async` (`arun`, an async port of rustils'
  `rrun`, as the forcing consumer).
- **Known limitation, stated plainly:** this repo started without a named,
  working consumer forcing the work — a deliberate, documented exception to
  rustils' own consumer-gate policy, recorded in `docs/adr/0001-native-async-rustils.md`.
  Windows/BSD backends and the fs/net async domains are reserved rows, not built.
- 24 tests passing, including real spawn+wait against actual child processes
  (not just mocks) exercising the epoll reactor end to end.
