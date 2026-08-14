# Release Notes

<!--
Two variants, pick the one that fits this repo's actual unit of change:

1. No version tags yet (pre-1.0, nothing published) — track by PR instead, same way
   AISF does it: one entry per merged PR against main, reverse chronological, each
   linking to its PR and (where one exists) to the doc that covers the change in full
   detail. Use "## PR #N — <summary>" headers.

2. Actual version tags exist — use "## vX.Y.Z - YYYY-MM-DD" headers instead, each
   linking to the PRs it shipped and a compare link to the previous tag. Add an
   "### Upgrade notes" subsection under any entry with a breaking change.

Either way, keep the tone AISF's file uses: bolded category tags inline in the
bullet (**Added:** / **Changed:** / **Fixed:**), not separate subheaders per
category — and state known limitations or deliberate scope cuts plainly instead of
leaving them implied.
-->

Tracks changes by merged PR against `main`, reverse chronological, one
entry per PR.

---

## PR #50 — Add in-memory storage backend (closes #7)
**2026-08-14** · [#50](https://github.com/baileyrd/rusty_-rusqlite/pull/50)

- **Added:** `Database`/`Table` — in-memory table storage: `create_table`
  (from a parsed `CreateTable`), `insert_row`, `table` (schema + row scan).
  Deliberately in-memory-only per `ARCHITECTURE.md`'s non-goals.
- **Added:** `Error::TableAlreadyExists`/`TableNotFound`/
  `ColumnCountMismatch` — extends the crate's error type rather than
  introducing a second one for the storage layer. `Error` now derives
  `PartialEq` so tests can assert on it directly.
- 5 new unit tests; all passing. `cargo clippy -- -D warnings` and
  `cargo fmt --check` clean.

---

## PR #49 — Add single-table SELECT parser (closes #6)
**2026-08-14** · [#49](https://github.com/baileyrd/rusty_-rusqlite/pull/49)

- **Added:** `parse_select`/`Select`/`SelectColumns`/`Expr`/`BinaryOp` —
  parses `SELECT * | cols FROM table [WHERE <comparison>]` for a single
  table, no joins/aggregates/subqueries. `WHERE` parses into an `Expr`
  tree but isn't evaluated yet (that's `A6`).
- 5 new unit tests; all passing. `cargo clippy -- -D warnings` and
  `cargo fmt --check` clean.

---

## PR #48 — Add INSERT parser (closes #5)
**2026-08-14** · [#48](https://github.com/baileyrd/rusty_-rusqlite/pull/48)

- **Added:** `parse_insert`/`Insert` — parses `INSERT INTO ... [(cols)]
  VALUES (...)` with literal values (no expressions yet — that's `A6`).
  Split from the original combined `INSERT`/`SELECT` gap row per the
  skill's issue-sizing rule.
- 5 new unit tests; all passing. `cargo clippy -- -D warnings` and
  `cargo fmt --check` clean.

---

## PR #47 — Add CREATE TABLE parser (closes #4)
**2026-08-14** · [#47](https://github.com/baileyrd/rusty_-rusqlite/pull/47)

- **Added:** `parse_create_table`/`CreateTable`/`ColumnDef`/`ParseError` —
  parses `CREATE TABLE` with a column list, declared type names, and
  `PRIMARY KEY`/`NOT NULL` constraints. No other statement types yet
  (`INSERT`/`SELECT` are A4a/A4b).
- 5 new unit tests; all passing. `cargo clippy -- -D warnings` and
  `cargo fmt --check` clean.

---

## PR #46 — Add SQL tokenizer (closes #3)
**2026-08-14** · [#46](https://github.com/baileyrd/rusty_-rusqlite/pull/46)

- **Added:** `tokenize`/`Token`/`TokenError` — lexes identifiers, keywords,
  integer/real/string/blob literals, and punctuation/operators. No
  statement-level grammar yet (that's the parser, A3/A4).
- 6 new unit tests; all passing. `cargo clippy -- -D warnings` and
  `cargo fmt --check` clean.

---

## PR #45 — Add Value/Type model (closes #2)
**2026-08-14** · [#45](https://github.com/baileyrd/rusty_-rusqlite/pull/45)

- **Added:** `Value`/`Type` — the SQLite five-storage-class model
  (Null/Integer/Real/Text/Blob), foundation for the tokenizer, parser, and
  storage layer that build on it.
- 2 new unit tests; both passing. `cargo clippy -- -D warnings` and
  `cargo fmt --check` clean.

---

## Initial commit — repo-config governance files + crate skeleton
**2026-08-14** · (pushed directly — no default branch existed yet to PR against)

- **Added:** standard governance file set (README, CONTRIBUTING,
  CODE_OF_CONDUCT, SECURITY, CHANGELOG, RELEASE_NOTES, ARCHITECTURE, ADR
  seed, PR/issue templates, CI workflow) via the repo-config process.
- **Added:** minimal crate skeleton (`Cargo.toml`, `Connection::open_in_memory`/
  `close`) — no SQL parsing or execution yet. This is the foundation the
  parity-loop gap-analysis and issue backlog build on top of.
- **Known limitation:** the repo-config skill's own `.github/` template
  payload (PR templates, issue templates, CI workflow) was missing from its
  asset bundle at the time this was applied; those files were hand-written
  here to match the skill's documented conventions instead.
- 2 unit tests; both passing. `cargo clippy -- -D warnings` and
  `cargo fmt --check` clean.
