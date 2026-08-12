# Release Notes

Notable changes to this repo, tracked one entry per merged PR against `main`
(no version tags yet), newest first, each linking to its PR.

---

## PR #6 — Hand-roll thiserror and r2d2/r2d2_sqlite out of the dependency tree
**2026-08-12** · [#6](https://github.com/baileyrd/rusty_sqlite/pull/6)

- **Changed:** dropped `thiserror`, `r2d2`, and `r2d2_sqlite` as direct dependencies.
  `Error` now has hand-written `Display`/`std::error::Error` impls; the `pool`
  feature's connection pool is a hand-rolled `Mutex`+`Condvar` structure scoped
  directly to `rusqlite::Connection`, with no external crates at all.
- **Added:** `build_pool_with_timeout` (configurable acquire timeout; `build_pool`
  now delegates to it with a 30s default), and new pool tests covering exhaustion,
  blocking-until-released, timeout, and connection reuse — the previous suite only
  covered the happy path.
- Per [`dependency-audit.md`](./dependency-audit.md): `thiserror` had no internal
  coverage (a candidate, `rusty_err`, claims a derive macro in its description that
  its source doesn't actually implement) and was hand-rolled since the enum is
  small. `r2d2`/`r2d2_sqlite` were this audit's one `keep external` recommendation
  — hand-rolled anyway per explicit sign-off, since the pool this crate needs
  (one connection type, no generic manager abstraction) is materially smaller than
  what those crates solve.
- Closes #4, closes #5.
- 19 tests (unit, integration, doctests) passing under `--all-features`; `cargo
  clippy --all-features --all-targets -- -D warnings` and `cargo fmt --check` clean.

## PR #3 — Apply standard governance file set (repo-config)
**2026-08-12** · [#3](https://github.com/baileyrd/rusty_sqlite/pull/3)

- **Added:** PR templates (feature/bug fix/docs/chore), issue templates
  (bug report/feature request), `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`,
  `SECURITY.md`, `CHANGELOG.md`, this file, `ARCHITECTURE.md` (filled in with the
  crate's actual boundaries, not left as scaffold), an ADR seed, and a CI workflow
  (`cargo fmt`/`clippy`/`build`/`test --all-features`, job named `check`).
- **Known limitation:** the CI workflow and PR/issue templates were hand-authored
  in this PR because the `repo-config` skill's own `assets/templates/.github/`
  payload was missing from the installed skill package in this environment (a
  packaging gap in the skill itself, not a decision about this repo) — everything
  else came from the skill's actual templates.
- Required-status-check and squash/rebase-disable settings still need to be set
  manually in GitHub (Settings → Branches, Settings → General → Pull Requests) —
  the workflow file alone doesn't gate merges until it's marked required.

## PR #2 — Bootstrap rusty_sqlite: connection lifecycle, FTS5 builder, migrations
**2026-08-12** · [#2](https://github.com/baileyrd/rusty_sqlite/pull/2)

- **Added:** `Connection` (wraps `rusqlite`'s `bundled` feature with default
  pragmas: WAL, foreign keys, busy timeout), `Fts5TableBuilder` (typed FTS5
  virtual-table schema generation), `Migrations` (`PRAGMA user_version`-based,
  ordered, transactional, idempotent), and an optional `pool` feature
  (`r2d2`-backed connection pool).
- **Known limitation:** no `sqlite-vec`/`vec0` support yet (tracked separately by
  `Rusty-Mill/rusty_knowledge#18`).
- 16 tests (unit, integration, doctests) passing under `--all-features`; `cargo
  clippy -D warnings` and `cargo fmt --check` clean.
- Closes #1.
