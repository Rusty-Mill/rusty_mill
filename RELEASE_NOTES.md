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

## PR #55 — Add ToSql trait (closes #36)
**2026-08-14** · [#55](https://github.com/baileyrd/rusty_-rusqlite/pull/55)

- **Added:** `ToSql` trait + blanket impls for `Value`/`i64`/`i32`/`f64`/
  `bool`/`String`/`str`/`Vec<u8>`/`Option<T>`.
- **Design deviation, stated plainly:** unlike `rusqlite::ToSql`,
  `to_sql` here isn't fallible — none of these impls have a failure case,
  so wrapping the return in `Result` would be error handling for a
  scenario that can't happen. A future impl that genuinely can fail can
  introduce a fallible variant then, without forcing today's impls to
  pretend they can fail.
- 3 new unit tests (51 total); all passing. `cargo clippy -- -D warnings`
  and `cargo fmt --check` clean.

---

## PR #54 — Add ValueRef (closes #35)
**2026-08-14** · [#54](https://github.com/baileyrd/rusty_-rusqlite/pull/54)

- **Added:** `ValueRef`/`Value::as_ref`/`ValueRef::to_owned` — a
  borrowed, non-owning view over `Value` that avoids cloning `Text`/`Blob`
  payloads. `Value`/`Type` already existed from `A1`; this issue's `Value`/
  `Type` portion was effectively already satisfied, so this PR is scoped
  to just the missing `ValueRef` piece.
- 2 new unit tests (48 total); all passing. `cargo clippy -- -D warnings`
  and `cargo fmt --check` clean.

---

## PR #53 — Wire Connection to the execution engine (closes #10)
**2026-08-14** · [#53](https://github.com/baileyrd/rusty_-rusqlite/pull/53)

- **Changed:** `Connection::execute` and `Connection::query_row` are now
  real — they tokenize, parse, and dispatch to the engine (`CREATE
  TABLE`/`INSERT` for `execute`, `SELECT` for `query_row`) instead of
  being stubs. This closes out foundation-tier Part A entirely (`A1`–`A8`
  from `gap-analysis.md`): a full `CREATE TABLE` → `INSERT` → `SELECT`
  round trip now works through the public `Connection` API.
- **Added:** `Error::Token`/`Parse`/`UnrecognizedStatement`/
  `QueryReturnedNoRows`, plus `From<TokenError>`/`From<ParseError>` for
  `Error` so the tokenizer/parser's own error types compose with `?`
  instead of needing per-call-site `map_err`.
- **Known limitation:** only `CREATE TABLE`/`INSERT`/`SELECT` are wired
  up; other statement types return `UnrecognizedStatement`. The
  `rusqlite`-shaped `Statement`/`Row` API (multi-row iteration, prepared
  statements, parameter binding) is Part B scope — tracked in the
  existing open `parity-gap` issues (#11 onward), not part of this PR.
- 4 new unit tests (46 total); all passing. `cargo clippy -- -D warnings`
  and `cargo fmt --check` clean.

---

## PR #52 — Add execution engine (closes #9)
**2026-08-14** · [#52](https://github.com/baileyrd/rusty_-rusqlite/pull/52)

- **Added:** `execute_create_table`/`execute_insert`/`execute_select` —
  ties the parser ASTs, storage layer, and expression evaluator together
  into a real query path: single-table scan + `WHERE` filter + column
  projection. `INSERT`'s explicit column list (reordered or partial) is
  now expanded into full table-definition order, missing columns filled
  with `NULL`.
- This is the last of foundation-tier Part A except `A8` (wiring
  `Connection`/`Statement` to this engine) — `CREATE TABLE`/`INSERT`/
  `SELECT` now work end-to-end against the in-memory backend via direct
  function calls; only the public `Connection` API surface is still
  stubbed.
- 5 new unit tests (43 total); all passing. `cargo clippy -- -D warnings`
  and `cargo fmt --check` clean.

---

## PR #51 — Add expression evaluator (closes #8)
**2026-08-14** · [#51](https://github.com/baileyrd/rusty_-rusqlite/pull/51)

- **Added:** `evaluate`/`evaluate_bool` — evaluates the `SELECT` parser's
  `Expr` tree (literals, column refs, the six comparison operators)
  against a single row, following SQLite's storage-class ordering for
  cross-type comparisons and truthiness rules for boolean filtering.
- **Added:** `Error::UnknownColumn`.
- 6 new unit tests; all passing. `cargo clippy -- -D warnings` and
  `cargo fmt --check` clean.

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
