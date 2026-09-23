# ADR-0105: MVCC History Reclaims Itself Every N Appended Entries

- Status: **Proposed and implemented on one branch; the owner chose
  it** (2026-09-22, "2" — the fork `ADR-0096` held open). No wire
  change.
- Date: 2026-09-22
- Deciders: baileyrd
- Related: `ADR-0096`/`docs/design/SERVER-MVCC-HISTORY-RECLAIM-DESIGN.md`
  (`HRC-FR-002`, `Compact`-only reclaim; option (b) "an automatic
  trigger"), `ADR-0072` (MVCC; the open-snapshot boundary),
  `ADR-0099` (`memory_server` defaults on, `0` off).
- Supersedes/Superseded by: amends `HRC-FR-002`'s "at `Compact`" to
  "at `Compact` and every N appends". Additive:
  `MvccIndex::appended_since_gc`, `MvccState::{set_reclaim_every,
  reclaim_every, auto_reclaims}`, the check in `with_index`,
  `with_mvcc_reclaim_every` on the three adapters,
  `SERVER_MVCC_RECLAIM_EVERY` (default 10,000).

## Context

`ADR-0096` reclaimed the history no open snapshot needs, but only
when a client sent `Compact`. A deployment that never compacts — the
consumer does not — still held every version ever written between
restarts, which was the review's original finding restated.

## Decision

Implement option (b): the index keeps a running count of entries
appended since its last `gc`; `MvccState` holds a threshold
(`0` never); `with_index` — the one gate every write and read of the
index passes through, under its lock — runs `gc` at the oldest open
snapshot once the count reaches the threshold and MVCC is active, and
counts the run. The three adapters expose
`with_mvcc_reclaim_every(Option<usize>)`, applied to a state held now
or activated later; `memory_server` reads
`SERVER_MVCC_RECLAIM_EVERY`, default 10,000, `0` leaving reclaim to
`Compact`, and says so in its banner. Proven at a threshold of two
through the adapter: with a snapshot held, the trigger fires and the
snapshot still reads its value (entries after it are kept, as
`Compact` keeps them); once released, the next threshold brings the
history back to a bounded size and the current value reads.

## Consequences

- Positive: the version index is bounded without an operator's
  `Compact`; the cost is one integer compare per index access and a
  `gc` walk once per N appends, inside a lock the write already held.
- Negative / tradeoffs: a policy with a number in it — 10,000 is a
  default, not a measurement; a very long-held snapshot still pins
  everything after it (by design, the same as `Compact`). The reclaim
  runs inside the committing session, whose own snapshot is registered,
  so the just-written chain keeps one prior entry until the next
  trigger — a bound of one entry, not a leak.
- Named, not hidden: the count is not yet a metric
  (`dogserver_mvcc_history_entries`, the design's other open question; *built by `ADR-0109`*)
  — the adapters do not hold `ServerMetrics`.

## Acceptance and implementation

- 2026-09-22: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.86.0 / `FR-098`.
  `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — 969 tests across 44 targets, 0 failed. Builder: Claude; independent Codex inspection owed.
