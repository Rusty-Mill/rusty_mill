# ADR-0112: The Review's Remaining Mediums and Lows, Fixed

- Status: **Proposed and implemented on one branch; the owner chose
  it** (2026-09-23, "Both" — Mediums 9–14 and Lows 15–23 of the
  review). No wire change.
- Date: 2026-09-23
- Deciders: baileyrd
- Related: `docs/reports/2026-09-23-hardening-line-review.md`,
  `ADR-0110` (the Highs), `ADR-0111` (Mediums 4–8), `ADR-0105` (the
  automatic reclaim), `ADR-0109` (the history metric), `ADR-0092`
  (directory syncs), `ADR-0107` (journaled updates), `ADR-0098`
  (the CI feature sets), `ADR-0103` (`Busy`).
- Supersedes/Superseded by: closes every open item of the review
  that is fixable in the crate. Additive: `MvccIndex::entries`,
  `MvccState::with_index_quiet`, `escape_label_value`, owned
  in-flight guards, `switch_env_with_default`.

## Decision

Implement, one item each:

- `RVL-FR-001` (M9) — `MvccIndex` keeps a running entry count;
  `history_len` is O(1), so a `Metrics` scrape no longer walks every
  chain under the index lock.
- `RVL-FR-002` (M10) — `MvccState::with_index_quiet` runs the index
  closure with the reclaim trigger held off and the append count
  reset after; the three adapters use it for baseline seeding and
  for the insert-log fold at open, so replayed and seeded entries
  neither fire nor count toward an automatic reclaim.
- `RVL-FR-003` (M11) — `sync_parent_dir` after the journal's creation,
  after the insert log's creation, and after `handle_backup`'s rename
  (the last before `BackedUp` is answered).
- `RVL-FR-004` (M12) — journal replay skips a `Transaction` op whose
  record is gone (`RecordNotFound`): the delete's tombstone was folded
  from the insert log before the replay, so the op is moot; any other
  apply error still aborts the open. Proven by journaling an update,
  deleting the record, and reopening.
- `RVL-FR-005` (L15, L16) — `SERVER_JOURNAL_UPDATES` is a `0`/`1`
  switch (default off); `bounded_env`/`switch_env` treat an
  exported-but-empty variable as unset and name an overflow as such.
- `RVL-FR-006` (M13, L17) — `TooLarge`'s and `with_max_query_rows`'s
  docs say a limitless `Query` is clamped; `with_max_query_rows(0)`
  is unset, as the binary's `0` is; the guard's precedence over
  validation is documented.
- `RVL-FR-007` (L18) — Prometheus label values are escaped.
- `RVL-FR-008` (L19) — connection and refusal threads are spawned
  through `thread::Builder`; a failed spawn releases its slot through
  an owned guard and drops the socket instead of panicking the accept
  loop; the connection case is counted as refused.
- `RVL-FR-009` (L20) — the Python client closes its socket on any
  `connect` failure; the stale "protocol 27" comment reads 30.
- `RVL-FR-010` (L21) — test names and messages say what they assert
  (`…is_told_busy_closed_and_counted`, "closed after its one frame",
  "only the entries below it are reclaimed", "the first append past
  the threshold after activation fires"); `Entity` and `Relation`
  gain the in-place MVCC record test.
- `RVL-FR-011` (L22, M14) — `SERVER-002` §6.3 names its third case;
  `ADR-0072`, `ADR-0092`, `ADR-0097`, `ADR-0105` carry a correcting
  note; `Cargo.toml`'s `rust-version` comment names the root job only.
- `RVL-FR-012` (L23) — the root feature-set job lints
  `server,research` too, the crate's own documented set.

## Consequences

- Positive: every item of the review that lives in this crate or its
  CI is closed; the automatic reclaim counts live writes only; a
  journaled update can no longer poison the next open.
- Negative / tradeoffs: the root-workflow edit runs the whole
  workspace's CI once (Windows shards included); an owned in-flight
  guard is one `Arc` clone per accept.
- Named, not hidden: L21's "allow-insecure warning text" and
  data-dir-lock ordering tests, L23's other untrimmed Windows tests,
  the `set_var` race in another crate's harness, the "TEMP" commit in
  history, and M14's check-name-embeds-the-pin are recorded, not
  changed — the first two would need a stderr-streaming harness, the
  rest live outside this crate or in history.

## Acceptance and implementation

- 2026-09-23: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.91.0 / `FR-104`.
  `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — 980 tests across 44 targets, 0 failed. Builder: Claude.
