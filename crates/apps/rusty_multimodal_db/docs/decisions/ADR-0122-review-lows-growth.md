# ADR-0122: The Growth-Line Review's Lows, Fixed or Recorded

- Status: **Proposed and implemented on one branch; the owner chose
  it** (2026-09-23, "proceed" — the report's recommended order, the
  Lows last). No wire change.
- Date: 2026-09-23
- Deciders: baileyrd
- Related: `docs/reports/2026-09-23-growth-line-review.md` items
  16–26; `ADR-0120`, `ADR-0121` (the Highs and Mediums before this).
- Supersedes/Superseded by: none. Additive: `STALE_STAGING` and the
  staging cleanup in `prune`.

## Decision

- `RGL-FR-001` (16) — `MvccState::open` resets the append count after
  folding the persisted chains, so a clean reopen of a large index no
  longer fires an automatic reclaim on its first live write.
- `RGL-FR-002` (17) — `with_index`/`with_index_quiet` recover a
  poisoned index lock instead of re-panicking, so a scrape or a fold
  survives a panic in an earlier closure.
- `RGL-FR-003` (20) — `prune` also removes `.refresh-tmp-*`
  directories older than an hour (a refresh that crashed mid-way);
  `.failed-*` directories are kept for the operator, said in the docs,
  as is the effect of a clock stepped backwards on ordering;
  `crash_writer reopen-check` without a path prints usage and exits 1;
  both trial scripts refuse when the writer dies early (`ADR-0120`).
- `RGL-FR-004` (26) — the old-layout refusal test compares the slot
  file byte for byte; the allow-insecure metrics-bind test asserts the
  warning names the metrics listener. Not added: a `waiting_writers >
  0` assertion (needs a stalled `fsync` to stage), a `.failed-`
  rename test and an `--every` test (the CLI loop is `main`'s), and a
  fix for the `free_port()` race (every server test in the crate
  shares it; a bind-and-hand-over would be its own round).
- `RGL-FR-005` (18) — `SERVER_METRICS_HTTP_ADDR=""` is unset.
- `RGL-FR-006` (19) — `escape_label_value` has its doc back;
  `serve_tables` debug-asserts unique table names; `ADR-0115` says
  which `len_bytes` is test-only.
- Recorded, with a note in place (21–25): `RESULTS.md` says what the
  crash-prefix copy can and cannot show; `ADR-0117` says a read-only
  or unknown field answers its own code before `Malformed`;
  `SERVER-002` §4's example is the fixture's own `Hello { 2 }` and rule
  3's prose names both content rewrites; `protocol.rs`'s pin-test doc
  starts at 31; `ADR-0113`'s ordering premise carries the exception;
  `SERVER-REPLICATION-DESIGN`, `ADR-0056`, the crash harness,
  `FUTURE-GROWTH` and `README` carry their back-references.
- Not changed: `mvcc_begin` still discards its baseline flush result
  (the trait returns the snapshot id and nothing else; a failed
  baseline flush re-seeds at the next open, so nothing acknowledged is
  lost — the "ever went active" signal is what a later flush restores).

## Consequences

- Positive: the review's list is closed; every remaining caveat is in
  the document it belongs to.
- Negative / tradeoffs: a recovered poisoned index is used as it is;
  the per-key invariants hold, a half-applied batch's other keys do
  not, which is the same exposure a crash mid-batch has today.
- Named, not hidden: the three tests not added, above.

## Acceptance and implementation

- 2026-09-23: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.100.0 / `FR-113`, in one PR with `ADR-0120` and `ADR-0121`.
  `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — 994 tests across 46 targets, 0 failed; Python 7 tests OK. Builder: Claude.
